//! Heap-free hardware-independent radio domain types.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

/// A domain value failed validation or checked arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainError {
    /// A value which must be non-zero was zero.
    Zero,
    /// A checked calculation exceeded its integer representation.
    Overflow,
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => formatter.write_str("value must be non-zero"),
            Self::Overflow => formatter.write_str("radio-domain arithmetic overflow"),
        }
    }
}

/// An absolute frequency stored as integer hertz.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Frequency(u32);

impl Frequency {
    /// Constructs a non-zero frequency.
    pub const fn from_hz(hz: u32) -> Result<Self, DomainError> {
        if hz == 0 {
            Err(DomainError::Zero)
        } else {
            Ok(Self(hz))
        }
    }

    /// Returns the frequency in hertz.
    pub const fn as_hz(self) -> u32 {
        self.0
    }

    /// Adds a number of equal frequency steps using checked arithmetic.
    pub fn checked_add_steps(self, step: FrequencyStep, count: u16) -> Result<Self, DomainError> {
        let delta = step
            .as_hz()
            .checked_mul(u32::from(count))
            .ok_or(DomainError::Overflow)?;
        let hz = self.0.checked_add(delta).ok_or(DomainError::Overflow)?;
        Self::from_hz(hz)
    }

    /// Applies a signed transmit or receive offset using checked arithmetic.
    pub fn checked_apply_offset(self, offset: Offset) -> Result<Self, DomainError> {
        let hz = self
            .0
            .checked_add_signed(offset.as_hz())
            .ok_or(DomainError::Overflow)?;
        Self::from_hz(hz)
    }
}

/// A positive channel spacing stored as integer hertz.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FrequencyStep(u32);

impl FrequencyStep {
    /// Constructs a non-zero frequency step.
    pub const fn from_hz(hz: u32) -> Result<Self, DomainError> {
        if hz == 0 {
            Err(DomainError::Zero)
        } else {
            Ok(Self(hz))
        }
    }

    /// Returns the step in hertz.
    pub const fn as_hz(self) -> u32 {
        self.0
    }
}

/// A signed frequency offset stored as integer hertz.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Offset(i32);

impl Offset {
    /// Constructs an offset. Zero represents simplex operation.
    pub const fn from_hz(hz: i32) -> Self {
        Self(hz)
    }

    /// Returns the signed offset in hertz.
    pub const fn as_hz(self) -> i32 {
        self.0
    }
}

/// A squelch access tone or code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    /// No tone or code is used.
    None,
    /// CTCSS frequency in tenths of a hertz.
    Ctcss(u16),
    /// DCS code and inversion state.
    Dcs {
        /// The three-digit DCS code represented as an integer.
        code: u16,
        /// Whether inverted DCS polarity is used.
        inverted: bool,
    },
}

/// Supported high-level modulation families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Modulation {
    /// Frequency modulation.
    Fm,
    /// Amplitude modulation.
    Am,
}

/// A hardware-independent occupied-bandwidth selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bandwidth {
    /// Narrow channel bandwidth.
    Narrow,
    /// Wide channel bandwidth.
    Wide,
}

/// A hardware-independent requested power level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerLevel {
    /// Lowest available radio power.
    Low,
    /// Intermediate radio power.
    Medium,
    /// Highest available radio power.
    High,
}

/// Stable identifier for a configured channel.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ChannelId(u16);

impl ChannelId {
    /// Constructs a channel identifier.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the numeric identifier.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Stable identifier for a channel bank.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BankId(u16);

impl BankId {
    /// Constructs a bank identifier.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the numeric identifier.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Stable identifier for an installed regional plan.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegionId(u16);

impl RegionId {
    /// Constructs a region identifier.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the numeric identifier.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Trusted policy classification assigned to an active channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TxClass {
    /// Transmission must never be authorised.
    Never = 0,
    /// A validated licence-free regional plan.
    LicenceFreePlan = 1,
    /// Amateur service.
    Amateur = 2,
    /// Marine service.
    Marine = 3,
    /// Aeronautical service.
    Aeronautical = 4,
    /// Business or commercial service.
    Business = 5,
    /// Experimental service.
    Experimental = 6,
}

impl TryFrom<u8> for TxClass {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Never),
            1 => Ok(Self::LicenceFreePlan),
            2 => Ok(Self::Amateur),
            3 => Ok(Self::Marine),
            4 => Ok(Self::Aeronautical),
            5 => Ok(Self::Business),
            6 => Ok(Self::Experimental),
            _ => Err(DomainError::Overflow),
        }
    }
}

/// Fully resolved frequencies and policy metadata for the selected channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveChannel {
    /// Receive frequency.
    pub receive: Frequency,
    /// Transmit frequency.
    pub transmit: Frequency,
    /// Centrally enforced transmit classification.
    pub tx_class: TxClass,
}

/// A bounded signal measurement independent of receiver hardware units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignalMeasurement {
    /// Normalised signal strength from 0 through 255.
    pub strength: u8,
    /// Whether the receiver's squelch criterion is open.
    pub squelch_open: bool,
}

#[cfg(test)]
mod tests {
    use super::{DomainError, Frequency, FrequencyStep, Offset};

    #[test]
    fn checked_frequency_arithmetic() {
        let base = Frequency::from_hz(446_006_250).unwrap();
        let step = FrequencyStep::from_hz(12_500).unwrap();
        assert_eq!(
            base.checked_add_steps(step, 6).unwrap().as_hz(),
            446_081_250
        );
        assert_eq!(
            base.checked_apply_offset(Offset::from_hz(-600_000))
                .unwrap()
                .as_hz(),
            445_406_250
        );
    }

    #[test]
    fn invalid_or_overflowing_frequency_is_rejected() {
        assert_eq!(Frequency::from_hz(0), Err(DomainError::Zero));
        let base = Frequency::from_hz(u32::MAX).unwrap();
        let step = FrequencyStep::from_hz(1).unwrap();
        assert_eq!(base.checked_add_steps(step, 1), Err(DomainError::Overflow));
    }
}
