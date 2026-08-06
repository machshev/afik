//! Bounded object encoding and logically transactional configuration storage.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use radio_channel_plan::{
    BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
    ChannelRecord, GeneratedBank, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN,
};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    RadioFlags, ScanResume, SquelchLevel, Tone, TxClass,
};

/// Current object encoding version.
pub const STORAGE_FORMAT_VERSION: u8 = 1;
/// Maximum bytes held by one device object in the first storage model.
pub const MAX_OBJECT_DATA: usize = 64;
/// Encoded byte length of a version-1 generated-bank object.
pub const GENERATED_BANK_ENCODED_LEN: usize = 31;
/// Encoded byte length of a version-1 explicit channel object.
pub const CHANNEL_ENCODED_LEN: usize = 42;
/// Encoded byte length of a version-1 named channel-bank object.
pub const CHANNEL_BANK_ENCODED_LEN: usize = 22;
/// Encoded byte length of a version-1 global radio-configuration object.
pub const RADIO_CONFIG_ENCODED_LEN: usize = 16;
/// Stable identifier of the single global radio-configuration object.
pub const RADIO_CONFIG_OBJECT_ID: u16 = 0;
/// Magic bytes at the start of every canonical configuration image.
pub const CONFIGURATION_IMAGE_MAGIC: [u8; 4] = *b"AFIK";
/// Current canonical configuration-image container version.
pub const CONFIGURATION_IMAGE_VERSION: u8 = 1;
/// Encoded byte length of a canonical configuration-image header.
pub const CONFIGURATION_IMAGE_HEADER_LEN: usize = 16;
/// Encoded byte length of an object envelope inside a configuration image.
pub const CONFIGURATION_IMAGE_OBJECT_HEADER_LEN: usize = 5;

/// Stable configuration object kinds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ObjectKind {
    /// A compact generated channel bank.
    GeneratedBank = 1,
    /// One explicit channel record.
    Channel = 2,
    /// Named metadata for one channel bank.
    ChannelBank = 3,
    /// The single global radio configuration.
    RadioConfig = 4,
}

impl TryFrom<u8> for ObjectKind {
    type Error = StorageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::GeneratedBank),
            2 => Ok(Self::Channel),
            3 => Ok(Self::ChannelBank),
            4 => Ok(Self::RadioConfig),
            _ => Err(StorageError::UnsupportedObject),
        }
    }
}

/// Stable object identity within the configuration store.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObjectKey {
    /// Object kind.
    pub kind: ObjectKind,
    /// Kind-local numeric identifier.
    pub id: u16,
}

/// A fixed-capacity encoded configuration object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageObject {
    key: ObjectKey,
    len: u8,
    data: [u8; MAX_OBJECT_DATA],
}

impl StorageObject {
    /// Copies encoded object data into a bounded object.
    pub fn new(key: ObjectKey, data: &[u8]) -> Result<Self, StorageError> {
        let len = u8::try_from(data.len()).map_err(|_| StorageError::ObjectTooLarge)?;
        if data.len() > MAX_OBJECT_DATA {
            return Err(StorageError::ObjectTooLarge);
        }
        let mut bytes = [0; MAX_OBJECT_DATA];
        bytes[..data.len()].copy_from_slice(data);
        Ok(Self {
            key,
            len,
            data: bytes,
        })
    }

    /// Returns the object identity.
    pub const fn key(self) -> ObjectKey {
        self.key
    }

    /// Returns the encoded object bytes.
    pub fn data(&self) -> &[u8] {
        &self.data[..usize::from(self.len)]
    }

    /// Returns the encoded object length.
    pub const fn len(self) -> usize {
        self.len as usize
    }

    /// Reports whether the object data is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Storage transaction or object-format failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    /// The object does not fit the bounded representation.
    ObjectTooLarge,
    /// The fixed object table has no free slot.
    StoreFull,
    /// A transaction is already open.
    TransactionAlreadyOpen,
    /// An operation requires an open transaction.
    NoTransaction,
    /// The candidate has not passed validation since its last write.
    CandidateNotValidated,
    /// Candidate validation rejected an object or the complete set.
    ValidationFailed,
    /// The active generation counter cannot advance.
    GenerationOverflow,
    /// The requested object is absent.
    ObjectNotFound,
    /// The object kind or format version is unsupported.
    UnsupportedObject,
    /// Encoded object bytes are malformed.
    MalformedObject,
    /// A caller-provided image buffer cannot hold the complete encoding.
    ImageBufferTooSmall,
    /// An image length or object count exceeds the format representation.
    ImageTooLarge,
    /// A configuration image has an invalid magic or structural length.
    MalformedImage,
    /// A configuration image uses an unsupported container or object version.
    UnsupportedImageVersion,
    /// A configuration image failed its CRC-32 integrity check.
    ImageIntegrity,
    /// Image objects are not in strict canonical stable-key order.
    NonCanonicalImage,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObjectTooLarge => formatter.write_str("configuration object is too large"),
            Self::StoreFull => formatter.write_str("configuration object table is full"),
            Self::TransactionAlreadyOpen => formatter.write_str("transaction already open"),
            Self::NoTransaction => formatter.write_str("no transaction is open"),
            Self::CandidateNotValidated => formatter.write_str("candidate is not validated"),
            Self::ValidationFailed => formatter.write_str("candidate validation failed"),
            Self::GenerationOverflow => formatter.write_str("storage generation overflow"),
            Self::ObjectNotFound => formatter.write_str("configuration object not found"),
            Self::UnsupportedObject => formatter.write_str("unsupported configuration object"),
            Self::MalformedObject => formatter.write_str("malformed configuration object"),
            Self::ImageBufferTooSmall => {
                formatter.write_str("configuration image buffer is too small")
            }
            Self::ImageTooLarge => formatter.write_str("configuration image is too large"),
            Self::MalformedImage => formatter.write_str("malformed configuration image"),
            Self::UnsupportedImageVersion => {
                formatter.write_str("unsupported configuration image version")
            }
            Self::ImageIntegrity => {
                formatter.write_str("configuration image integrity check failed")
            }
            Self::NonCanonicalImage => {
                formatter.write_str("configuration image object order is not canonical")
            }
        }
    }
}

