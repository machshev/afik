//! Bounded, hardware-independent channel-plan encodings.

#![no_std]
#![forbid(unsafe_code)]

use core::{fmt, str};
use radio_domain::{ActiveChannel, BankId, DomainError, Frequency, FrequencyStep, TxClass};

/// Maximum encoded byte length of a generated bank name.
pub const MAX_BANK_NAME_LEN: usize = 16;

/// Failure while constructing or expanding a channel plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// The bank name was empty, too long, or contained unsupported bytes.
    InvalidName,
    /// A generated bank contained no channels.
    EmptyBank,
    /// The requested channel index was outside the bank.
    ChannelOutOfRange,
    /// Channel expansion overflowed the frequency representation.
    FrequencyOverflow,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("invalid generated-bank name"),
            Self::EmptyBank => formatter.write_str("generated bank must contain channels"),
            Self::ChannelOutOfRange => formatter.write_str("channel index is outside bank"),
            Self::FrequencyOverflow => formatter.write_str("generated frequency overflow"),
        }
    }
}

impl From<DomainError> for PlanError {
    fn from(_: DomainError) -> Self {
        Self::FrequencyOverflow
    }
}

/// A compact, display-safe bank name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BankName {
    bytes: [u8; MAX_BANK_NAME_LEN],
    len: u8,
}

impl BankName {
    /// Constructs a non-empty printable ASCII name.
    pub fn new(name: &str) -> Result<Self, PlanError> {
        if name.is_empty()
            || name.len() > MAX_BANK_NAME_LEN
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        {
            return Err(PlanError::InvalidName);
        }

        let mut bytes = [0; MAX_BANK_NAME_LEN];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            bytes,
            len: u8::try_from(name.len()).map_err(|_| PlanError::InvalidName)?,
        })
    }

    /// Reconstructs a name from its fixed field and explicit length.
    pub fn from_field(bytes: [u8; MAX_BANK_NAME_LEN], len: u8) -> Result<Self, PlanError> {
        let length = usize::from(len);
        if length == 0
            || length > MAX_BANK_NAME_LEN
            || bytes[length..].iter().any(|byte| *byte != 0)
        {
            return Err(PlanError::InvalidName);
        }
        let name = str::from_utf8(&bytes[..length]).map_err(|_| PlanError::InvalidName)?;
        Self::new(name)
    }

    /// Returns the name as a string.
    pub fn as_str(&self) -> &str {
        // Construction permits ASCII only, so this cannot fail.
        str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }

    /// Returns the fixed-size encoded field.
    pub const fn field(self) -> [u8; MAX_BANK_NAME_LEN] {
        self.bytes
    }

    /// Returns the encoded string length.
    pub const fn len(self) -> u8 {
        self.len
    }

    /// Reports whether the name is empty. Valid constructed names are never empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Compact channel-plan encoding families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PlanEncoding {
    /// Arithmetic receive frequencies with simplex transmit frequencies.
    LinearSimplex = 0,
    /// Arithmetic receive frequencies with one fixed transmit offset.
    LinearFixedOffset = 1,
    /// Arithmetic frequencies with a common access tone.
    LinearToned = 2,
    /// Explicit simplex frequency table.
    TableSimplex = 3,
    /// Explicit mixed simplex and duplex table.
    TableMixedDuplex = 4,
    /// Generated plan plus sparse per-channel exceptions.
    SparseExceptions = 5,
}

impl PlanEncoding {
    /// Returns the bit used in capability negotiation.
    pub const fn capability_bit(self) -> u16 {
        1_u16 << (self as u8)
    }
}

/// A bounded arithmetic simplex channel bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeneratedBank {
    id: BankId,
    name: BankName,
    base: Frequency,
    spacing: FrequencyStep,
    channel_count: u16,
    tx_class: TxClass,
}

impl GeneratedBank {
    /// Constructs and validates an arithmetic simplex bank.
    pub fn linear_simplex(
        id: BankId,
        name: BankName,
        base: Frequency,
        spacing: FrequencyStep,
        channel_count: u16,
        tx_class: TxClass,
    ) -> Result<Self, PlanError> {
        if channel_count == 0 {
            return Err(PlanError::EmptyBank);
        }
        base.checked_add_steps(spacing, channel_count - 1)?;
        Ok(Self {
            id,
            name,
            base,
            spacing,
            channel_count,
            tx_class,
        })
    }

    /// Returns the bank identifier.
    pub const fn id(self) -> BankId {
        self.id
    }

    /// Returns the bank name.
    pub const fn name(self) -> BankName {
        self.name
    }

    /// Returns the first channel frequency.
    pub const fn base(self) -> Frequency {
        self.base
    }

    /// Returns the channel spacing.
    pub const fn spacing(self) -> FrequencyStep {
        self.spacing
    }

    /// Returns the number of channels.
    pub const fn channel_count(self) -> u16 {
        self.channel_count
    }

    /// Returns the trusted TX classification.
    pub const fn tx_class(self) -> TxClass {
        self.tx_class
    }

    /// Returns the compact encoding family.
    pub const fn encoding(self) -> PlanEncoding {
        PlanEncoding::LinearSimplex
    }

    /// Expands one channel without materialising the rest of the bank.
    pub fn channel(self, index: u16) -> Result<ActiveChannel, PlanError> {
        if index >= self.channel_count {
            return Err(PlanError::ChannelOutOfRange);
        }
        let frequency = self.base.checked_add_steps(self.spacing, index)?;
        Ok(ActiveChannel {
            receive: frequency,
            transmit: frequency,
            tx_class: self.tx_class,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{BankName, GeneratedBank, PlanError};
    use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};

    #[test]
    fn linear_bank_expands_lazily() {
        let bank = GeneratedBank::linear_simplex(
            BankId::new(7),
            BankName::new("PMR446").unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
        )
        .unwrap();

        assert_eq!(bank.channel(6).unwrap().receive.as_hz(), 446_081_250);
        assert_eq!(bank.channel(16), Err(PlanError::ChannelOutOfRange));
    }

    #[test]
    fn names_and_bank_bounds_are_validated() {
        assert_eq!(BankName::new(""), Err(PlanError::InvalidName));
        assert_eq!(BankName::new("not\nprintable"), Err(PlanError::InvalidName));
        let result = GeneratedBank::linear_simplex(
            BankId::new(1),
            BankName::new("empty").unwrap(),
            Frequency::from_hz(1).unwrap(),
            FrequencyStep::from_hz(1).unwrap(),
            0,
            TxClass::Never,
        );
        assert_eq!(result, Err(PlanError::EmptyBank));
    }
}
