//! Bounded object encoding and logically transactional configuration storage.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use radio_channel_plan::{
    BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
    ChannelRecord, ChannelTemplate, Designator, GeneratedBank, PlanEncoding, MAX_BANK_NAME_LEN,
    MAX_CHANNEL_NAME_LEN, MAX_DESIGNATOR_LEN,
};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, Offset, PowerLevel,
    RadioConfig, RadioFlags, ScanResume, SquelchLevel, Tone, TxClass,
};

/// Encoded calling-channel index meaning a plan marks no calling channel.
///
/// A plan holds at most `MAX_GENERATED_CHANNELS` channels, so this value is
/// outside every index a plan can legitimately mark.
const NO_CALLING_INDEX: u16 = u16::MAX;

/// Current object encoding version.
///
/// Version 4 splits a generated bank into a shared core and a per-encoding
/// tail, and stores the encoding family the plan declares rather than inferring
/// it from a zero offset, so each plan costs what its own family needs. Version
/// 3 carried the designator, numbering, calling index and a transmit offset
/// every plan paid for. Earlier versions are rejected rather than guessed at.
pub const STORAGE_FORMAT_VERSION: u8 = 4;
/// Maximum bytes held by one device object in the first storage model.
pub const MAX_OBJECT_DATA: usize = 64;
/// Encoded byte length of the shared core every generated-bank object carries.
///
/// What follows it is the declared encoding's own tail, which is empty for a
/// simplex plan and four bytes of transmit offset for a fixed-offset one.
pub const GENERATED_BANK_CORE_LEN: usize = 56;
/// Longest generated-bank object any implemented encoding produces.
pub const MAX_GENERATED_BANK_ENCODED_LEN: usize = GENERATED_BANK_CORE_LEN + 4;
/// Encoded byte length of a version-3 explicit channel object.
pub const CHANNEL_ENCODED_LEN: usize = 42;
/// Encoded byte length of a version-3 named channel-bank object.
pub const CHANNEL_BANK_ENCODED_LEN: usize = 22;
/// Encoded byte length of a version-2 global radio-configuration object.
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

/// Anything which presents one identified encoded configuration object.
///
/// A store holds its objects packed end to end and lends them out as borrowed
/// slices; an encoder produces one owned object at a time. Both are objects to
/// every decoder and validator here, so nothing has to be copied into a
/// worst-case buffer merely to be read.
pub trait Object {
    /// Returns the object identity.
    fn key(&self) -> ObjectKey;

    /// Returns the encoded object bytes.
    fn data(&self) -> &[u8];

    /// Returns the encoded object length.
    fn len(&self) -> usize {
        self.data().len()
    }

    /// Reports whether the object data is empty.
    fn is_empty(&self) -> bool {
        self.data().is_empty()
    }
}

impl<T: Object + ?Sized> Object for &T {
    fn key(&self) -> ObjectKey {
        (**self).key()
    }

    fn data(&self) -> &[u8] {
        (**self).data()
    }
}

/// One encoded configuration object borrowed from the bytes holding it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectRef<'a> {
    key: ObjectKey,
    data: &'a [u8],
}

impl<'a> ObjectRef<'a> {
    /// Borrows encoded object data without copying it.
    pub const fn new(key: ObjectKey, data: &'a [u8]) -> Self {
        Self { key, data }
    }

    /// Returns the object identity.
    pub const fn key(self) -> ObjectKey {
        self.key
    }

    /// Returns the encoded object bytes.
    pub const fn data(self) -> &'a [u8] {
        self.data
    }

    /// Returns the encoded object length.
    pub const fn len(self) -> usize {
        self.data.len()
    }

    /// Reports whether the object data is empty.
    pub const fn is_empty(self) -> bool {
        self.data.is_empty()
    }
}

impl PartialEq<StorageObject> for ObjectRef<'_> {
    fn eq(&self, other: &StorageObject) -> bool {
        self.key == other.key() && self.data == other.data()
    }
}

impl PartialEq<ObjectRef<'_>> for StorageObject {
    fn eq(&self, other: &ObjectRef<'_>) -> bool {
        other == self
    }
}

impl Object for ObjectRef<'_> {
    fn key(&self) -> ObjectKey {
        self.key
    }

    fn data(&self) -> &[u8] {
        self.data
    }
}

/// A fixed-capacity encoded configuration object.
///
/// This is the interchange form an encoder returns and a host holds. A store
/// does not use it: [`MAX_OBJECT_DATA`] is what one object may carry over the
/// wire, not what one object costs a device to keep.
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

    /// Borrows this object without copying its bytes.
    pub fn as_ref(&self) -> ObjectRef<'_> {
        ObjectRef {
            key: self.key,
            data: self.data(),
        }
    }
}

impl Object for StorageObject {
    fn key(&self) -> ObjectKey {
        self.key
    }

    fn data(&self) -> &[u8] {
        self.data()
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
    /// A plan encoding family is declared but not implemented here.
    UnsupportedEncoding(PlanEncoding),
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
            Self::UnsupportedEncoding(encoding) => {
                write!(formatter, "unimplemented plan encoding {encoding:?}")
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

/// Bytes one arena entry spends on identity and length before its payload.
///
/// This is the canonical image object header, and deliberately so: an arena
/// holds exactly the bytes an image payload holds, in exactly the same order.
pub const OBJECT_ENTRY_HEADER_LEN: usize = CONFIGURATION_IMAGE_OBJECT_HEADER_LEN;

/// Smallest payload any currently defined object encodes to.
///
/// A device derives the object count it can hold from its byte bound and this,
/// so what it advertises is an honest upper bound rather than a second limit.
pub const MIN_OBJECT_ENCODED_LEN: usize = RADIO_CONFIG_ENCODED_LEN;

/// A packed byte arena holding one complete object set in canonical order.
///
/// Objects are stored end to end as `(kind, id, length, payload)` entries and
/// kept in strict `(kind, id)` order, so what an arena holds is byte for byte
/// what a canonical configuration image carries after its header. What bounds
/// an arena is `BYTES` and nothing else: no object count, no per-kind count,
/// and no worst-case size charged to every object whatever it holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectArena<const BYTES: usize> {
    bytes: [u8; BYTES],
    len: usize,
}

impl<const BYTES: usize> Default for ObjectArena<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BYTES: usize> ObjectArena<BYTES> {
    /// Constructs an empty arena.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [0; BYTES],
            len: 0,
        }
    }