/// Object-count and encoded-payload usage for one snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageUsage {
    /// Number of occupied object slots.
    pub object_count: u16,
    /// Sum of encoded object payload bytes.
    pub payload_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectSet<const OBJECTS: usize> {
    slots: [Option<StorageObject>; OBJECTS],
}

impl<const OBJECTS: usize> ObjectSet<OBJECTS> {
    const fn empty() -> Self {
        Self {
            slots: [None; OBJECTS],
        }
    }

    fn write(&mut self, object: StorageObject) -> Result<(), StorageError> {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_some_and(|stored| stored.key == object.key))
        {
            *slot = Some(object);
            return Ok(());
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(StorageError::StoreFull)?;
        *slot = Some(object);
        Ok(())
    }

    fn read(&self, key: ObjectKey) -> Option<&StorageObject> {
        self.slots.iter().flatten().find(|object| object.key == key)
    }

    fn iter(&self) -> impl Iterator<Item = &StorageObject> {
        self.slots.iter().flatten()
    }

    fn usage(&self) -> StorageUsage {
        self.iter()
            .fold(StorageUsage::default(), |mut usage, object| {
                usage.object_count += 1;
                usage.payload_bytes += u32::from(object.len);
                usage
            })
    }
}

/// A bounded store with isolated active and candidate snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionalStore<const OBJECTS: usize> {
    active: ObjectSet<OBJECTS>,
    candidate: Option<ObjectSet<OBJECTS>>,
    candidate_validated: bool,
    generation: u32,
}

impl<const OBJECTS: usize> Default for TransactionalStore<OBJECTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const OBJECTS: usize> TransactionalStore<OBJECTS> {
    /// Constructs an empty generation-zero store.
    pub const fn new() -> Self {
        Self {
            active: ObjectSet::empty(),
            candidate: None,
            candidate_validated: false,
            generation: 0,
        }
    }

    /// Returns the active snapshot generation.
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Starts a transaction from an exact copy of the active snapshot.
    pub fn begin(&mut self) -> Result<(), StorageError> {
        if self.candidate.is_some() {
            return Err(StorageError::TransactionAlreadyOpen);
        }
        self.candidate = Some(self.active);
        self.candidate_validated = false;
        Ok(())
    }

    /// Adds or replaces an object in the candidate snapshot.
    pub fn write(&mut self, object: StorageObject) -> Result<(), StorageError> {
        self.candidate
            .as_mut()
            .ok_or(StorageError::NoTransaction)?
            .write(object)?;
        self.candidate_validated = false;
        Ok(())
    }

    /// Validates every candidate object and marks the unchanged candidate valid.
    pub fn validate<F>(&mut self, mut validator: F) -> Result<(), StorageError>
    where
        F: FnMut(&StorageObject) -> bool,
    {
        let candidate = self.candidate.as_ref().ok_or(StorageError::NoTransaction)?;
        if !candidate.iter().all(&mut validator) {
            self.candidate_validated = false;
            return Err(StorageError::ValidationFailed);
        }
        self.candidate_validated = true;
        Ok(())
    }

    /// Atomically replaces the active snapshot with a validated candidate.
    pub fn commit(&mut self) -> Result<u32, StorageError> {
        if !self.candidate_validated {
            return Err(if self.candidate.is_some() {
                StorageError::CandidateNotValidated
            } else {
                StorageError::NoTransaction
            });
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(StorageError::GenerationOverflow)?;
        self.active = self.candidate.take().ok_or(StorageError::NoTransaction)?;
        self.candidate_validated = false;
        self.generation = generation;
        Ok(generation)
    }

    /// Discards an open candidate without affecting active data.
    pub fn abort(&mut self) -> Result<(), StorageError> {
        if self.candidate.take().is_none() {
            return Err(StorageError::NoTransaction);
        }
        self.candidate_validated = false;
        Ok(())
    }

    /// Reads an object from the active snapshot.
    pub fn read(&self, key: ObjectKey) -> Result<&StorageObject, StorageError> {
        self.active.read(key).ok_or(StorageError::ObjectNotFound)
    }

    /// Iterates over active objects without exposing candidate data.
    ///
    /// The store does not define an external ordering; protocol users must sort
    /// stable object keys before serialising a listing.
    pub fn active_objects(&self) -> impl Iterator<Item = &StorageObject> {
        self.active.iter()
    }

    /// Reports active-snapshot usage.
    pub fn usage(&self) -> StorageUsage {
        self.active.usage()
    }
}

/// Encodes a generated bank as a versioned storage object.
pub fn encode_generated_bank(bank: GeneratedBank) -> Result<StorageObject, StorageError> {
    let mut data = [0_u8; GENERATED_BANK_ENCODED_LEN];
    data[0] = STORAGE_FORMAT_VERSION;
    data[1..3].copy_from_slice(&bank.id().get().to_le_bytes());
    data[3] = bank.name().len();
    data[4..4 + MAX_BANK_NAME_LEN].copy_from_slice(&bank.name().field());
    data[20..24].copy_from_slice(&bank.base().as_hz().to_le_bytes());
    data[24..28].copy_from_slice(&bank.spacing().as_hz().to_le_bytes());
    data[28..30].copy_from_slice(&bank.channel_count().to_le_bytes());
    data[30] = bank.tx_class() as u8;
    StorageObject::new(
        ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: bank.id().get(),
        },
        &data,
    )
}

