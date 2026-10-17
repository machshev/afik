//! Library-first host programmer and bounded configuration compiler.

#![forbid(unsafe_code)]

use core::fmt;
use radio_channel_plan::{
    BankGroup, BankOrigin, ChannelBank, ChannelRecord, GeneratedBank, PlanEncoding, StartPlace,
};
use radio_domain::{BankId, ChannelId, RadioConfig};
use radio_platform::memory_link::IMAGE_REGION_BYTES;
pub use radio_protocol::DeviceCapabilities;
use radio_protocol::{
    encode_frame, encode_list_objects_request, Command, ControlRequest, DeviceErrorCode, Frame,
    ObjectListPage, PayloadReader, PayloadWriter, ProtocolError, ReceiveMetricsReport,
    ReceiveStateReport, Service, StreamDecoder, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME,
    MAX_PAYLOAD, PROTOCOL_VERSION,
};
use radio_storage::radio_image;
use radio_storage::{
    configuration_image_len, decode_bank_group, decode_bank_origin, decode_channel,
    decode_channel_bank, decode_configuration_image, decode_generated_bank, decode_radio_config,
    decode_start_place, decode_tx_range, encode_bank_group, encode_bank_origin, encode_channel,
    encode_channel_bank, encode_configuration_image, encode_generated_bank, encode_radio_config,
    encode_start_place, encode_tx_range, ObjectKey, ObjectKind, StorageError, StorageObject,
    StorageUsage, MAX_OBJECT_DATA, OBJECT_ENTRY_HEADER_LEN, STORAGE_FORMAT_VERSION,
};
use radio_tx_policy::{RangeError, TxPolicy, TxRange};

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
    /// Post-commit generation or object read-back did not match the request.
    VerificationMismatch,
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
            Self::VerificationMismatch => {
                formatter.write_str("configuration read-back verification failed")
            }
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

pub use radio_storage::radio_image::CompileError as RadioImageError;

/// Bytes a radio holds for its configuration image (`ADR-100`).
pub const RADIO_IMAGE_BYTES: usize = IMAGE_REGION_BYTES as usize;

/// The longest image [`RadioProject::radio_image_len`] measures; one this
/// long is far past any radio's region.
const MEASURED_IMAGE_BYTES: usize = 0x1_0000;

/// Rich host-side project input prior to device compilation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RadioProject {
    generated_banks: Vec<GeneratedBank>,
    channels: Vec<ChannelRecord>,
    banks: Vec<ChannelBank>,
    config: Option<RadioConfig>,
    tx_ranges: Vec<TxRange>,
    groups: Vec<BankGroup>,
    start: Option<StartPlace>,
    origins: Vec<BankOrigin>,
}

impl RadioProject {
    /// Constructs an empty offline project.
    pub const fn new() -> Self {
        Self {
            generated_banks: Vec::new(),
            channels: Vec::new(),
            banks: Vec::new(),
            config: None,
            tx_ranges: Vec::new(),
            groups: Vec::new(),
            start: None,
            origins: Vec::new(),
        }
    }

    /// Records which generator made a bank (`ADR-117`); the project file
    /// keeps it, a radio image never holds it.
    pub fn add_origin(&mut self, origin: BankOrigin) {
        self.origins.push(origin);
    }

    /// Returns where each generated bank came from.
    pub fn origins(&self) -> &[BankOrigin] {
        &self.origins
    }

    /// Sets where the radio starts, which a write sends as its place record
    /// (`ADR-116`); `None` leaves the radio's own.
    pub fn set_start(&mut self, start: Option<StartPlace>) {
        self.start = start;
    }

    /// Returns where the radio starts, if the project says.
    pub const fn start(&self) -> Option<StartPlace> {
        self.start
    }

    /// Adds one bank group, or location (`ADR-116`).
    pub fn add_group(&mut self, group: BankGroup) {
        self.groups.push(group);
    }

    /// Returns the bank groups.
    pub fn groups(&self) -> &[BankGroup] {
        &self.groups
    }

    /// Adds one compact generated bank.
    pub fn add_generated_bank(&mut self, bank: GeneratedBank) {
        self.generated_banks.push(bank);
    }

    /// Adds one explicit channel record.
    pub fn add_channel(&mut self, channel: ChannelRecord) {
        self.channels.push(channel);
    }

    /// Adds one named channel bank.
    pub fn add_bank(&mut self, bank: ChannelBank) {
        self.banks.push(bank);
    }

    /// Sets the single global radio configuration.
    pub fn set_config(&mut self, config: RadioConfig) {
        self.config = Some(config);
    }

    /// Adds one programmed TX range (`ADR-101`); order is kept.
    pub fn add_tx_range(&mut self, range: TxRange) {
        self.tx_ranges.push(range);
    }

    /// Returns the programmed TX ranges in order.
    pub fn tx_ranges(&self) -> &[TxRange] {
        &self.tx_ranges
    }

    /// Returns project generated banks in stable insertion order.
    pub fn generated_banks(&self) -> &[GeneratedBank] {
        &self.generated_banks
    }

    /// Returns project channels in stable insertion order.
    pub fn channels(&self) -> &[ChannelRecord] {
        &self.channels
    }

    /// Returns project named banks in stable insertion order.
    pub fn banks(&self) -> &[ChannelBank] {
        &self.banks
    }

    /// Returns the global radio configuration when one is set.
    pub const fn config(&self) -> Option<RadioConfig> {
        self.config
    }

    /// Compiles the project into the bank-grouped image a radio runs
    /// (storage format 8, `ADR-116`), sized to the radio's image region.
    ///
    /// Each channel must be in at most one bank: in this format a channel is
    /// stored in its bank. Channels in no bank are kept in a bankless block.
    pub fn compile_radio_image(&self) -> Result<Vec<u8>, RadioImageError> {
        self.compile_radio_image_within(usize::from(IMAGE_REGION_BYTES))
    }

    /// Returns how long the radio image would be with room for any length,
    /// so a project too big for the radio says by how much.
    pub fn radio_image_len(&self) -> Result<usize, RadioImageError> {
        self.compile_radio_image_within(MEASURED_IMAGE_BYTES)
            .map(|image| image.len())
    }

