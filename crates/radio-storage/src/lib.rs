//! Bounded object encoding and logically transactional configuration storage.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use radio_channel_plan::{BankName, GeneratedBank, MAX_BANK_NAME_LEN};
use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};

/// Current object encoding version.
pub const STORAGE_FORMAT_VERSION: u8 = 1;
/// Maximum bytes held by one device object in the first storage model.
pub const MAX_OBJECT_DATA: usize = 64;
/// Encoded byte length of a version-1 generated-bank object.
pub const GENERATED_BANK_ENCODED_LEN: usize = 31;

/// Stable configuration object kinds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ObjectKind {
    /// A compact generated channel bank.
    GeneratedBank = 1,
}

impl TryFrom<u8> for ObjectKind {
    type Error = StorageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::GeneratedBank),
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

/// Validates any currently supported configuration object.
pub fn validate_object(object: &StorageObject) -> bool {
    match object.key.kind {
        ObjectKind::GeneratedBank => decode_generated_bank(object).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_generated_bank, encode_generated_bank, validate_object, ObjectKey, ObjectKind,
        StorageError, TransactionalStore,
    };
    use radio_channel_plan::{BankName, GeneratedBank};
    use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};

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

    #[test]
    fn generated_bank_encoding_round_trips() {
        let expected = bank(4, "PMR446");
        let encoded = encode_generated_bank(expected).unwrap();
        assert_eq!(decode_generated_bank(&encoded).unwrap(), expected);
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
}
