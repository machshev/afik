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
    /// A value was outside its defined domain envelope.
    OutOfRange,
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => formatter.write_str("value must be non-zero"),
            Self::Overflow => formatter.write_str("radio-domain arithmetic overflow"),
            Self::OutOfRange => formatter.write_str("value is outside its accepted range"),
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

/// Lowest CTCSS frequency accepted in tenths of a hertz.
pub const MIN_CTCSS_TENTHS_HZ: u16 = 670;
/// Highest CTCSS frequency accepted in tenths of a hertz.
pub const MAX_CTCSS_TENTHS_HZ: u16 = 2541;

/// A squelch access tone or code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    /// No tone or code is used.
    None,
    /// CTCSS frequency in tenths of a hertz.
    Ctcss(u16),
    /// DCS code and inversion state.
    Dcs {
        /// The three-digit DCS code represented as octal digits.
        code: u16,
        /// Whether inverted DCS polarity is used.
        inverted: bool,
    },
}

impl Tone {
    /// Constructs a CTCSS tone inside the accepted tenths-of-a-hertz envelope.
    pub const fn ctcss(tenths_hz: u16) -> Result<Self, DomainError> {
        if tenths_hz < MIN_CTCSS_TENTHS_HZ || tenths_hz > MAX_CTCSS_TENTHS_HZ {
            Err(DomainError::OutOfRange)
        } else {
            Ok(Self::Ctcss(tenths_hz))
        }
    }

    /// Constructs a DCS code from three octal digits held as a decimal integer.
    pub const fn dcs(code: u16, inverted: bool) -> Result<Self, DomainError> {
        if code == 0 || code > 777 {
            return Err(DomainError::OutOfRange);
        }
        let mut remaining = code;
        while remaining != 0 {
            if remaining % 10 > 7 {
                return Err(DomainError::OutOfRange);
            }
            remaining /= 10;
        }
        Ok(Self::Dcs { code, inverted })
    }

    /// Reports whether this tone requires receive-side tone squelch.
    pub const fn is_some(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Supported high-level modulation families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Modulation {
    /// Frequency modulation.
    Fm = 0,
    /// Amplitude modulation.
    Am = 1,
    /// Upper-sideband receive-only demodulation.
    Usb = 2,
}

impl TryFrom<u8> for Modulation {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Fm),
            1 => Ok(Self::Am),
            2 => Ok(Self::Usb),
            _ => Err(DomainError::OutOfRange),
        }
    }
}

/// Highest accepted squelch level. Level zero disables carrier squelch.
pub const MAX_SQUELCH_LEVEL: u8 = 9;

/// A bounded operator squelch level.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct SquelchLevel(u8);

impl SquelchLevel {
    /// The level at which carrier squelch is disabled and audio always passes.
    pub const OPEN: Self = Self(0);

    /// The level a radio uses until an operator chooses another.
    pub const CONSERVATIVE: Self = Self(3);

    /// Constructs a squelch level inside the accepted envelope.
    pub const fn new(level: u8) -> Result<Self, DomainError> {
        if level > MAX_SQUELCH_LEVEL {
            Err(DomainError::OutOfRange)
        } else {
            Ok(Self(level))
        }
    }

    /// Returns the numeric level.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Reports whether carrier squelch is disabled at this level.
    pub const fn is_open(self) -> bool {
        self.0 == 0
    }
}

/// A hardware-independent occupied-bandwidth selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Bandwidth {
    /// Narrow channel bandwidth.
    Narrow = 0,
    /// Wide channel bandwidth.
    Wide = 1,
}

impl TryFrom<u8> for Bandwidth {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Narrow),
            1 => Ok(Self::Wide),
            _ => Err(DomainError::OutOfRange),
        }
    }
}

/// A hardware-independent requested power level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PowerLevel {
    /// Lowest available radio power.
    Low = 0,
    /// Intermediate radio power.
    Medium = 1,
    /// Highest available radio power.
    High = 2,
}

impl TryFrom<u8> for PowerLevel {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Low),
            1 => Ok(Self::Medium),
            2 => Ok(Self::High),
            _ => Err(DomainError::OutOfRange),
        }
    }
}