    /// Compiles the radio image into at most `bytes`.
    fn compile_radio_image_within(&self, bytes: usize) -> Result<Vec<u8>, RadioImageError> {
        let mut image = vec![0_u8; bytes];
        let len = radio_image::compile(
            self.config.unwrap_or_else(RadioConfig::conservative),
            &self.tx_ranges,
            &self.banks,
            &self.channels,
            &self.generated_banks,
            &self.groups,
            &mut image,
        )?;
        image.truncate(len);
        Ok(image)
    }

    /// Reads back an image [`Self::compile_radio_image`] produced, as a radio
    /// would: checksum and every bank first, then each bank, plan and stored
    /// channel. Stored channels are numbered by their position on the radio.
    pub fn from_radio_image(image: &[u8]) -> Result<Self, StorageError> {
        let mut header = [0_u8; radio_image::HEADER_LEN];
        header.copy_from_slice(
            image
                .get(..radio_image::HEADER_LEN)
                .ok_or(StorageError::MalformedImage)?,
        );
        let payload = &image[radio_image::HEADER_LEN..];
        let read = |offset: usize, buffer: &mut [u8]| {
            payload
                .get(offset..offset + buffer.len())
                .is_some_and(|bytes| {
                    buffer.copy_from_slice(bytes);
                    true
                })
        };
        let index = radio_image::Index::build(&header, read)?;
        let mut project = Self::new();
        project.set_config(index.config());
        for range in index.tx_policy(read).ranges() {
            project.add_tx_range(*range);
        }
        for id in 0..radio_channel_plan::MAX_BANKS {
            let bank = BankId::new(id);
            if let Some(named) = index.bank(read, bank) {
                project.add_bank(named);
            }
            if let Some(plan) = index.plan(read, bank) {
                project.add_generated_bank(plan);
                continue;
            }
            let numbers = (0..).map_while(|position| index.index_of(bank, position));
            // Banks first, so a group below finds every bank it names.
            for number in numbers {
                let channel = index
                    .channel_at(read, number)
                    .and_then(|channel| channel.with_id(ChannelId::new(number + 1)).ok())
                    .ok_or(StorageError::MalformedObject)?;
                project.add_channel(channel);
            }
        }
        for id in 0..index.group_count() {
            let (name, banks) = index.group(read, id).ok_or(StorageError::MalformedObject)?;
            project.add_group(
                BankGroup::new(id, name, banks).map_err(|_| StorageError::MalformedObject)?,
            );
        }
        Ok(project)
    }

    /// Reports whether the project defines no objects at all.
    pub fn is_empty(&self) -> bool {
        self.generated_banks.is_empty()
            && self.channels.is_empty()
            && self.banks.is_empty()
            && self.config.is_none()
            && self.tx_ranges.is_empty()
            && self.groups.is_empty()
    }
}

/// Device-capacity report emitted by configuration compilation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapacityReport {
    /// Number of device objects produced.
    pub object_count: u16,
    /// Packed configuration bytes these objects occupy in a device store.
    ///
    /// This counts each object's entry header as well as its payload, because
    /// that is what a device charges for it and what its declared
    /// `configuration_bytes` bounds.
    pub storage_bytes: u32,
    /// Generated channel count represented by those objects.
    pub generated_channels: u32,
    /// Explicit channel records represented by those objects.
    pub explicit_channels: u32,
    /// Named channel banks represented by those objects.
    pub banks: u16,
    /// Whether the configuration carries the global radio configuration.
    pub has_radio_config: bool,
}

/// Validated bounded objects ready for a target transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledConfiguration {
    objects: Vec<StorageObject>,
    report: CapacityReport,
}

impl CompiledConfiguration {
    /// Returns compiled objects in canonical stable-key order.
    pub fn objects(&self) -> &[StorageObject] {
        &self.objects
    }

    /// Returns the compilation capacity report.
    pub const fn report(&self) -> CapacityReport {
        self.report
    }

    /// Calculates the exact canonical image length for this configuration.
    pub fn image_len(&self) -> Result<usize, StorageError> {
        configuration_image_len(&self.objects)
    }

    /// Encodes this configuration as a canonical image in a caller buffer.
    pub fn encode_image(&self, output: &mut [u8]) -> Result<usize, StorageError> {
        encode_configuration_image(&self.objects, output)
    }
}

