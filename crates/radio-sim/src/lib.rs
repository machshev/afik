//! Deterministic functional device and in-memory programmer transport.

#![forbid(unsafe_code)]

use core::{cmp, convert::Infallible};
use radio_channel_plan::PlanEncoding;
use radio_programmer::ProtocolTransport;
use radio_protocol::{
    decode_list_objects_request, encode_frame, Command, DeviceCapabilities, DeviceErrorCode, Frame,
    ObjectDescriptor, ObjectListPage, PayloadReader, PayloadWriter, ProtocolError, Service,
    StreamDecoder, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_LIST_OBJECTS_PER_PAGE,
    MAX_PAYLOAD, PROTOCOL_VERSION,
};
use radio_storage::{
    validate_object, ObjectKey, ObjectKind, StorageError, StorageObject, TransactionalStore,
    MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
};
use std::collections::VecDeque;

/// Maximum configuration objects in the first simulated device profile.
pub const SIM_MAX_OBJECTS: usize = 8;

/// Explicitly advanced deterministic virtual clock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SimClock {
    now_ms: u64,
}

impl SimClock {
    /// Constructs a clock at virtual time zero.
    pub const fn new() -> Self {
        Self { now_ms: 0 }
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(self) -> u64 {
        self.now_ms
    }

    /// Advances virtual time by an exact duration.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(duration_ms);
    }
}

/// One deterministic simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    /// Explicit virtual timestamp.
    pub at_ms: u64,
    /// Observable event details.
    pub kind: TraceKind,
}

/// Observable protocol and storage events emitted by the simulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceKind {
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
    /// A response frame was queued for the host.
    Response {
        /// Response sequence number.
        sequence: u16,
        /// Response command, including the error command.
        command: Command,
    },
}

/// Deterministic protocol-level simulated radio.
pub struct SimDevice {
    clock: SimClock,
    decoder: StreamDecoder,
    store: TransactionalStore<SIM_MAX_OBJECTS>,
    active_transaction: Option<u32>,
    trace: Vec<TraceEvent>,
    capabilities: DeviceCapabilities,
}

impl Default for SimDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl SimDevice {
    /// Constructs an empty, generation-zero device with fixed capabilities.
    pub fn new() -> Self {
        Self {
            clock: SimClock::new(),
            decoder: StreamDecoder::new(),
            store: TransactionalStore::new(),
            active_transaction: None,
            trace: Vec::new(),
            capabilities: DeviceCapabilities {
                protocol_version: PROTOCOL_VERSION,
                storage_version: STORAGE_FORMAT_VERSION,
                max_frame_payload: u16::try_from(MAX_PAYLOAD).unwrap_or(u16::MAX),
                max_objects: u16::try_from(SIM_MAX_OBJECTS).unwrap_or(u16::MAX),
                max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
                plan_encodings: PlanEncoding::LinearSimplex.capability_bit(),
            },
        }
    }

    /// Returns the deterministic virtual clock.
    pub const fn clock(&self) -> SimClock {
        self.clock
    }

    /// Returns the fixed capability profile used for offline compilation.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }

