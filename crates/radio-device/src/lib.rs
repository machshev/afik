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
    decode_configuration_image, validate_object, ConfigurationImageWriter, ObjectKey, ObjectKind,
    StorageError, StorageObject, StorageUsage, TransactionalStore, MAX_OBJECT_DATA,
    STORAGE_FORMAT_VERSION,
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

/// How many objects of each kind a device will activate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KindLimits {
    /// Compact generated banks.
    pub generated_banks: u16,
    /// Explicit channel records.
    pub channels: u16,
    /// Named channel banks.
    pub channel_banks: u16,
    /// Global radio configurations.
    pub radio_configs: u16,
}

impl KindLimits {
    /// Limits which only the store capacity bounds.
    #[must_use]
    pub fn unbounded<const OBJECTS: usize>() -> Self {
        // A store larger than the wire representation is bounded to it, which
        // is the same bound the advertised capability reports.
        let capacity = u16::try_from(OBJECTS).unwrap_or(u16::MAX);
        Self {
            generated_banks: capacity,
            channels: capacity,
            channel_banks: capacity,
            radio_configs: capacity,
        }
    }

    const fn limit(self, kind: ObjectKind) -> u16 {
        match kind {
            ObjectKind::GeneratedBank => self.generated_banks,
            ObjectKind::Channel => self.channels,
            ObjectKind::ChannelBank => self.channel_banks,
            ObjectKind::RadioConfig => self.radio_configs,
        }
    }
}