    /// Copies one packed payload into an arena, checking it as it goes.
    ///
    /// This is how a snapshot is shared without rebuilding it object by
    /// object: the bytes an arena holds are the bytes a canonical image
    /// carries, so a payload from either is the same payload. Entries which
    /// are out of order, overrun the payload, or name an unknown kind are
    /// refused rather than half copied.
    pub fn from_payload(payload: &[u8]) -> Result<Self, StorageError> {
        if payload.len() > BYTES {
            return Err(StorageError::StoreFull);
        }
        let mut arena = Self::new();
        arena.bytes[..payload.len()].copy_from_slice(payload);
        arena.len = payload.len();
        let mut previous = None;
        let mut seen = 0;
        for object in &arena {
            if previous.is_some_and(|key| key >= object.key()) {
                return Err(StorageError::NonCanonicalImage);
            }
            previous = Some(object.key());
            seen += OBJECT_ENTRY_HEADER_LEN + object.len();
        }
        if seen != payload.len() {
            return Err(StorageError::MalformedImage);
        }
        Ok(arena)
    }

    /// Returns the total bytes this arena can hold.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        BYTES
    }

    /// Returns the packed entry bytes, which are a canonical image payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Adds or replaces one object, compacting the entries around it.
    pub fn write(&mut self, object: &impl Object) -> Result<(), StorageError> {
        let key = object.key();
        let data = object.data();
        if data.len() > MAX_OBJECT_DATA {
            return Err(StorageError::ObjectTooLarge);
        }
        let entry_len = OBJECT_ENTRY_HEADER_LEN + data.len();
        let (offset, replaced) = self.locate(key);
        let old_len = replaced.unwrap_or(0);
        let new_len = self
            .len
            .checked_sub(old_len)
            .and_then(|len| len.checked_add(entry_len))
            .ok_or(StorageError::StoreFull)?;
        if new_len > BYTES {
            return Err(StorageError::StoreFull);
        }
        // One memmove keeps the entries packed and ordered whether this
        // replaces a shorter object, a longer one, or none at all.
        self.bytes
            .copy_within(offset + old_len..self.len, offset + entry_len);
        self.bytes[offset] = key.kind as u8;
        self.bytes[offset + 1..offset + 3].copy_from_slice(&key.id.to_le_bytes());
        let encoded_len = u16::try_from(data.len()).map_err(|_| StorageError::ObjectTooLarge)?;
        self.bytes[offset + 3..offset + OBJECT_ENTRY_HEADER_LEN]
            .copy_from_slice(&encoded_len.to_le_bytes());
        self.bytes[offset + OBJECT_ENTRY_HEADER_LEN..offset + entry_len].copy_from_slice(data);
        self.len = new_len;
        Ok(())
    }

    /// Removes one object, closing the gap it leaves.
    pub fn remove(&mut self, key: ObjectKey) -> Result<(), StorageError> {
        let (offset, replaced) = self.locate(key);
        let entry_len = replaced.ok_or(StorageError::ObjectNotFound)?;
        self.bytes.copy_within(offset + entry_len..self.len, offset);
        self.len -= entry_len;
        Ok(())
    }

    /// Reads one object without copying its bytes.
    #[must_use]
    pub fn read(&self, key: ObjectKey) -> Option<ObjectRef<'_>> {
        self.iter().find(|object| object.key() == key)
    }

    /// Iterates over every object in strict `(kind, id)` order.
    #[must_use]
    pub fn iter(&self) -> ObjectArenaIter<'_> {
        ObjectArenaIter {
            payload: self.payload(),
            offset: 0,
        }
    }

    /// Returns the number of objects held.
    #[must_use]
    pub fn object_count(&self) -> u16 {
        u16::try_from(self.iter().count()).unwrap_or(u16::MAX)
    }

    /// Reports object-count and payload usage.
    #[must_use]
    pub fn usage(&self) -> StorageUsage {
        self.iter()
            .fold(StorageUsage::default(), |mut usage, object| {
                usage.object_count += 1;
                usage.payload_bytes += u32::try_from(object.len()).unwrap_or(u32::MAX);
                usage
            })
    }

    /// Returns where one key belongs and, if it is present, its entry length.
    ///
    /// Entries are ordered, so the first entry which does not sort before the
    /// key is either that key or the entry it must be inserted in front of.
    fn locate(&self, key: ObjectKey) -> (usize, Option<usize>) {
        let mut offset = 0;
        while offset + OBJECT_ENTRY_HEADER_LEN <= self.len {
            let entry_len = OBJECT_ENTRY_HEADER_LEN + self.entry_data_len(offset);
            match self.entry_key(offset) {
                Some(entry) if entry == key => return (offset, Some(entry_len)),
                Some(entry) if entry > key => return (offset, None),
                _ => offset += entry_len,
            }
        }
        (self.len, None)
    }

    fn entry_data_len(&self, offset: usize) -> usize {
        usize::from(u16::from_le_bytes([
            self.bytes[offset + 3],
            self.bytes[offset + 4],
        ]))
    }

    /// Returns one entry's key, or `None` for a kind byte never written here.
    fn entry_key(&self, offset: usize) -> Option<ObjectKey> {
        Some(ObjectKey {
            kind: ObjectKind::try_from(self.bytes[offset]).ok()?,
            id: u16::from_le_bytes([self.bytes[offset + 1], self.bytes[offset + 2]]),
        })
    }
}