/// Configuration compilation failure before device mutation begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    /// Two project objects have the same stable identity.
    DuplicateObject(ObjectKey),
    /// The target does not support a required plan encoding.
    UnsupportedPlanEncoding(PlanEncoding),
    /// The configuration needs more stored bytes than the target declares.
    ConfigurationTooLarge {
        /// Packed bytes this configuration occupies.
        needed: u32,
        /// Packed bytes the target declared it has.
        available: u32,
    },
    /// One object exceeds target or protocol payload limits.
    ObjectTooLarge,
    /// A bounded storage encoding failed.
    Storage(StorageError),
    /// The target does not support the image's logical storage version.
    UnsupportedStorageVersion,
    /// A capacity calculation overflowed.
    CapacityOverflow,
    /// A channel claims membership of a bank the project does not define.
    UnknownBank {
        /// The referencing channel.
        channel: u16,
        /// The undefined bank identifier.
        bank: u16,
    },
    /// A channel differs from a setting its bank shares with every channel.
    BankSetting {
        /// The disagreeing channel.
        channel: u16,
        /// The bank whose shared setting it differs from.
        bank: u16,
    },
    /// A TX range is malformed, or there are too many (`ADR-101`).
    TxRange(RangeError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateObject(key) => write!(formatter, "duplicate object {key:?}"),
            Self::UnsupportedPlanEncoding(encoding) => {
                write!(formatter, "target does not support {encoding:?}")
            }
            Self::ConfigurationTooLarge { needed, available } => write!(
                formatter,
                "configuration needs {needed} stored bytes and the target has {available}"
            ),
            Self::ObjectTooLarge => formatter.write_str("configuration object is too large"),
            Self::Storage(error) => write!(formatter, "storage encoding failed: {error}"),
            Self::UnsupportedStorageVersion => {
                formatter.write_str("target does not support the configuration storage version")
            }
            Self::CapacityOverflow => formatter.write_str("configuration capacity overflow"),
            Self::UnknownBank { channel, bank } => write!(
                formatter,
                "channel {channel} references undefined bank {bank}"
            ),
            Self::BankSetting { channel, bank } => write!(
                formatter,
                "channel {channel} differs from a setting bank {bank} shares"
            ),
            Self::TxRange(error) => write!(formatter, "{error}"),
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
        self.require_storage_version()?;
        Self::require_defined_banks(project)?;
        let object_count = project.generated_banks.len()
            + project.channels.len()
            + project.banks.len()
            + usize::from(project.config.is_some())
            + project.tx_ranges.len()
            + project.groups.len()
            + usize::from(project.start.is_some())
            + project.origins.len();
        let mut objects = Vec::with_capacity(object_count);
        for bank in &project.generated_banks {
            objects.push(encode_generated_bank(*bank)?);
        }
        for channel in &project.channels {
            objects.push(encode_channel(*channel)?);
        }
        for bank in &project.banks {
            objects.push(encode_channel_bank(*bank)?);
        }
        if let Some(config) = project.config {
            objects.push(encode_radio_config(config)?);
        }
        TxPolicy::new(&project.tx_ranges).map_err(CompileError::TxRange)?;
        for (position, range) in project.tx_ranges.iter().enumerate() {
            let position = u16::try_from(position).map_err(|_| CompileError::CapacityOverflow)?;
            objects.push(encode_tx_range(position, *range)?);
        }
        for group in &project.groups {
            objects.push(encode_bank_group(*group)?);
        }
        if let Some(start) = project.start {
            objects.push(encode_start_place(start)?);
        }
        for origin in &project.origins {
            objects.push(encode_bank_origin(*origin)?);
        }
        objects.sort_unstable_by_key(|object| object.key());
        if let Some(duplicate) = objects
            .windows(2)
            .find(|pair| pair[0].key() == pair[1].key())
        {
            return Err(CompileError::DuplicateObject(duplicate[0].key()));
        }
        let report = self.validate_objects(&objects)?;
        Ok(CompiledConfiguration { objects, report })
    }

    /// Validates a canonical image against this target and reconstructs its
    /// compiled configuration without contacting or mutating a radio.
    pub fn decode_image(&self, bytes: &[u8]) -> Result<CompiledConfiguration, CompileError> {
        self.require_storage_version()?;
        let image = decode_configuration_image(bytes)?;
        let objects = image
            .objects()
            .map(|object| StorageObject::new(object.key(), object.data()))
            .collect::<Result<Vec<_>, _>>()?;
        let report = self.validate_objects(&objects)?;
        Ok(CompiledConfiguration { objects, report })
    }

    fn require_defined_banks(project: &RadioProject) -> Result<(), CompileError> {
        for channel in &project.channels {
            if let Some(id) = channel.bank() {
                let Some(bank) = project.banks.iter().find(|named| named.id() == id) else {
                    return Err(CompileError::UnknownBank {
                        channel: channel.id().get(),
                        bank: id.get(),
                    });
                };
                if !bank.settings().agrees(channel) {
                    return Err(CompileError::BankSetting {
                        channel: channel.id().get(),
                        bank: id.get(),
                    });
                }
            }
        }
        Ok(())
    }

    fn require_storage_version(&self) -> Result<(), CompileError> {
        if self.capabilities.storage_version != STORAGE_FORMAT_VERSION {
            return Err(CompileError::UnsupportedStorageVersion);
        }
        Ok(())
    }

    fn validate_objects(&self, objects: &[StorageObject]) -> Result<CapacityReport, CompileError> {
        let mut report = CapacityReport::default();
        for object in objects {
            if object.len() > usize::from(self.capabilities.max_object_size)
                || object.len() + WRITE_ENVELOPE_LEN
                    > usize::from(self.capabilities.max_frame_payload)
                || object.len() > MAX_OBJECT_DATA
                || object.len() + WRITE_ENVELOPE_LEN > MAX_PAYLOAD
            {
                return Err(CompileError::ObjectTooLarge);
            }
            match object.key().kind {
                ObjectKind::GeneratedBank => {
                    let bank = decode_generated_bank(object)?;
                    let encoding = bank.encoding();
                    if self.capabilities.plan_encodings & encoding.capability_bit() == 0 {
                        return Err(CompileError::UnsupportedPlanEncoding(encoding));
                    }
                    report.generated_channels = report
                        .generated_channels
                        .checked_add(u32::from(bank.channel_count()))
                        .ok_or(CompileError::CapacityOverflow)?;
                }
                ObjectKind::Channel => {
                    decode_channel(object)?;
                    report.explicit_channels = report
                        .explicit_channels
                        .checked_add(1)
                        .ok_or(CompileError::CapacityOverflow)?;
                }
                ObjectKind::ChannelBank => {
                    decode_channel_bank(object)?;
                    report.banks = report
                        .banks
                        .checked_add(1)
                        .ok_or(CompileError::CapacityOverflow)?;
                }
                ObjectKind::RadioConfig => {
                    decode_radio_config(object)?;
                    report.has_radio_config = true;
                }
                ObjectKind::TxRange => {
                    decode_tx_range(object)?;
                }
                ObjectKind::BankGroup => {
                    decode_bank_group(object)?;
                }
                ObjectKind::StartPlace => {
                    decode_start_place(object)?;
                }
                ObjectKind::BankOrigin => {
                    decode_bank_origin(object)?;
                }
            }
            report.object_count = report
                .object_count
                .checked_add(1)
                .ok_or(CompileError::CapacityOverflow)?;
            report.storage_bytes = report
                .storage_bytes
                .checked_add(
                    u32::try_from(object.len() + OBJECT_ENTRY_HEADER_LEN)
                        .map_err(|_| CompileError::CapacityOverflow)?,
                )
                .ok_or(CompileError::CapacityOverflow)?;
        }
        // The one bound a target declares. A device which reports no
        // configuration storage declares nothing rather than a full store, so
        // it is left to refuse what it cannot hold as the bytes arrive.
        let available = self.capabilities.configuration_bytes;
        if available != 0 && report.storage_bytes > available {
            return Err(CompileError::ConfigurationTooLarge {
                needed: report.storage_bytes,
                available,
            });
        }
        Ok(report)
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

/// One active object reported by the device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListedObject {
    /// Stable object identity.
    pub key: ObjectKey,
    /// Encoded object payload length in bytes.
    pub encoded_len: u16,
}

