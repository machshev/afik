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
    decode_list_objects_request, encode_frame, Command, DeviceCapabilities, DeviceErrorCode, Frame,
    ObjectDescriptor, ObjectListPage, PayloadReader, PayloadWriter, ProtocolError, Service,
    StreamDecoder, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_LIST_OBJECTS_PER_PAGE,
    MAX_PAYLOAD, PROTOCOL_VERSION,
};
use radio_storage::{
    validate_object, ObjectKey, ObjectKind, StorageError, StorageObject, StorageUsage,
    TransactionalStore, MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
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

/// Bounded device-side configuration service over one byte stream.
pub struct DeviceService<const OBJECTS: usize> {
    decoder: StreamDecoder,
    store: TransactionalStore<OBJECTS>,
    active_transaction: Option<u32>,
    last_exchange: Option<(Frame, Frame)>,
    capabilities: DeviceCapabilities,
}

impl<const OBJECTS: usize> Default for DeviceService<OBJECTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const OBJECTS: usize> DeviceService<OBJECTS> {
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
                max_objects: u16::try_from(OBJECTS).unwrap_or(u16::MAX),
                max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
                plan_encodings,
            },
        }
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

    /// Iterates over active objects without exposing candidate data.
    pub fn active_objects(&self) -> impl Iterator<Item = &StorageObject> {
        self.store.active_objects()
    }

    /// Replaces the active snapshot without a protocol transaction.
    ///
    /// This exists for a device which restores a retained configuration at
    /// start-up. Every object is validated and staged through the ordinary
    /// transactional path, so a rejected restore leaves the store empty rather
    /// than partly filled, and no candidate is left open.
    pub fn load(&mut self, objects: &[StorageObject]) -> Result<u32, StorageError> {
        if self.active_transaction.is_some() {
            return Err(StorageError::TransactionAlreadyOpen);
        }
        self.store.begin()?;
        for object in objects {
            if let Err(error) = self.store.write(*object) {
                let _ = self.store.abort();
                return Err(error);
            }
        }
        if let Err(error) = self.store.validate(validate_object) {
            let _ = self.store.abort();
            return Err(error);
        }
        self.store.commit()
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

    fn handle_exchange<F: FnMut(DeviceEvent)>(
        &mut self,
        request: &Frame,
        observer: &mut F,
    ) -> Result<Frame, ProtocolError> {
        if let Some((previous_request, previous_response)) = self.last_exchange {
            if previous_request.sequence() == request.sequence() {
                if previous_request == *request {
                    observer(DeviceEvent::DuplicateRequestReplayed {
                        sequence: request.sequence(),
                    });
                    return Ok(previous_response);
                }
                observer(DeviceEvent::SequenceConflictRejected {
                    sequence: request.sequence(),
                });
                return error_response(request, DeviceErrorCode::SequenceConflict);
            }
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
        let mut descriptors = [EMPTY_DESCRIPTOR; OBJECTS];
        let mut count = 0_usize;
        for object in self.store.active_objects() {
            let descriptor = descriptors
                .get_mut(count)
                .ok_or(ProtocolError::PayloadTooLarge)?;
            *descriptor = ObjectDescriptor {
                kind: object.key().kind as u8,
                id: object.key().id,
                encoded_len: u16::try_from(object.len())
                    .map_err(|_| ProtocolError::PayloadTooLarge)?,
            };
            count += 1;
        }
        let descriptors = &mut descriptors[..count];
        descriptors.sort_unstable_by_key(|descriptor| (descriptor.kind, descriptor.id));
        let total_objects = u16::try_from(count).map_err(|_| ProtocolError::PayloadTooLarge)?;
        if offset > total_objects {
            return error_response(request, DeviceErrorCode::MalformedPayload);
        }
        let start = usize::from(offset);
        let end = start
            .saturating_add(MAX_LIST_OBJECTS_PER_PAGE)
            .min(descriptors.len());
        let page = &descriptors[start..end];
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
            Ok(object) => *object,
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
        if let Err(error) = self.store.write(object) {
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
        if let Err(error) = self.store.validate(validate_object) {
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
    use super::{DeviceEvent, DeviceService};
    use radio_protocol::{
        decode_packet, encode_frame, Command, DeviceCapabilities, DeviceErrorCode, Frame,
        PayloadWriter, Service, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME,
    };
    use radio_storage::{encode_channel, ObjectKey, ObjectKind, StorageObject, MAX_OBJECT_DATA};

    use radio_channel_plan::{
        BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel, Tone,
        TxClass,
    };

    const OBJECTS: usize = 4;

    struct Harness {
        service: DeviceService<OBJECTS>,
        events: std::vec::Vec<DeviceEvent>,
    }

    impl Harness {
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

    #[test]
    fn capabilities_report_the_compiled_object_capacity() {
        let mut harness = Harness::new();
        let request =
            Frame::new(Service::DeviceInfo, 0, 1, Command::GetCapabilities, &[]).expect("request");
        let response = harness.exchange(&request);
        let capabilities = DeviceCapabilities::decode(response.payload()).expect("capabilities");
        assert_eq!(capabilities.max_objects, u16::try_from(OBJECTS).unwrap());
        assert_eq!(
            capabilities.max_object_size,
            u16::try_from(MAX_OBJECT_DATA).unwrap()
        );
        assert_eq!(capabilities.plan_encodings, 0);
    }

    #[test]
    fn a_committed_transaction_activates_listed_and_readable_objects() {
        let mut harness = Harness::new();
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
        let mut harness = Harness::new();
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
        let mut harness = Harness::new();
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
        let mut service = DeviceService::<OBJECTS>::new();
        let objects = [
            encode_channel(channel(1, 145_500_000)).expect("object"),
            encode_channel(channel(2, 433_500_000)).expect("object"),
        ];
        assert_eq!(service.load(&objects), Ok(1));
        assert_eq!(service.active_objects().count(), 2);
        assert_eq!(service.open_transaction(), None);
    }

    #[test]
    fn a_rejected_load_leaves_no_active_objects_and_no_open_transaction() {
        let mut service = DeviceService::<OBJECTS>::new();
        let malformed = StorageObject::new(
            ObjectKey {
                kind: ObjectKind::Channel,
                id: 1,
            },
            &[0_u8; 4],
        )
        .expect("object");
        assert!(service.load(&[malformed]).is_err());
        assert_eq!(service.generation(), 0);
        assert_eq!(service.active_objects().count(), 0);
        assert_eq!(service.open_transaction(), None);
    }
}