/// Behaviour after a scan stops on an occupied channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ScanResume {
    /// Resume scanning after the configured hold expires.
    TimeOut = 0,
    /// Resume scanning only once the carrier disappears.
    Carrier = 1,
    /// Stop scanning and stay on the channel.
    Stop = 2,
}

impl TryFrom<u8> for ScanResume {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::TimeOut),
            1 => Ok(Self::Carrier),
            2 => Ok(Self::Stop),
            _ => Err(DomainError::OutOfRange),
        }
    }
}

/// Highest accepted receive battery-save duty ratio. Zero disables saving.
pub const MAX_BATTERY_SAVE_RATIO: u8 = 5;
/// Backlight timeout value meaning the backlight never switches off.
pub const BACKLIGHT_ALWAYS_ON: u8 = u8::MAX;

/// Global receive-side radio behaviour flags.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RadioFlags {
    bits: u8,
}

impl RadioFlags {
    /// Key presses produce an audible confirmation.
    pub const KEY_BEEP: u8 = 0b0000_0001;
    /// New channels default to busy-channel lockout.
    pub const BUSY_LOCKOUT_DEFAULT: u8 = 0b0000_0010;
    /// The receiver applies the AM gain-compensation workaround.
    pub const AM_FIX: u8 = 0b0000_0100;
    /// Squelch tail elimination is requested on tone-coded channels.
    pub const TONE_TAIL_ELIMINATION: u8 = 0b0000_1000;
    /// Bits which must be zero in this format version.
    pub const RESERVED: u8 = 0b1111_0000;

    /// Validates a raw flag field.
    pub const fn from_bits(bits: u8) -> Result<Self, DomainError> {
        if bits & Self::RESERVED != 0 {
            Err(DomainError::OutOfRange)
        } else {
            Ok(Self { bits })
        }
    }

    /// Returns the raw flag field.
    pub const fn bits(self) -> u8 {
        self.bits
    }

    /// Reports whether every requested flag is set.
    pub const fn contains(self, flag: u8) -> bool {
        self.bits & flag == flag
    }

    /// Returns these flags with one flag set or cleared.
    #[must_use]
    pub const fn with(self, flag: u8, enabled: bool) -> Self {
        let bits = if enabled {
            self.bits | flag
        } else {
            self.bits & !flag
        };
        Self { bits }
    }
}

/// Global receive-side radio configuration carrying no transmit authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RadioConfig {
    /// Squelch level applied when no channel overrides it.
    pub squelch: SquelchLevel,
    /// Backlight timeout in seconds; zero disables and 255 never times out.
    pub backlight_seconds: u8,
    /// Behaviour after a scan stops on an occupied channel.
    pub scan_resume: ScanResume,
    /// Non-zero no-signal scan dwell in milliseconds.
    ///
    /// How long a scan listens to a channel before moving on. The useful floor
    /// is a property of the radio rather than of this type: an image has to
    /// retune, let its receiver settle, and take a reading that means something
    /// before the dwell expires. `EVID-K1-069` and `EVID-K1-071` record where
    /// that floor was found on the K1. Nothing is clamped here, so a device
    /// reads back exactly what a host wrote to it.
    pub scan_dwell_ms: u32,
    /// Non-zero open-squelch scan hold in milliseconds.
    pub scan_hold_ms: u32,
    /// Whether the alternate-channel dual watch is enabled.
    pub dual_watch: bool,
    /// Receive battery-save duty ratio; zero disables saving.
    pub battery_save_ratio: u8,
    /// Global behaviour flags.
    pub flags: RadioFlags,
}

impl RadioConfig {
    /// Returns a conservative default configuration.
    pub const fn conservative() -> Self {
        Self {
            squelch: SquelchLevel::CONSERVATIVE,
            backlight_seconds: 10,
            scan_resume: ScanResume::TimeOut,
            // Measured on the exact K1 unit rather than chosen: 100 ms stopped
            // on a signal and 60 ms did not, and bisecting that bracket landed
            // here. `EVID-K1-071`. A radio which needs longer is programmed
            // with longer; this is only what one arrives with.
            scan_dwell_ms: 90,
            scan_hold_ms: 5_000,
            dual_watch: false,
            battery_save_ratio: 0,
            flags: RadioFlags { bits: 0 },
        }
    }

