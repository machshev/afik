//! Heap-free device-side configuration protocol service.
//!
//! This is the one implementation of the device half of the AFIK serial
//! protocol. The deterministic simulator and the target firmware both drive it,
//! so a radio and its simulation cannot disagree about transactions, listings,
//! replay, or error codes.
//!
//! The service owns the stream decoder, the bounded transactional store, the
//! open transaction identifier, and the single-exchange replay cache. It is
//! allocation-free and reports observable steps through a caller-supplied
//! observer so a host can trace them without the service knowing about time.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

use radio_protocol::{
    decode_list_objects_request, encode_frame, Command, ControlRequest, DeviceCapabilities,
    DeviceErrorCode, Frame, ObjectDescriptor, ObjectListPage, PayloadReader, PayloadWriter,
    ProtocolError, ReceiveMetricsReport, ReceiveStateReport, Service, StreamDecoder, FLAG_ERROR,
    FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_LIST_OBJECTS_PER_PAGE, MAX_PAYLOAD, PROTOCOL_VERSION,
};
use radio_storage::{
    decode_configuration_image, validate_object, ConfigurationImageWriter, Object, ObjectArenaIter,
    ObjectKey, ObjectKind, StorageError, StorageObject, StorageUsage, TransactionalStore,
    CONFIGURATION_IMAGE_HEADER_LEN, MAX_OBJECT_DATA, MIN_OBJECT_ENCODED_LEN,
    OBJECT_ENTRY_HEADER_LEN, STORAGE_FORMAT_VERSION,
};

const EMPTY_DESCRIPTOR: ObjectDescriptor = ObjectDescriptor {
    kind: 0,
    id: 0,
    encoded_len: 0,
};

/// One observable device-side protocol or storage step.
///
/// Events carry no timestamp: the observer decides whether and how to record
/// time, so the service stays independent of any clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceEvent {
    /// A valid request frame arrived.
    Request {
        /// Request sequence number.
        sequence: u16,
        /// Selected service.
        service: Service,
        /// Selected command.
        command: Command,
    },
    /// A malformed stream packet was discarded at a delimiter.
    PacketDiscarded(ProtocolError),
    /// An identical immediately repeated request reused its cached response.
    DuplicateRequestReplayed {
        /// Repeated request sequence number.
        sequence: u16,
    },
    /// A sequence was immediately reused for different request bytes.
    SequenceConflictRejected {
        /// Conflicting request sequence number.
        sequence: u16,
    },
    /// A candidate transaction began from the active generation.
    TransactionBegan {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// Active generation copied into the candidate.
        generation: u32,
    },
    /// One object was staged in the candidate snapshot.
    ObjectStaged {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// Stable object key.
        key: ObjectKey,
    },
    /// The complete candidate passed validation.
    TransactionValidated {
        /// Host-selected transaction identifier.
        transaction: u32,
    },
    /// A candidate transaction was explicitly discarded.
    TransactionAborted {
        /// Host-selected transaction identifier.
        transaction: u32,
    },
    /// A candidate became the active snapshot.
    TransactionCommitted {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// New active generation.
        generation: u32,
    },
    /// An active configuration object was read.
    ObjectRead(ObjectKey),
    /// One deterministic page of active object descriptors was listed.
    ObjectsListed {
        /// Active storage generation described by the page.
        generation: u32,
        /// Zero-based object offset requested by the host.
        offset: u16,
        /// Number of descriptors returned in the page.
        count: u16,
    },
    /// A response frame was encoded for the host.
    Response {
        /// Response sequence number.
        sequence: u16,
        /// Response command, including the error command.
        command: Command,
    },
}

/// Byte-bounded device-side configuration service over one byte stream.
///
/// `BYTES` is the packed configuration store this device has, and it is the
/// only capacity there is. What the device advertises is derived from it, so a
/// device cannot claim room it does not have or refuse a project which fits.
pub struct DeviceService<const BYTES: usize> {
    decoder: StreamDecoder,
    store: TransactionalStore<BYTES>,
    active_transaction: Option<u32>,
    last_exchange: Option<(Frame, Frame)>,
    capabilities: DeviceCapabilities,
}

impl<const BYTES: usize> Default for DeviceService<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BYTES: usize> DeviceService<BYTES> {
    /// Constructs an empty, generation-zero service with derived capabilities.
    ///
    /// Every reported bound comes from the compiled types rather than a
    /// separately maintained constant, so a device cannot advertise a capacity
    /// it does not have.
    pub fn new() -> Self {
        Self::with_plan_encodings(0)
    }

    /// Constructs a service which additionally advertises plan encodings.
    pub fn with_plan_encodings(plan_encodings: u16) -> Self {
        Self {
            decoder: StreamDecoder::new(),
            store: TransactionalStore::new(),
            active_transaction: None,
            last_exchange: None,
            capabilities: DeviceCapabilities {
                protocol_version: PROTOCOL_VERSION,
                storage_version: STORAGE_FORMAT_VERSION,
                max_frame_payload: u16::try_from(MAX_PAYLOAD).unwrap_or(u16::MAX),
                // An upper bound derived from the bytes, not a second limit: no
                // object encodes shorter than this, so nothing which fits the
                // store can exceed the count reported here.
                max_objects: u16::try_from(
                    BYTES / (OBJECT_ENTRY_HEADER_LEN + MIN_OBJECT_ENCODED_LEN),
                )
                .unwrap_or(u16::MAX),
                max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
                plan_encodings,
                configuration_bytes: u32::try_from(BYTES).unwrap_or(u32::MAX),
            },
        }
    }