impl<'a, const BYTES: usize> IntoIterator for &'a ObjectArena<BYTES> {
    type Item = ObjectRef<'a>;
    type IntoIter = ObjectArenaIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the packed objects of one arena.
#[derive(Clone, Debug)]
pub struct ObjectArenaIter<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for ObjectArenaIter<'a> {
    type Item = ObjectRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + OBJECT_ENTRY_HEADER_LEN > self.payload.len() {
            return None;
        }
        let kind = ObjectKind::try_from(self.payload[self.offset]).ok()?;
        let id = u16::from_le_bytes([self.payload[self.offset + 1], self.payload[self.offset + 2]]);
        let data_len = usize::from(u16::from_le_bytes([
            self.payload[self.offset + 3],
            self.payload[self.offset + 4],
        ]));
        let start = self.offset + OBJECT_ENTRY_HEADER_LEN;
        let end = start
            .checked_add(data_len)
            .filter(|end| *end <= self.payload.len())?;
        self.offset = end;
        Some(ObjectRef {
            key: ObjectKey { kind, id },
            data: &self.payload[start..end],
        })
    }
}

/// A byte-bounded store with isolated active and candidate snapshots.
///
/// `BYTES` is the whole bound. A store holds as many objects of whatever kinds
/// as its packed bytes have room for, so a device declares one number and a
/// host can decide what fits from that number alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionalStore<const BYTES: usize> {
    active: ObjectArena<BYTES>,
    candidate: Option<ObjectArena<BYTES>>,
    candidate_validated: bool,
    generation: u32,
}

impl<const BYTES: usize> Default for TransactionalStore<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BYTES: usize> TransactionalStore<BYTES> {
    /// Constructs an empty generation-zero store.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active: ObjectArena::new(),
            candidate: None,
            candidate_validated: false,
            generation: 0,
        }
    }

    /// Returns the bytes one snapshot can hold.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        BYTES
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
    pub fn write(&mut self, object: &impl Object) -> Result<(), StorageError> {
        self.candidate
            .as_mut()
            .ok_or(StorageError::NoTransaction)?
            .write(object)?;
        self.candidate_validated = false;
        Ok(())
    }

    /// Removes an object from the candidate snapshot.
    pub fn remove(&mut self, key: ObjectKey) -> Result<(), StorageError> {
        self.candidate
            .as_mut()
            .ok_or(StorageError::NoTransaction)?
            .remove(key)?;
        self.candidate_validated = false;
        Ok(())
    }

    /// Validates every candidate object and marks the unchanged candidate valid.
    pub fn validate<F>(&mut self, mut validator: F) -> Result<(), StorageError>
    where
        F: FnMut(ObjectRef<'_>) -> bool,
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
    pub fn read(&self, key: ObjectKey) -> Result<ObjectRef<'_>, StorageError> {
        self.active.read(key).ok_or(StorageError::ObjectNotFound)
    }

    /// Iterates over active objects, in canonical order, without exposing
    /// candidate data.
    pub fn active_objects(&self) -> ObjectArenaIter<'_> {
        self.active.iter()
    }

    /// Returns the active snapshot as a canonical image payload.
    #[must_use]
    pub fn active_payload(&self) -> &[u8] {
        self.active.payload()
    }

    /// Reports active-snapshot usage.
    pub fn usage(&self) -> StorageUsage {
        self.active.usage()
    }
}

/// Encodes a generated bank as a versioned storage object.
///
/// The shared core is written first and the declared encoding's own tail after
/// it, so a plan costs what its family actually needs: a simplex band carries
/// no transmit offset at all, and a repeater sub-band carries exactly one.
pub fn encode_generated_bank(bank: GeneratedBank) -> Result<StorageObject, StorageError> {
    let mut data = [0_u8; MAX_GENERATED_BANK_ENCODED_LEN];
    data[0] = STORAGE_FORMAT_VERSION;
    data[1] = bank.encoding() as u8;
    data[2..4].copy_from_slice(&bank.id().get().to_le_bytes());
    data[4] = bank.name().len();
    data[5..5 + MAX_BANK_NAME_LEN].copy_from_slice(&bank.name().field());
    data[21..25].copy_from_slice(&bank.base().as_hz().to_le_bytes());
    data[25..29].copy_from_slice(&bank.spacing().as_hz().to_le_bytes());
    data[29..31].copy_from_slice(&bank.channel_count().to_le_bytes());
    data[31] = bank.tx_class() as u8;
    let template = bank.template();
    encode_tone(template.rx_tone, &mut data[32..35]);
    encode_tone(template.tx_tone, &mut data[35..38]);
    data[38] = template.modulation as u8;
    data[39] = template.bandwidth as u8;
    data[40] = template.power as u8;
    data[41..45].copy_from_slice(&template.step.as_hz().to_le_bytes());
    data[45] = template.squelch.get();
    data[46] = template.flags.bits();
    data[47] = bank.designator().len();
    data[48..48 + MAX_DESIGNATOR_LEN].copy_from_slice(&bank.designator().field());
    data[52..54].copy_from_slice(&bank.first_number().to_le_bytes());
    // No index is representable as a channel number, so the absent marker is
    // the one value a bank can never mark: the whole plan index space is
    // bounded well below it.
    data[54..56].copy_from_slice(
        &bank
            .calling_index()
            .unwrap_or(NO_CALLING_INDEX)
            .to_le_bytes(),
    );
    let length = generated_bank_encoded_len(bank.encoding())?;
    match bank.encoding() {
        PlanEncoding::LinearSimplex => {}
        PlanEncoding::LinearFixedOffset => {
            data[56..60].copy_from_slice(&bank.offset().as_hz().to_le_bytes());
        }
        encoding => return Err(StorageError::UnsupportedEncoding(encoding)),
    }
    StorageObject::new(
        ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: bank.id().get(),
        },
        &data[..length],
    )
}