/// Decodes and fully validates a generated-bank storage object.
pub fn decode_generated_bank(object: &StorageObject) -> Result<GeneratedBank, StorageError> {
    if object.key.kind != ObjectKind::GeneratedBank {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() != GENERATED_BANK_ENCODED_LEN || data[0] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[1], data[2]]);
    if id != object.key.id {
        return Err(StorageError::MalformedObject);
    }
    let mut name_field = [0_u8; MAX_BANK_NAME_LEN];
    name_field.copy_from_slice(&data[4..20]);
    let name =
        BankName::from_field(name_field, data[3]).map_err(|_| StorageError::MalformedObject)?;
    let base = Frequency::from_hz(u32::from_le_bytes([data[20], data[21], data[22], data[23]]))
        .map_err(|_| StorageError::MalformedObject)?;
    let spacing =
        FrequencyStep::from_hz(u32::from_le_bytes([data[24], data[25], data[26], data[27]]))
            .map_err(|_| StorageError::MalformedObject)?;
    let channel_count = u16::from_le_bytes([data[28], data[29]]);
    let tx_class = TxClass::try_from(data[30]).map_err(|_| StorageError::MalformedObject)?;
    GeneratedBank::linear_simplex(
        BankId::new(id),
        name,
        base,
        spacing,
        channel_count,
        tx_class,
    )
    .map_err(|_| StorageError::MalformedObject)
}

/// Encodes an explicit channel as a versioned storage object.
pub fn encode_channel(channel: ChannelRecord) -> Result<StorageObject, StorageError> {
    let mut data = [0_u8; CHANNEL_ENCODED_LEN];
    data[0] = STORAGE_FORMAT_VERSION;
    data[1..3].copy_from_slice(&channel.id().get().to_le_bytes());
    data[3] = channel.name().len();
    data[4..4 + MAX_CHANNEL_NAME_LEN].copy_from_slice(&channel.name().field());
    data[16..20].copy_from_slice(&channel.receive().as_hz().to_le_bytes());
    data[20..24].copy_from_slice(&channel.transmit().as_hz().to_le_bytes());
    encode_tone(channel.rx_tone(), &mut data[24..27]);
    encode_tone(channel.tx_tone(), &mut data[27..30]);
    data[30] = channel.modulation() as u8;
    data[31] = channel.bandwidth() as u8;
    data[32] = channel.power() as u8;
    data[33..37].copy_from_slice(&channel.step().as_hz().to_le_bytes());
    data[37] = channel.squelch().get();
    data[38] = channel.flags().bits();
    data[39] = channel.tx_class() as u8;
    data[40..42].copy_from_slice(&channel.banks().bits().to_le_bytes());
    StorageObject::new(
        ObjectKey {
            kind: ObjectKind::Channel,
            id: channel.id().get(),
        },
        &data,
    )
}

/// Decodes and fully validates an explicit channel storage object.
pub fn decode_channel(object: &StorageObject) -> Result<ChannelRecord, StorageError> {
    if object.key.kind != ObjectKind::Channel {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() != CHANNEL_ENCODED_LEN || data[0] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[1], data[2]]);
    if id != object.key.id {
        return Err(StorageError::MalformedObject);
    }
    let mut name_field = [0_u8; MAX_CHANNEL_NAME_LEN];
    name_field.copy_from_slice(&data[4..16]);
    let name =
        ChannelName::from_field(name_field, data[3]).map_err(|_| StorageError::MalformedObject)?;
    let receive = Frequency::from_hz(u32::from_le_bytes([data[16], data[17], data[18], data[19]]))
        .map_err(|_| StorageError::MalformedObject)?;
    let transmit = Frequency::from_hz(u32::from_le_bytes([data[20], data[21], data[22], data[23]]))
        .map_err(|_| StorageError::MalformedObject)?;
    let rx_tone = decode_tone(&data[24..27])?;
    let tx_tone = decode_tone(&data[27..30])?;
    let modulation = Modulation::try_from(data[30]).map_err(|_| StorageError::MalformedObject)?;
    let bandwidth = Bandwidth::try_from(data[31]).map_err(|_| StorageError::MalformedObject)?;
    let power = PowerLevel::try_from(data[32]).map_err(|_| StorageError::MalformedObject)?;
    let step = FrequencyStep::from_hz(u32::from_le_bytes([data[33], data[34], data[35], data[36]]))
        .map_err(|_| StorageError::MalformedObject)?;
    let squelch = SquelchLevel::new(data[37]).map_err(|_| StorageError::MalformedObject)?;
    let flags = ChannelFlags::from_bits(data[38]).map_err(|_| StorageError::MalformedObject)?;
    let tx_class = TxClass::try_from(data[39]).map_err(|_| StorageError::MalformedObject)?;
    let banks = BankMask::from_bits(u16::from_le_bytes([data[40], data[41]]));
    ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(id),
        name,
        receive,
        transmit,
        rx_tone,
        tx_tone,
        modulation,
        bandwidth,
        power,
        step,
        squelch,
        flags,
        banks,
        tx_class,
    })
    .map_err(|_| StorageError::MalformedObject)
}