    /// Returns the bytes one complete retained image of this store occupies.
    #[must_use]
    pub const fn image_bytes() -> usize {
        CONFIGURATION_IMAGE_HEADER_LEN + BYTES
    }

    /// Returns the advertised capability profile.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }

    /// Returns the active snapshot generation.
    pub const fn generation(&self) -> u32 {
        self.store.generation()
    }

    /// Returns the identifier of the open transaction, if any.
    pub const fn open_transaction(&self) -> Option<u32> {
        self.active_transaction
    }

    /// Reports active-snapshot usage.
    pub fn usage(&self) -> StorageUsage {
        self.store.usage()
    }

    /// Iterates over active objects, in canonical order, without exposing
    /// candidate data.
    pub fn active_objects(&self) -> ObjectArenaIter<'_> {
        self.store.active_objects()
    }

    /// Returns the active snapshot as a canonical image payload.
    #[must_use]
    pub fn active_payload(&self) -> &[u8] {
        self.store.active_payload()
    }

    /// Replaces the active snapshot without a protocol transaction.
    ///
    /// This exists for a device which restores a retained configuration at
    /// start-up. Every object is validated and staged through the ordinary
    /// transactional path, so a rejected restore leaves the store empty rather
    /// than partly filled, and no candidate is left open.
    pub fn load<O: Object, I: IntoIterator<Item = O>>(
        &mut self,
        objects: I,
    ) -> Result<u32, StorageError> {
        if self.active_transaction.is_some() {
            return Err(StorageError::TransactionAlreadyOpen);
        }
        self.store.begin()?;
        for object in objects {
            if let Err(error) = self.store.write(&object) {
                let _ = self.store.abort();
                return Err(error);
            }
        }
        if let Err(error) = self.validate_candidate() {
            let _ = self.store.abort();
            return Err(error);
        }
        self.store.commit()
    }

    /// Validates the open candidate against the object formats.
    ///
    /// There is no count to check. A candidate which does not fit was refused
    /// by the byte bound as it was written, so what reaches validation is a
    /// configuration this device has the room to run.
    fn validate_candidate(&mut self) -> Result<(), StorageError> {
        self.store.validate(|object| validate_object(&object))
    }

    /// Adds or replaces one object through a complete validated transaction.
    ///
    /// A device which changes its own settings — a squelch level chosen on the
    /// handset — needs the same isolation the host gets, so a rejected result
    /// leaves the radio exactly as it was rather than half reconfigured.
    pub fn store_object(&mut self, object: &impl Object) -> Result<u32, StorageError> {
        if self.active_transaction.is_some() {
            return Err(StorageError::TransactionAlreadyOpen);
        }
        self.store.begin()?;
        if let Err(error) = self.store.write(&object) {
            let _ = self.store.abort();
            return Err(error);
        }
        if let Err(error) = self.validate_candidate() {
            let _ = self.store.abort();
            return Err(error);
        }
        self.store.commit()
    }

    /// Restores one complete canonical configuration image.
    ///
    /// The image is fully validated before any object is staged, so retained
    /// bytes which are absent, erased, truncated, or corrupt leave the active
    /// snapshot untouched.
    pub fn load_image(&mut self, bytes: &[u8]) -> Result<u32, StorageError> {
        self.load(decode_configuration_image(bytes)?.objects())
    }

    /// Encodes the active snapshot as a canonical configuration image.
    ///
    /// The store holds its objects packed in strict `(kind, id)` order, which
    /// is the order an image requires, so this needs no sort, no key index, and
    /// no second object-sized buffer.
    pub fn encode_active_image(&self, output: &mut [u8]) -> Result<usize, StorageError> {
        let count = self.store.active_objects().count();
        let mut writer = ConfigurationImageWriter::new(
            output,
            u16::try_from(count).map_err(|_| StorageError::ImageTooLarge)?,
        )?;
        for object in self.store.active_objects() {
            writer.push(&object)?;
        }
        writer.finish()
    }

    /// Consumes one stream byte and encodes at most one response frame.
    ///
    /// Returns the encoded response length written to `response`, or `None`
    /// when the byte did not complete an answerable frame. A response buffer
    /// shorter than [`MAX_ENCODED_FRAME`] can truncate an otherwise valid
    /// answer, so callers provide at least that much space.
    pub fn push<F: FnMut(DeviceEvent)>(
        &mut self,
        byte: u8,
        response: &mut [u8],
        observer: &mut F,
    ) -> Option<usize> {
        let result = self.decoder.push(byte)?;
        let request = match result {
            Ok(request) => request,
            Err(error) => {
                observer(DeviceEvent::PacketDiscarded(error));
                return None;
            }
        };
        observer(DeviceEvent::Request {
            sequence: request.sequence(),
            service: request.service(),
            command: request.command(),
        });
        let frame = match self.handle_exchange(&request, observer) {
            Ok(frame) => frame,
            Err(error) => {
                observer(DeviceEvent::PacketDiscarded(error));
                return None;
            }
        };
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let length = encode_frame(&frame, &mut encoded).ok()?;
        let destination = response.get_mut(..length)?;
        destination.copy_from_slice(&encoded[..length]);
        observer(DeviceEvent::Response {
            sequence: frame.sequence(),
            command: frame.command(),
        });
        Some(length)
    }

    /// Pushes one byte, surfacing a runtime-control request instead of
    /// refusing it.
    ///
    /// This service owns configuration, not the receiver, so it cannot answer a
    /// runtime-control request itself. A caller which holds the receive
    /// controller drives the exchange in two steps: this call decodes and
    /// returns the request, the caller performs it, and
    /// [`DeviceService::answer_control`] encodes the reply. Everything else
    /// behaves exactly as [`DeviceService::push`], including replay.
    ///
    /// A caller which does not own a receiver should keep using
    /// [`DeviceService::push`], which reports runtime control as an unsupported
    /// service.
    pub fn push_control<F: FnMut(DeviceEvent)>(
        &mut self,
        byte: u8,
        response: &mut [u8],
        observer: &mut F,
    ) -> Push {
        let Some(result) = self.decoder.push(byte) else {
            return Push::Idle;
        };
        let request = match result {
            Ok(request) => request,
            Err(error) => {
                observer(DeviceEvent::PacketDiscarded(error));
                return Push::Idle;
            }
        };
        observer(DeviceEvent::Request {
            sequence: request.sequence(),
            service: request.service(),
            command: request.command(),
        });

        // Replay is checked before the request is classified, so a resent
        // runtime-control frame replays its cached answer rather than
        // performing the operation a second time. Starting a scan twice is not
        // the same as starting it once.
        let settled = match self.replayed(&request, observer) {
            Some(frame) => Some(frame),
            None if request.flags() == 0 && request.service() == Service::RuntimeControl => {
                match ControlRequest::decode(request.command(), request.payload()) {
                    Ok(control) => {
                        return Push::Control(PendingControl { request, control });
                    }
                    // The command exists in the protocol but not in this
                    // service, or its payload is not the shape the command
                    // requires. Neither is something the receiver should be
                    // asked about.
                    Err(ProtocolError::UnknownCommand) => Some(self.settle(
                        &request,
                        error_response(&request, DeviceErrorCode::UnsupportedCommand),
                    )),
                    Err(_) => Some(self.settle(
                        &request,
                        error_response(&request, DeviceErrorCode::MalformedPayload),
                    )),
                }
            }
            None => None,
        };

        let frame = match settled {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => {
                observer(DeviceEvent::PacketDiscarded(error));
                return Push::Idle;
            }
            None => match self.handle_request(&request, observer) {
                Ok(frame) => {
                    self.last_exchange = Some((request, frame));
                    frame
                }
                Err(error) => {
                    observer(DeviceEvent::PacketDiscarded(error));
                    return Push::Idle;
                }
            },
        };

        match write_response(&frame, response, observer) {
            Some(length) => Push::Response(length),
            None => Push::Idle,
        }
    }

    /// Encodes the reply to a request returned by
    /// [`DeviceService::push_control`].
    ///
    /// Returns the response length written into `response`, or `None` if the
    /// reply could not be encoded.
    pub fn answer_control<F: FnMut(DeviceEvent)>(
        &mut self,
        pending: PendingControl,
        answer: ControlAnswer,
        response: &mut [u8],
        observer: &mut F,
    ) -> Option<usize> {
        let request = pending.request;
        let mut payload = [0_u8; MAX_PAYLOAD];
        let built = match answer {
            ControlAnswer::State(report) => report
                .encode(&mut payload)
                .and_then(|length| success_response(&request, &payload[..length])),
            ControlAnswer::Metrics(report) => report
                .encode(&mut payload)
                .and_then(|length| success_response(&request, &payload[..length])),
            ControlAnswer::Refused(code) => error_response(&request, code),
        };
        let frame = match self.settle(&request, built) {
            Ok(frame) => frame,
            Err(error) => {
                observer(DeviceEvent::PacketDiscarded(error));
                return None;
            }
        };
        write_response(&frame, response, observer)
    }

    /// Records one settled exchange so an identical resend replays it.
    fn settle(
        &mut self,
        request: &Frame,
        built: Result<Frame, ProtocolError>,
    ) -> Result<Frame, ProtocolError> {
        let frame = built?;
        self.last_exchange = Some((*request, frame));
        Ok(frame)
    }

    /// Answers a resent or conflicting sequence without performing the request.
    fn replayed<F: FnMut(DeviceEvent)>(
        &self,
        request: &Frame,
        observer: &mut F,
    ) -> Option<Result<Frame, ProtocolError>> {
        let (previous_request, previous_response) = self.last_exchange?;
        if previous_request.sequence() != request.sequence() {
            return None;
        }
        if previous_request == *request {
            observer(DeviceEvent::DuplicateRequestReplayed {
                sequence: request.sequence(),
            });
            return Some(Ok(previous_response));
        }
        observer(DeviceEvent::SequenceConflictRejected {
            sequence: request.sequence(),
        });
        Some(error_response(request, DeviceErrorCode::SequenceConflict))
    }

    fn handle_exchange<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        if let Some(replayed) = self.replayed(request, observer) {
            return replayed;
        }
        let response = self.handle_request(request, observer)?;
        self.last_exchange = Some((*request, response));
        Ok(response)
    }

    fn handle_request<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        if request.flags() != 0 {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        }
        match request.service() {
            Service::DeviceInfo => self.handle_device_info(request),
            Service::Configuration => self.handle_configuration(request, observer),
            Service::RuntimeControl | Service::FirmwareUpdate | Service::Diagnostics => {
                error_response(request, DeviceErrorCode::UnsupportedService)
            }
        }
    }

    fn handle_device_info(&self, request: &Frame) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::Hello => {
                if request.payload() == [PROTOCOL_VERSION] {
                    success_response(request, &[PROTOCOL_VERSION])
                } else {
                    error_response(request, DeviceErrorCode::MalformedPayload)
                }
            }
            Command::GetCapabilities => {
                if !request.payload().is_empty() {
                    return error_response(request, DeviceErrorCode::MalformedPayload);
                }
                let mut payload = [0_u8; DeviceCapabilities::ENCODED_LEN];
                let length = self.capabilities.encode(&mut payload)?;
                success_response(request, &payload[..length])
            }
            _ => error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn handle_configuration<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::ListObjects => self.list_objects(request, observer),
            Command::ReadObject => self.read_object(request, observer),
            Command::BeginTransaction => self.begin_transaction(request, observer),
            Command::WriteObject => self.write_object(request, observer),
            Command::ValidateTransaction => self.validate_transaction(request, observer),
            Command::CommitTransaction => self.commit_transaction(request, observer),
            Command::AbortTransaction => self.abort_transaction(request, observer),
            _ => error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn list_objects<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let Ok(offset) = decode_list_objects_request(request.payload()) else {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let total_objects = u16::try_from(self.store.active_objects().count())
            .map_err(|_| ProtocolError::PayloadTooLarge)?;
        if offset > total_objects {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        }
        // The store is already in the canonical order a listing must report,
        // so one page is a skip and a take rather than a sorted copy of every
        // descriptor the device holds.
        let mut descriptors = [EMPTY_DESCRIPTOR; MAX_LIST_OBJECTS_PER_PAGE];
        let mut count = 0_usize;
        for object in self
            .store
            .active_objects()
            .skip(usize::from(offset))
            .take(MAX_LIST_OBJECTS_PER_PAGE)
        {
            descriptors[count] = ObjectDescriptor {
                kind: object.key().kind as u8,
                id: object.key().id,
                encoded_len: u16::try_from(object.len())
                    .map_err(|_| ProtocolError::PayloadTooLarge)?,
            };
            count += 1;
        }
        let page = &descriptors[..count];
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = ObjectListPage::encode(
            self.store.generation(),
            total_objects,
            offset,
            page,
            &mut payload,
        )?;
        observer(DeviceEvent::ObjectsListed {
            generation: self.store.generation(),
            offset,
            count: u16::try_from(page.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
        });
        success_response(request, &payload[..length])
    }

    fn read_object<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let Ok(key) = parse_object_key(request.payload()) else {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let object = match self.store.read(key) {
            Ok(object) => object,
            Err(StorageError::ObjectNotFound) => {
                return error_response(request, DeviceErrorCode::ObjectNotFound);
            }
            Err(error) => return error_response(request, map_storage_error(error)),
        };
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = {
            let mut writer = PayloadWriter::new(&mut payload);
            writer.write_u8(object.key().kind as u8)?;
            writer.write_u16(object.key().id)?;
            writer.write_u16(
                u16::try_from(object.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
            )?;
            writer.write_bytes(object.data())?;
            writer.len()
        };
        observer(DeviceEvent::ObjectRead(key));
        success_response(request, &payload[..length])
    }

    fn begin_transaction<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let Ok(transaction) = parse_transaction(request.payload()) else {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction.is_some() {
            return error_response(request, DeviceErrorCode::TransactionAlreadyOpen);
        }
        if let Err(error) = self.store.begin() {
            return error_response(request, map_storage_error(error));
        }
        self.active_transaction = Some(transaction);
        observer(DeviceEvent::TransactionBegan {
            transaction,
            generation: self.store.generation(),
        });
        success_response(request, &self.store.generation().to_le_bytes())
    }

    fn write_object<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let Ok((transaction, object)) = parse_write_object(request.payload()) else {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction != Some(transaction) {
            return error_response(request, DeviceErrorCode::NoTransaction);
        }
        if let Err(error) = self.store.write(&object) {
            return error_response(request, map_storage_error(error));
        }
        observer(DeviceEvent::ObjectStaged {
            transaction,
            key: object.key(),
        });
        success_response(request, &[])
    }

    fn validate_transaction<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let transaction = match self.require_transaction(request) {
            Ok(transaction) => transaction,
            Err(code) => return error_response(request, code),
        };
        if let Err(error) = self.validate_candidate() {
            return error_response(request, map_storage_error(error));
        }
        observer(DeviceEvent::TransactionValidated { transaction });
        success_response(request, &[])
    }

    fn commit_transaction<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let transaction = match self.require_transaction(request) {
            Ok(transaction) => transaction,
            Err(code) => return error_response(request, code),
        };
        let generation = match self.store.commit() {
            Ok(generation) => generation,
            Err(error) => return error_response(request, map_storage_error(error)),
        };
        self.active_transaction = None;
        observer(DeviceEvent::TransactionCommitted {
            transaction,
            generation,
        });
        success_response(request, &generation.to_le_bytes())
    }

    fn abort_transaction<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        let transaction = match self.require_transaction(request) {
            Ok(transaction) => transaction,
            Err(code) => return error_response(request, code),
        };
        if let Err(error) = self.store.abort() {
            return error_response(request, map_storage_error(error));
        }
        self.active_transaction = None;
        observer(DeviceEvent::TransactionAborted { transaction });
        success_response(request, &[])
    }

    fn require_transaction(&self, request: &Frame) -> Result<u32, DeviceErrorCode> {
        let transaction =
            parse_transaction(request.payload()).map_err(|_| DeviceErrorCode::MalformedPayload)?;
        if self.active_transaction != Some(transaction) {
            return Err(DeviceErrorCode::NoTransaction);
        }
        Ok(transaction)
    }
}

