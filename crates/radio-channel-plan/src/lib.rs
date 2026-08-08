//! Bounded, hardware-independent channel-plan encodings.

#![no_std]
#![forbid(unsafe_code)]

use core::{fmt, str};
use radio_domain::{
    ActiveChannel, Bandwidth, BankId, ChannelId, DomainError, Frequency, FrequencyStep, Modulation,
    PowerLevel, SquelchLevel, Tone, TxClass,
};

/// Maximum encoded byte length of a generated bank name.
pub const MAX_BANK_NAME_LEN: usize = 16;
/// Maximum encoded byte length of an explicit channel name.
pub const MAX_CHANNEL_NAME_LEN: usize = 12;
/// Number of banks addressable by one channel membership mask.
pub const MAX_BANKS: u16 = 16;
/// Lowest channel identifier reserved for channels a radio expands itself.
///
/// Explicit channel records are stored one object each and may use any
/// identifier below this. Everything from here up is minted by expanding a
/// generated plan, so a stored channel can never collide with an expanded one.
pub const GENERATED_CHANNEL_ID_BASE: u16 = 0x8000;
/// Channels one generated plan may contain.
///
/// The identifier of an expanded channel packs the bank identifier and the
/// index into the reserved range, which bounds both.
pub const MAX_GENERATED_CHANNELS: u16 = 1 << 11;

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
    /// A CTCSS frequency or DCS code was outside its accepted envelope.
    InvalidTone,
    /// A bank identifier was outside the addressable membership range.
    BankOutOfRange,
    /// A reserved flag bit was set.
    ReservedFlag,
    /// A generated plan held more channels than one plan may contain.
    TooManyChannels,
    /// An explicit channel claimed an identifier reserved for expansion.
    ReservedChannelId,
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("invalid generated-bank name"),
            Self::EmptyBank => formatter.write_str("generated bank must contain channels"),
            Self::ChannelOutOfRange => formatter.write_str("channel index is outside bank"),
            Self::FrequencyOverflow => formatter.write_str("generated frequency overflow"),
            Self::InvalidTone => formatter.write_str("invalid CTCSS frequency or DCS code"),
            Self::BankOutOfRange => formatter.write_str("bank identifier is outside range"),
            Self::ReservedFlag => formatter.write_str("reserved flag bit is set"),
            Self::TooManyChannels => formatter.write_str("generated plan holds too many channels"),
            Self::ReservedChannelId => {
                formatter.write_str("channel identifier is reserved for generated plans")
            }
        }
    }
}

impl From<DomainError> for PlanError {
    fn from(_: DomainError) -> Self {
        Self::FrequencyOverflow
    }
}

/// A compact, display-safe fixed-capacity name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedName<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    len: u8,
}