/// Returns the encoded length of one plan encoding, core and tail together.
///
/// Every remaining declared family is variable length, so this answers only
/// for those which are implemented and refuses the rest by name rather than
/// guessing at a size.
pub const fn generated_bank_encoded_len(encoding: PlanEncoding) -> Result<usize, StorageError> {
    match encoding {
        PlanEncoding::LinearSimplex => Ok(GENERATED_BANK_CORE_LEN),
        PlanEncoding::LinearFixedOffset => Ok(GENERATED_BANK_CORE_LEN + 4),
        other => Err(StorageError::UnsupportedEncoding(other)),
    }
}

/// Decodes and fully validates a generated-bank storage object.
pub fn decode_generated_bank(object: &impl Object) -> Result<GeneratedBank, StorageError> {
    let key = object.key();
    if key.kind != ObjectKind::GeneratedBank {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() < GENERATED_BANK_CORE_LEN || data[0] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::MalformedObject);
    }
    let encoding = PlanEncoding::try_from(data[1]).map_err(|_| StorageError::MalformedObject)?;
    if data.len() != generated_bank_encoded_len(encoding)? {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[2], data[3]]);
    if id != key.id {
        return Err(StorageError::MalformedObject);
    }
    let mut name_field = [0_u8; MAX_BANK_NAME_LEN];
    name_field.copy_from_slice(&data[5..21]);
    let name =
        BankName::from_field(name_field, data[4]).map_err(|_| StorageError::MalformedObject)?;
    let base = Frequency::from_hz(u32::from_le_bytes([data[21], data[22], data[23], data[24]]))
        .map_err(|_| StorageError::MalformedObject)?;
    let spacing =
        FrequencyStep::from_hz(u32::from_le_bytes([data[25], data[26], data[27], data[28]]))
            .map_err(|_| StorageError::MalformedObject)?;
    let channel_count = u16::from_le_bytes([data[29], data[30]]);
    let tx_class = TxClass::try_from(data[31]).map_err(|_| StorageError::MalformedObject)?;
    let template = ChannelTemplate {
        rx_tone: decode_tone(&data[32..35])?,
        tx_tone: decode_tone(&data[35..38])?,
        modulation: Modulation::try_from(data[38]).map_err(|_| StorageError::MalformedObject)?,
        bandwidth: Bandwidth::try_from(data[39]).map_err(|_| StorageError::MalformedObject)?,
        power: PowerLevel::try_from(data[40]).map_err(|_| StorageError::MalformedObject)?,
        step: FrequencyStep::from_hz(u32::from_le_bytes([data[41], data[42], data[43], data[44]]))
            .map_err(|_| StorageError::MalformedObject)?,
        squelch: SquelchLevel::new(data[45]).map_err(|_| StorageError::MalformedObject)?,
        flags: ChannelFlags::from_bits(data[46]).map_err(|_| StorageError::MalformedObject)?,
    };
    let mut designator_field = [0_u8; MAX_DESIGNATOR_LEN];
    designator_field.copy_from_slice(&data[48..52]);
    let designator = Designator::from_field(designator_field, data[47])
        .map_err(|_| StorageError::MalformedObject)?;
    let first_number = u16::from_le_bytes([data[52], data[53]]);
    let calling = match u16::from_le_bytes([data[54], data[55]]) {
        NO_CALLING_INDEX => None,
        index => Some(index),
    };
    let bank = match encoding {
        PlanEncoding::LinearSimplex => GeneratedBank::linear_simplex_with(
            BankId::new(id),
            name,
            base,
            spacing,
            channel_count,
            tx_class,
            template,
        ),
        PlanEncoding::LinearFixedOffset => GeneratedBank::linear_fixed_offset_with(
            BankId::new(id),
            name,
            base,
            spacing,
            channel_count,
            tx_class,
            template,
            Offset::from_hz(i32::from_le_bytes([data[56], data[57], data[58], data[59]])),
        ),
        other => return Err(StorageError::UnsupportedEncoding(other)),
    };
    bank.and_then(|bank| bank.with_designator(designator, first_number))
        .and_then(|bank| bank.with_calling_index(calling))
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
pub fn decode_channel(object: &impl Object) -> Result<ChannelRecord, StorageError> {
    let key = object.key();
    if key.kind != ObjectKind::Channel {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() != CHANNEL_ENCODED_LEN || data[0] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[1], data[2]]);
    if id != key.id {
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
pub fn decode_channel_bank(object: &impl Object) -> Result<ChannelBank, StorageError> {
    let key = object.key();
    if key.kind != ObjectKind::ChannelBank {
        return Err(StorageError::UnsupportedObject);
    }
    let data = object.data();
    if data.len() != CHANNEL_BANK_ENCODED_LEN || data[0] != STORAGE_FORMAT_VERSION || data[21] != 0
    {
        return Err(StorageError::MalformedObject);
    }
    let id = u16::from_le_bytes([data[1], data[2]]);
    if id != key.id {
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
pub fn decode_radio_config(object: &impl Object) -> Result<RadioConfig, StorageError> {
    let key = object.key();
    if key.kind != ObjectKind::RadioConfig {
        return Err(StorageError::UnsupportedObject);
    }
    if key.id != RADIO_CONFIG_OBJECT_ID {
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
pub fn validate_object(object: &impl Object) -> bool {
    match object.key().kind {
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

impl<'a> Iterator for ConfigurationImageObjects<'a> {
    type Item = ObjectRef<'a>;

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
pub fn configuration_image_len(objects: &[impl Object]) -> Result<usize, StorageError> {
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
    objects: &[impl Object],
    output: &mut [u8],
) -> Result<usize, StorageError> {
    validate_canonical_objects(objects)?;
    if output.len() < configuration_image_len(objects)? {
        return Err(StorageError::ImageBufferTooSmall);
    }
    let mut writer = ConfigurationImageWriter::new(
        output,
        u16::try_from(objects.len()).map_err(|_| StorageError::ImageTooLarge)?,
    )?;
    for object in objects {
        writer.push(object)?;
    }
    writer.finish()
}

/// Incremental canonical configuration image encoder.
///
/// A device holds its objects in a fixed table rather than a contiguous ordered
/// slice, so this writer accepts them one at a time and needs no second copy of
/// the object set. It enforces the same strict `(kind, id)` order, exact object
/// count, and buffer bound as the slice encoder, and produces identical bytes.
pub struct ConfigurationImageWriter<'a> {
    output: &'a mut [u8],
    object_count: u16,
    written: u16,
    offset: usize,
    previous_key: Option<ObjectKey>,
}

impl<'a> ConfigurationImageWriter<'a> {
    /// Starts an image which must receive exactly `object_count` objects.
    pub fn new(output: &'a mut [u8], object_count: u16) -> Result<Self, StorageError> {
        if output.len() < CONFIGURATION_IMAGE_HEADER_LEN {
            return Err(StorageError::ImageBufferTooSmall);
        }
        Ok(Self {
            output,
            object_count,
            written: 0,
            offset: CONFIGURATION_IMAGE_HEADER_LEN,
            previous_key: None,
        })
    }

    /// Appends the next object in strict canonical key order.
    pub fn push(&mut self, object: &impl Object) -> Result<(), StorageError> {
        if self.written == self.object_count {
            return Err(StorageError::ImageTooLarge);
        }
        let key = object.key();
        if self.previous_key.is_some_and(|previous| previous >= key) {
            return Err(StorageError::NonCanonicalImage);
        }
        if !validate_object(&object) {
            return Err(StorageError::MalformedObject);
        }
        let object_len = u16::try_from(object.len()).map_err(|_| StorageError::ObjectTooLarge)?;
        let header_end = self
            .offset
            .checked_add(CONFIGURATION_IMAGE_OBJECT_HEADER_LEN)
            .ok_or(StorageError::ImageTooLarge)?;
        let data_end = header_end
            .checked_add(object.len())
            .ok_or(StorageError::ImageTooLarge)?;
        if data_end > self.output.len() {
            return Err(StorageError::ImageBufferTooSmall);
        }
        self.output[self.offset] = key.kind as u8;
        self.output[self.offset + 1..self.offset + 3].copy_from_slice(&key.id.to_le_bytes());
        self.output[self.offset + 3..header_end].copy_from_slice(&object_len.to_le_bytes());
        self.output[header_end..data_end].copy_from_slice(object.data());
        self.offset = data_end;
        self.written += 1;
        self.previous_key = Some(key);
        Ok(())
    }

    /// Writes the header and checksum, returning the exact image length.
    ///
    /// Fewer objects than the declared count is an error: a truncated image
    /// would otherwise claim to be a complete configuration.
    pub fn finish(self) -> Result<usize, StorageError> {
        if self.written != self.object_count {
            return Err(StorageError::MalformedImage);
        }
        let image_len = self.offset;
        let payload_len = image_len - CONFIGURATION_IMAGE_HEADER_LEN;
        let encoded_payload_len =
            u32::try_from(payload_len).map_err(|_| StorageError::ImageTooLarge)?;

        self.output[..4].copy_from_slice(&CONFIGURATION_IMAGE_MAGIC);
        self.output[4] = CONFIGURATION_IMAGE_VERSION;
        self.output[5] = STORAGE_FORMAT_VERSION;
        self.output[6..8].copy_from_slice(&self.object_count.to_le_bytes());
        self.output[8..12].copy_from_slice(&encoded_payload_len.to_le_bytes());
        self.output[12..16].fill(0);

        let checksum = configuration_image_crc(&self.output[..12], &self.output[16..image_len]);
        self.output[12..16].copy_from_slice(&checksum.to_le_bytes());
        Ok(image_len)
    }
}

/// Reads the exact image length from a canonical configuration image header.
///
/// A device which retains an image in a fixed region needs its length before it
/// can hand exactly those bytes to the decoder. This checks the magic and both
/// versions and nothing else: the checksum, object order, and every object are
/// still validated by [`decode_configuration_image`].
pub fn configuration_image_len_from_header(bytes: &[u8]) -> Result<usize, StorageError> {
    if bytes.len() < CONFIGURATION_IMAGE_HEADER_LEN || bytes[..4] != CONFIGURATION_IMAGE_MAGIC {
        return Err(StorageError::MalformedImage);
    }
    if bytes[4] != CONFIGURATION_IMAGE_VERSION || bytes[5] != STORAGE_FORMAT_VERSION {
        return Err(StorageError::UnsupportedImageVersion);
    }
    let payload_len = usize::try_from(u32::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11],
    ]))
    .map_err(|_| StorageError::ImageTooLarge)?;
    CONFIGURATION_IMAGE_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(StorageError::ImageTooLarge)
}

/// Validates and borrows one exact canonical configuration image.
///
/// The complete checksum, structure, object order, and every object payload are
/// validated before the returned image exposes its object iterator.
pub fn decode_configuration_image(bytes: &[u8]) -> Result<ConfigurationImage<'_>, StorageError> {
    let expected_len = configuration_image_len_from_header(bytes)?;
    if bytes.len() != expected_len {
        return Err(StorageError::MalformedImage);
    }
    let object_count = u16::from_le_bytes([bytes[6], bytes[7]]);
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

fn validate_canonical_objects(objects: &[impl Object]) -> Result<(), StorageError> {
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
) -> Result<(ObjectRef<'_>, usize), StorageError> {
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
    if usize::from(encoded_len) > MAX_OBJECT_DATA {
        return Err(StorageError::ObjectTooLarge);
    }
    let object = ObjectRef::new(ObjectKey { kind, id }, &payload[header_end..data_end]);
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
        encode_radio_config, validate_object, ConfigurationImageWriter, ObjectArena, ObjectKey,
        ObjectKind, ObjectRef, StorageError, StorageObject, TransactionalStore,
        CHANNEL_BANK_ENCODED_LEN, CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN,
        CONFIGURATION_IMAGE_MAGIC, CONFIGURATION_IMAGE_VERSION, GENERATED_BANK_CORE_LEN,
        OBJECT_ENTRY_HEADER_LEN, RADIO_CONFIG_ENCODED_LEN, STORAGE_FORMAT_VERSION,
    };
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord, ChannelTemplate, GeneratedBank, PlanEncoding, GENERATED_CHANNEL_ID_BASE,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, Offset, PowerLevel,
        RadioConfig, RadioFlags, ScanResume, SquelchLevel, Tone, TxClass,
    };
    use std::{vec, vec::Vec};

    fn bank(id: u16, name: &str) -> GeneratedBank {
        GeneratedBank::linear_simplex_with(
            BankId::new(id),
            BankName::new(name).unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
            // A template unlike the conservative default, so the round trip
            // proves every per-channel field survives storage.
            ChannelTemplate {
                rx_tone: Tone::Ctcss(1_000),
                tx_tone: Tone::Dcs {
                    code: 23,
                    inverted: true,
                },
                power: PowerLevel::Medium,
                squelch: SquelchLevel::new(4).unwrap(),
                flags: ChannelFlags::default().with(ChannelFlags::BUSY_LOCKOUT, true),
                ..ChannelTemplate::narrow_fm(FrequencyStep::from_hz(12_500).unwrap())
            },
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

    /// Each plan family is stored at its own length, and says which it is.
    #[test]
    fn a_simplex_plan_encodes_shorter_than_a_repeater_plan() {
        let simplex = bank(4, "2M SIMPLEX");
        let encoded = encode_generated_bank(simplex).unwrap();
        assert_eq!(simplex.encoding(), PlanEncoding::LinearSimplex);
        assert_eq!(
            encoded.len(),
            GENERATED_BANK_CORE_LEN,
            "a simplex plan carries no transmit offset at all"
        );

        let repeater = GeneratedBank::linear_fixed_offset_with(
            BankId::new(5),
            BankName::new("2M REPEATERS").unwrap(),
            Frequency::from_hz(145_600_000).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            8,
            TxClass::Amateur,
            ChannelTemplate::narrow_fm(FrequencyStep::from_hz(12_500).unwrap()),
            Offset::from_hz(-600_000),
        )
        .unwrap();
        let encoded_repeater = encode_generated_bank(repeater).unwrap();
        assert_eq!(
            encoded_repeater.len(),
            GENERATED_BANK_CORE_LEN + 4,
            "a repeater plan carries exactly one"
        );
        assert!(encoded.len() < encoded_repeater.len());
        assert_eq!(decode_generated_bank(&encoded_repeater).unwrap(), repeater);

        // The declared family survives storage even where an offset cannot
        // distinguish it, which is what an inference from zero could not do.
        let degenerate = GeneratedBank::linear_fixed_offset_with(
            BankId::new(6),
            BankName::new("PARKED").unwrap(),
            Frequency::from_hz(145_600_000).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            8,
            TxClass::Amateur,
            ChannelTemplate::narrow_fm(FrequencyStep::from_hz(12_500).unwrap()),
            Offset::from_hz(0),
        )
        .unwrap();
        let stored = encode_generated_bank(degenerate).unwrap();
        assert_eq!(stored.len(), GENERATED_BANK_CORE_LEN + 4);
        assert_eq!(
            decode_generated_bank(&stored).unwrap().encoding(),
            PlanEncoding::LinearFixedOffset
        );
        assert_ne!(degenerate.encoding(), simplex.encoding());
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
            (38, 0x20),
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
        let mut store = TransactionalStore::<256>::new();
        store.begin().unwrap();
        store
            .write(&encode_generated_bank(bank(4, "PMR446")).unwrap())
            .unwrap();
        assert_eq!(store.read(key), Err(StorageError::ObjectNotFound));
        assert_eq!(store.commit(), Err(StorageError::CandidateNotValidated));
        store.validate(|object| validate_object(&object)).unwrap();
        assert_eq!(store.commit().unwrap(), 1);
        assert_eq!(
            decode_generated_bank(&store.read(key).unwrap()).unwrap(),
            bank(4, "PMR446")
        );
    }

    #[test]
    fn abort_preserves_active_snapshot() {
        let key = ObjectKey {
            kind: ObjectKind::GeneratedBank,
            id: 4,
        };
        let mut store = TransactionalStore::<256>::new();
        store.begin().unwrap();
        store
            .write(&encode_generated_bank(bank(4, "original")).unwrap())
            .unwrap();
        store.validate(|object| validate_object(&object)).unwrap();
        store.commit().unwrap();

        store.begin().unwrap();
        store
            .write(&encode_generated_bank(bank(4, "candidate")).unwrap())
            .unwrap();
        store.abort().unwrap();
        assert_eq!(
            decode_generated_bank(&store.read(key).unwrap()).unwrap(),
            bank(4, "original")
        );
    }

    /// A packed arena has to survive objects arriving in any order, changing
    /// length under replacement, and leaving.
    #[test]
    fn the_arena_stays_packed_and_ordered_through_replacement_and_removal() {
        let mut arena = ObjectArena::<512>::new();
        let plan = encode_generated_bank(bank(1, "PLAN")).unwrap();
        let first = encode_channel(channel(2, "TWO")).unwrap();
        let second = encode_channel(channel(9, "NINE")).unwrap();
        let config = encode_radio_config(RadioConfig::conservative()).unwrap();

        // Written last-first: the arena orders them, so its bytes are already
        // a canonical image payload.
        for object in [&config, &second, &first, &plan] {
            arena.write(object).unwrap();
        }
        assert_eq!(
            arena.iter().map(ObjectRef::key).collect::<Vec<_>>(),
            vec![plan.key(), first.key(), second.key(), config.key()]
        );
        let expected = arena.usage();
        assert_eq!(expected.object_count, 4);
        assert_eq!(
            usize::try_from(expected.payload_bytes).unwrap() + 4 * OBJECT_ENTRY_HEADER_LEN,
            arena.payload().len()
        );

        // Replacing the shortest object with the longest one moves every entry
        // after it, and nothing before it.
        let grown = StorageObject::new(config.key(), &[0; 40]).unwrap();
        arena.write(&grown).unwrap();
        assert_eq!(arena.read(config.key()).unwrap().len(), 40);
        assert_eq!(
            decode_channel(&arena.read(first.key()).unwrap()).unwrap(),
            channel(2, "TWO")
        );
        arena.write(&config).unwrap();
        assert_eq!(arena.usage(), expected, "shrinking again leaves no gap");

        arena.remove(first.key()).unwrap();
        assert_eq!(
            arena.iter().map(ObjectRef::key).collect::<Vec<_>>(),
            vec![plan.key(), second.key(), config.key()]
        );
        assert_eq!(arena.remove(first.key()), Err(StorageError::ObjectNotFound));
        assert_eq!(
            decode_channel(&arena.read(second.key()).unwrap()).unwrap(),
            channel(9, "NINE")
        );
    }

    /// Bytes are the whole bound, and a transaction which exceeds them has to
    /// leave the running configuration exactly as it was.
    #[test]
    fn a_store_is_bounded_by_bytes_and_a_failed_transaction_changes_nothing() {
        let plan = encode_generated_bank(bank(1, "PLAN")).unwrap();
        let entry = OBJECT_ENTRY_HEADER_LEN + plan.len();
        let mut store = TransactionalStore::<
            { 2 * (OBJECT_ENTRY_HEADER_LEN + GENERATED_BANK_CORE_LEN) },
        >::new();
        assert_eq!(store.capacity(), 2 * entry);

        store.begin().unwrap();
        store.write(&plan).unwrap();
        store.validate(|object| validate_object(&object)).unwrap();
        store.commit().unwrap();
        let active = store.active_payload().to_vec();

        store.begin().unwrap();
        store
            .write(&encode_generated_bank(bank(2, "SECOND")).unwrap())
            .unwrap();
        assert_eq!(
            store.write(&encode_generated_bank(bank(3, "THIRD")).unwrap()),
            Err(StorageError::StoreFull),
            "one declared number bounds the configuration"
        );
        store.abort().unwrap();
        assert_eq!(store.active_payload(), active);
        assert_eq!(store.usage().object_count, 1);

        // The same bytes as one object of a kind with no count of its own.
        store.begin().unwrap();
        for id in 2..4 {
            store
                .write(&encode_generated_bank(bank(id, "PLAN")).unwrap())
                .ok();
        }
        store.abort().unwrap();
        assert_eq!(store.active_payload(), active);
    }

    /// The store holds what an image carries, so one is the other's payload.
    #[test]
    fn an_active_snapshot_is_the_image_payload_it_encodes_to() {
        let mut store = TransactionalStore::<512>::new();
        store.begin().unwrap();
        store
            .write(&encode_channel(channel(2, "TWO")).unwrap())
            .unwrap();
        store
            .write(&encode_generated_bank(bank(1, "PLAN")).unwrap())
            .unwrap();
        store.validate(|object| validate_object(&object)).unwrap();
        store.commit().unwrap();

        let mut image = [0_u8; 256];
        let mut writer = ConfigurationImageWriter::new(&mut image, 2).unwrap();
        for object in store.active_objects() {
            writer.push(&object).unwrap();
        }
        let length = writer.finish().unwrap();
        assert_eq!(
            &image[CONFIGURATION_IMAGE_HEADER_LEN..length],
            store.active_payload()
        );
    }

    #[test]
    fn canonical_image_has_an_exact_format_and_round_trips() {
        let object = encode_generated_bank(bank(4, "A")).unwrap();
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
                0x41, 0x46, 0x49, 0x4B, 0x01, 0x04, 0x01, 0x00, 0x3D, 0x00, 0x00, 0x00, 0x0B, 0x08,
                0x7A, 0x8F, 0x01, 0x04, 0x00, 0x38, 0x00, 0x04, 0x00, 0x04, 0x00, 0x01, 0x41, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0xEA, 0x83, 0x95, 0x1A, 0xD4, 0x30, 0x00, 0x00, 0x10, 0x00, 0x01, 0x01, 0xE8, 0x03,
                0x03, 0x17, 0x00, 0x00, 0x00, 0x01, 0xD4, 0x30, 0x00, 0x00, 0x04, 0x02, 0x02, 0x41,
                0x20, 0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF,
            ]
        );
        let decoded = decode_configuration_image(&image[..image_len]).unwrap();
        assert_eq!(decoded.object_count(), 1);
        assert_eq!(decoded.objects().collect::<Vec<_>>(), vec![object]);
    }

    /// The empty object set, typed: an image of nothing is still an image.
    const NO_OBJECTS: &[StorageObject] = &[];

    #[test]
    fn empty_image_is_valid_and_buffers_are_bounded() {
        let mut image = [0_u8; CONFIGURATION_IMAGE_HEADER_LEN];
        assert_eq!(
            encode_configuration_image(NO_OBJECTS, &mut image).unwrap(),
            CONFIGURATION_IMAGE_HEADER_LEN
        );
        assert_eq!(
            decode_configuration_image(&image).unwrap().objects().len(),
            0
        );
        assert_eq!(
            encode_configuration_image(NO_OBJECTS, &mut image[..15]),
            Err(StorageError::ImageBufferTooSmall)
        );

        // Every distinct explicit channel identifier: the largest object set an
        // image can hold now that the reserved range belongs to expansion.
        let maximum = (0..GENERATED_CHANNEL_ID_BASE)
            .map(|id| encode_channel(channel(id, "A")).unwrap())
            .collect::<Vec<_>>();
        let maximum_len = configuration_image_len(&maximum).unwrap();
        let mut maximum_image = vec![0; maximum_len];
        assert_eq!(
            encode_configuration_image(&maximum, &mut maximum_image).unwrap(),
            maximum_len
        );
        let decoded_maximum = decode_configuration_image(&maximum_image).unwrap();
        assert_eq!(decoded_maximum.object_count(), GENERATED_CHANNEL_ID_BASE);
        assert_eq!(
            decoded_maximum.objects().len(),
            usize::from(GENERATED_CHANNEL_ID_BASE)
        );

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

#[cfg(test)]
mod image_writer_tests {
    use super::{
        configuration_image_len_from_header, decode_configuration_image, encode_channel,
        encode_configuration_image, encode_radio_config, ConfigurationImageWriter, StorageError,
        CONFIGURATION_IMAGE_HEADER_LEN,
    };
    use radio_channel_plan::{
        BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
        SquelchLevel, Tone, TxClass,
    };

    fn channel(id: u16) -> ChannelRecord {
        let receive = Frequency::from_hz(145_000_000 + u32::from(id) * 12_500).expect("frequency");
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("CH").expect("name"),
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
        .expect("channel")
    }

    #[test]
    fn the_incremental_writer_produces_the_slice_encoding_byte_for_byte() {
        let objects = [
            encode_channel(channel(1)).expect("channel"),
            encode_channel(channel(2)).expect("channel"),
            encode_radio_config(RadioConfig::conservative()).expect("config"),
        ];
        let mut expected = [0_u8; 256];
        let expected_len = encode_configuration_image(&objects, &mut expected).expect("encode");

        let mut streamed = [0_u8; 256];
        let mut writer = ConfigurationImageWriter::new(&mut streamed, 3).expect("writer");
        for object in &objects {
            writer.push(object).expect("push");
        }
        assert_eq!(writer.finish(), Ok(expected_len));
        assert_eq!(streamed[..expected_len], expected[..expected_len]);
        assert!(decode_configuration_image(&streamed[..expected_len]).is_ok());
    }

    #[test]
    fn the_writer_refuses_unordered_extra_and_missing_objects() {
        let first = encode_channel(channel(1)).expect("channel");
        let second = encode_channel(channel(2)).expect("channel");

        let mut buffer = [0_u8; 256];
        let mut writer = ConfigurationImageWriter::new(&mut buffer, 2).expect("writer");
        writer.push(&second).expect("push");
        assert_eq!(writer.push(&first), Err(StorageError::NonCanonicalImage));
        assert_eq!(writer.finish(), Err(StorageError::MalformedImage));

        let mut buffer = [0_u8; 256];
        let mut writer = ConfigurationImageWriter::new(&mut buffer, 1).expect("writer");
        writer.push(&first).expect("push");
        assert_eq!(writer.push(&second), Err(StorageError::ImageTooLarge));

        let mut small = [0_u8; CONFIGURATION_IMAGE_HEADER_LEN + 4];
        let mut writer = ConfigurationImageWriter::new(&mut small, 1).expect("writer");
        assert_eq!(writer.push(&first), Err(StorageError::ImageBufferTooSmall));
        assert_eq!(
            ConfigurationImageWriter::new(&mut [0_u8; 4], 1).err(),
            Some(StorageError::ImageBufferTooSmall)
        );
    }

    #[test]
    fn a_header_reports_its_exact_length_and_rejects_erased_bytes() {
        let objects = [encode_channel(channel(1)).expect("channel")];
        let mut image = [0_u8; 256];
        let length = encode_configuration_image(&objects, &mut image).expect("encode");
        assert_eq!(configuration_image_len_from_header(&image), Ok(length));

        assert_eq!(
            configuration_image_len_from_header(&[0xFF_u8; 32]),
            Err(StorageError::MalformedImage)
        );
        let mut wrong_version = image;
        wrong_version[4] = 9;
        assert_eq!(
            configuration_image_len_from_header(&wrong_version),
            Err(StorageError::UnsupportedImageVersion)
        );
    }
}