/// What one byte pushed through [`DeviceService::push_control`] produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Push {
    /// No complete frame yet, or one which was discarded.
    Idle,
    /// A complete response of this many bytes was written.
    Response(usize),
    /// A runtime-control request the caller must perform and answer.
    Control(PendingControl),
}

/// A decoded runtime-control request awaiting the caller's answer.
///
/// It carries the request frame so the answer can be addressed to the same
/// sequence and command, which is what makes a resend replay rather than
/// perform the operation again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingControl {
    request: Frame,
    control: ControlRequest,
}

impl PendingControl {
    /// Returns the operation the host asked for.
    pub const fn request(&self) -> ControlRequest {
        self.control
    }
}

/// The caller's answer to one runtime-control request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlAnswer {
    /// What the receiver is doing, after any requested change.
    State(ReceiveStateReport),
    /// One raw metrics sample.
    Metrics(ReceiveMetricsReport),
    /// The operation was refused, with the reason.
    Refused(DeviceErrorCode),
}

/// Encodes one response frame into the caller's buffer.
fn write_response<F: FnMut(DeviceEvent)>(
    frame: &Frame,
    response: &mut [u8],
    observer: &mut F,
) -> Option<usize> {
    let mut encoded = [0_u8; MAX_ENCODED_FRAME];
    let length = encode_frame(frame, &mut encoded).ok()?;
    let destination = response.get_mut(..length)?;
    destination.copy_from_slice(&encoded[..length]);
    observer(DeviceEvent::Response {
        sequence: frame.sequence(),
        command: frame.command(),
    });
    Some(length)
}