/// Encodes named channel-bank metadata as a versioned storage object.
pub fn encode_channel_bank(bank: ChannelBank) -> Result<StorageObject, StorageError> {
    let mut data = [0_u8; CHANNEL_BANK_ENCODED_LEN];
    data[0] = STORAGE_FORMAT_VERSION;
    data[1..3].copy_from_slice(&bank.id().get().to_le_bytes());
    data[3] = bank.name().len();
    data[4..4 + MAX_BANK_NAME_LEN].copy_from_slice(&bank.name().field());
    data[20] = bank.flags().bits();
    StorageObject::new(
        ObjectKey {
            kind: ObjectKind::ChannelBank,
            id: bank.id().get(),
        },
        &data,
    )
}

/// Decodes and fully validates a named channel-bank storage object.
pub fn decode_channel_bank(object: &StorageObject) -> Result<ChannelBank, StorageError> {
    if object.key.kind != ObjectKind::ChannelBank {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() != CHANNEL_BANK_ENCODED_LEN || data[0] != STORAGE_FORMAT_VERSION || data[21] != 0
    {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[1], data[2]]);
    if id != object.key.id {
        return Err(StorageError::MalformedObject);
    }
    let mut name_field = [0_u8; MAX_BANK_NAME_LEN];
    name_field.copy_from_slice(&data[4..20]);
    let name =
        BankName::from_field(name_field, data[3]).map_err(|_| StorageError::MalformedObject)?;
    let flags = BankFlags::from_bits(data[20]).map_err(|_| StorageError::MalformedObject)?;
    ChannelBank::new(BankId::new(id), name, flags).map_err(|_| StorageError::MalformedObject)
}

/// Encodes the global radio configuration as a versioned storage object.
pub fn encode_radio_config(config: RadioConfig) -> Result<StorageObject, StorageError> {
    let config = config
        .validate()
        .map_err(|_| StorageError::MalformedObject)?;
    let mut data = [0_u8; RADIO_CONFIG_ENCODED_LEN];
    data[0] = STORAGE_FORMAT_VERSION;
    data[1] = config.squelch.get();
    data[2] = config.backlight_seconds;
    data[3] = config.scan_resume as u8;
    data[4..8].copy_from_slice(&config.scan_dwell_ms.to_le_bytes());
    data[8..12].copy_from_slice(&config.scan_hold_ms.to_le_bytes());
    data[12] = u8::from(config.dual_watch);
    data[13] = config.battery_save_ratio;
    data[14] = config.flags.bits();
    StorageObject::new(
        ObjectKey {
            kind: ObjectKind::RadioConfig,
            id: RADIO_CONFIG_OBJECT_ID,
        },
        &data,
    )
}

/// Decodes and fully validates the global radio-configuration object.
pub fn decode_radio_config(object: &StorageObject) -> Result<RadioConfig, StorageError> {
    if object.key.kind != ObjectKind::RadioConfig {
        return Err(StorageError::UnsupportedObject);
    }
    if object.key.id != RADIO_CONFIG_OBJECT_ID {
        return Err(StorageError::MalformedObject);
    }
    let data = object.data();
    if data.len() != RADIO_CONFIG_ENCODED_LEN
        || data[0] != STORAGE_FORMAT_VERSION
        || data[15] != 0
        || data[12] > 1
    {
        return Err(StorageError::MalformedObject);
    }
    RadioConfig {
        squelch: SquelchLevel::new(data[1]).map_err(|_| StorageError::MalformedObject)?,
        backlight_seconds: data[2],
        scan_resume: ScanResume::try_from(data[3]).map_err(|_| StorageError::MalformedObject)?,
        scan_dwell_ms: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        scan_hold_ms: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        dual_watch: data[12] == 1,
        battery_save_ratio: data[13],
        flags: RadioFlags::from_bits(data[14]).map_err(|_| StorageError::MalformedObject)?,
    }
    .validate()
    .map_err(|_| StorageError::MalformedObject)
}

fn encode_tone(tone: Tone, field: &mut [u8]) {
    let (kind, code) = match tone {
        Tone::None => (0, 0),
        Tone::Ctcss(tenths_hz) => (1, tenths_hz),
        Tone::Dcs {
            code,
            inverted: false,
        } => (2, code),
        Tone::Dcs {
            code,
            inverted: true,
        } => (3, code),
    };
    field[0] = kind;
    field[1..3].copy_from_slice(&code.to_le_bytes());
}

fn decode_tone(field: &[u8]) -> Result<Tone, StorageError> {
    let code = u16::from_le_bytes([field[1], field[2]]);
    let tone = match field[0] {
        0 if code == 0 => Ok(Tone::None),
        1 => Tone::ctcss(code),
        2 => Tone::dcs(code, false),
        3 => Tone::dcs(code, true),
        _ => return Err(StorageError::MalformedObject),
    };
    tone.map_err(|_| StorageError::MalformedObject)
}

/// Validates any currently supported configuration object.
pub fn validate_object(object: &StorageObject) -> bool {
    match object.key.kind {
        ObjectKind::GeneratedBank => decode_generated_bank(object).is_ok(),
        ObjectKind::Channel => decode_channel(object).is_ok(),
        ObjectKind::ChannelBank => decode_channel_bank(object).is_ok(),
        ObjectKind::RadioConfig => decode_radio_config(object).is_ok(),
    }
}

/// A completely validated canonical configuration image borrowed from bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationImage<'a> {
    payload: &'a [u8],
    object_count: u16,
}

impl ConfigurationImage<'_> {
    /// Returns the number of encoded configuration objects.
    pub const fn object_count(&self) -> u16 {
        self.object_count
    }

    /// Iterates over validated objects in strict stable-key order.
    pub fn objects(&self) -> ConfigurationImageObjects<'_> {
        ConfigurationImageObjects {
            payload: self.payload,
            offset: 0,
            remaining: self.object_count,
        }
    }
}

