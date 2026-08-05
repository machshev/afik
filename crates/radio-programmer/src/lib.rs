//! Library-first host programmer and bounded configuration compiler.

#![forbid(unsafe_code)]

use core::fmt;
use radio_channel_plan::{GeneratedBank, PlanEncoding};
pub use radio_protocol::DeviceCapabilities;
use radio_protocol::{
    encode_frame, Command, DeviceErrorCode, Frame, PayloadReader, PayloadWriter, ProtocolError,
    Service, StreamDecoder, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_PAYLOAD,
    PROTOCOL_VERSION,
};
use radio_storage::{
    decode_generated_bank, encode_generated_bank, ObjectKey, ObjectKind, StorageError,
    StorageObject, StorageUsage, MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
};

const MAX_RECEIVE_CALLS: usize = MAX_ENCODED_FRAME + 1;
const WRITE_ENVELOPE_LEN: usize = 9;

/// Byte-oriented transport shared by serial, simulation, Renode, and replay.
pub trait ProtocolTransport {
    /// Transport-specific failure.
    type Error;

    /// Sends bytes in order. A call may contain one complete framed request.
    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error>;

    /// Receives available ordered bytes, returning zero when none are available.
    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error>;
}

/// Programmer operation failure.
#[derive(Debug)]
pub enum ProgrammerError<E> {
    /// The underlying transport failed.
    Transport(E),
    /// Protocol framing or payload parsing failed.
    Protocol(ProtocolError),
    /// The device rejected a valid request.
    Device {
        /// Command rejected by the device.
        command: Command,
        /// Stable rejection reason.
        code: DeviceErrorCode,
    },
    /// No complete response arrived within the bounded receive loop.
    NoResponse,
    /// A response did not correspond to the outstanding request.
    UnexpectedResponse,
    /// Negotiated capabilities are inconsistent or unusable.
    IncompatibleDevice,
    /// Configuration object encoding or decoding failed.
    Storage(StorageError),
}

impl<E: fmt::Display> fmt::Display for ProgrammerError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "transport error: {error}"),
            Self::Protocol(error) => write!(formatter, "protocol error: {error}"),
            Self::Device { command, code } => {
                write!(formatter, "device rejected {command:?}: {code:?}")
            }
            Self::NoResponse => formatter.write_str("device did not return a complete response"),
            Self::UnexpectedResponse => formatter.write_str("unexpected protocol response"),
            Self::IncompatibleDevice => formatter.write_str("incompatible device capabilities"),
            Self::Storage(error) => write!(formatter, "configuration storage error: {error}"),
        }
    }
}

impl<E> From<ProtocolError> for ProgrammerError<E> {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl<E> From<StorageError> for ProgrammerError<E> {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Rich host-side project input prior to device compilation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RadioProject {
    generated_banks: Vec<GeneratedBank>,
}

impl RadioProject {
    /// Constructs an empty offline project.
    pub const fn new() -> Self {
        Self {
            generated_banks: Vec::new(),
        }
    }

    /// Adds one compact generated bank.
    pub fn add_generated_bank(&mut self, bank: GeneratedBank) {
        self.generated_banks.push(bank);
    }

    /// Returns project banks in stable insertion order.
    pub fn generated_banks(&self) -> &[GeneratedBank] {
        &self.generated_banks
    }
}

/// Device-capacity report emitted by configuration compilation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapacityReport {
    /// Number of device objects produced.
    pub object_count: u16,
    /// Encoded object payload bytes used.
    pub storage_bytes: u32,
    /// Generated channel count represented by those objects.
    pub generated_channels: u32,
}

/// Validated bounded objects ready for a target transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledConfiguration {
    objects: Vec<StorageObject>,
    report: CapacityReport,
}

impl CompiledConfiguration {
    /// Returns compiled objects in deterministic project order.
    pub fn objects(&self) -> &[StorageObject] {
        &self.objects
    }

    /// Returns the compilation capacity report.
    pub const fn report(&self) -> CapacityReport {
        self.report
    }
}