/// Complete, generation-tagged active object listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectListing {
    /// Active storage generation shared by every decoded page.
    pub generation: u32,
    /// Objects in strict `(kind, id)` order.
    pub objects: Vec<ListedObject>,
}

/// Complete active configuration read from one stable device generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationSnapshot {
    /// Active storage generation confirmed before and after object reads.
    pub generation: u32,
    /// Active objects in strict `(kind, id)` order.
    pub objects: Vec<StorageObject>,
}

/// Verified configuration mutation receipt shared by host front ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedConfigurationReceipt {
    /// New active generation confirmed by complete read-back.
    pub generation: u32,
    /// Exact capacity of the committed configuration.
    pub report: CapacityReport,
}

/// Complete validated snapshot plus its canonical backup image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationBackup {
    /// Stable active generation read before and after every object.
    pub generation: u32,
    /// Exact validated snapshot capacity.
    pub report: CapacityReport,
    /// Canonical configuration image bytes.
    pub image: Vec<u8>,
}

/// Restore workflow failure before or during device mutation.
#[derive(Debug)]
pub enum RestoreError<E> {
    /// Canonical image or negotiated target compilation failed before mutation.
    Compile(CompileError),
    /// Programmer transport, protocol, device, storage, or verification failed.
    Programmer(ProgrammerError<E>),
}

impl<E: fmt::Display> fmt::Display for RestoreError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "restore image rejected: {error}"),
            Self::Programmer(error) => write!(formatter, "restore failed: {error}"),
        }
    }
}

impl ConfigurationSnapshot {
    /// Validates the snapshot and reports exact object and channel capacity.
    pub fn report(&self) -> Result<CapacityReport, StorageError> {
        let object_count =
            u16::try_from(self.objects.len()).map_err(|_| StorageError::ImageTooLarge)?;
        let mut storage_bytes = 0_u32;
        let mut generated_channels = 0_u32;
        let mut explicit_channels = 0_u32;
        let mut banks = 0_u16;
        let mut has_radio_config = false;
        let mut previous_key = None;

        for object in &self.objects {
            if previous_key.is_some_and(|key| key >= object.key()) {
                return Err(StorageError::NonCanonicalImage);
            }
            previous_key = Some(object.key());
            match object.key().kind {
                ObjectKind::GeneratedBank => {
                    let bank = decode_generated_bank(object)?;
                    generated_channels = generated_channels
                        .checked_add(u32::from(bank.channel_count()))
                        .ok_or(StorageError::ImageTooLarge)?;
                }
                ObjectKind::Channel => {
                    decode_channel(object)?;
                    explicit_channels = explicit_channels
                        .checked_add(1)
                        .ok_or(StorageError::ImageTooLarge)?;
                }
                ObjectKind::ChannelBank => {
                    decode_channel_bank(object)?;
                    banks = banks.checked_add(1).ok_or(StorageError::ImageTooLarge)?;
                }
                ObjectKind::RadioConfig => {
                    decode_radio_config(object)?;
                    has_radio_config = true;
                }
                ObjectKind::TxRange => {
                    decode_tx_range(object)?;
                }
                ObjectKind::BankGroup => {
                    decode_bank_group(object)?;
                }
                ObjectKind::StartPlace => {
                    decode_start_place(object)?;
                }
                ObjectKind::BankOrigin => {
                    decode_bank_origin(object)?;
                }
            }
            storage_bytes = storage_bytes
                .checked_add(
                    u32::try_from(object.len() + OBJECT_ENTRY_HEADER_LEN)
                        .map_err(|_| StorageError::ImageTooLarge)?,
                )
                .ok_or(StorageError::ImageTooLarge)?;
        }

        Ok(CapacityReport {
            object_count,
            storage_bytes,
            generated_channels,
            explicit_channels,
            banks,
            has_radio_config,
        })
    }

    /// Calculates the exact validated canonical backup-image length.
    pub fn image_len(&self) -> Result<usize, StorageError> {
        let _report = self.report()?;
        configuration_image_len(&self.objects)
    }

    /// Encodes this validated snapshot into a caller-provided image buffer.
    pub fn encode_image(&self, output: &mut [u8]) -> Result<usize, StorageError> {
        let _report = self.report()?;
        encode_configuration_image(&self.objects, output)
    }
}