fn success_response(request: &Frame, payload: &[u8]) -> Result<Frame, ProtocolError> {
    Frame::new(
        request.service(),
        FLAG_RESPONSE,
        request.sequence(),
        request.command(),
        payload,
    )
}

fn error_response(request: &Frame, code: DeviceErrorCode) -> Result<Frame, ProtocolError> {
    Frame::new(
        request.service(),
        FLAG_RESPONSE | FLAG_ERROR,
        request.sequence(),
        Command::Error,
        &[request.command() as u8, code as u8],
    )
}

fn parse_object_key(payload: &[u8]) -> Result<ObjectKey, ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let kind =
        ObjectKind::try_from(reader.read_u8()?).map_err(|_| ProtocolError::MalformedPayload)?;
    let id = reader.read_u16()?;
    reader.finish()?;
    Ok(ObjectKey { kind, id })
}

fn parse_transaction(payload: &[u8]) -> Result<u32, ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let transaction = reader.read_u32()?;
    reader.finish()?;
    Ok(transaction)
}

fn parse_write_object(payload: &[u8]) -> Result<(u32, StorageObject), ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let transaction = reader.read_u32()?;
    let kind =
        ObjectKind::try_from(reader.read_u8()?).map_err(|_| ProtocolError::MalformedPayload)?;
    let id = reader.read_u16()?;
    let length = usize::from(reader.read_u16()?);
    let data = reader.read_bytes(length)?;
    reader.finish()?;
    let object = StorageObject::new(ObjectKey { kind, id }, data)
        .map_err(|_| ProtocolError::MalformedPayload)?;
    Ok((transaction, object))
}