/// Configuration compilation failure before device mutation begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    /// Two project objects have the same stable identity.
    DuplicateObject(ObjectKey),
    /// The target does not support a required plan encoding.
    UnsupportedPlanEncoding(PlanEncoding),
    /// The object count exceeds the negotiated target limit.
    TooManyObjects,
    /// One object exceeds target or protocol payload limits.
    ObjectTooLarge,
    /// A bounded storage encoding failed.
    Storage(StorageError),
    /// A capacity calculation overflowed.
    CapacityOverflow,
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateObject(key) => write!(formatter, "duplicate object {key:?}"),
            Self::UnsupportedPlanEncoding(encoding) => {
                write!(formatter, "target does not support {encoding:?}")
            }
            Self::TooManyObjects => formatter.write_str("configuration has too many objects"),
            Self::ObjectTooLarge => formatter.write_str("configuration object is too large"),
            Self::Storage(error) => write!(formatter, "storage encoding failed: {error}"),
            Self::CapacityOverflow => formatter.write_str("configuration capacity overflow"),
        }
    }
}

impl From<StorageError> for CompileError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Compiler from host projects to a negotiated bounded device representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationCompiler {
    capabilities: DeviceCapabilities,
}

impl ConfigurationCompiler {
    /// Constructs a compiler for one negotiated target capability set.
    pub const fn new(capabilities: DeviceCapabilities) -> Self {
        Self { capabilities }
    }

    /// Validates and compiles a project without contacting or mutating a radio.
    pub fn compile(&self, project: &RadioProject) -> Result<CompiledConfiguration, CompileError> {
        if project.generated_banks.len() > usize::from(self.capabilities.max_objects) {
            return Err(CompileError::TooManyObjects);
        }
        let mut objects = Vec::with_capacity(project.generated_banks.len());
        let mut report = CapacityReport::default();
        for bank in &project.generated_banks {
            let encoding = bank.encoding();
            if self.capabilities.plan_encodings & encoding.capability_bit() == 0 {
                return Err(CompileError::UnsupportedPlanEncoding(encoding));
            }
            let object = encode_generated_bank(*bank)?;
            if objects
                .iter()
                .any(|existing: &StorageObject| existing.key() == object.key())
            {
                return Err(CompileError::DuplicateObject(object.key()));
            }
            if object.len() > usize::from(self.capabilities.max_object_size)
                || object.len() + WRITE_ENVELOPE_LEN
                    > usize::from(self.capabilities.max_frame_payload)
                || object.len() > MAX_OBJECT_DATA
                || object.len() + WRITE_ENVELOPE_LEN > MAX_PAYLOAD
            {
                return Err(CompileError::ObjectTooLarge);
            }
            report.object_count = report
                .object_count
                .checked_add(1)
                .ok_or(CompileError::CapacityOverflow)?;
            report.storage_bytes = report
                .storage_bytes
                .checked_add(
                    u32::try_from(object.len()).map_err(|_| CompileError::CapacityOverflow)?,
                )
                .ok_or(CompileError::CapacityOverflow)?;
            report.generated_channels = report
                .generated_channels
                .checked_add(u32::from(bank.channel_count()))
                .ok_or(CompileError::CapacityOverflow)?;
            objects.push(object);
        }
        Ok(CompiledConfiguration { objects, report })
    }
}

/// Successful configuration commit details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    /// New active device generation.
    pub generation: u32,
    /// Capacity report of the transferred configuration.
    pub report: CapacityReport,
}

/// Connected synchronous programmer over an arbitrary byte transport.
pub struct Programmer<T: ProtocolTransport> {
    transport: T,
    capabilities: DeviceCapabilities,
    next_sequence: u16,
    next_transaction: u32,
    decoder: StreamDecoder,
}

impl<T: ProtocolTransport> Programmer<T> {
    /// Performs HELLO and capability negotiation over a transport.
    pub fn connect(transport: T) -> Result<Self, ProgrammerError<T::Error>> {
        let mut programmer = Self {
            transport,
            capabilities: DeviceCapabilities {
                protocol_version: 0,
                storage_version: 0,
                max_frame_payload: 0,
                max_objects: 0,
                max_object_size: 0,
                plan_encodings: 0,
            },
            next_sequence: 1,
            next_transaction: 1,
            decoder: StreamDecoder::new(),
        };

        let hello =
            programmer.exchange(Service::DeviceInfo, Command::Hello, &[PROTOCOL_VERSION])?;
        if hello.payload() != [PROTOCOL_VERSION] {
            return Err(ProgrammerError::IncompatibleDevice);
        }
        let response = programmer.exchange(Service::DeviceInfo, Command::GetCapabilities, &[])?;
        let capabilities = DeviceCapabilities::decode(response.payload())?;
        if capabilities.protocol_version != PROTOCOL_VERSION
            || capabilities.storage_version != STORAGE_FORMAT_VERSION
            || capabilities.max_frame_payload == 0
            || capabilities.max_object_size == 0
        {
            return Err(ProgrammerError::IncompatibleDevice);
        }
        programmer.capabilities = capabilities;
        Ok(programmer)
    }