    /// Revalidates every constrained field.
    pub const fn validate(self) -> Result<Self, DomainError> {
        if self.scan_dwell_ms == 0 || self.scan_hold_ms == 0 {
            return Err(DomainError::Zero);
        }
        if self.battery_save_ratio > MAX_BATTERY_SAVE_RATIO {
            return Err(DomainError::OutOfRange);
        }
        Ok(self)
    }
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
    fn tones_squelch_and_enumerations_validate_their_envelopes() {
        use super::{Bandwidth, Modulation, PowerLevel, SquelchLevel, Tone};

        assert_eq!(Tone::ctcss(670), Ok(Tone::Ctcss(670)));
        assert_eq!(Tone::ctcss(2541), Ok(Tone::Ctcss(2541)));
        assert_eq!(Tone::ctcss(669), Err(DomainError::OutOfRange));
        assert_eq!(Tone::ctcss(2542), Err(DomainError::OutOfRange));
        assert_eq!(
            Tone::dcs(23, true),
            Ok(Tone::Dcs {
                code: 23,
                inverted: true
            })
        );
        assert!(Tone::dcs(754, false).unwrap().is_some());
        assert_eq!(Tone::dcs(0, false), Err(DomainError::OutOfRange));
        assert_eq!(Tone::dcs(778, false), Err(DomainError::OutOfRange));
        assert_eq!(Tone::dcs(800, false), Err(DomainError::OutOfRange));
        assert!(!Tone::None.is_some());

        assert_eq!(SquelchLevel::new(9).unwrap().get(), 9);
        assert!(SquelchLevel::new(0).unwrap().is_open());
        assert_eq!(SquelchLevel::new(10), Err(DomainError::OutOfRange));

        assert_eq!(Modulation::try_from(2), Ok(Modulation::Usb));
        assert_eq!(Modulation::try_from(3), Err(DomainError::OutOfRange));
        assert_eq!(Bandwidth::try_from(1), Ok(Bandwidth::Wide));
        assert_eq!(Bandwidth::try_from(2), Err(DomainError::OutOfRange));
        assert_eq!(PowerLevel::try_from(2), Ok(PowerLevel::High));
        assert_eq!(PowerLevel::try_from(3), Err(DomainError::OutOfRange));
    }

    #[test]
    fn radio_configuration_validates_its_envelope() {
        use super::{RadioConfig, RadioFlags, ScanResume};

        let config = RadioConfig::conservative();
        assert_eq!(config.validate(), Ok(config));
        assert_eq!(config.scan_resume, ScanResume::TimeOut);

        let mut zero_dwell = config;
        zero_dwell.scan_dwell_ms = 0;
        assert_eq!(zero_dwell.validate(), Err(DomainError::Zero));
        let mut zero_hold = config;
        zero_hold.scan_hold_ms = 0;
        assert_eq!(zero_hold.validate(), Err(DomainError::Zero));
        let mut saving = config;
        saving.battery_save_ratio = 6;
        assert_eq!(saving.validate(), Err(DomainError::OutOfRange));

        assert_eq!(ScanResume::try_from(2), Ok(ScanResume::Stop));
        assert_eq!(ScanResume::try_from(3), Err(DomainError::OutOfRange));
        assert_eq!(RadioFlags::from_bits(0x10), Err(DomainError::OutOfRange));
        assert!(RadioFlags::from_bits(0x0F)
            .unwrap()
            .contains(RadioFlags::AM_FIX));
    }

    #[test]
    fn invalid_or_overflowing_frequency_is_rejected() {
        assert_eq!(Frequency::from_hz(0), Err(DomainError::Zero));
        let base = Frequency::from_hz(u32::MAX).unwrap();
        let step = FrequencyStep::from_hz(1).unwrap();
        assert_eq!(base.checked_add_steps(step, 1), Err(DomainError::Overflow));
    }
}