impl<const CAPACITY: usize> FixedName<CAPACITY> {
    /// Constructs a non-empty printable ASCII name.
    pub fn new(name: &str) -> Result<Self, PlanError> {
        if name.is_empty()
            || name.len() > CAPACITY
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        {
            return Err(PlanError::InvalidName);
        }

        let mut bytes = [0; CAPACITY];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            bytes,
            len: u8::try_from(name.len()).map_err(|_| PlanError::InvalidName)?,
        })
    }

    /// Reconstructs a name from its fixed field and explicit length.
    pub fn from_field(bytes: [u8; CAPACITY], len: u8) -> Result<Self, PlanError> {
        let length = usize::from(len);
        if length == 0 || length > CAPACITY || bytes[length..].iter().any(|byte| *byte != 0) {
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
    pub const fn field(self) -> [u8; CAPACITY] {
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

/// A compact, display-safe bank name.
pub type BankName = FixedName<MAX_BANK_NAME_LEN>;
/// A compact, display-safe explicit channel name.
pub type ChannelName = FixedName<MAX_CHANNEL_NAME_LEN>;

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

/// The per-channel settings every channel of a generated plan shares.
///
/// A generated plan stores one of these instead of one complete channel record
/// per channel, which is what makes the plan cheap: the whole bank costs one
/// object however many channels it expands to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelTemplate {
    /// Receive-side tone squelch requirement.
    pub rx_tone: Tone,
    /// Transmit-side tone requirement.
    pub tx_tone: Tone,
    /// Requested modulation family.
    pub modulation: Modulation,
    /// Requested occupied bandwidth.
    pub bandwidth: Bandwidth,
    /// Requested transmit power level.
    pub power: PowerLevel,
    /// Manual tuning step used from an expanded channel.
    pub step: FrequencyStep,
    /// Channel squelch level.
    pub squelch: SquelchLevel,
    /// Channel behaviour flags.
    pub flags: ChannelFlags,
}

impl ChannelTemplate {
    /// Returns the conservative narrow-FM template tuned by the plan spacing.
    ///
    /// The manual tuning step follows the channel spacing, so stepping off an
    /// expanded channel lands on the next one in the plan.
    #[must_use]
    pub const fn narrow_fm(spacing: FrequencyStep) -> Self {
        Self {
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step: spacing,
            squelch: SquelchLevel::CONSERVATIVE,
            flags: ChannelFlags::empty(),
        }
    }

    fn validate(self) -> Result<Self, PlanError> {
        validate_tone(self.rx_tone)?;
        validate_tone(self.tx_tone)?;
        Ok(self)
    }
}

/// A bounded arithmetic simplex channel bank.
///
/// One stored plan expands to [`channel_count`](Self::channel_count) complete
/// channel records without storing any of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeneratedBank {
    id: BankId,
    name: BankName,
    base: Frequency,
    spacing: FrequencyStep,
    channel_count: u16,
    tx_class: TxClass,
    template: ChannelTemplate,
}

impl GeneratedBank {
    /// Constructs an arithmetic simplex bank with the conservative template.
    pub fn linear_simplex(
        id: BankId,
        name: BankName,
        base: Frequency,
        spacing: FrequencyStep,
        channel_count: u16,
        tx_class: TxClass,
    ) -> Result<Self, PlanError> {
        Self::linear_simplex_with(
            id,
            name,
            base,
            spacing,
            channel_count,
            tx_class,
            ChannelTemplate::narrow_fm(spacing),
        )
    }

    /// Constructs and validates an arithmetic simplex bank and its template.
    pub fn linear_simplex_with(
        id: BankId,
        name: BankName,
        base: Frequency,
        spacing: FrequencyStep,
        channel_count: u16,
        tx_class: TxClass,
        template: ChannelTemplate,
    ) -> Result<Self, PlanError> {
        if channel_count == 0 {
            return Err(PlanError::EmptyBank);
        }
        if id.get() >= MAX_BANKS {
            return Err(PlanError::BankOutOfRange);
        }
        if channel_count > MAX_GENERATED_CHANNELS {
            return Err(PlanError::TooManyChannels);
        }
        base.checked_add_steps(spacing, channel_count - 1)?;
        Ok(Self {
            id,
            name,
            base,
            spacing,
            channel_count,
            tx_class,
            template: template.validate()?,
        })
    }

    /// Returns the per-channel settings every expanded channel shares.
    pub const fn template(self) -> ChannelTemplate {
        self.template
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

    /// Expands one complete channel record without materialising the rest.
    ///
    /// The record is indistinguishable from a stored one: it carries the plan
    /// template, a derived name, membership of this bank only, and an
    /// identifier from the reserved generated range, so a radio can select,
    /// filter, and scan it beside explicit channels.
    pub fn channel_record(self, index: u16) -> Result<ChannelRecord, PlanError> {
        if index >= self.channel_count {
            return Err(PlanError::ChannelOutOfRange);
        }
        let frequency = self.base.checked_add_steps(self.spacing, index)?;
        ChannelRecord::expanded(ChannelDefinition {
            id: generated_channel_id(self.id, index)?,
            name: self.channel_name(index)?,
            receive: frequency,
            // Linear simplex: an expanded channel transmits where it receives.
            transmit: frequency,
            rx_tone: self.template.rx_tone,
            tx_tone: self.template.tx_tone,
            modulation: self.template.modulation,
            bandwidth: self.template.bandwidth,
            power: self.template.power,
            step: self.template.step,
            squelch: self.template.squelch,
            flags: self.template.flags,
            banks: BankMask::default().with(self.id, true)?,
            tx_class: self.tx_class,
        })
    }

    /// Returns the derived name of one expanded channel.
    ///
    /// The plan name is truncated so the position always fits, because the
    /// number is what the operator matches against the plan's documentation.
    pub fn channel_name(self, index: u16) -> Result<ChannelName, PlanError> {
        if index >= self.channel_count {
            return Err(PlanError::ChannelOutOfRange);
        }
        let digits = decimal_digits(self.channel_count);
        let name = self.name.as_str().as_bytes();
        let mut prefix = name.len().min(MAX_CHANNEL_NAME_LEN - 1 - digits);
        while prefix > 0 && name[prefix - 1] == b' ' {
            prefix -= 1;
        }
        let mut field = [0_u8; MAX_CHANNEL_NAME_LEN];
        field[..prefix].copy_from_slice(&name[..prefix]);
        field[prefix] = b' ';
        let len = prefix + 1 + digits;
        let mut position = index + 1;
        let mut digit = len;
        while digit > prefix + 1 {
            digit -= 1;
            field[digit] = b'0' + u8::try_from(position % 10).unwrap_or(0);
            position /= 10;
        }
        ChannelName::from_field(
            field,
            u8::try_from(len).map_err(|_| PlanError::InvalidName)?,
        )
    }
}

/// Returns the reserved identifier of one channel expanded from a plan.
pub const fn generated_channel_id(bank: BankId, index: u16) -> Result<ChannelId, PlanError> {
    if bank.get() >= MAX_BANKS {
        return Err(PlanError::BankOutOfRange);
    }
    if index >= MAX_GENERATED_CHANNELS {
        return Err(PlanError::TooManyChannels);
    }
    Ok(ChannelId::new(
        GENERATED_CHANNEL_ID_BASE | (bank.get() << 11) | index,
    ))
}

/// Reports whether one identifier belongs to a channel expanded from a plan.
pub const fn is_generated_channel_id(id: ChannelId) -> bool {
    id.get() >= GENERATED_CHANNEL_ID_BASE
}

const fn decimal_digits(count: u16) -> usize {
    if count >= 1_000 {
        4
    } else if count >= 100 {
        3
    } else {
        2
    }
}

/// Per-channel behaviour flags which carry no transmit authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChannelFlags {
    bits: u8,
}

impl ChannelFlags {
    /// The channel is skipped by bank scanning.
    pub const SCAN_SKIP: u8 = 0b0000_0001;
    /// Transmission is inhibited while the channel is busy.
    pub const BUSY_LOCKOUT: u8 = 0b0000_0010;
    /// Receive and transmit frequencies are exchanged when active.
    pub const REVERSE: u8 = 0b0000_0100;
    /// The audio compander is requested for this channel.
    pub const COMPANDER: u8 = 0b0000_1000;
    /// Bits which must be zero in this format version.
    pub const RESERVED: u8 = 0b1111_0000;

    /// Returns the flag field with no flag set.
    #[must_use]
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// Validates a raw flag field.
    pub const fn from_bits(bits: u8) -> Result<Self, PlanError> {
        if bits & Self::RESERVED != 0 {
            Err(PlanError::ReservedFlag)
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

/// Membership of one channel in up to [`MAX_BANKS`] banks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BankMask {
    bits: u16,
}

impl BankMask {
    /// Constructs a membership mask from its raw bit field.
    pub const fn from_bits(bits: u16) -> Self {
        Self { bits }
    }

    /// Returns the raw membership bit field.
    pub const fn bits(self) -> u16 {
        self.bits
    }

    /// Reports whether the channel belongs to the bank.
    pub const fn contains(self, bank: BankId) -> bool {
        match Self::bit(bank) {
            Ok(bit) => self.bits & bit != 0,
            Err(_) => false,
        }
    }

    /// Returns this mask with one addressable bank added or removed.
    pub const fn with(self, bank: BankId, member: bool) -> Result<Self, PlanError> {
        let bit = match Self::bit(bank) {
            Ok(bit) => bit,
            Err(error) => return Err(error),
        };
        let bits = if member {
            self.bits | bit
        } else {
            self.bits & !bit
        };
        Ok(Self { bits })
    }

    /// Reports whether the channel belongs to no bank.
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    const fn bit(bank: BankId) -> Result<u16, PlanError> {
        if bank.get() >= MAX_BANKS {
            Err(PlanError::BankOutOfRange)
        } else {
            Ok(1_u16 << bank.get())
        }
    }
}

/// A complete explicit channel definition independent of any radio hardware.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelRecord {
    id: ChannelId,
    name: ChannelName,
    receive: Frequency,
    transmit: Frequency,
    rx_tone: Tone,
    tx_tone: Tone,
    modulation: Modulation,
    bandwidth: Bandwidth,
    power: PowerLevel,
    step: FrequencyStep,
    squelch: SquelchLevel,
    flags: ChannelFlags,
    banks: BankMask,
    tx_class: TxClass,
}

/// Complete validated input required to construct a [`ChannelRecord`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelDefinition {
    /// Stable channel identifier.
    pub id: ChannelId,
    /// Display name.
    pub name: ChannelName,
    /// Receive frequency.
    pub receive: Frequency,
    /// Transmit frequency, equal to `receive` for simplex channels.
    pub transmit: Frequency,
    /// Receive-side tone squelch requirement.
    pub rx_tone: Tone,
    /// Transmit-side tone requirement.
    pub tx_tone: Tone,
    /// Requested modulation family.
    pub modulation: Modulation,
    /// Requested occupied bandwidth.
    pub bandwidth: Bandwidth,
    /// Requested transmit power level.
    pub power: PowerLevel,
    /// Manual tuning step used from this channel.
    pub step: FrequencyStep,
    /// Channel squelch level.
    pub squelch: SquelchLevel,
    /// Channel behaviour flags.
    pub flags: ChannelFlags,
    /// Bank membership mask.
    pub banks: BankMask,
    /// Trusted transmit classification.
    pub tx_class: TxClass,
}

impl ChannelRecord {
    /// Constructs a channel after revalidating every constrained field.
    ///
    /// Records built here are stored one object each, including the ones a
    /// generated plan expands, so only expansion may mint an identifier from
    /// the reserved range.
    pub fn new(definition: ChannelDefinition) -> Result<Self, PlanError> {
        if is_generated_channel_id(definition.id) {
            return Err(PlanError::ReservedChannelId);
        }
        Self::expanded(definition)
    }

    /// Constructs a channel which may carry a reserved expansion identifier.
    fn expanded(definition: ChannelDefinition) -> Result<Self, PlanError> {
        validate_tone(definition.rx_tone)?;
        validate_tone(definition.tx_tone)?;
        Ok(Self {
            id: definition.id,
            name: definition.name,
            receive: definition.receive,
            transmit: definition.transmit,
            rx_tone: definition.rx_tone,
            tx_tone: definition.tx_tone,
            modulation: definition.modulation,
            bandwidth: definition.bandwidth,
            power: definition.power,
            step: definition.step,
            squelch: definition.squelch,
            flags: definition.flags,
            banks: definition.banks,
            tx_class: definition.tx_class,
        })
    }

    /// Returns the stable channel identifier.
    pub const fn id(self) -> ChannelId {
        self.id
    }

    /// Returns the channel name.
    pub const fn name(self) -> ChannelName {
        self.name
    }

    /// Returns the stored receive frequency before any reverse flag applies.
    pub const fn receive(self) -> Frequency {
        self.receive
    }

    /// Returns the stored transmit frequency before any reverse flag applies.
    pub const fn transmit(self) -> Frequency {
        self.transmit
    }

    /// Returns the receive-side tone squelch requirement.
    pub const fn rx_tone(self) -> Tone {
        self.rx_tone
    }

    /// Returns the transmit-side tone requirement.
    pub const fn tx_tone(self) -> Tone {
        self.tx_tone
    }

    /// Returns the requested modulation family.
    pub const fn modulation(self) -> Modulation {
        self.modulation
    }

    /// Returns the requested occupied bandwidth.
    pub const fn bandwidth(self) -> Bandwidth {
        self.bandwidth
    }

    /// Returns the requested transmit power level.
    pub const fn power(self) -> PowerLevel {
        self.power
    }

    /// Returns the manual tuning step.
    pub const fn step(self) -> FrequencyStep {
        self.step
    }

    /// Returns the channel squelch level.
    pub const fn squelch(self) -> SquelchLevel {
        self.squelch
    }

    /// Returns the channel behaviour flags.
    pub const fn flags(self) -> ChannelFlags {
        self.flags
    }

    /// Returns the bank membership mask.
    pub const fn banks(self) -> BankMask {
        self.banks
    }

    /// Returns the trusted transmit classification.
    pub const fn tx_class(self) -> TxClass {
        self.tx_class
    }

    /// Reports whether the radio expanded this channel from a generated plan.
    pub const fn is_generated(self) -> bool {
        is_generated_channel_id(self.id)
    }

    /// Reports whether the channel belongs to a bank.
    pub const fn is_member_of(self, bank: BankId) -> bool {
        self.banks.contains(bank)
    }

    /// Reports whether bank scanning skips this channel.
    pub const fn is_scan_skipped(self) -> bool {
        self.flags.contains(ChannelFlags::SCAN_SKIP)
    }

    /// Resolves the operating frequencies, honouring the reverse flag.
    pub const fn active(self) -> ActiveChannel {
        let reversed = self.flags.contains(ChannelFlags::REVERSE);
        let (receive, transmit) = if reversed {
            (self.transmit, self.receive)
        } else {
            (self.receive, self.transmit)
        };
        ActiveChannel {
            receive,
            transmit,
            tx_class: self.tx_class,
        }
    }
}

/// Named bank metadata addressed by a channel membership mask.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelBank {
    id: BankId,
    name: BankName,
    flags: BankFlags,
}