    /// Returns the negotiated target capabilities.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }

    /// Returns a compiler bound to the negotiated target capabilities.
    pub const fn compiler(&self) -> ConfigurationCompiler {
        ConfigurationCompiler::new(self.capabilities)
    }

    /// Atomically writes every object in a compiled configuration.
    pub fn write_configuration(
        &mut self,
        configuration: &CompiledConfiguration,
    ) -> Result<CommitReceipt, ProgrammerError<T::Error>> {
        let transaction = self.next_transaction;
        self.next_transaction = self.next_transaction.wrapping_add(1).max(1);
        self.begin_transaction(transaction)?;

        for object in configuration.objects() {
            if let Err(error) = self.write_object(transaction, object) {
                self.abort_after_failure(transaction);
                return Err(error);
            }
        }
        if let Err(error) = self.transaction_command(Command::ValidateTransaction, transaction) {
            self.abort_after_failure(transaction);
            return Err(error);
        }
        let response = match self.transaction_command(Command::CommitTransaction, transaction) {
            Ok(response) => response,
            Err(error) => {
                self.abort_after_failure(transaction);
                return Err(error);
            }
        };
        let mut reader = PayloadReader::new(response.payload());
        let generation = reader.read_u32()?;
        reader.finish()?;
        Ok(CommitReceipt {
            generation,
            report: configuration.report(),
        })
    }

    /// Reads one active object by stable key.
    pub fn read_object(
        &mut self,
        key: ObjectKey,
    ) -> Result<StorageObject, ProgrammerError<T::Error>> {
        let mut payload = [0_u8; 3];
        let mut writer = PayloadWriter::new(&mut payload);
        writer.write_u8(key.kind as u8)?;
        writer.write_u16(key.id)?;
        let response = self.exchange(Service::Configuration, Command::ReadObject, &payload)?;
        let mut reader = PayloadReader::new(response.payload());
        let response_kind = ObjectKind::try_from(reader.read_u8()?)?;
        let response_id = reader.read_u16()?;
        let length = usize::from(reader.read_u16()?);
        let data = reader.read_bytes(length)?;
        reader.finish()?;
        if response_kind != key.kind || response_id != key.id {
            return Err(ProgrammerError::UnexpectedResponse);
        }
        StorageObject::new(key, data).map_err(ProgrammerError::Storage)
    }