/// Maps a storage failure onto its stable device error code.
#[must_use]
pub const fn map_storage_error(error: StorageError) -> DeviceErrorCode {
    match error {
        StorageError::ObjectTooLarge | StorageError::StoreFull => DeviceErrorCode::CapacityExceeded,
        StorageError::TransactionAlreadyOpen => DeviceErrorCode::TransactionAlreadyOpen,
        StorageError::NoTransaction => DeviceErrorCode::NoTransaction,
        StorageError::CandidateNotValidated => DeviceErrorCode::NotValidated,
        StorageError::ValidationFailed
        | StorageError::UnsupportedObject
        | StorageError::UnsupportedEncoding(_)
        | StorageError::MalformedObject => DeviceErrorCode::ValidationFailed,
        StorageError::ObjectNotFound => DeviceErrorCode::ObjectNotFound,
        StorageError::GenerationOverflow
        | StorageError::ImageBufferTooSmall
        | StorageError::ImageTooLarge
        | StorageError::MalformedImage
        | StorageError::UnsupportedImageVersion
        | StorageError::ImageIntegrity
        | StorageError::NonCanonicalImage => DeviceErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlAnswer, DeviceEvent, DeviceService, Push};
    use radio_protocol::{
        decode_packet, encode_frame, Command, ControlRequest, DeviceCapabilities, DeviceErrorCode,
        Frame, PayloadWriter, ReceiveMetricsReport, ReceiveMode, ReceiveStateReport, ScanActivity,
        Service, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME,
    };
    use radio_storage::{
        encode_channel, ObjectKey, ObjectKind, ObjectRef, StorageObject, CHANNEL_ENCODED_LEN,
        MAX_OBJECT_DATA, MIN_OBJECT_ENCODED_LEN, OBJECT_ENTRY_HEADER_LEN,
    };

    use radio_channel_plan::{
        BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel, Tone,
        TxClass,
    };

    /// A store with room for four channels and nothing else to say about it.
    const STORE_BYTES: usize = 4 * (OBJECT_ENTRY_HEADER_LEN + CHANNEL_ENCODED_LEN);

    /// A store with room for exactly one.
    const ONE_CHANNEL: usize = OBJECT_ENTRY_HEADER_LEN + CHANNEL_ENCODED_LEN;

    struct Harness<const BYTES: usize> {
        service: DeviceService<BYTES>,
        events: std::vec::Vec<DeviceEvent>,
    }

    impl<const BYTES: usize> Harness<BYTES> {
        fn new() -> Self {
            Self {
                service: DeviceService::new(),
                events: std::vec::Vec::new(),
            }
        }

        fn exchange(&mut self, request: &Frame) -> Frame {
            let mut encoded = [0_u8; MAX_ENCODED_FRAME];
            let length = encode_frame(request, &mut encoded).expect("encode request");
            let mut response = [0_u8; MAX_ENCODED_FRAME];
            let mut answer = None;
            for byte in &encoded[..length] {
                let events = &mut self.events;
                if let Some(len) = self
                    .service
                    .push(*byte, &mut response, &mut |event| events.push(event))
                {
                    answer = Some(len);
                }
            }
            let length = answer.expect("one response frame");
            decode_packet(&response[..length - 1]).expect("decode response")
        }

        fn configuration(&mut self, sequence: u16, command: Command, payload: &[u8]) -> Frame {
            let request = Frame::new(Service::Configuration, 0, sequence, command, payload)
                .expect("request frame");
            self.exchange(&request)
        }
    }

    fn channel(id: u16, hz: u32) -> ChannelRecord {
        let receive = Frequency::from_hz(hz).expect("frequency");
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("TEST").expect("name"),
            receive,
            transmit: receive,
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step: FrequencyStep::from_hz(12_500).expect("step"),
            squelch: SquelchLevel::new(3).expect("squelch"),
            flags: ChannelFlags::default(),
            banks: BankMask::default(),
            tx_class: TxClass::Never,
        })
        .expect("channel record")
    }

    fn write_payload(transaction: u32, object: &StorageObject) -> std::vec::Vec<u8> {
        let mut payload = [0_u8; 9 + MAX_OBJECT_DATA];
        let length = {
            let mut writer = PayloadWriter::new(&mut payload);
            writer.write_u32(transaction).expect("transaction");
            writer.write_u8(object.key().kind as u8).expect("kind");
            writer.write_u16(object.key().id).expect("id");
            writer
                .write_u16(u16::try_from(object.len()).expect("length"))
                .expect("length");
            writer.write_bytes(object.data()).expect("data");
            writer.len()
        };
        payload[..length].to_vec()
    }

    /// One declared number bounds a configuration; the rest is derived from it.
    #[test]
    fn capabilities_report_the_compiled_byte_capacity() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let request =
            Frame::new(Service::DeviceInfo, 0, 1, Command::GetCapabilities, &[]).expect("request");
        let response = harness.exchange(&request);
        let capabilities = DeviceCapabilities::decode(response.payload()).expect("capabilities");
        assert_eq!(
            capabilities.configuration_bytes,
            u32::try_from(STORE_BYTES).unwrap()
        );
        assert_eq!(
            capabilities.max_objects,
            u16::try_from(STORE_BYTES / (OBJECT_ENTRY_HEADER_LEN + MIN_OBJECT_ENCODED_LEN))
                .unwrap(),
            "the object count is an upper bound the bytes imply, not a second limit"
        );
        assert_eq!(
            capabilities.max_object_size,
            u16::try_from(MAX_OBJECT_DATA).unwrap()
        );
        assert_eq!(capabilities.plan_encodings, 0);
    }

    #[test]
    fn a_committed_transaction_activates_listed_and_readable_objects() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let object = encode_channel(channel(1, 145_500_000)).expect("object");
        assert_eq!(
            harness
                .configuration(1, Command::BeginTransaction, &1_u32.to_le_bytes())
                .command(),
            Command::BeginTransaction
        );
        let response = harness.configuration(2, Command::WriteObject, &write_payload(1, &object));
        assert_eq!(response.flags(), FLAG_RESPONSE);
        harness.configuration(3, Command::ValidateTransaction, &1_u32.to_le_bytes());
        let commit = harness.configuration(4, Command::CommitTransaction, &1_u32.to_le_bytes());
        assert_eq!(commit.payload(), 1_u32.to_le_bytes());
        assert_eq!(harness.service.generation(), 1);

        let listing = harness.configuration(5, Command::ListObjects, &0_u16.to_le_bytes());
        let page = radio_protocol::ObjectListPage::decode(listing.payload()).expect("page");
        assert_eq!(page.total_objects(), 1);
        assert_eq!(page.objects()[0].kind, ObjectKind::Channel as u8);

        let read =
            harness.configuration(6, Command::ReadObject, &[ObjectKind::Channel as u8, 1, 0]);
        assert_eq!(&read.payload()[5..], object.data());
        assert!(harness.events.iter().any(|event| matches!(
            event,
            DeviceEvent::TransactionCommitted { generation: 1, .. }
        )));
    }

    #[test]
    fn a_failed_write_leaves_the_active_snapshot_untouched() {
        let mut harness = Harness::<STORE_BYTES>::new();
        harness.configuration(1, Command::BeginTransaction, &7_u32.to_le_bytes());
        // A transaction identifier which was never opened cannot stage data.
        let object = encode_channel(channel(1, 145_500_000)).expect("object");
        let response = harness.configuration(2, Command::WriteObject, &write_payload(9, &object));
        assert_eq!(response.flags(), FLAG_RESPONSE | FLAG_ERROR);
        assert_eq!(
            response.payload(),
            [
                Command::WriteObject as u8,
                DeviceErrorCode::NoTransaction as u8
            ]
        );
        assert_eq!(harness.service.generation(), 0);
        assert_eq!(harness.service.active_objects().count(), 0);
        assert_eq!(harness.service.open_transaction(), Some(7));
    }

    #[test]
    fn an_immediately_repeated_frame_replays_and_a_reused_sequence_conflicts() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let request = Frame::new(Service::DeviceInfo, 0, 5, Command::Hello, &[1]).expect("request");
        let first = harness.exchange(&request);
        let second = harness.exchange(&request);
        assert_eq!(first, second);
        assert!(harness
            .events
            .iter()
            .any(|event| matches!(event, DeviceEvent::DuplicateRequestReplayed { sequence: 5 })));

        let conflicting =
            Frame::new(Service::DeviceInfo, 0, 5, Command::GetCapabilities, &[]).expect("request");
        let response = harness.exchange(&conflicting);
        assert_eq!(
            response.payload(),
            [
                Command::GetCapabilities as u8,
                DeviceErrorCode::SequenceConflict as u8
            ]
        );
    }

    #[test]
    fn loading_a_retained_configuration_activates_it_without_a_transaction() {
        let mut service = DeviceService::<STORE_BYTES>::new();
        let objects = [
            encode_channel(channel(1, 145_500_000)).expect("object"),
            encode_channel(channel(2, 433_500_000)).expect("object"),
        ];
        assert_eq!(service.load(objects), Ok(1));
        assert_eq!(service.active_objects().count(), 2);
        assert_eq!(service.open_transaction(), None);
    }

    /// A device refuses what it has no room for as it is written, and says so
    /// with the one code a full store has.
    #[test]
    fn a_candidate_beyond_the_byte_bound_is_refused_and_stays_inactive() {
        let mut harness = Harness::<ONE_CHANNEL> {
            service: DeviceService::new(),
            events: std::vec::Vec::new(),
        };
        harness.configuration(1, Command::BeginTransaction, &4_u32.to_le_bytes());
        let first = encode_channel(channel(1, 145_500_000)).expect("object");
        let response = harness.configuration(2, Command::WriteObject, &write_payload(4, &first));
        assert_eq!(response.flags(), FLAG_RESPONSE, "the first write fits");

        let second = encode_channel(channel(2, 433_500_000)).expect("object");
        let refused = harness.configuration(3, Command::WriteObject, &write_payload(4, &second));
        assert_eq!(
            refused.payload(),
            [
                Command::WriteObject as u8,
                DeviceErrorCode::CapacityExceeded as u8
            ],
            "bytes are the bound, and they are checked where the bytes arrive"
        );

        harness.configuration(4, Command::ValidateTransaction, &4_u32.to_le_bytes());
        harness.configuration(5, Command::CommitTransaction, &4_u32.to_le_bytes());
        assert_eq!(
            harness.service.active_objects().count(),
            1,
            "what fitted is what runs"
        );
    }

    /// A retained image which no longer fits the store leaves the radio as it
    /// was rather than partly restored.
    #[test]
    fn a_retained_image_beyond_the_byte_bound_is_not_restored() {
        let mut service = DeviceService::<ONE_CHANNEL>::new();
        assert!(service
            .load([
                encode_channel(channel(1, 145_500_000)).expect("object"),
                encode_channel(channel(2, 433_500_000)).expect("object"),
            ])
            .is_err());
        assert_eq!(service.generation(), 0);
        assert_eq!(service.active_objects().count(), 0);
    }

    #[test]
    fn a_retained_image_round_trips_through_the_active_snapshot() {
        let mut service = DeviceService::<STORE_BYTES>::new();
        service
            .load([
                encode_channel(channel(2, 433_500_000)).expect("object"),
                encode_channel(channel(1, 145_500_000)).expect("object"),
            ])
            .expect("load");
        let mut image = [0_u8; 512];
        let length = service.encode_active_image(&mut image).expect("encode");

        let mut restored = DeviceService::<STORE_BYTES>::new();
        assert_eq!(restored.load_image(&image[..length]), Ok(1));
        let mut keys: std::vec::Vec<_> = restored.active_objects().map(ObjectRef::key).collect();
        keys.sort_unstable();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].id, 1);
        assert_eq!(
            restored.encode_active_image(&mut [0_u8; 512]),
            Ok(length),
            "the retained encoding is stable across a restore"
        );
    }

    #[test]
    fn erased_and_corrupt_retained_bytes_leave_the_snapshot_empty() {
        let mut service = DeviceService::<STORE_BYTES>::new();
        assert!(service.load_image(&[0xFF_u8; 256]).is_err());

        let mut source = DeviceService::<STORE_BYTES>::new();
        source
            .load([encode_channel(channel(1, 145_500_000)).expect("object")])
            .expect("load");
        let mut image = [0_u8; 512];
        let length = source.encode_active_image(&mut image).expect("encode");
        image[length - 1] ^= 0xFF;
        assert!(service.load_image(&image[..length]).is_err());
        assert!(service.load_image(&image[..length - 1]).is_err());
        assert_eq!(service.generation(), 0);
        assert_eq!(service.active_objects().count(), 0);
        assert_eq!(service.open_transaction(), None);
    }

    #[test]
    fn a_rejected_load_leaves_no_active_objects_and_no_open_transaction() {
        let mut service = DeviceService::<STORE_BYTES>::new();
        let malformed = StorageObject::new(
            ObjectKey {
                kind: ObjectKind::Channel,
                id: 1,
            },
            &[0_u8; 4],
        )
        .expect("object");
        assert!(service.load([malformed]).is_err());
        assert_eq!(service.generation(), 0);
        assert_eq!(service.active_objects().count(), 0);
        assert_eq!(service.open_transaction(), None);
    }

    /// A state report distinct enough that a wrong one cannot pass for it.
    fn state_report() -> ReceiveStateReport {
        ReceiveStateReport {
            mode: ReceiveMode::Vfo,
            scan: ScanActivity::Idle,
            bank: Some(2),
            index: 5,
            channel_id: 11,
            visible_channels: 40,
            frequency_hz: 145_512_500,
        }
    }

    impl<const BYTES: usize> Harness<BYTES> {
        /// Drives one runtime-control exchange, answering whatever surfaces.
        ///
        /// Returns the pending request the service surfaced and the decoded
        /// response, so a test can assert on both halves of the split.
        fn control(
            &mut self,
            sequence: u16,
            command: Command,
            payload: &[u8],
            answer: ControlAnswer,
        ) -> (Option<ControlRequest>, Frame) {
            let request = Frame::new(Service::RuntimeControl, 0, sequence, command, payload)
                .expect("request frame");
            let mut encoded = [0_u8; MAX_ENCODED_FRAME];
            let length = encode_frame(&request, &mut encoded).expect("encode request");
            let mut response = [0_u8; MAX_ENCODED_FRAME];
            let mut answered = None;
            let mut surfaced = None;
            for byte in &encoded[..length] {
                let events = &mut self.events;
                match self
                    .service
                    .push_control(*byte, &mut response, &mut |event| events.push(event))
                {
                    Push::Idle => {}
                    Push::Response(len) => answered = Some(len),
                    Push::Control(pending) => {
                        surfaced = Some(pending.request());
                        let events = &mut self.events;
                        answered = self.service.answer_control(
                            pending,
                            answer,
                            &mut response,
                            &mut |event| events.push(event),
                        );
                    }
                }
            }
            let length = answered.expect("one response frame");
            (
                surfaced,
                decode_packet(&response[..length - 1]).expect("decode response"),
            )
        }
    }

    #[test]
    fn a_control_request_surfaces_to_the_caller_and_its_answer_reaches_the_host() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let (surfaced, response) = harness.control(
            1,
            Command::TuneTo,
            &145_512_500_u32.to_le_bytes(),
            ControlAnswer::State(state_report()),
        );

        assert_eq!(
            surfaced,
            Some(ControlRequest::TuneTo {
                frequency_hz: 145_512_500
            })
        );
        assert_eq!(response.flags(), FLAG_RESPONSE);
        assert_eq!(response.command(), Command::TuneTo);
        assert_eq!(
            ReceiveStateReport::decode(response.payload()),
            Ok(state_report())
        );
    }

    #[test]
    fn a_metrics_answer_reaches_the_host_unchanged() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let metrics = ReceiveMetricsReport {
            frequency_hz: 145_512_500,
            samples: 1234,
            rssi_dbm_x2: -238,
            glitch: 9,
            noise: 40,
            squelch_open: true,
        };
        let (surfaced, response) = harness.control(
            1,
            Command::GetReceiveMetrics,
            &[],
            ControlAnswer::Metrics(metrics),
        );

        assert_eq!(surfaced, Some(ControlRequest::GetMetrics));
        assert_eq!(
            ReceiveMetricsReport::decode(response.payload()),
            Ok(metrics)
        );
    }

    #[test]
    fn a_refused_control_request_answers_with_its_reason() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let (surfaced, response) = harness.control(
            1,
            Command::StartScan,
            &[],
            ControlAnswer::Refused(DeviceErrorCode::Internal),
        );

        assert_eq!(surfaced, Some(ControlRequest::StartScan));
        assert_eq!(response.flags(), FLAG_RESPONSE | FLAG_ERROR);
        assert_eq!(response.command(), Command::Error);
        assert_eq!(response.payload()[1], DeviceErrorCode::Internal as u8);
    }

    #[test]
    fn a_malformed_control_payload_is_refused_without_reaching_the_receiver() {
        let mut harness = Harness::<STORE_BYTES>::new();
        // Three bytes is not a frequency, and the receiver is never asked to
        // tune to whatever those bytes might be padded into.
        let (surfaced, response) = harness.control(
            1,
            Command::TuneTo,
            &[0, 0, 0],
            ControlAnswer::State(state_report()),
        );

        assert_eq!(surfaced, None);
        assert_eq!(response.flags(), FLAG_RESPONSE | FLAG_ERROR);
        assert_eq!(
            response.payload()[1],
            DeviceErrorCode::MalformedPayload as u8
        );
    }

    #[test]
    fn a_configuration_command_on_the_control_service_is_an_unsupported_command() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let (surfaced, response) = harness.control(
            1,
            Command::ListObjects,
            &[],
            ControlAnswer::State(state_report()),
        );

        assert_eq!(surfaced, None);
        assert_eq!(
            response.payload()[1],
            DeviceErrorCode::UnsupportedCommand as u8
        );
    }

    #[test]
    fn a_resent_control_request_replays_instead_of_performing_it_again() {
        let mut harness = Harness::<STORE_BYTES>::new();
        let (first, first_response) = harness.control(
            7,
            Command::StartScan,
            &[],
            ControlAnswer::State(state_report()),
        );
        assert_eq!(first, Some(ControlRequest::StartScan));

        // The same sequence and bytes again. Starting a scan twice is not the
        // same as starting it once, so this must never reach the receiver.
        let (second, second_response) = harness.control(
            7,
            Command::StartScan,
            &[],
            ControlAnswer::Refused(DeviceErrorCode::Internal),
        );

        assert_eq!(second, None);
        assert_eq!(second_response, first_response);
        assert!(harness
            .events
            .iter()
            .any(|event| matches!(event, DeviceEvent::DuplicateRequestReplayed { sequence: 7 })));
    }

    #[test]
    fn a_service_without_a_receiver_still_refuses_runtime_control() {
        // `push` is what the simulator and any host-side driver use. It owns no
        // receiver, so it must keep answering that this service is unsupported
        // rather than silently doing nothing.
        let mut harness = Harness::<STORE_BYTES>::new();
        let request = Frame::new(Service::RuntimeControl, 0, 1, Command::GetReceiveState, &[])
            .expect("request frame");
        let response = harness.exchange(&request);

        assert_eq!(response.flags(), FLAG_RESPONSE | FLAG_ERROR);
        assert_eq!(
            response.payload()[1],
            DeviceErrorCode::UnsupportedService as u8
        );
    }
}