/// Bounded device-side configuration service over one byte stream.
pub struct DeviceService<const OBJECTS: usize> {
    decoder: StreamDecoder,
    store: TransactionalStore<OBJECTS>,
    active_transaction: Option<u32>,
    last_exchange: Option<(Frame, Frame)>,
    limits: KindLimits,
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
        Self::with_limits(plan_encodings, KindLimits::unbounded::<OBJECTS>())
    }

    /// Constructs a service which also bounds how many objects of each kind it
    /// will activate.
    ///
    /// A device whose application can use fewer objects of one kind than its
    /// store can hold must say so at a defined point. These limits are enforced
    /// when the host validates a candidate, so an over-large configuration is
    /// rejected with the stable `ValidationFailed` code before it can become
    /// active, and the previous configuration keeps running.
    pub fn with_limits(plan_encodings: u16, limits: KindLimits) -> Self {
        Self {
            decoder: StreamDecoder::new(),
            store: TransactionalStore::new(),
            active_transaction: None,
            last_exchange: None,
            limits,
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
    pub fn load<I: IntoIterator<Item = StorageObject>>(
        &mut self,
        objects: I,
    ) -> Result<u32, StorageError> {
        if self.active_transaction.is_some() {
            return Err(StorageError::TransactionAlreadyOpen);
        }
        self.store.begin()?;
        for object in objects {
            if let Err(error) = self.store.write(object) {
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

    /// Validates the open candidate against object formats and kind limits.
    fn validate_candidate(&mut self) -> Result<(), StorageError> {
        let limits = self.limits;
        let mut counts = [0_u16; 4];
        self.store.validate(|object| {
            if !validate_object(object) {
                return false;
            }
            let kind = object.key().kind;
            let slot = &mut counts[kind as usize - 1];
            *slot = slot.saturating_add(1);
            *slot <= limits.limit(kind)
        })
    }

    /// Restores one complete canonical configuration image.
    ///
    /// The image is fully validated before any object is staged, so retained
    /// bytes which are absent, erased, truncated, or corrupt leave the active
    /// snapshot untouched.
    pub fn load_image(&mut self, bytes: &[u8]) -> Result<u32, StorageError> {
        let image = decode_configuration_image(bytes)?;
        if usize::from(image.object_count()) > OBJECTS {
            return Err(StorageError::StoreFull);
        }
        self.load(image.objects())
    }

    /// Encodes the active snapshot as a canonical configuration image.
    ///
    /// Objects are emitted in strict `(kind, id)` order without copying the
    /// object table: only a bounded key index is sorted, so a device can retain
    /// its configuration without a second object-sized buffer.
    pub fn encode_active_image(&self, output: &mut [u8]) -> Result<usize, StorageError> {
        let mut keys = [ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: 0,
        }; OBJECTS];
        let mut count = 0_usize;
        for object in self.store.active_objects() {
            *keys.get_mut(count).ok_or(StorageError::StoreFull)? = object.key();
            count += 1;
        }
        let keys = &mut keys[..count];
        sort_bounded(keys, |key| *key);
        let mut writer = ConfigurationImageWriter::new(
            output,
            u16::try_from(count).map_err(|_| StorageError::ImageTooLarge)?,
        )?;
        for key in keys {
            writer.push(self.store.read(*key)?)?;
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
        sort_bounded(descriptors, |descriptor| (descriptor.kind, descriptor.id));
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

/// Orders a small bounded slice in place by a copyable key.
///
/// Insertion sort is used deliberately. These slices hold at most one object
/// per store slot, and the general-purpose sort in `core` costs tens of
/// kilobytes of target flash for the same canonical ordering.
fn sort_bounded<T: Copy, K: Ord, F: Fn(&T) -> K>(items: &mut [T], key: F) {
    for position in 1..items.len() {
        let mut index = position;
        while index > 0 && key(&items[index - 1]) > key(&items[index]) {
            items.swap(index - 1, index);
            index -= 1;
        }
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
    use super::{DeviceEvent, DeviceService, KindLimits};
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
        assert_eq!(service.load(objects), Ok(1));
        assert_eq!(service.active_objects().count(), 2);
        assert_eq!(service.open_transaction(), None);
    }

    #[test]
    fn a_candidate_beyond_a_kind_limit_fails_validation_and_stays_inactive() {
        let mut harness = Harness {
            service: DeviceService::with_limits(
                0,
                KindLimits {
                    channels: 1,
                    ..KindLimits::unbounded::<OBJECTS>()
                },
            ),
            events: std::vec::Vec::new(),
        };
        harness.configuration(1, Command::BeginTransaction, &4_u32.to_le_bytes());
        for (sequence, id) in [(2_u16, 1_u16), (3, 2)] {
            let object = encode_channel(channel(id, 145_500_000)).expect("object");
            let response =
                harness.configuration(sequence, Command::WriteObject, &write_payload(4, &object));
            assert_eq!(response.flags(), FLAG_RESPONSE, "both writes are staged");
        }
        let response = harness.configuration(4, Command::ValidateTransaction, &4_u32.to_le_bytes());
        assert_eq!(
            response.payload(),
            [
                Command::ValidateTransaction as u8,
                DeviceErrorCode::ValidationFailed as u8
            ]
        );
        let commit = harness.configuration(5, Command::CommitTransaction, &4_u32.to_le_bytes());
        assert_eq!(
            commit.payload(),
            [
                Command::CommitTransaction as u8,
                DeviceErrorCode::NotValidated as u8
            ],
            "an unvalidated candidate cannot be activated"
        );
        assert_eq!(harness.service.generation(), 0);
        assert_eq!(harness.service.active_objects().count(), 0);
    }

    #[test]
    fn a_retained_image_beyond_a_kind_limit_is_not_restored() {
        let mut service = DeviceService::<OBJECTS>::with_limits(
            0,
            KindLimits {
                channels: 1,
                ..KindLimits::unbounded::<OBJECTS>()
            },
        );
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
        let mut service = DeviceService::<OBJECTS>::new();
        service
            .load([
                encode_channel(channel(2, 433_500_000)).expect("object"),
                encode_channel(channel(1, 145_500_000)).expect("object"),
            ])
            .expect("load");
        let mut image = [0_u8; 512];
        let length = service.encode_active_image(&mut image).expect("encode");

        let mut restored = DeviceService::<OBJECTS>::new();
        assert_eq!(restored.load_image(&image[..length]), Ok(1));
        let mut keys: std::vec::Vec<_> = restored
            .active_objects()
            .map(|object| object.key())
            .collect();
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
        let mut service = DeviceService::<OBJECTS>::new();
        assert!(service.load_image(&[0xFF_u8; 256]).is_err());

        let mut source = DeviceService::<OBJECTS>::new();
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
        let mut service = DeviceService::<OBJECTS>::new();
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
}