    /// Reads and decodes one generated bank.
    pub fn read_generated_bank(
        &mut self,
        id: u16,
    ) -> Result<GeneratedBank, ProgrammerError<T::Error>> {
        let object = self.read_object(ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id,
        })?;
        decode_generated_bank(&object).map_err(ProgrammerError::Storage)
    }

    /// Returns a shared reference to the underlying transport.
    pub const fn transport(&self) -> &T {
        &self.transport
    }

    /// Returns a mutable reference to the underlying transport.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Consumes the programmer and returns its transport.
    pub fn into_transport(self) -> T {
        self.transport
    }

    fn begin_transaction(&mut self, transaction: u32) -> Result<(), ProgrammerError<T::Error>> {
        let response = self.transaction_command(Command::BeginTransaction, transaction)?;
        let mut reader = PayloadReader::new(response.payload());
        let _active_generation = reader.read_u32()?;
        reader.finish()?;
        Ok(())
    }

    fn write_object(
        &mut self,
        transaction: u32,
        object: &StorageObject,
    ) -> Result<(), ProgrammerError<T::Error>> {
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = {
            let mut writer = PayloadWriter::new(&mut payload);
            writer.write_u32(transaction)?;
            writer.write_u8(object.key().kind as u8)?;
            writer.write_u16(object.key().id)?;
            writer.write_u16(
                u16::try_from(object.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
            )?;
            writer.write_bytes(object.data())?;
            writer.len()
        };
        let response = self.exchange(
            Service::Configuration,
            Command::WriteObject,
            &payload[..length],
        )?;
        if !response.payload().is_empty() {
            return Err(ProgrammerError::UnexpectedResponse);
        }
        Ok(())
    }

    fn transaction_command(
        &mut self,
        command: Command,
        transaction: u32,
    ) -> Result<Frame, ProgrammerError<T::Error>> {
        let payload = transaction.to_le_bytes();
        self.exchange(Service::Configuration, command, &payload)
    }

    fn abort_after_failure(&mut self, transaction: u32) {
        let _ignored = self.transaction_command(Command::AbortTransaction, transaction);
    }

    fn exchange(
        &mut self,
        service: Service,
        command: Command,
        payload: &[u8],
    ) -> Result<Frame, ProgrammerError<T::Error>> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        let request = Frame::new(service, 0, sequence, command, payload)?;
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let encoded_len = encode_frame(&request, &mut encoded)?;
        self.transport
            .send(&encoded[..encoded_len])
            .map_err(ProgrammerError::Transport)?;

        let mut receive_buffer = [0_u8; MAX_ENCODED_FRAME];
        for _ in 0..MAX_RECEIVE_CALLS {
            let received = self
                .transport
                .receive(&mut receive_buffer)
                .map_err(ProgrammerError::Transport)?;
            if received == 0 {
                continue;
            }
            for byte in &receive_buffer[..received] {
                let Some(result) = self.decoder.push(*byte) else {
                    continue;
                };
                let Ok(response) = result else {
                    continue;
                };
                if response.sequence() != sequence
                    || response.service() != service
                    || response.flags() & FLAG_RESPONSE == 0
                {
                    continue;
                }
                if response.flags() & FLAG_ERROR != 0 || response.command() == Command::Error {
                    return Err(parse_device_error(command, &response));
                }
                if response.command() != command {
                    return Err(ProgrammerError::UnexpectedResponse);
                }
                return Ok(response);
            }
        }
        Err(ProgrammerError::NoResponse)
    }
}

fn parse_device_error<E>(requested: Command, response: &Frame) -> ProgrammerError<E> {
    let mut reader = PayloadReader::new(response.payload());
    let parsed = (|| {
        let rejected = Command::try_from(reader.read_u8()?)?;
        let code = DeviceErrorCode::try_from(reader.read_u8()?)?;
        reader.finish()?;
        Ok::<_, ProtocolError>((rejected, code))
    })();
    match parsed {
        Ok((rejected, code)) if rejected == requested => ProgrammerError::Device {
            command: rejected,
            code,
        },
        Ok(_) => ProgrammerError::UnexpectedResponse,
        Err(error) => ProgrammerError::Protocol(error),
    }
}

/// Converts active storage usage into a programmer capacity report.
pub const fn report_active_usage(usage: StorageUsage) -> CapacityReport {
    CapacityReport {
        object_count: usage.object_count,
        storage_bytes: usage.payload_bytes,
        generated_channels: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{CompileError, ConfigurationCompiler, DeviceCapabilities, RadioProject};
    use radio_channel_plan::{BankName, GeneratedBank, PlanEncoding};
    use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};

    fn capabilities() -> DeviceCapabilities {
        DeviceCapabilities {
            protocol_version: 1,
            storage_version: 1,
            max_frame_payload: 128,
            max_objects: 1,
            max_object_size: 64,
            plan_encodings: PlanEncoding::LinearSimplex.capability_bit(),
        }
    }

    fn bank(id: u16) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new("PMR446").unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
        )
        .unwrap()
    }

    #[test]
    fn compiler_reports_compact_storage_and_expanded_channels() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        let compiled = ConfigurationCompiler::new(capabilities())
            .compile(&project)
            .unwrap();
        assert_eq!(compiled.report().object_count, 1);
        assert_eq!(compiled.report().storage_bytes, 31);
        assert_eq!(compiled.report().generated_channels, 16);
    }

    #[test]
    fn duplicate_keys_are_rejected_before_transfer() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_generated_bank(bank(1));
        let mut limits = capabilities();
        limits.max_objects = 2;
        assert!(matches!(
            ConfigurationCompiler::new(limits).compile(&project),
            Err(CompileError::DuplicateObject(_))
        ));
    }
}