/// What one runtime-control request answered with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlOutcome {
    /// What the receiver is doing, after any requested change.
    State(ReceiveStateReport),
    /// One raw metrics sample.
    Metrics(ReceiveMetricsReport),
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
                configuration_bytes: 0,
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

    /// Performs one runtime-control request against the connected radio.
    ///
    /// The radio is driven exactly as its keypad drives it: this changes live
    /// receive state and never writes configuration. An operation the radio
    /// cannot perform from where it is — tuning while a memory channel is
    /// selected — is reported as a device rejection rather than forced.
    pub fn control(
        &mut self,
        request: ControlRequest,
    ) -> Result<ControlOutcome, ProgrammerError<T::Error>> {
        let mut payload = [0_u8; ControlRequest::MAX_PAYLOAD_LEN];
        let length = request.encode_payload(&mut payload)?;
        let response = self.exchange(
            Service::RuntimeControl,
            request.command(),
            &payload[..length],
        )?;
        match request {
            ControlRequest::GetMetrics => Ok(ControlOutcome::Metrics(
                ReceiveMetricsReport::decode(response.payload())?,
            )),
            _ => Ok(ControlOutcome::State(ReceiveStateReport::decode(
                response.payload(),
            )?)),
        }
    }

    /// Asks what the receiver is currently doing.
    pub fn receive_state(&mut self) -> Result<ReceiveStateReport, ProgrammerError<T::Error>> {
        match self.control(ControlRequest::GetState)? {
            ControlOutcome::State(state) => Ok(state),
            ControlOutcome::Metrics(_) => {
                Err(ProgrammerError::Protocol(ProtocolError::MalformedPayload))
            }
        }
    }

    /// Reads one raw metrics sample from the receiver.
    ///
    /// The sample carries the frequency it was taken at and a counter which
    /// advances once per completed sample. A caller which has just retuned and
    /// wants a settled reading waits for that counter to pass what it last saw:
    /// the receiver needs a settling time this project has not measured, and a
    /// reading taken too soon describes the settling rather than the signal.
    pub fn receive_metrics(&mut self) -> Result<ReceiveMetricsReport, ProgrammerError<T::Error>> {
        match self.control(ControlRequest::GetMetrics)? {
            ControlOutcome::Metrics(metrics) => Ok(metrics),
            ControlOutcome::State(_) => {
                Err(ProgrammerError::Protocol(ProtocolError::MalformedPayload))
            }
        }
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

    /// Writes a configuration and requires exact generation/object read-back.
    pub fn write_configuration_verified(
        &mut self,
        configuration: &CompiledConfiguration,
    ) -> Result<VerifiedConfigurationReceipt, ProgrammerError<T::Error>> {
        let receipt = self.write_configuration(configuration)?;
        let snapshot = self.read_configuration()?;
        if snapshot.generation != receipt.generation || snapshot.objects != configuration.objects {
            return Err(ProgrammerError::VerificationMismatch);
        }
        Ok(VerifiedConfigurationReceipt {
            generation: receipt.generation,
            report: receipt.report,
        })
    }

    /// Reads, validates, reports, and canonically encodes one stable backup.
    pub fn backup_configuration(
        &mut self,
    ) -> Result<ConfigurationBackup, ProgrammerError<T::Error>> {
        let snapshot = self.read_configuration()?;
        let report = snapshot.report()?;
        let mut image = vec![0; snapshot.image_len()?];
        snapshot.encode_image(&mut image)?;
        Ok(ConfigurationBackup {
            generation: snapshot.generation,
            report,
            image,
        })
    }

    /// Validates a canonical image against negotiated limits, writes it, and
    /// requires exact generation/object read-back.
    pub fn restore_configuration_image(
        &mut self,
        image: &[u8],
    ) -> Result<VerifiedConfigurationReceipt, RestoreError<T::Error>> {
        let configuration = self
            .compiler()
            .decode_image(image)
            .map_err(RestoreError::Compile)?;
        self.write_configuration_verified(&configuration)
            .map_err(RestoreError::Programmer)
    }

    /// Lists every active object in deterministic stable-key order.
    pub fn list_objects(&mut self) -> Result<ObjectListing, ProgrammerError<T::Error>> {
        let mut offset = 0_u16;
        let mut expected_generation = None;
        let mut expected_total = None;
        let mut objects: Vec<ListedObject> = Vec::new();

        loop {
            let mut request_payload = [0_u8; 2];
            let request_len = encode_list_objects_request(offset, &mut request_payload)?;
            let response = self.exchange(
                Service::Configuration,
                Command::ListObjects,
                &request_payload[..request_len],
            )?;
            let page = ObjectListPage::decode(response.payload())?;
            if page.offset() != offset || page.total_objects() > self.capabilities.max_objects {
                return Err(ProgrammerError::UnexpectedResponse);
            }

            match (expected_generation, expected_total) {
                (None, None) => {
                    expected_generation = Some(page.generation());
                    expected_total = Some(page.total_objects());
                    objects.reserve(usize::from(page.total_objects()));
                }
                (Some(generation), Some(total))
                    if generation == page.generation() && total == page.total_objects() => {}
                _ => return Err(ProgrammerError::UnexpectedResponse),
            }

            if page.objects().is_empty() && offset < page.total_objects() {
                return Err(ProgrammerError::UnexpectedResponse);
            }
            for descriptor in page.objects() {
                let key = ObjectKey {
                    kind: ObjectKind::try_from(descriptor.kind)?,
                    id: descriptor.id,
                };
                if descriptor.encoded_len > self.capabilities.max_object_size
                    || objects.last().is_some_and(|previous| previous.key >= key)
                {
                    return Err(ProgrammerError::UnexpectedResponse);
                }
                objects.push(ListedObject {
                    key,
                    encoded_len: descriptor.encoded_len,
                });
            }

            let returned = u16::try_from(page.objects().len())
                .map_err(|_| ProgrammerError::UnexpectedResponse)?;
            offset = offset
                .checked_add(returned)
                .ok_or(ProgrammerError::UnexpectedResponse)?;
            if offset == page.total_objects() {
                break;
            }
        }

        Ok(ObjectListing {
            generation: expected_generation.ok_or(ProgrammerError::UnexpectedResponse)?,
            objects,
        })
    }

    /// Reads every active object from one stable listed generation.
    pub fn read_configuration(
        &mut self,
    ) -> Result<ConfigurationSnapshot, ProgrammerError<T::Error>> {
        let listing = self.list_objects()?;
        let mut objects = Vec::with_capacity(listing.objects.len());
        for listed in &listing.objects {
            let object = self.read_object(listed.key)?;
            if object.len() != usize::from(listed.encoded_len) {
                return Err(ProgrammerError::UnexpectedResponse);
            }
            objects.push(object);
        }
        if self.list_objects()? != listing {
            return Err(ProgrammerError::UnexpectedResponse);
        }
        Ok(ConfigurationSnapshot {
            generation: listing.generation,
            objects,
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
pub fn report_active_usage(usage: StorageUsage) -> CapacityReport {
    CapacityReport {
        object_count: usage.object_count,
        // What a device charges for its objects, headers included, so an
        // active snapshot and a compiled project are measured the same way.
        storage_bytes: usage.payload_bytes.saturating_add(
            u32::from(usage.object_count)
                .saturating_mul(u32::try_from(OBJECT_ENTRY_HEADER_LEN).unwrap_or(u32::MAX)),
        ),
        generated_channels: 0,
        explicit_channels: 0,
        banks: 0,
        has_radio_config: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompileError, ConfigurationCompiler, ConfigurationSnapshot, DeviceCapabilities,
        RadioProject, WRITE_ENVELOPE_LEN,
    };
    use radio_channel_plan::{BankName, ChannelBank, ChannelRecord, GeneratedBank, PlanEncoding};
    use radio_domain::{BankId, Frequency, FrequencyStep, RadioConfig};
    use radio_storage::{
        decode_configuration_image, encode_generated_bank, ObjectKey, ObjectKind, StorageError,
        StorageObject, GENERATED_BANK_CORE_LEN, OBJECT_ENTRY_HEADER_LEN, STORAGE_FORMAT_VERSION,
    };

    fn capabilities() -> DeviceCapabilities {
        DeviceCapabilities {
            protocol_version: 1,
            storage_version: STORAGE_FORMAT_VERSION,
            max_frame_payload: 128,
            max_objects: 1,
            max_object_size: 64,
            plan_encodings: PlanEncoding::LinearSimplex.capability_bit(),
            configuration_bytes: 4_096,
        }
    }

    /// A target declaring room for exactly `objects` generated banks.
    fn capabilities_for(objects: u16) -> DeviceCapabilities {
        DeviceCapabilities {
            configuration_bytes: u32::from(objects)
                * u32::try_from(OBJECT_ENTRY_HEADER_LEN + GENERATED_BANK_CORE_LEN).unwrap(),
            ..capabilities()
        }
    }

    fn bank(id: u16) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new("PMR446").unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
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
        assert_eq!(
            compiled.report().storage_bytes,
            u32::try_from(OBJECT_ENTRY_HEADER_LEN + GENERATED_BANK_CORE_LEN).unwrap()
        );
        assert_eq!(compiled.report().generated_channels, 16);
    }

    fn named_bank(id: u16) -> ChannelBank {
        ChannelBank::new(
            BankId::new(id),
            BankName::new("Amateur 2m").unwrap(),
            radio_channel_plan::BankFlags::default(),
        )
        .unwrap()
    }

    fn channel(id: u16, bank: u16) -> ChannelRecord {
        ChannelRecord::new(radio_channel_plan::ChannelDefinition {
            id: radio_domain::ChannelId::new(id),
            name: radio_channel_plan::ChannelName::new("GB3AB").unwrap(),
            receive: Frequency::from_hz(145_725_000).unwrap(),
            transmit: Frequency::from_hz(145_125_000).unwrap(),
            rx_tone: radio_domain::Tone::Ctcss(1_000),
            tx_tone: radio_domain::Tone::Ctcss(1_000),
            modulation: radio_domain::Modulation::Fm,
            bandwidth: radio_domain::Bandwidth::Wide,
            power: radio_domain::PowerLevel::Mid,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            flags: radio_channel_plan::ChannelFlags::default(),
            bank: Some(BankId::new(bank)),
        })
        .unwrap()
    }

    #[test]
    fn a_project_too_big_for_the_radio_says_how_big_it_is() {
        let mut project = RadioProject::new();
        project.add_bank(named_bank(0));
        project.add_channel(channel(0, 0));
        let fits = project.compile_radio_image().expect("radio image");
        assert_eq!(project.radio_image_len(), Ok(fits.len()));

        // Channels unlike each other, so none share their encoding.
        for id in 1..1_000 {
            project.add_channel(
                ChannelRecord::new(radio_channel_plan::ChannelDefinition {
                    id: radio_domain::ChannelId::new(id),
                    name: radio_channel_plan::ChannelName::new(&format!("CH{id}")).unwrap(),
                    receive: Frequency::from_hz(430_000_000 + u32::from(id) * 12_500).unwrap(),
                    transmit: Frequency::from_hz(440_000_000 + u32::from(id) * 25_000).unwrap(),
                    rx_tone: radio_domain::Tone::Ctcss(670 + id % 500),
                    tx_tone: radio_domain::Tone::Ctcss(700 + id % 500),
                    modulation: radio_domain::Modulation::Fm,
                    bandwidth: radio_domain::Bandwidth::Wide,
                    power: radio_domain::PowerLevel::Mid,
                    step: FrequencyStep::from_hz(12_500).unwrap(),
                    flags: radio_channel_plan::ChannelFlags::default(),
                    bank: Some(BankId::new(0)),
                })
                .unwrap(),
            );
        }
        assert!(matches!(
            project.compile_radio_image(),
            Err(super::RadioImageError::TooLarge)
        ));
        let len = project.radio_image_len().expect("measured");
        assert!(len > super::RADIO_IMAGE_BYTES, "{len}");
    }

    #[test]
    fn a_project_compiles_into_the_image_a_radio_runs() {
        let mut project = RadioProject::new();
        // A bank is a list or a plan, never both.
        project.add_generated_bank(bank(1));
        project.add_channel(channel(2, 0));
        project.add_channel(channel(1, 0));
        project.add_bank(named_bank(0));
        let image = project.compile_radio_image().expect("radio image");
        let mut header = [0_u8; radio_storage::radio_image::HEADER_LEN];
        let (head, payload) = image.split_at(header.len());
        header.copy_from_slice(head);
        let index = radio_storage::radio_image::Index::build(&header, |offset, buffer| {
            payload
                .get(offset..offset + buffer.len())
                .is_some_and(|bytes| {
                    buffer.copy_from_slice(bytes);
                    true
                })
        })
        .expect("the radio accepts it");
        assert_eq!(index.len(), 2 + bank(1).channel_count());
        assert!(image.len() <= usize::from(super::IMAGE_REGION_BYTES));
    }

    #[test]
    fn a_radio_image_reads_back_into_the_project_it_came_from() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_channel(channel(2, 0));
        project.add_bank(named_bank(0));
        let image = project.compile_radio_image().expect("radio image");
        let read = RadioProject::from_radio_image(&image).expect("reads back");
        assert_eq!(read.banks(), project.banks());
        assert_eq!(read.generated_banks(), project.generated_banks());
        assert_eq!(read.channels().len(), 1);
        assert_eq!(read.channels()[0].name(), project.channels()[0].name());
        assert_eq!(read.channels()[0].bank(), project.channels()[0].bank());
        assert_eq!(read.config(), Some(RadioConfig::conservative()));
        assert_eq!(read.compile_radio_image(), Ok(image.clone()));

        let mut damaged = image;
        damaged[20] ^= 1;
        assert!(RadioProject::from_radio_image(&damaged).is_err());
        assert!(RadioProject::from_radio_image(&[0; 4]).is_err());
    }

    const AMATEUR_70CM: radio_tx_policy::TxRange = radio_tx_policy::TxRange {
        class: radio_domain::TxClass::Amateur,
        first_hz: 430_000_000,
        last_hz: 440_000_000,
        max_mw: 5_000,
        wide_fm: true,
        raster_hz: 0,
    };

    #[test]
    fn tx_ranges_survive_the_radio_image_and_the_project_file() {
        let pmr = radio_tx_policy::TxRange {
            class: radio_domain::TxClass::LicenceFreePlan,
            first_hz: 446_006_250,
            last_hz: 446_193_750,
            max_mw: 500,
            wide_fm: false,
            raster_hz: 12_500,
        };
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_tx_range(AMATEUR_70CM);
        project.add_tx_range(pmr);
        assert!(!RadioProject {
            generated_banks: Vec::new(),
            ..project.clone()
        }
        .is_empty());

        let image = project.compile_radio_image().expect("radio image");
        let read = RadioProject::from_radio_image(&image).expect("reads back");
        assert_eq!(read.tx_ranges(), &[AMATEUR_70CM, pmr]);

        let compiled = ConfigurationCompiler::new(capabilities_for(8))
            .compile(&project)
            .unwrap();
        let kinds: Vec<_> = compiled
            .objects()
            .iter()
            .map(|object| (object.key().kind, object.key().id))
            .collect();
        assert!(kinds.contains(&(ObjectKind::TxRange, 0)));
        assert!(kinds.contains(&(ObjectKind::TxRange, 1)));
    }

    #[test]
    fn groups_survive_the_project_file_and_reach_the_radio_without_their_locator() {
        use radio_channel_plan::{BankGroup, BankSet, Locator};
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_channel(channel(2, 0));
        project.add_bank(named_bank(0));
        let both = BankSet::EMPTY.with(BankId::new(0)).with(BankId::new(1));
        let home = BankGroup::new(0, BankName::new("HOME").unwrap(), both)
            .unwrap()
            .with_locator(Some(Locator::new("JO02EH").unwrap()));
        project.add_group(home);
        assert!(!project.is_empty());

        let compiled = ConfigurationCompiler::new(capabilities_for(8))
            .compile(&project)
            .unwrap();
        assert!(compiled
            .objects()
            .iter()
            .any(|object| object.key().kind == ObjectKind::BankGroup));

        let image = project.compile_radio_image().expect("radio image");
        let read = RadioProject::from_radio_image(&image).expect("reads back");
        assert_eq!(read.groups(), &[home.with_locator(None)]);
    }

    #[test]
    fn a_banks_origin_survives_the_project_file_and_never_reaches_the_radio() {
        use radio_channel_plan::{BankOrigin, BankSource, Locator};
        let mut project = RadioProject::new();
        project.add_channel(channel(2, 0));
        project.add_bank(named_bank(0));
        let image = project.compile_radio_image().expect("radio image");
        let origin = BankOrigin::new(
            BankId::new(0),
            BankSource::Repeaters,
            Some(Locator::new("JO02EH").unwrap()),
        )
        .unwrap();
        project.add_origin(origin);
        assert_eq!(project.origins(), &[origin]);

        let compiled = ConfigurationCompiler::new(capabilities_for(8))
            .compile(&project)
            .unwrap();
        let kept: Vec<_> = compiled
            .objects()
            .iter()
            .filter(|object| object.key().kind == ObjectKind::BankOrigin)
            .map(|object| radio_storage::decode_bank_origin(object).unwrap())
            .collect();
        assert_eq!(kept, [origin]);
        assert_eq!(
            project.compile_radio_image().expect("radio image"),
            image,
            "the radio image is the same with or without it"
        );
    }

    #[test]
    fn a_bad_tx_range_is_refused_before_anything_is_written() {
        let mut project = RadioProject::new();
        project.add_tx_range(radio_tx_policy::TxRange {
            last_hz: 1,
            ..AMATEUR_70CM
        });
        assert_eq!(
            ConfigurationCompiler::new(capabilities_for(8)).compile(&project),
            Err(CompileError::TxRange(radio_tx_policy::RangeError::Span))
        );
        assert_eq!(
            project.compile_radio_image(),
            Err(super::RadioImageError::TxRange(
                radio_tx_policy::RangeError::Span
            ))
        );
    }

    #[test]
    fn banked_projects_compile_into_one_canonical_configuration() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_channel(channel(2, 1));
        project.add_channel(channel(1, 1));
        project.add_bank(named_bank(1));
        project.set_config(RadioConfig::conservative());
        assert!(!project.is_empty());

        let compiled = ConfigurationCompiler::new(capabilities_for(8))
            .compile(&project)
            .unwrap();
        let report = compiled.report();
        assert_eq!(report.object_count, 5);
        assert_eq!(report.generated_channels, 16);
        assert_eq!(report.explicit_channels, 2);
        assert_eq!(report.banks, 1);
        assert!(report.has_radio_config);

        let keys = compiled
            .objects()
            .iter()
            .map(|object| (object.key().kind, object.key().id))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                (ObjectKind::GeneratedBank, 1),
                (ObjectKind::Channel, 1),
                (ObjectKind::Channel, 2),
                (ObjectKind::ChannelBank, 1),
                (ObjectKind::RadioConfig, 0),
            ]
        );

        let mut image = vec![0; compiled.image_len().unwrap()];
        compiled.encode_image(&mut image).unwrap();
        assert_eq!(
            ConfigurationCompiler::new(capabilities_for(8))
                .decode_image(&image)
                .unwrap()
                .report(),
            report
        );
    }

    #[test]
    fn channels_cannot_reference_an_undefined_bank() {
        let mut project = RadioProject::new();
        project.add_channel(channel(1, 2));
        project.add_bank(named_bank(1));
        assert_eq!(
            ConfigurationCompiler::new(capabilities_for(8)).compile(&project),
            Err(CompileError::UnknownBank {
                channel: 1,
                bank: 2
            })
        );
    }

    #[test]
    fn a_channel_must_agree_with_what_its_bank_shares() {
        // The test channel is wide FM at medium power, 600 kHz below.
        let shares = |setting: u8, settings: radio_channel_plan::BankSettings| {
            let mut project = RadioProject::new();
            project.add_channel(channel(1, 1));
            project.add_bank(
                named_bank(1)
                    .with_settings(radio_channel_plan::BankSettings {
                        shared: radio_channel_plan::SharedSettings::none().with(setting, true),
                        ..settings
                    })
                    .unwrap(),
            );
            ConfigurationCompiler::new(capabilities_for(8)).compile(&project)
        };
        let base = radio_channel_plan::BankSettings::unshared();
        let agreeing = radio_channel_plan::BankSettings {
            bandwidth: radio_domain::Bandwidth::Wide,
            offset: radio_domain::Offset::from_hz(-600_000),
            ..base
        };
        assert!(shares(radio_channel_plan::SharedSettings::BANDWIDTH, agreeing).is_ok());
        assert!(shares(radio_channel_plan::SharedSettings::OFFSET, agreeing).is_ok());
        assert_eq!(
            shares(radio_channel_plan::SharedSettings::POWER, base),
            Err(CompileError::BankSetting {
                channel: 1,
                bank: 1
            })
        );
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

    #[test]
    fn compiler_images_are_canonical_across_project_order() {
        let compiler = ConfigurationCompiler::new(capabilities_for(2));
        let mut forward = RadioProject::new();
        forward.add_generated_bank(bank(1));
        forward.add_generated_bank(bank(2));
        let mut reverse = RadioProject::new();
        reverse.add_generated_bank(bank(2));
        reverse.add_generated_bank(bank(1));

        let forward = compiler.compile(&forward).unwrap();
        let reverse = compiler.compile(&reverse).unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .objects()
                .iter()
                .map(|object| object.key().id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );

        let mut forward_image = vec![0; forward.image_len().unwrap()];
        let mut reverse_image = vec![0; reverse.image_len().unwrap()];
        forward.encode_image(&mut forward_image).unwrap();
        reverse.encode_image(&mut reverse_image).unwrap();
        assert_eq!(forward_image, reverse_image);
    }

    #[test]
    fn compiler_image_round_trip_preserves_objects_and_capacity() {
        let compiler = ConfigurationCompiler::new(capabilities_for(2));
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(2));
        project.add_generated_bank(bank(1));
        let configuration = compiler.compile(&project).unwrap();
        let mut image = vec![0; configuration.image_len().unwrap()];
        configuration.encode_image(&mut image).unwrap();

        assert_eq!(compiler.decode_image(&image).unwrap(), configuration);
        image[20] ^= 1;
        assert_eq!(
            compiler.decode_image(&image),
            Err(CompileError::Storage(
                radio_storage::StorageError::ImageIntegrity
            ))
        );
    }

    #[test]
    fn image_import_enforces_every_target_capability() {
        let source_compiler = ConfigurationCompiler::new(capabilities_for(2));
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1));
        project.add_generated_bank(bank(2));
        let compiled = source_compiler.compile(&project).unwrap();
        let mut image = vec![0; compiled.image_len().unwrap()];
        compiled.encode_image(&mut image).unwrap();

        let one_bank = capabilities_for(1);
        assert_eq!(
            ConfigurationCompiler::new(one_bank).decode_image(&image),
            Err(CompileError::ConfigurationTooLarge {
                needed: one_bank.configuration_bytes * 2,
                available: one_bank.configuration_bytes,
            })
        );
        let mut limits = capabilities_for(2);
        limits.max_object_size = 30;
        assert_eq!(
            ConfigurationCompiler::new(limits).decode_image(&image),
            Err(CompileError::ObjectTooLarge)
        );
        limits = capabilities_for(2);
        limits.max_frame_payload = u16::try_from(31 + WRITE_ENVELOPE_LEN - 1).unwrap();
        assert_eq!(
            ConfigurationCompiler::new(limits).decode_image(&image),
            Err(CompileError::ObjectTooLarge)
        );
        limits = capabilities_for(2);
        limits.plan_encodings = 0;
        assert_eq!(
            ConfigurationCompiler::new(limits).decode_image(&image),
            Err(CompileError::UnsupportedPlanEncoding(
                PlanEncoding::LinearSimplex
            ))
        );
        limits = capabilities_for(2);
        limits.storage_version += 1;
        assert_eq!(
            ConfigurationCompiler::new(limits).decode_image(&image),
            Err(CompileError::UnsupportedStorageVersion)
        );
    }

    #[test]
    fn snapshot_backup_is_validated_canonical_and_capacity_exact() {
        let first = encode_generated_bank(bank(1)).unwrap();
        let second = encode_generated_bank(bank(2)).unwrap();
        let snapshot = ConfigurationSnapshot {
            generation: 9,
            objects: vec![first, second],
        };
        assert_eq!(
            snapshot.report().unwrap(),
            super::CapacityReport {
                object_count: 2,
                storage_bytes: u32::try_from(
                    (OBJECT_ENTRY_HEADER_LEN + GENERATED_BANK_CORE_LEN) * 2
                )
                .unwrap(),
                generated_channels: 32,
                ..super::CapacityReport::default()
            }
        );

        let mut image = vec![0; snapshot.image_len().unwrap()];
        assert_eq!(snapshot.encode_image(&mut image).unwrap(), image.len());
        assert_eq!(
            decode_configuration_image(&image)
                .unwrap()
                .objects()
                .collect::<Vec<_>>(),
            snapshot.objects
        );
        let mut second_encoding = vec![0; image.len()];
        snapshot.encode_image(&mut second_encoding).unwrap();
        assert_eq!(second_encoding, image);

        let reversed = ConfigurationSnapshot {
            generation: 9,
            objects: vec![second, first],
        };
        assert_eq!(reversed.report(), Err(StorageError::NonCanonicalImage));
        let malformed = ConfigurationSnapshot {
            generation: 9,
            objects: vec![StorageObject::new(
                ObjectKey {
                    kind: ObjectKind::GeneratedBank,
                    id: 3,
                },
                &[0],
            )
            .unwrap()],
        };
        assert_eq!(malformed.report(), Err(StorageError::MalformedObject));
        assert_eq!(
            snapshot.encode_image(&mut [0; 1]),
            Err(StorageError::ImageBufferTooSmall)
        );
    }
}