    /// Advances device virtual time explicitly.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
    }

    /// Returns the ordered deterministic event trace.
    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    /// Returns the active storage generation.
    pub const fn generation(&self) -> u32 {
        self.store.generation()
    }

    /// Consumes stream bytes and returns zero or more complete response frames.
    pub fn ingest(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut responses = Vec::new();
        for byte in bytes {
            let Some(result) = self.decoder.push(*byte) else {
                continue;
            };
            match result {
                Ok(request) => {
                    self.record(TraceKind::Request {
                        sequence: request.sequence(),
                        service: request.service(),
                        command: request.command(),
                    });
                    match self.handle_request(&request) {
                        Ok(response) => {
                            let mut encoded = [0_u8; MAX_ENCODED_FRAME];
                            if let Ok(length) = encode_frame(&response, &mut encoded) {
                                responses.extend_from_slice(&encoded[..length]);
                                self.record(TraceKind::Response {
                                    sequence: response.sequence(),
                                    command: response.command(),
                                });
                            }
                        }
                        Err(error) => self.record(TraceKind::PacketDiscarded(error)),
                    }
                }
                Err(error) => self.record(TraceKind::PacketDiscarded(error)),
            }
        }
        responses
    }

    fn handle_request(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        if request.flags() != 0 {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        }
        match request.service() {
            Service::DeviceInfo => self.handle_device_info(request),
            Service::Configuration => self.handle_configuration(request),
            Service::RuntimeControl | Service::FirmwareUpdate | Service::Diagnostics => {
                Self::error_response(request, DeviceErrorCode::UnsupportedService)
            }
        }
    }

    fn handle_device_info(&self, request: &Frame) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::Hello => {
                if request.payload() == [PROTOCOL_VERSION] {
                    Self::success_response(request, &[PROTOCOL_VERSION])
                } else {
                    Self::error_response(request, DeviceErrorCode::MalformedPayload)
                }
            }
            Command::GetCapabilities => {
                if !request.payload().is_empty() {
                    return Self::error_response(request, DeviceErrorCode::MalformedPayload);
                }
                let mut payload = [0_u8; DeviceCapabilities::ENCODED_LEN];
                let length = self.capabilities.encode(&mut payload)?;
                Self::success_response(request, &payload[..length])
            }
            _ => Self::error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn handle_configuration(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::ListObjects => self.list_objects(request),
            Command::ReadObject => self.read_object(request),
            Command::BeginTransaction => self.begin_transaction(request),
            Command::WriteObject => self.write_object(request),
            Command::ValidateTransaction => self.validate_transaction(request),
            Command::CommitTransaction => self.commit_transaction(request),
            Command::AbortTransaction => self.abort_transaction(request),
            _ => Self::error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn list_objects(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(offset) = decode_list_objects_request(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let mut objects = Vec::new();
        for object in self.store.active_objects() {
            objects.push(ObjectDescriptor {
                kind: object.key().kind as u8,
                id: object.key().id,
                encoded_len: u16::try_from(object.len())
                    .map_err(|_| ProtocolError::PayloadTooLarge)?,
            });
        }
        objects.sort_unstable_by_key(|object| (object.kind, object.id));
        let total_objects =
            u16::try_from(objects.len()).map_err(|_| ProtocolError::PayloadTooLarge)?;
        if offset > total_objects {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        }
        let start = usize::from(offset);
        let end = cmp::min(
            start.saturating_add(MAX_LIST_OBJECTS_PER_PAGE),
            objects.len(),
        );
        let page = &objects[start..end];
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = ObjectListPage::encode(
            self.store.generation(),
            total_objects,
            offset,
            page,
            &mut payload,
        )?;
        self.record(TraceKind::ObjectsListed {
            generation: self.store.generation(),
            offset,
            count: u16::try_from(page.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
        });
        Self::success_response(request, &payload[..length])
    }

    fn read_object(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(key) = parse_object_key(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let object = match self.store.read(key) {
            Ok(object) => *object,
            Err(StorageError::ObjectNotFound) => {
                return Self::error_response(request, DeviceErrorCode::ObjectNotFound);
            }
            Err(error) => return Self::error_response(request, map_storage_error(error)),
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
        self.record(TraceKind::ObjectRead(key));
        Self::success_response(request, &payload[..length])
    }

    fn begin_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(transaction) = parse_transaction(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction.is_some() {
            return Self::error_response(request, DeviceErrorCode::TransactionAlreadyOpen);
        }
        if let Err(error) = self.store.begin() {
            return Self::error_response(request, map_storage_error(error));
        }
        self.active_transaction = Some(transaction);
        self.record(TraceKind::TransactionBegan {
            transaction,
            generation: self.store.generation(),
        });
        Self::success_response(request, &self.store.generation().to_le_bytes())
    }

    fn write_object(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok((transaction, object)) = parse_write_object(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction != Some(transaction) {
            return Self::error_response(request, DeviceErrorCode::NoTransaction);
        }
        if let Err(error) = self.store.write(object) {
            return Self::error_response(request, map_storage_error(error));
        }
        self.record(TraceKind::ObjectStaged {
            transaction,
            key: object.key(),
        });
        Self::success_response(request, &[])
    }

    fn validate_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        if let Err(error) = self.store.validate(validate_object) {
            return Self::error_response(request, map_storage_error(error));
        }
        self.record(TraceKind::TransactionValidated { transaction });
        Self::success_response(request, &[])
    }

    fn commit_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        let generation = match self.store.commit() {
            Ok(generation) => generation,
            Err(error) => return Self::error_response(request, map_storage_error(error)),
        };
        self.active_transaction = None;
        self.record(TraceKind::TransactionCommitted {
            transaction,
            generation,
        });
        Self::success_response(request, &generation.to_le_bytes())
    }

    fn abort_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        if let Err(error) = self.store.abort() {
            return Self::error_response(request, map_storage_error(error));
        }
        self.active_transaction = None;
        let _ = transaction;
        Self::success_response(request, &[])
    }

    fn require_transaction(&self, request: &Frame) -> Result<u32, DeviceErrorCode> {
        let transaction =
            parse_transaction(request.payload()).map_err(|_| DeviceErrorCode::MalformedPayload)?;
        if self.active_transaction != Some(transaction) {
            return Err(DeviceErrorCode::NoTransaction);
        }
        Ok(transaction)
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

    fn record(&mut self, kind: TraceKind) {
        self.trace.push(TraceEvent {
            at_ms: self.clock.now_ms(),
            kind,
        });
    }
}

/// In-memory byte transport backed by a deterministic simulated device.
pub struct SimTransport {
    device: SimDevice,
    receive_queue: VecDeque<u8>,
    max_read_size: usize,
}

impl SimTransport {
    /// Constructs a transport returning up to one encoded frame per receive call.
    pub fn new(device: SimDevice) -> Self {
        Self {
            device,
            receive_queue: VecDeque::new(),
            max_read_size: MAX_ENCODED_FRAME,
        }
    }

    /// Limits each receive call to exercise fragmented transport reads.
    #[must_use]
    pub fn with_max_read_size(mut self, max_read_size: usize) -> Self {
        self.max_read_size = max_read_size.max(1);
        self
    }

    /// Returns the underlying simulated device.
    pub const fn device(&self) -> &SimDevice {
        &self.device
    }

    /// Returns a mutable reference to the underlying simulated device.
    pub fn device_mut(&mut self) -> &mut SimDevice {
        &mut self.device
    }

    /// Consumes the transport and returns the simulated device.
    pub fn into_device(self) -> SimDevice {
        self.device
    }
}

impl ProtocolTransport for SimTransport {
    type Error = Infallible;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.receive_queue.extend(self.device.ingest(frame));
        Ok(())
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        let count = cmp::min(
            cmp::min(buffer.len(), self.max_read_size),
            self.receive_queue.len(),
        );
        for destination in &mut buffer[..count] {
            if let Some(byte) = self.receive_queue.pop_front() {
                *destination = byte;
            }
        }
        Ok(count)
    }
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

const fn map_storage_error(error: StorageError) -> DeviceErrorCode {
    match error {
        StorageError::ObjectTooLarge | StorageError::StoreFull => DeviceErrorCode::CapacityExceeded,
        StorageError::TransactionAlreadyOpen => DeviceErrorCode::TransactionAlreadyOpen,
        StorageError::NoTransaction => DeviceErrorCode::NoTransaction,
        StorageError::CandidateNotValidated => DeviceErrorCode::NotValidated,
        StorageError::ValidationFailed
        | StorageError::UnsupportedObject
        | StorageError::MalformedObject => DeviceErrorCode::ValidationFailed,
        StorageError::ObjectNotFound => DeviceErrorCode::ObjectNotFound,
        StorageError::GenerationOverflow => DeviceErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{SimDevice, SimTransport, TraceKind};
    use radio_channel_plan::{BankName, GeneratedBank, PlanEncoding};
    use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};
    use radio_programmer::{ListedObject, Programmer, RadioProject};
    use radio_protocol::{encode_frame, Command, Frame, Service, MAX_ENCODED_FRAME};
    use radio_storage::{ObjectKey, ObjectKind, GENERATED_BANK_ENCODED_LEN};
    use radio_tx_policy::TxPolicy;

    fn bank(id: u16, name: &str) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new(name).unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
        )
        .unwrap()
    }

    fn expected_bank() -> GeneratedBank {
        bank(6, "PMR446")
    }

    fn run_milestone() -> (GeneratedBank, u32, Vec<super::TraceEvent>) {
        let mut project = RadioProject::new();
        project.add_generated_bank(expected_bank());
        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        assert_eq!(compiled.report().storage_bytes, 31);
        assert_eq!(compiled.report().generated_channels, 16);

        let transport = SimTransport::new(device).with_max_read_size(1);
        let mut programmer = Programmer::connect(transport).unwrap();
        assert_eq!(
            programmer.capabilities().plan_encodings,
            PlanEncoding::LinearSimplex.capability_bit()
        );
        let receipt = programmer.write_configuration(&compiled).unwrap();
        let actual = programmer.read_generated_bank(6).unwrap();
        let device = programmer.into_transport().into_device();
        (actual, receipt.generation, device.trace().to_vec())
    }

    #[test]
    fn first_milestone_round_trip_is_deterministic_and_tx_safe() {
        let first = run_milestone();
        let second = run_milestone();
        assert_eq!(first, second);
        assert_eq!(first.0, expected_bank());
        assert_eq!(first.1, 1);
        assert!(first.2.iter().any(|event| matches!(
            event.kind,
            TraceKind::TransactionCommitted { generation: 1, .. }
        )));

        let tx_policy = TxPolicy::default();
        assert!(tx_policy.authorise(TxClass::LicenceFreePlan).is_err());
    }

    #[test]
    fn simulated_stream_recovers_after_a_corrupt_frame() {
        let request = Frame::new(Service::DeviceInfo, 0, 1, Command::Hello, &[1]).unwrap();
        let mut valid = [0_u8; MAX_ENCODED_FRAME];
        let length = encode_frame(&request, &mut valid).unwrap();
        let mut corrupt = valid;
        corrupt[length - 2] ^= 0x20;
        let mut stream = Vec::from(&corrupt[..length]);
        stream.extend_from_slice(&valid[..length]);

        let mut device = SimDevice::new();
        let response = device.ingest(&stream);
        assert!(!response.is_empty());
        assert!(device
            .trace()
            .iter()
            .any(|event| matches!(event.kind, TraceKind::PacketDiscarded(_))));
        assert!(device.trace().iter().any(|event| matches!(
            event.kind,
            TraceKind::Request {
                command: Command::Hello,
                ..
            }
        )));
    }

    #[test]
    fn object_listing_is_bounded_and_independent_of_insertion_order() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(7, "seven"));
        project.add_generated_bank(bank(1, "one"));
        project.add_generated_bank(bank(4, "four"));

        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        let transport = SimTransport::new(device).with_max_read_size(3);
        let mut programmer = Programmer::connect(transport).unwrap();
        let empty = programmer.list_objects().unwrap();
        assert_eq!(empty.generation, 0);
        assert!(empty.objects.is_empty());

        let receipt = programmer.write_configuration(&compiled).unwrap();
        let listing = programmer.list_objects().unwrap();
        let encoded_len = u16::try_from(GENERATED_BANK_ENCODED_LEN).unwrap();
        assert_eq!(listing.generation, receipt.generation);
        assert_eq!(
            listing.objects,
            vec![
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 1,
                    },
                    encoded_len,
                },
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 4,
                    },
                    encoded_len,
                },
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 7,
                    },
                    encoded_len,
                },
            ]
        );
        assert!(programmer.transport().device().trace().iter().any(|event| {
            matches!(
                event.kind,
                TraceKind::ObjectsListed {
                    generation: 1,
                    offset: 0,
                    count: 3,
                }
            )
        }));
    }
}