/// Iterator over the objects of a validated canonical configuration image.
#[derive(Clone, Debug)]
pub struct ConfigurationImageObjects<'a> {
    payload: &'a [u8],
    offset: usize,
    remaining: u16,
}

impl Iterator for ConfigurationImageObjects<'_> {
    type Item = StorageObject;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (object, next_offset) = decode_image_entry(self.payload, self.offset).ok()?;
        self.offset = next_offset;
        self.remaining -= 1;
        Some(object)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.remaining);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ConfigurationImageObjects<'_> {}

/// Calculates the exact canonical image length for a complete object set.
pub fn configuration_image_len(objects: &[StorageObject]) -> Result<usize, StorageError> {
    let _object_count = u16::try_from(objects.len()).map_err(|_| StorageError::ImageTooLarge)?;
    let payload_len = objects.iter().try_fold(0_usize, |length, object| {
        length
            .checked_add(CONFIGURATION_IMAGE_OBJECT_HEADER_LEN)
            .and_then(|value| value.checked_add(object.len()))
            .ok_or(StorageError::ImageTooLarge)
    })?;
    let _encoded_payload_len =
        u32::try_from(payload_len).map_err(|_| StorageError::ImageTooLarge)?;
    CONFIGURATION_IMAGE_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(StorageError::ImageTooLarge)
}

/// Encodes a complete validated object set into a caller-provided buffer.
///
/// Objects must be in strict `(kind, id)` order. The returned length is the
/// only portion of `output` written by this function.
pub fn encode_configuration_image(
    objects: &[StorageObject],
    output: &mut [u8],
) -> Result<usize, StorageError> {
    validate_canonical_objects(objects)?;
    let image_len = configuration_image_len(objects)?;
    if output.len() < image_len {
        return Err(StorageError::ImageBufferTooSmall);
    }
    let object_count = u16::try_from(objects.len()).map_err(|_| StorageError::ImageTooLarge)?;
    let payload_len = image_len - CONFIGURATION_IMAGE_HEADER_LEN;
    let encoded_payload_len =
        u32::try_from(payload_len).map_err(|_| StorageError::ImageTooLarge)?;

    output[..4].copy_from_slice(&CONFIGURATION_IMAGE_MAGIC);
    output[4] = CONFIGURATION_IMAGE_VERSION;
    output[5] = STORAGE_FORMAT_VERSION;
    output[6..8].copy_from_slice(&object_count.to_le_bytes());
    output[8..12].copy_from_slice(&encoded_payload_len.to_le_bytes());
    output[12..16].fill(0);

    let mut offset = CONFIGURATION_IMAGE_HEADER_LEN;
    for object in objects {
        output[offset] = object.key.kind as u8;
        output[offset + 1..offset + 3].copy_from_slice(&object.key.id.to_le_bytes());
        let object_len = u16::try_from(object.len()).map_err(|_| StorageError::ObjectTooLarge)?;
        output[offset + 3..offset + 5].copy_from_slice(&object_len.to_le_bytes());
        offset += CONFIGURATION_IMAGE_OBJECT_HEADER_LEN;
        let data_end = offset + object.len();
        output[offset..data_end].copy_from_slice(object.data());
        offset = data_end;
    }

    let checksum = configuration_image_crc(&output[..12], &output[16..image_len]);
    output[12..16].copy_from_slice(&checksum.to_le_bytes());
    Ok(image_len)
}

/// Validates and borrows one exact canonical configuration image.
///
/// The complete checksum, structure, object order, and every object payload are
/// validated before the returned image exposes its object iterator.
pub fn decode_configuration_image(bytes: &[u8]) -> Result<ConfigurationImage<'_>, StorageError> {
    if bytes.len() < CONFIGURATION_IMAGE_HEADER_LEN || bytes[..4] != CONFIGURATION_IMAGE_MAGIC {
        return Err(StorageError::MalformedImage);
    }
    if bytes[4] != CONFIGURATION_IMAGE_VERSION || bytes[5] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::UnsupportedImageVersion);
    }
    let object_count = u16::from_le_bytes([bytes[6], bytes[7]]);
    let encoded_payload_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let payload_len =
        usize::try_from(encoded_payload_len).map_err(|_| StorageError::ImageTooLarge)?;
    let expected_len = CONFIGURATION_IMAGE_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(StorageError::ImageTooLarge)?;
    if bytes.len() != expected_len {
        return Err(StorageError::MalformedImage);
    }
    let expected_checksum = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if configuration_image_crc(&bytes[..12], &bytes[16..]) != expected_checksum {
        return Err(StorageError::ImageIntegrity);
    }

    let payload = &bytes[CONFIGURATION_IMAGE_HEADER_LEN..];
    let mut offset = 0;
    let mut previous_key = None;
    for _ in 0..object_count {
        let (object, next_offset) = decode_image_entry(payload, offset)?;
        if previous_key.is_some_and(|key| key >= object.key()) {
            return Err(StorageError::NonCanonicalImage);
        }
        if !validate_object(&object) {
            return Err(StorageError::MalformedObject);
        }
        previous_key = Some(object.key());
        offset = next_offset;
    }
    if offset != payload.len() {
        return Err(StorageError::MalformedImage);
    }
    Ok(ConfigurationImage {
        payload,
        object_count,
    })
}

fn validate_canonical_objects(objects: &[StorageObject]) -> Result<(), StorageError> {
    let mut previous_key = None;
    for object in objects {
        if previous_key.is_some_and(|key| key >= object.key()) {
            return Err(StorageError::NonCanonicalImage);
        }
        if !validate_object(object) {
            return Err(StorageError::MalformedObject);
        }
        previous_key = Some(object.key());
    }
    Ok(())
}