/// Bank-level behaviour flags.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BankFlags {
    bits: u8,
}

impl BankFlags {
    /// The bank participates in scanning.
    pub const SCAN_ENABLED: u8 = 0b0000_0001;
    /// Bits which must be zero in this format version.
    pub const RESERVED: u8 = 0b1111_1110;

    /// Validates a raw flag field.
    pub const fn from_bits(bits: u8) -> Result<Self, PlanError> {
        if bits & Self::RESERVED != 0 {
            Err(PlanError::ReservedFlag)
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

impl ChannelBank {
    /// Constructs a bank whose identifier is addressable by a membership mask.
    pub const fn new(id: BankId, name: BankName, flags: BankFlags) -> Result<Self, PlanError> {
        if id.get() >= MAX_BANKS {
            return Err(PlanError::BankOutOfRange);
        }
        Ok(Self { id, name, flags })
    }

    /// Returns the bank identifier.
    pub const fn id(self) -> BankId {
        self.id
    }

    /// Returns the bank name.
    pub const fn name(self) -> BankName {
        self.name
    }

    /// Returns the bank flags.
    pub const fn flags(self) -> BankFlags {
        self.flags
    }

    /// Reports whether the bank participates in scanning.
    pub const fn is_scan_enabled(self) -> bool {
        self.flags.contains(BankFlags::SCAN_ENABLED)
    }
}

fn validate_tone(tone: Tone) -> Result<(), PlanError> {
    match tone {
        Tone::None => Ok(()),
        Tone::Ctcss(tenths_hz) => Tone::ctcss(tenths_hz)
            .map(|_| ())
            .map_err(|_| PlanError::InvalidTone),
        Tone::Dcs { code, inverted } => Tone::dcs(code, inverted)
            .map(|_| ())
            .map_err(|_| PlanError::InvalidTone),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        generated_channel_id, is_generated_channel_id, BankFlags, BankMask, ChannelBank,
        ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord, ChannelTemplate,
        GENERATED_CHANNEL_ID_BASE, MAX_GENERATED_CHANNELS,
    };
    use radio_domain::{Bandwidth, ChannelId, Modulation, PowerLevel, SquelchLevel, Tone};

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
    fn a_generated_bank_expands_to_complete_channel_records() {
        let template = ChannelTemplate {
            rx_tone: Tone::Ctcss(1_000),
            bandwidth: Bandwidth::Wide,
            ..ChannelTemplate::narrow_fm(FrequencyStep::from_hz(12_500).unwrap())
        };
        let bank = GeneratedBank::linear_simplex_with(
            BankId::new(3),
            BankName::new("PMR446").unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
            template,
        )
        .unwrap();

        let first = bank.channel_record(0).unwrap();
        assert_eq!(first.name().as_str(), "PMR446 01");
        assert_eq!(first.receive().as_hz(), 446_006_250);
        assert_eq!(first.transmit().as_hz(), 446_006_250);
        assert_eq!(first.rx_tone(), Tone::Ctcss(1_000));
        assert_eq!(first.bandwidth(), Bandwidth::Wide);
        assert_eq!(first.tx_class(), TxClass::LicenceFreePlan);
        assert!(first.is_member_of(BankId::new(3)));
        assert!(!first.is_member_of(BankId::new(4)));
        assert!(first.is_generated());

        let last = bank.channel_record(15).unwrap();
        assert_eq!(last.name().as_str(), "PMR446 16");
        assert_eq!(last.receive().as_hz(), 446_193_750);
        assert_ne!(first.id(), last.id());
        assert_eq!(bank.channel_record(16), Err(PlanError::ChannelOutOfRange));
    }

    #[test]
    fn expanded_identifiers_are_reserved_and_never_collide() {
        let low = generated_channel_id(BankId::new(0), 0).unwrap();
        let high = generated_channel_id(BankId::new(15), MAX_GENERATED_CHANNELS - 1).unwrap();
        assert!(is_generated_channel_id(low));
        assert!(is_generated_channel_id(high));
        assert!(!is_generated_channel_id(ChannelId::new(
            GENERATED_CHANNEL_ID_BASE - 1
        )));
        assert_eq!(
            generated_channel_id(BankId::new(16), 0),
            Err(PlanError::BankOutOfRange)
        );
        assert_eq!(
            generated_channel_id(BankId::new(0), MAX_GENERATED_CHANNELS),
            Err(PlanError::TooManyChannels)
        );

        let mut explicit = definition();
        explicit.id = ChannelId::new(GENERATED_CHANNEL_ID_BASE);
        assert_eq!(
            ChannelRecord::new(explicit),
            Err(PlanError::ReservedChannelId),
            "a stored channel cannot claim an expanded identifier"
        );
    }

    #[test]
    fn expanded_names_keep_the_position_when_the_plan_name_is_long() {
        let bank = GeneratedBank::linear_simplex(
            BankId::new(1),
            BankName::new("Marine channel").unwrap(),
            Frequency::from_hz(156_050_000).unwrap(),
            FrequencyStep::from_hz(50_000).unwrap(),
            120,
            TxClass::Never,
        )
        .unwrap();
        assert_eq!(
            bank.channel_record(0).unwrap().name().as_str(),
            "Marine c 001"
        );
        assert_eq!(
            bank.channel_record(119).unwrap().name().as_str(),
            "Marine c 120"
        );
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

    fn definition() -> ChannelDefinition {
        ChannelDefinition {
            id: ChannelId::new(3),
            name: ChannelName::new("GB3AB").unwrap(),
            receive: Frequency::from_hz(145_725_000).unwrap(),
            transmit: Frequency::from_hz(145_125_000).unwrap(),
            rx_tone: Tone::Ctcss(1_000),
            tx_tone: Tone::Ctcss(1_000),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Wide,
            power: PowerLevel::Medium,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            squelch: SquelchLevel::new(4).unwrap(),
            flags: ChannelFlags::default(),
            banks: BankMask::default().with(BankId::new(2), true).unwrap(),
            tx_class: TxClass::Amateur,
        }
    }

    #[test]
    fn channel_records_validate_tones_and_resolve_reverse() {
        let record = ChannelRecord::new(definition()).unwrap();
        assert_eq!(record.active().receive.as_hz(), 145_725_000);
        assert_eq!(record.active().transmit.as_hz(), 145_125_000);
        assert!(record.is_member_of(BankId::new(2)));
        assert!(!record.is_member_of(BankId::new(3)));
        assert!(!record.is_scan_skipped());

        let mut reversed = definition();
        reversed.flags = ChannelFlags::default().with(ChannelFlags::REVERSE, true);
        let reversed = ChannelRecord::new(reversed).unwrap();
        assert_eq!(reversed.active().receive.as_hz(), 145_125_000);
        assert_eq!(reversed.active().transmit.as_hz(), 145_725_000);
        assert_eq!(reversed.active().tx_class, TxClass::Amateur);

        let mut bad_tone = definition();
        bad_tone.rx_tone = Tone::Ctcss(1);
        assert_eq!(ChannelRecord::new(bad_tone), Err(PlanError::InvalidTone));
        let mut bad_code = definition();
        bad_code.tx_tone = Tone::Dcs {
            code: 799,
            inverted: false,
        };
        assert_eq!(ChannelRecord::new(bad_code), Err(PlanError::InvalidTone));
    }

    #[test]
    fn membership_masks_and_flags_are_bounded() {
        assert_eq!(
            BankMask::default().with(BankId::new(16), true),
            Err(PlanError::BankOutOfRange)
        );
        assert!(!BankMask::default().contains(BankId::new(16)));
        let mask = BankMask::default().with(BankId::new(0), true).unwrap();
        assert_eq!(mask.bits(), 1);
        assert!(mask.with(BankId::new(0), false).unwrap().is_empty());

        assert_eq!(ChannelFlags::from_bits(0x10), Err(PlanError::ReservedFlag));
        assert_eq!(ChannelFlags::from_bits(0x0F).unwrap().bits(), 0x0F);
        assert_eq!(BankFlags::from_bits(0x02), Err(PlanError::ReservedFlag));

        let bank = ChannelBank::new(
            BankId::new(2),
            BankName::new("Amateur 2m").unwrap(),
            BankFlags::default().with(BankFlags::SCAN_ENABLED, true),
        )
        .unwrap();
        assert!(bank.is_scan_enabled());
        assert_eq!(bank.name().as_str(), "Amateur 2m");
        assert_eq!(
            ChannelBank::new(
                BankId::new(16),
                BankName::new("out of range").unwrap(),
                BankFlags::default()
            ),
            Err(PlanError::BankOutOfRange)
        );
    }
}