fn decode_image_entry(
    payload: &[u8],
    offset: usize,
) -> Result<(StorageObject, usize), StorageError> {
    let header_end = offset
        .checked_add(CONFIGURATION_IMAGE_OBJECT_HEADER_LEN)
        .filter(|end| *end <= payload.len())
        .ok_or(StorageError::MalformedImage)?;
    let kind = ObjectKind::try_from(payload[offset])?;
    let id = u16::from_le_bytes([payload[offset + 1], payload[offset + 2]]);
    let encoded_len = u16::from_le_bytes([payload[offset + 3], payload[offset + 4]]);
    let data_end = header_end
        .checked_add(usize::from(encoded_len))
        .filter(|end| *end <= payload.len())
        .ok_or(StorageError::MalformedImage)?;
    let object = StorageObject::new(ObjectKey { kind, id }, &payload[header_end..data_end])?;
    Ok((object, data_end))
}

fn configuration_image_crc(header: &[u8], payload: &[u8]) -> u32 {
    let crc = crc32_update(u32::MAX, header);
    !crc32_update(crc, payload)
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let low_bit_mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & low_bit_mask);
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        configuration_image_crc, configuration_image_len, decode_channel, decode_channel_bank,
        decode_configuration_image, decode_generated_bank, decode_radio_config, encode_channel,
        encode_channel_bank, encode_configuration_image, encode_generated_bank,
        encode_radio_config, validate_object, ObjectKey, ObjectKind, StorageError, StorageObject,
        TransactionalStore, CHANNEL_BANK_ENCODED_LEN, CHANNEL_ENCODED_LEN,
        CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_MAGIC, CONFIGURATION_IMAGE_VERSION,
        RADIO_CONFIG_ENCODED_LEN, STORAGE_FORMAT_VERSION,
    };
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord, GeneratedBank,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, RadioFlags, ScanResume, SquelchLevel, Tone, TxClass,
    };
    use std::{vec, vec::Vec};

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

    fn raw_image(objects: &[&[u8]]) -> Vec<u8> {
        let payload_len = objects.iter().map(|object| object.len()).sum::<usize>();
        let mut image = vec![0; CONFIGURATION_IMAGE_HEADER_LEN + payload_len];
        image[..4].copy_from_slice(&CONFIGURATION_IMAGE_MAGIC);
        image[4] = CONFIGURATION_IMAGE_VERSION;
        image[5] = STORAGE_FORMAT_VERSION;
        image[6..8].copy_from_slice(
            &u16::try_from(objects.len())
                .expect("test object count")
                .to_le_bytes(),
        );
        image[8..12].copy_from_slice(
            &u32::try_from(payload_len)
                .expect("test payload length")
                .to_le_bytes(),
        );
        let mut offset = CONFIGURATION_IMAGE_HEADER_LEN;
        for object in objects {
            let end = offset + object.len();
            image[offset..end].copy_from_slice(object);
            offset = end;
        }
        let checksum = configuration_image_crc(&image[..12], &image[16..]);
        image[12..16].copy_from_slice(&checksum.to_le_bytes());
        image
    }

    fn image_entry(object: &super::StorageObject) -> Vec<u8> {
        let mut entry = Vec::with_capacity(5 + object.len());
        entry.push(object.key().kind as u8);
        entry.extend_from_slice(&object.key().id.to_le_bytes());
        entry.extend_from_slice(
            &u16::try_from(object.len())
                .expect("test object length")
                .to_le_bytes(),
        );
        entry.extend_from_slice(object.data());
        entry
    }

    #[test]
    fn generated_bank_encoding_round_trips() {
        let expected = bank(4, "PMR446");
        let encoded = encode_generated_bank(expected).unwrap();
        assert_eq!(decode_generated_bank(&encoded).unwrap(), expected);
    }

    fn channel(id: u16, name: &str) -> ChannelRecord {
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new(name).unwrap(),
            receive: Frequency::from_hz(145_725_000).unwrap(),
            transmit: Frequency::from_hz(145_125_000).unwrap(),
            rx_tone: Tone::Ctcss(1_000),
            tx_tone: Tone::Dcs {
                code: 23,
                inverted: true,
            },
            modulation: Modulation::Usb,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::High,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            squelch: SquelchLevel::new(9).unwrap(),
            flags: ChannelFlags::from_bits(ChannelFlags::SCAN_SKIP | ChannelFlags::REVERSE)
                .unwrap(),
            banks: BankMask::from_bits(0b0000_0000_0000_0101),
            tx_class: TxClass::Amateur,
        })
        .unwrap()
    }

    #[test]
    fn channel_bank_and_configuration_objects_round_trip() {
        let expected = channel(12, "GB3AB");
        let encoded = encode_channel(expected).unwrap();
        assert_eq!(encoded.len(), CHANNEL_ENCODED_LEN);
        assert_eq!(
            encoded.key(),
            ObjectKey {
                kind: ObjectKind::Channel,
                id: 12
            }
        );
        assert_eq!(decode_channel(&encoded).unwrap(), expected);
        assert!(validate_object(&encoded));

        let bank = ChannelBank::new(
            BankId::new(3),
            BankName::new("Amateur 2m").unwrap(),
            BankFlags::default().with(BankFlags::SCAN_ENABLED, true),
        )
        .unwrap();
        let encoded_bank = encode_channel_bank(bank).unwrap();
        assert_eq!(encoded_bank.len(), CHANNEL_BANK_ENCODED_LEN);
        assert_eq!(decode_channel_bank(&encoded_bank).unwrap(), bank);
        assert!(validate_object(&encoded_bank));

        let mut config = RadioConfig::conservative();
        config.dual_watch = true;
        config.battery_save_ratio = 4;
        config.scan_resume = ScanResume::Carrier;
        config.flags = RadioFlags::default().with(RadioFlags::KEY_BEEP, true);
        let encoded_config = encode_radio_config(config).unwrap();
        assert_eq!(encoded_config.len(), RADIO_CONFIG_ENCODED_LEN);
        assert_eq!(decode_radio_config(&encoded_config).unwrap(), config);
        assert!(validate_object(&encoded_config));
    }

    #[test]
    fn channel_and_configuration_decoding_rejects_every_invalid_field() {
        let encoded = encode_channel(channel(12, "GB3AB")).unwrap();
        let mutate = |index: usize, value: u8| {
            let mut data = encoded.data().to_vec();
            data[index] = value;
            StorageObject::new(encoded.key(), &data).unwrap()
        };

        // version, identity, name length, tone kind, tone code, modulation,
        // bandwidth, power, squelch, reserved flag, and TX class are all checked.
        for (index, value) in [
            (0, STORAGE_FORMAT_VERSION + 1),
            (1, 13),
            (3, 13),
            (24, 4),
            (26, 0),
            (30, 3),
            (31, 2),
            (32, 3),
            (37, 10),
            (38, 0x10),
            (39, 7),
        ] {
            assert_eq!(
                decode_channel(&mutate(index, value)),
                Err(StorageError::MalformedObject),
                "index {index} value {value} was accepted"
            );
        }

        let mut zero_step = encoded.data().to_vec();
        zero_step[33..37].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            decode_channel(&StorageObject::new(encoded.key(), &zero_step).unwrap()),
            Err(StorageError::MalformedObject)
        );
        assert_eq!(
            decode_channel(&StorageObject::new(encoded.key(), &encoded.data()[..41]).unwrap()),
            Err(StorageError::MalformedObject)
        );
        assert_eq!(
            decode_channel(&encode_generated_bank(bank(1, "A")).unwrap()),
            Err(StorageError::UnsupportedObject)
        );

        let config = encode_radio_config(RadioConfig::conservative()).unwrap();
        let mutate_config = |index: usize, value: u8| {
            let mut data = config.data().to_vec();
            data[index] = value;
            StorageObject::new(config.key(), &data).unwrap()
        };
        for (index, value) in [(1, 10), (3, 3), (12, 2), (14, 0x10), (15, 1)] {
            assert_eq!(
                decode_radio_config(&mutate_config(index, value)),
                Err(StorageError::MalformedObject),
                "configuration index {index} value {value} was accepted"
            );
        }
        let mut zero_dwell = config.data().to_vec();
        zero_dwell[4..8].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            decode_radio_config(&StorageObject::new(config.key(), &zero_dwell).unwrap()),
            Err(StorageError::MalformedObject)
        );
        assert_eq!(
            decode_radio_config(
                &StorageObject::new(
                    ObjectKey {
                        kind: ObjectKind::RadioConfig,
                        id: 1
                    },
                    config.data()
                )
                .unwrap()
            ),
            Err(StorageError::MalformedObject)
        );

        let bank_object = encode_channel_bank(
            ChannelBank::new(
                BankId::new(3),
                BankName::new("Amateur 2m").unwrap(),
                BankFlags::default(),
            )
            .unwrap(),
        )
        .unwrap();
        let mut reserved = bank_object.data().to_vec();
        reserved[21] = 1;
        assert_eq!(
            decode_channel_bank(&StorageObject::new(bank_object.key(), &reserved).unwrap()),
            Err(StorageError::MalformedObject)
        );
        let mut reserved_flag = bank_object.data().to_vec();
        reserved_flag[20] = 0x02;
        assert_eq!(
            decode_channel_bank(&StorageObject::new(bank_object.key(), &reserved_flag).unwrap()),
            Err(StorageError::MalformedObject)
        );
    }

    #[test]
    fn canonical_images_order_every_object_kind() {
        let objects = &mut [
            encode_radio_config(RadioConfig::conservative()).unwrap(),
            encode_channel(channel(2, "B")).unwrap(),
            encode_generated_bank(bank(1, "A")).unwrap(),
            encode_channel_bank(
                ChannelBank::new(
                    BankId::new(0),
                    BankName::new("Bank 0").unwrap(),
                    BankFlags::default(),
                )
                .unwrap(),
            )
            .unwrap(),
        ];
        assert_eq!(
            encode_configuration_image(objects, &mut [0; 256]),
            Err(StorageError::NonCanonicalImage)
        );
        objects.sort_unstable_by_key(|object| object.key());
        let image_len = configuration_image_len(objects).unwrap();
        let mut image = vec![0; image_len];
        encode_configuration_image(objects, &mut image).unwrap();
        let decoded = decode_configuration_image(&image).unwrap();
        assert_eq!(decoded.object_count(), 4);
        assert_eq!(decoded.objects().collect::<Vec<_>>(), objects.to_vec());
    }

    #[test]
    fn candidate_is_invisible_until_validated_commit() {
        let key = ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: 4,
        };
        let mut store = TransactionalStore::<2>::new();
        store.begin().unwrap();
        store
            .write(encode_generated_bank(bank(4, "PMR446")).unwrap())
            .unwrap();
        assert_eq!(store.read(key), Err(StorageError::ObjectNotFound));
        assert_eq!(store.commit(), Err(StorageError::CandidateNotValidated));
        store.validate(validate_object).unwrap();
        assert_eq!(store.commit().unwrap(), 1);
        assert_eq!(
            decode_generated_bank(store.read(key).unwrap()).unwrap(),
            bank(4, "PMR446")
        );
    }

    #[test]
    fn abort_preserves_active_snapshot() {
        let key = ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: 4,
        };
        let mut store = TransactionalStore::<2>::new();
        store.begin().unwrap();
        store
            .write(encode_generated_bank(bank(4, "original")).unwrap())
            .unwrap();
        store.validate(validate_object).unwrap();
        store.commit().unwrap();

        store.begin().unwrap();
        store
            .write(encode_generated_bank(bank(4, "candidate")).unwrap())
            .unwrap();
        store.abort().unwrap();
        assert_eq!(
            decode_generated_bank(store.read(key).unwrap()).unwrap(),
            bank(4, "original")
        );
    }

    #[test]
    fn canonical_image_has_an_exact_format_and_round_trips() {
        let object = encode_generated_bank(bank(0x1234, "A")).unwrap();
        let image_len = configuration_image_len(&[object]).unwrap();
        let mut image = vec![0xAA; image_len + 1];
        assert_eq!(
            encode_configuration_image(&[object], &mut image).unwrap(),
            image_len
        );
        assert_eq!(image[image_len], 0xAA);
        assert_eq!(
            &image[..image_len],
            &[
                0x41, 0x46, 0x49, 0x4B, 0x01, 0x01, 0x01, 0x00, 0x24, 0x00, 0x00, 0x00, 0x04, 0xAC,
                0x45, 0x9C, 0x01, 0x34, 0x12, 0x1F, 0x00, 0x01, 0x34, 0x12, 0x01, 0x41, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEA,
                0x83, 0x95, 0x1A, 0xD4, 0x30, 0x00, 0x00, 0x10, 0x00, 0x01,
            ]
        );
        let decoded = decode_configuration_image(&image[..image_len]).unwrap();
        assert_eq!(decoded.object_count(), 1);
        assert_eq!(decoded.objects().collect::<Vec<_>>(), vec![object]);
    }

    #[test]
    fn empty_image_is_valid_and_buffers_are_bounded() {
        let mut image = [0_u8; CONFIGURATION_IMAGE_HEADER_LEN];
        assert_eq!(
            encode_configuration_image(&[], &mut image).unwrap(),
            CONFIGURATION_IMAGE_HEADER_LEN
        );
        assert_eq!(
            decode_configuration_image(&image).unwrap().objects().len(),
            0
        );
        assert_eq!(
            encode_configuration_image(&[], &mut image[..15]),
            Err(StorageError::ImageBufferTooSmall)
        );

        let maximum = (0..u16::MAX)
            .map(|id| encode_generated_bank(bank(id, "A")).unwrap())
            .collect::<Vec<_>>();
        let maximum_len = configuration_image_len(&maximum).unwrap();
        let mut maximum_image = vec![0; maximum_len];
        assert_eq!(
            encode_configuration_image(&maximum, &mut maximum_image).unwrap(),
            maximum_len
        );
        let decoded_maximum = decode_configuration_image(&maximum_image).unwrap();
        assert_eq!(decoded_maximum.object_count(), u16::MAX);
        assert_eq!(decoded_maximum.objects().len(), usize::from(u16::MAX));

        let repeated = vec![maximum[0]; usize::from(u16::MAX) + 1];
        assert_eq!(
            configuration_image_len(&repeated),
            Err(StorageError::ImageTooLarge)
        );
    }

    #[test]
    fn image_rejects_integrity_version_and_length_failures() {
        let object = encode_generated_bank(bank(1, "A")).unwrap();
        let mut image = vec![0; configuration_image_len(&[object]).unwrap()];
        encode_configuration_image(&[object], &mut image).unwrap();

        let mut bad_magic = image.clone();
        bad_magic[0] ^= 1;
        assert_eq!(
            decode_configuration_image(&bad_magic),
            Err(StorageError::MalformedImage)
        );
        let mut bad_version = image.clone();
        bad_version[4] += 1;
        assert_eq!(
            decode_configuration_image(&bad_version),
            Err(StorageError::UnsupportedImageVersion)
        );
        let mut bad_checksum = image.clone();
        bad_checksum[20] ^= 1;
        assert_eq!(
            decode_configuration_image(&bad_checksum),
            Err(StorageError::ImageIntegrity)
        );
        assert_eq!(
            decode_configuration_image(&image[..image.len() - 1]),
            Err(StorageError::MalformedImage)
        );
        let mut trailing = image;
        trailing.push(0);
        assert_eq!(
            decode_configuration_image(&trailing),
            Err(StorageError::MalformedImage)
        );
    }

    #[test]
    fn image_rejects_noncanonical_and_malformed_objects() {
        let first = encode_generated_bank(bank(1, "A")).unwrap();
        let second = encode_generated_bank(bank(2, "B")).unwrap();
        let first_entry = image_entry(&first);
        let second_entry = image_entry(&second);

        assert_eq!(
            decode_configuration_image(&raw_image(&[&second_entry, &first_entry])),
            Err(StorageError::NonCanonicalImage)
        );
        assert_eq!(
            decode_configuration_image(&raw_image(&[&first_entry, &first_entry])),
            Err(StorageError::NonCanonicalImage)
        );
        assert_eq!(
            encode_configuration_image(&[second, first], &mut [0; 128]),
            Err(StorageError::NonCanonicalImage)
        );

        let mut malformed_entry = first_entry;
        malformed_entry[5] = STORAGE_FORMAT_VERSION + 1;
        assert_eq!(
            decode_configuration_image(&raw_image(&[&malformed_entry])),
            Err(StorageError::MalformedObject)
        );
        let mut truncated_entry = second_entry;
        truncated_entry.pop();
        assert_eq!(
            decode_configuration_image(&raw_image(&[&truncated_entry])),
            Err(StorageError::MalformedImage)
        );
    }
}
