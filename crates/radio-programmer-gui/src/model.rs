//! Editable project model shared by the native editor and its tests.
//!
//! A project is a list of banks. Each bank either holds channels or is a
//! generated plan, and every channel is in exactly one bank: the radio stores
//! each channel in its bank (storage format 5, `ADR-100`). A bank's row is its
//! identifier, so the radio lists banks, and their channels, in this order.
//!
//! Drafts hold partially edited operator input as text. Validation is the only
//! path from a draft to a typed record, so an invalid field can never reach a
//! configuration image or a radio.

use std::fmt;

use radio_channel_plan::{
    BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
    ChannelRecord, ChannelTemplate, Designator, GeneratedBank, PlanEncoding,
    GENERATED_CHANNEL_ID_BASE, MAX_BANKS, MAX_DESIGNATOR_LEN, MAX_GENERATED_CHANNELS,
};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, Offset, PowerLevel,
    RadioConfig, RadioFlags, ScanResume, SquelchLevel, Tone, TxClass, MAX_BATTERY_SAVE_RATIO,
    MAX_SQUELCH_LEVEL,
};
use radio_programmer::{ConfigurationCompiler, DeviceCapabilities, RadioProject};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_configuration_image, decode_generated_bank,
    decode_radio_config, radio_image, ObjectKind, MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
};

/// Maximum objects an offline host project may hold.
pub const MAX_PROJECT_OBJECTS: u16 = 1_024;
/// Maximum accepted project file size.
pub const MAX_PROJECT_IMAGE_BYTES: usize = 256 * 1024;
/// Bytes a radio holds for its configuration image (`ADR-100`).
pub const RADIO_IMAGE_BYTES: usize = radio_programmer::RADIO_IMAGE_BYTES;

/// Host-side capabilities used to encode a project file.
///
/// A project file is the format 4 object set, which every front end reads;
/// a radio is written the format 5 image compiled from the same project.
pub fn host_capabilities() -> DeviceCapabilities {
    DeviceCapabilities {
        protocol_version: 1,
        storage_version: STORAGE_FORMAT_VERSION,
        max_frame_payload: 128,
        max_objects: MAX_PROJECT_OBJECTS,
        max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
        plan_encodings: PlanEncoding::LinearSimplex.capability_bit()
            | PlanEncoding::LinearFixedOffset.capability_bit(),
        configuration_bytes: 0,
    }
}

/// One field-level validation failure with the offending row and field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelError {
    /// Which part of the project failed.
    pub scope: ModelScope,
    /// The offending field name.
    pub field: &'static str,
    /// Human-readable reason.
    pub detail: String,
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {} {}", self.scope, self.field, self.detail)
    }
}

/// Which part of the project a validation failure came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelScope {
    /// One channel, by zero-based bank row and row within that bank.
    Channel {
        /// Bank row.
        bank: usize,
        /// Channel row within the bank.
        row: usize,
    },
    /// One bank row, identified by its zero-based row index.
    Bank(usize),
    /// The global radio configuration.
    Config,
    /// The whole project.
    Project,
}

impl fmt::Display for ModelScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel { bank, row } => {
                write!(formatter, "bank {} channel {}", bank + 1, row + 1)
            }
            Self::Bank(row) => write!(formatter, "bank {}", row + 1),
            Self::Config => formatter.write_str("radio configuration"),
            Self::Project => formatter.write_str("project"),
        }
    }
}

fn error(scope: ModelScope, field: &'static str, detail: impl Into<String>) -> ModelError {
    ModelError {
        scope,
        field,
        detail: detail.into(),
    }
}

/// Tone selection kind used by the editor.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ToneKind {
    /// No tone squelch.
    #[default]
    None,
    /// CTCSS tone.
    Ctcss,
    /// DCS code with normal polarity.
    Dcs,
    /// DCS code with inverted polarity.
    DcsInverted,
}

impl ToneKind {
    /// Returns the editor label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Ctcss => "CTCSS",
            Self::Dcs => "DCS",
            Self::DcsInverted => "DCS inverted",
        }
    }

    /// Returns every selectable kind in display order.
    pub const fn all() -> [Self; 4] {
        [Self::None, Self::Ctcss, Self::Dcs, Self::DcsInverted]
    }
}

/// An editable tone field.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToneDraft {
    /// Selected tone kind.
    pub kind: ToneKind,
    /// CTCSS frequency in hertz, or the DCS code as octal digits.
    pub value: String,
}

impl ToneDraft {
    /// Builds a draft from a validated tone.
    pub fn from_tone(tone: Tone) -> Self {
        match tone {
            Tone::None => Self {
                kind: ToneKind::None,
                value: String::new(),
            },
            Tone::Ctcss(tenths_hz) => Self {
                kind: ToneKind::Ctcss,
                value: format_tenths(tenths_hz),
            },
            Tone::Dcs { code, inverted } => Self {
                kind: if inverted {
                    ToneKind::DcsInverted
                } else {
                    ToneKind::Dcs
                },
                value: format!("{code:03}"),
            },
        }
    }

    fn validate(&self, scope: ModelScope, field: &'static str) -> Result<Tone, ModelError> {
        match self.kind {
            ToneKind::None => Ok(Tone::None),
            ToneKind::Ctcss => {
                let tenths = parse_tenths(self.value.trim())
                    .ok_or_else(|| error(scope, field, "is not a CTCSS frequency in hertz"))?;
                Tone::ctcss(tenths)
                    .map_err(|_| error(scope, field, "is outside 67.0 to 254.1 hertz"))
            }
            ToneKind::Dcs | ToneKind::DcsInverted => {
                let code = self
                    .value
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| error(scope, field, "is not a DCS code"))?;
                Tone::dcs(code, matches!(self.kind, ToneKind::DcsInverted))
                    .map_err(|_| error(scope, field, "is not three octal digits from 023 to 754"))
            }
        }
    }
}

/// One editable channel row.
// Each flag mirrors one stored device flag bit, so they stay separate fields.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelDraft {
    /// Display name.
    pub name: String,
    /// Receive frequency in megahertz.
    pub receive_mhz: String,
    /// Transmit frequency in megahertz.
    pub transmit_mhz: String,
    /// Receive-side tone squelch.
    pub rx_tone: ToneDraft,
    /// Transmit-side tone.
    pub tx_tone: ToneDraft,
    /// Modulation family.
    pub modulation: Modulation,
    /// Occupied bandwidth.
    pub bandwidth: Bandwidth,
    /// Requested power level.
    pub power: PowerLevel,
    /// Manual tuning step in hertz.
    pub step_hz: u32,
    /// Squelch level.
    pub squelch: u8,
    /// Skip this channel while scanning.
    pub scan_skip: bool,
    /// Inhibit transmission while the channel is busy.
    pub busy_lockout: bool,
    /// Exchange receive and transmit frequencies.
    pub reverse: bool,
    /// Request the audio compander.
    pub compander: bool,
    /// Trusted transmit classification.
    pub tx_class: TxClass,
}

impl Default for ChannelDraft {
    fn default() -> Self {
        Self {
            name: "CH1".to_owned(),
            receive_mhz: "145.500000".to_owned(),
            transmit_mhz: "145.500000".to_owned(),
            rx_tone: ToneDraft::default(),
            tx_tone: ToneDraft::default(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step_hz: 12_500,
            squelch: 3,
            scan_skip: false,
            busy_lockout: false,
            reverse: false,
            compander: false,
            tx_class: TxClass::Never,
        }
    }
}

impl ChannelDraft {
    /// Builds a draft from a validated channel record.
    pub fn from_record(record: ChannelRecord) -> Self {
        let flags = record.flags();
        Self {
            name: record.name().as_str().to_owned(),
            receive_mhz: format_mhz(record.receive().as_hz()),
            transmit_mhz: format_mhz(record.transmit().as_hz()),
            rx_tone: ToneDraft::from_tone(record.rx_tone()),
            tx_tone: ToneDraft::from_tone(record.tx_tone()),
            modulation: record.modulation(),
            bandwidth: record.bandwidth(),
            power: record.power(),
            step_hz: record.step().as_hz(),
            squelch: record.squelch().get(),
            scan_skip: flags.contains(ChannelFlags::SCAN_SKIP),
            busy_lockout: flags.contains(ChannelFlags::BUSY_LOCKOUT),
            reverse: flags.contains(ChannelFlags::REVERSE),
            compander: flags.contains(ChannelFlags::COMPANDER),
            tx_class: record.tx_class(),
        }
    }

    /// Validates every field into one channel record numbered `id` and
    /// stored in `bank`.
    pub fn validate(
        &self,
        scope: ModelScope,
        id: u16,
        bank: BankId,
    ) -> Result<ChannelRecord, ModelError> {
        let name = ChannelName::new(self.name.trim())
            .map_err(|_| error(scope, "name", "must be 1 to 12 printable characters"))?;
        let receive = parse_frequency(&self.receive_mhz)
            .ok_or_else(|| error(scope, "receive", "is not a frequency in megahertz"))?;
        let transmit = parse_frequency(&self.transmit_mhz)
            .ok_or_else(|| error(scope, "transmit", "is not a frequency in megahertz"))?;
        let step = FrequencyStep::from_hz(self.step_hz)
            .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?;
        let squelch = SquelchLevel::new(self.squelch)
            .map_err(|_| error(scope, "squelch", "must be 0 to 9"))?;
        let mut flags = ChannelFlags::default()
            .with(ChannelFlags::SCAN_SKIP, self.scan_skip)
            .with(ChannelFlags::BUSY_LOCKOUT, self.busy_lockout)
            .with(ChannelFlags::REVERSE, self.reverse);
        flags = flags.with(ChannelFlags::COMPANDER, self.compander);
        if id >= GENERATED_CHANNEL_ID_BASE {
            return Err(error(
                scope,
                "channel",
                format!(
                    "is one too many: a project stores at most {} channels",
                    GENERATED_CHANNEL_ID_BASE - 1
                ),
            ));
        }
        let banks = BankMask::default()
            .with(bank, true)
            .map_err(|_| error(scope, "bank", "is not addressable"))?;
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name,
            receive,
            transmit,
            rx_tone: self.rx_tone.validate(scope, "receive tone")?,
            tx_tone: self.tx_tone.validate(scope, "transmit tone")?,
            modulation: self.modulation,
            bandwidth: self.bandwidth,
            power: self.power,
            step,
            squelch,
            flags,
            banks,
            tx_class: self.tx_class,
        })
        .map_err(|failure| error(scope, "channel", failure.to_string()))
    }
}

/// What one bank holds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BankKind {
    /// Channels entered one by one.
    #[default]
    Named,
    /// A compact arithmetic plan the radio expands for itself.
    Generated,
}

impl BankKind {
    /// Returns the editor label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Named => "Channels",
            Self::Generated => "Generated plan",
        }
    }

    /// Returns a one-line explanation of what the kind stores.
    pub const fn detail(self) -> &'static str {
        match self {
            Self::Named => {
                "Holds the channels listed below. Values they all share are stored \
                 once for the bank."
            }
            Self::Generated => {
                "One stored plan the radio expands into complete channels, however many \
                 it holds, so no channel is stored for any of them."
            }
        }
    }

    /// Returns every selectable kind in display order.
    pub const fn all() -> [Self; 2] {
        [Self::Named, Self::Generated]
    }
}

/// What one project costs a radio.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageSummary {
    /// Bank rows, valid or not.
    pub banks: usize,
    /// Channels stored one record each.
    pub stored_channels: usize,
    /// Channels the radio expands from stored plans.
    pub expanded_channels: usize,
    /// Stored plans which expand into channels.
    pub plans: usize,
    /// Bytes of the radio image, when the whole project compiles into one.
    pub image_bytes: Option<usize>,
}

impl StorageSummary {
    /// Returns every channel the radio can select.
    pub const fn selectable_channels(&self) -> usize {
        self.stored_channels + self.expanded_channels
    }

    /// Returns the radio memory the image takes in tenths of a percent,
    /// rounded up so a nearly full radio never reads as having room.
    pub const fn used_per_mille(&self) -> Option<usize> {
        match self.image_bytes {
            Some(bytes) => Some((bytes * 1_000).div_ceil(RADIO_IMAGE_BYTES)),
            None => None,
        }
    }
}

/// One validated bank, which either holds channels or is a generated plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatedBank {
    /// A bank holding channels.
    Named(ChannelBank),
    /// A compact arithmetic plan.
    Generated(GeneratedBank),
}

/// One editable bank row.
// Each flag mirrors one stored device flag bit, so they stay separate fields.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BankDraft {
    /// Display name.
    pub name: String,
    /// Which kind of bank this row defines.
    pub kind: BankKind,
    /// Whether the bank participates in scanning.
    pub scan_enabled: bool,
    /// First generated receive frequency in megahertz.
    pub base_mhz: String,
    /// Generated channel spacing in hertz.
    pub spacing_hz: u32,
    /// Number of generated channels.
    pub channel_count: u16,
    /// Prefix the radio builds each expanded channel name from.
    ///
    /// The plan `name` is what this editor shows; the designator is what the
    /// operator reads on the radio, so a plan named `2M SIMPLEX` designated `S`
    /// expands to `S8` through `S23`. Left empty, the radio derives one.
    pub designator: String,
    /// Number the first expanded channel carries.
    pub first_number: u16,
    /// Zero-based index this plan marks as its calling channel, if any.
    pub calling_index: Option<u16>,
    /// Fixed transmit offset in hertz. Zero is simplex; non-zero is a repeater.
    pub offset_hz: i32,
    /// Trusted transmit classification of every generated channel.
    pub tx_class: TxClass,
    /// Receive-side tone squelch shared by every generated channel.
    pub rx_tone: ToneDraft,
    /// Transmit-side tone shared by every generated channel.
    pub tx_tone: ToneDraft,
    /// Modulation family shared by every generated channel.
    pub modulation: Modulation,
    /// Occupied bandwidth shared by every generated channel.
    pub bandwidth: Bandwidth,
    /// Power level shared by every generated channel.
    pub power: PowerLevel,
    /// Manual tuning step used from a generated channel.
    pub step_hz: u32,
    /// Squelch level shared by every generated channel.
    pub squelch: u8,
    /// Skip every generated channel while scanning.
    pub scan_skip: bool,
    /// Inhibit transmission on a busy generated channel.
    pub busy_lockout: bool,
    /// Request the audio compander on every generated channel.
    pub compander: bool,
    /// The channels a channel bank holds, in the radio's order.
    pub channels: Vec<ChannelDraft>,
}

impl Default for BankDraft {
    fn default() -> Self {
        Self {
            name: "Bank".to_owned(),
            kind: BankKind::default(),
            scan_enabled: true,
            base_mhz: "446.006250".to_owned(),
            spacing_hz: 12_500,
            channel_count: 16,
            designator: String::new(),
            first_number: 1,
            calling_index: None,
            offset_hz: 0,
            tx_class: TxClass::Never,
            rx_tone: ToneDraft::default(),
            tx_tone: ToneDraft::default(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step_hz: 12_500,
            squelch: 3,
            scan_skip: false,
            busy_lockout: false,
            compander: false,
            channels: Vec::new(),
        }
    }
}

impl BankDraft {
    /// Builds a draft from a validated named bank.
    pub fn from_record(bank: ChannelBank) -> Self {
        Self {
            name: bank.name().as_str().to_owned(),
            kind: BankKind::Named,
            scan_enabled: bank.is_scan_enabled(),
            ..Self::default()
        }
    }

    /// Builds a draft from a validated generated plan.
    pub fn from_generated(bank: GeneratedBank) -> Self {
        let template = bank.template();
        Self {
            name: bank.name().as_str().to_owned(),
            kind: BankKind::Generated,
            base_mhz: format_mhz(bank.base().as_hz()),
            spacing_hz: bank.spacing().as_hz(),
            channel_count: bank.channel_count(),
            designator: bank.designator().as_str().to_owned(),
            first_number: bank.first_number(),
            calling_index: bank.calling_index(),
            offset_hz: bank.offset().as_hz(),
            tx_class: bank.tx_class(),
            rx_tone: ToneDraft::from_tone(template.rx_tone),
            tx_tone: ToneDraft::from_tone(template.tx_tone),
            modulation: template.modulation,
            bandwidth: template.bandwidth,
            power: template.power,
            step_hz: template.step.as_hz(),
            squelch: template.squelch.get(),
            scan_skip: template.flags.contains(ChannelFlags::SCAN_SKIP),
            busy_lockout: template.flags.contains(ChannelFlags::BUSY_LOCKOUT),
            compander: template.flags.contains(ChannelFlags::COMPANDER),
            ..Self::default()
        }
    }

    /// Validates every field of the bank in row `row` into one bank.
    ///
    /// A bank's identifier is its row: the radio lists banks, and their
    /// channels, in this order.
    pub fn validate(&self, row: usize) -> Result<ValidatedBank, ModelError> {
        let scope = ModelScope::Bank(row);
        let name = BankName::new(self.name.trim())
            .map_err(|_| error(scope, "name", "must be 1 to 16 printable characters"))?;
        let id = u16::try_from(row)
            .ok()
            .filter(|id| *id < MAX_BANKS)
            .ok_or_else(|| {
                error(
                    scope,
                    "bank",
                    format!("a radio holds {MAX_BANKS} banks at most"),
                )
            })?;
        match self.kind {
            BankKind::Named => ChannelBank::new(
                BankId::new(id),
                name,
                BankFlags::default().with(BankFlags::SCAN_ENABLED, self.scan_enabled),
            )
            .map(ValidatedBank::Named)
            .map_err(|_| {
                error(
                    scope,
                    "bank",
                    format!("a radio holds {MAX_BANKS} banks at most"),
                )
            }),
            BankKind::Generated => {
                let base = parse_frequency(&self.base_mhz)
                    .ok_or_else(|| error(scope, "base", "is not a frequency in megahertz"))?;
                let spacing = FrequencyStep::from_hz(self.spacing_hz)
                    .map_err(|_| error(scope, "spacing", "must be a non-zero number of hertz"))?;
                if self.channel_count == 0 {
                    return Err(error(scope, "channels", "must be at least one channel"));
                }
                if self.channel_count > MAX_GENERATED_CHANNELS {
                    return Err(error(
                        scope,
                        "channels",
                        format!("must be at most {MAX_GENERATED_CHANNELS} in one plan"),
                    ));
                }
                let plan = GeneratedBank::linear_from_offset_with(
                    BankId::new(id),
                    name,
                    base,
                    spacing,
                    self.channel_count,
                    self.tx_class,
                    self.template(scope)?,
                    Offset::from_hz(self.offset_hz),
                )
                .map_err(|_| {
                    error(
                        scope,
                        "channels",
                        "span past the highest representable frequency",
                    )
                })?;
                // An empty designator leaves the radio's own derivation in
                // place, so a plan an operator has not designated still names
                // its channels readably. A trailing space is kept rather than
                // trimmed: it is what separates `PMR 1` from `PMR1`, so it is
                // part of the designator an operator chose.
                let plan = if self.designator.trim().is_empty() {
                    plan
                } else {
                    let designator = Designator::new(&self.designator).map_err(|_| {
                        error(
                            scope,
                            "designator",
                            format!("must be 1 to {MAX_DESIGNATOR_LEN} printable characters"),
                        )
                    })?;
                    plan.with_designator(designator, self.first_number)
                        .map_err(|_| {
                            error(
                                scope,
                                "designator",
                                "and numbering derive a name longer than the radio can show",
                            )
                        })?
                };
                plan.with_calling_index(self.calling_index)
                    .map(ValidatedBank::Generated)
                    .map_err(|_| {
                        error(
                            scope,
                            "calling channel",
                            "is outside the channels this plan expands to",
                        )
                    })
            }
        }
    }

    /// Validates the per-channel settings every generated channel shares.
    fn template(&self, scope: ModelScope) -> Result<ChannelTemplate, ModelError> {
        Ok(ChannelTemplate {
            rx_tone: self.rx_tone.validate(scope, "receive tone")?,
            tx_tone: self.tx_tone.validate(scope, "transmit tone")?,
            modulation: self.modulation,
            bandwidth: self.bandwidth,
            power: self.power,
            step: FrequencyStep::from_hz(self.step_hz)
                .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?,
            squelch: SquelchLevel::new(self.squelch)
                .map_err(|_| error(scope, "squelch", "must be 0 to 9"))?,
            flags: ChannelFlags::default()
                .with(ChannelFlags::SCAN_SKIP, self.scan_skip)
                .with(ChannelFlags::BUSY_LOCKOUT, self.busy_lockout)
                .with(ChannelFlags::COMPANDER, self.compander),
        })
    }

    /// Returns the channels this plan expands to, name and frequency in order.
    ///
    /// The plan is what the radio stores, so the editor shows what it becomes
    /// rather than asking the operator to picture it. At most `limit` channels
    /// are returned; the caller reports how many were left out.
    pub fn expansion(&self, limit: usize) -> Vec<(String, String)> {
        let Ok(ValidatedBank::Generated(plan)) = self.validate(0) else {
            return Vec::new();
        };
        (0..plan.channel_count())
            .take(limit)
            .filter_map(|index| {
                let channel = plan.channel_record(index).ok()?;
                Some((
                    channel.name().as_str().to_owned(),
                    format_mhz(channel.receive().as_hz()),
                ))
            })
            .collect()
    }

    /// Returns the first and last generated frequency when the plan is valid.
    ///
    /// The editor shows the span a compact plan covers, which is otherwise only
    /// visible after the plan has been written to a radio.
    pub fn generated_span(&self) -> Option<(String, String)> {
        if !matches!(self.kind, BankKind::Generated) || self.channel_count == 0 {
            return None;
        }
        let base = parse_frequency(&self.base_mhz)?;
        let spacing = FrequencyStep::from_hz(self.spacing_hz).ok()?;
        let last = base
            .checked_add_steps(spacing, self.channel_count - 1)
            .ok()?;
        Some((format_mhz(base.as_hz()), format_mhz(last.as_hz())))
    }
}

/// The editable global radio configuration.
// Each flag mirrors one stored device flag bit, so they stay separate fields.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigDraft {
    /// Default squelch level.
    pub squelch: u8,
    /// Backlight timeout in seconds.
    pub backlight_seconds: u8,
    /// Scan resume behaviour.
    pub scan_resume: ScanResume,
    /// No-signal scan dwell in milliseconds.
    pub scan_dwell_ms: u32,
    /// Open-squelch scan hold in milliseconds.
    pub scan_hold_ms: u32,
    /// Whether dual watch is enabled.
    pub dual_watch: bool,
    /// Receive battery-save duty ratio.
    pub battery_save_ratio: u8,
    /// Audible key confirmation.
    pub key_beep: bool,
    /// New channels default to busy lockout.
    pub busy_lockout_default: bool,
    /// Apply the AM gain-compensation workaround.
    pub am_fix: bool,
    /// Request squelch tail elimination on tone-coded channels.
    pub tone_tail_elimination: bool,
}

impl Default for ConfigDraft {
    fn default() -> Self {
        Self::from_config(RadioConfig::conservative())
    }
}

impl ConfigDraft {
    /// Builds a draft from a validated configuration.
    pub fn from_config(config: RadioConfig) -> Self {
        Self {
            squelch: config.squelch.get(),
            backlight_seconds: config.backlight_seconds,
            scan_resume: config.scan_resume,
            scan_dwell_ms: config.scan_dwell_ms,
            scan_hold_ms: config.scan_hold_ms,
            dual_watch: config.dual_watch,
            battery_save_ratio: config.battery_save_ratio,
            key_beep: config.flags.contains(RadioFlags::KEY_BEEP),
            busy_lockout_default: config.flags.contains(RadioFlags::BUSY_LOCKOUT_DEFAULT),
            am_fix: config.flags.contains(RadioFlags::AM_FIX),
            tone_tail_elimination: config.flags.contains(RadioFlags::TONE_TAIL_ELIMINATION),
        }
    }

    /// Validates every field into one radio configuration.
    pub fn validate(&self) -> Result<RadioConfig, ModelError> {
        let scope = ModelScope::Config;
        let squelch = SquelchLevel::new(self.squelch).map_err(|_| {
            error(
                scope,
                "squelch",
                format!("must be 0 to {MAX_SQUELCH_LEVEL}"),
            )
        })?;
        if self.battery_save_ratio > MAX_BATTERY_SAVE_RATIO {
            return Err(error(
                scope,
                "battery save",
                format!("must be 0 to {MAX_BATTERY_SAVE_RATIO}"),
            ));
        }
        let flags = RadioFlags::default()
            .with(RadioFlags::KEY_BEEP, self.key_beep)
            .with(RadioFlags::BUSY_LOCKOUT_DEFAULT, self.busy_lockout_default)
            .with(RadioFlags::AM_FIX, self.am_fix)
            .with(
                RadioFlags::TONE_TAIL_ELIMINATION,
                self.tone_tail_elimination,
            );
        RadioConfig {
            squelch,
            backlight_seconds: self.backlight_seconds,
            scan_resume: self.scan_resume,
            scan_dwell_ms: self.scan_dwell_ms,
            scan_hold_ms: self.scan_hold_ms,
            dual_watch: self.dual_watch,
            battery_save_ratio: self.battery_save_ratio,
            flags,
        }
        .validate()
        .map_err(|_| error(scope, "scan timing", "dwell and hold must be non-zero"))
    }
}

/// A complete editable project.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectModel {
    /// Editable banks in the radio's order; each row's index is its identifier.
    pub banks: Vec<BankDraft>,
    /// The editable global radio configuration.
    pub config: ConfigDraft,
}

impl ProjectModel {
    /// Returns an empty project holding the conservative configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one bank which holds channels.
    pub fn add_bank(&mut self) {
        self.push_bank(BankKind::Named);
    }

    /// Appends one generated-plan bank.
    pub fn add_generated_bank(&mut self) {
        self.push_bank(BankKind::Generated);
    }

    fn push_bank(&mut self, kind: BankKind) {
        let number = self.banks.len() + 1;
        self.banks.push(BankDraft {
            name: format!("Bank {number}"),
            kind,
            ..BankDraft::default()
        });
    }

    /// Moves bank `row` one place earlier (`up`) or later in the radio's order.
    pub fn move_bank(&mut self, row: usize, up: bool) {
        let other = if up {
            row.checked_sub(1)
        } else {
            Some(row + 1)
        };
        if let Some(other) = other.filter(|other| *other < self.banks.len()) {
            if row < self.banks.len() {
                self.banks.swap(row, other);
            }
        }
    }

    /// Appends one channel to bank `bank`, taking the tuning of its last channel.
    pub fn add_channel(&mut self, bank: usize) {
        let Some(bank) = self.banks.get_mut(bank) else {
            return;
        };
        let number = bank.channels.len() + 1;
        let mut draft = ChannelDraft {
            name: format!("CH{number}"),
            ..ChannelDraft::default()
        };
        if let Some(previous) = bank.channels.last() {
            draft.step_hz = previous.step_hz;
            draft.modulation = previous.modulation;
            draft.bandwidth = previous.bandwidth;
            draft.tx_class = previous.tx_class;
        }
        bank.channels.push(draft);
    }

    /// Inserts a copy of one channel after it, in the same bank.
    ///
    /// Copying is how an operator enters a repeater pair or a group of similar
    /// channels without retyping every field.
    pub fn duplicate_channel(&mut self, bank: usize, row: usize) {
        if let Some(bank) = self.banks.get_mut(bank) {
            if let Some(source) = bank.channels.get(row).cloned() {
                bank.channels.insert(row + 1, source);
            }
        }
    }

    /// Returns how many channel rows the project holds.
    pub fn channel_count(&self) -> usize {
        self.banks.iter().map(|bank| bank.channels.len()).sum()
    }

    /// Returns what this project costs a radio.
    ///
    /// Only rows which validate are counted, and the image size is given only
    /// when the whole project compiles, so the figures always describe a
    /// configuration which could be written.
    pub fn storage_summary(&self) -> StorageSummary {
        let mut summary = StorageSummary {
            banks: self.banks.len(),
            ..StorageSummary::default()
        };
        let mut id = 1_u16;
        for (row, draft) in self.banks.iter().enumerate() {
            match draft.validate(row) {
                Ok(ValidatedBank::Named(bank)) => {
                    for (channel_row, channel) in draft.channels.iter().enumerate() {
                        let scope = ModelScope::Channel {
                            bank: row,
                            row: channel_row,
                        };
                        if channel.validate(scope, id, bank.id()).is_ok() {
                            summary.stored_channels += 1;
                        }
                        id = id.saturating_add(1);
                    }
                }
                Ok(ValidatedBank::Generated(plan)) => {
                    summary.plans += 1;
                    summary.expanded_channels += usize::from(plan.channel_count());
                }
                Err(_) => {}
            }
        }
        summary.image_bytes = self.to_radio_image().ok().map(|image| image.len());
        summary
    }

    /// Validates every row and builds one project.
    ///
    /// Channels are numbered through the project in the radio's order.
    pub fn validate(&self) -> Result<RadioProject, Vec<ModelError>> {
        let mut errors = Vec::new();
        let mut project = RadioProject::new();
        let mut id = 1_u16;

        for (row, draft) in self.banks.iter().enumerate() {
            match draft.validate(row) {
                Ok(ValidatedBank::Named(bank)) => {
                    project.add_bank(bank);
                    for (channel_row, channel) in draft.channels.iter().enumerate() {
                        let scope = ModelScope::Channel {
                            bank: row,
                            row: channel_row,
                        };
                        match channel.validate(scope, id, bank.id()) {
                            Ok(record) => project.add_channel(record),
                            Err(failure) => errors.push(failure),
                        }
                        id = id.saturating_add(1);
                    }
                }
                Ok(ValidatedBank::Generated(plan)) => {
                    if draft.channels.is_empty() {
                        project.add_generated_bank(plan);
                    } else {
                        errors.push(error(
                            ModelScope::Bank(row),
                            "channels",
                            "belong to a channel bank; a generated plan holds none",
                        ));
                    }
                }
                Err(failure) => errors.push(failure),
            }
        }
        match self.config.validate() {
            Ok(config) => project.set_config(config),
            Err(failure) => errors.push(failure),
        }

        if errors.is_empty() {
            Ok(project)
        } else {
            Err(errors)
        }
    }

    /// Validates and compiles the image a radio runs (format 5).
    pub fn to_radio_image(&self) -> Result<Vec<u8>, Vec<ModelError>> {
        self.validate()?.compile_radio_image().map_err(|failure| {
            vec![error(
                ModelScope::Project,
                "radio image",
                failure.to_string(),
            )]
        })
    }

    /// Validates and encodes the project file, the canonical format 4 image.
    pub fn to_image(&self) -> Result<Vec<u8>, Vec<ModelError>> {
        let project = self.validate()?;
        let compiled = ConfigurationCompiler::new(host_capabilities())
            .compile(&project)
            .map_err(|failure| vec![error(ModelScope::Project, "compile", failure.to_string())])?;
        let mut image = vec![
            0;
            compiled.image_len().map_err(|failure| vec![error(
                ModelScope::Project,
                "image",
                failure.to_string()
            )])?
        ];
        compiled
            .encode_image(&mut image)
            .map_err(|failure| vec![error(ModelScope::Project, "image", failure.to_string())])?;
        Ok(image)
    }

    /// Builds a project from a project file or an image read from a radio.
    ///
    /// Both the format 4 object set and the format 5 radio image are read.
    pub fn from_image(bytes: &[u8]) -> Result<Self, ModelError> {
        if bytes.len() > MAX_PROJECT_IMAGE_BYTES {
            return Err(error(
                ModelScope::Project,
                "image",
                format!("exceeds {MAX_PROJECT_IMAGE_BYTES} bytes"),
            ));
        }
        let invalid = |failure: radio_storage::StorageError| {
            error(ModelScope::Project, "image", failure.to_string())
        };
        if bytes.get(4..6) == Some(&[radio_image::IMAGE_VERSION, radio_image::FORMAT_VERSION]) {
            return RadioProject::from_radio_image(bytes)
                .map(|project| Self::from_project(&project))
                .map_err(invalid);
        }
        let image = decode_configuration_image(bytes).map_err(invalid)?;
        let mut project = RadioProject::new();
        for object in image.objects() {
            match object.key().kind {
                ObjectKind::Channel => {
                    project.add_channel(decode_channel(&object).map_err(invalid)?);
                }
                ObjectKind::ChannelBank => {
                    project.add_bank(decode_channel_bank(&object).map_err(invalid)?);
                }
                ObjectKind::RadioConfig => {
                    project.set_config(decode_radio_config(&object).map_err(invalid)?);
                }
                ObjectKind::GeneratedBank => {
                    project.add_generated_bank(decode_generated_bank(&object).map_err(invalid)?);
                }
            }
        }
        Ok(Self::from_project(&project))
    }

    /// Groups a project's objects into banks.
    ///
    /// Banks keep their identifier order, a bank's channels before its plan.
    /// A channel goes into the first bank it names; a channel in none goes into
    /// a last bank of its own, because every channel here is in a bank.
    pub fn from_project(project: &RadioProject) -> Self {
        let first_bank = |channel: &ChannelRecord| {
            (0..MAX_BANKS).find(|id| channel.is_member_of(BankId::new(*id)))
        };
        let channels_in = |bank: Option<u16>| {
            project
                .channels()
                .iter()
                .filter(|channel| first_bank(channel) == bank)
                .map(|channel| ChannelDraft::from_record(*channel))
                .collect::<Vec<_>>()
        };
        let mut model = Self {
            banks: Vec::new(),
            config: project
                .config()
                .map_or_else(ConfigDraft::default, ConfigDraft::from_config),
        };
        for id in 0..MAX_BANKS {
            let named = project.banks().iter().find(|bank| bank.id().get() == id);
            let plan = project
                .generated_banks()
                .iter()
                .find(|plan| plan.id().get() == id);
            let channels = channels_in(Some(id));
            if named.is_some() && (!channels.is_empty() || plan.is_none())
                || named.is_none() && !channels.is_empty()
            {
                let mut draft = named.map_or_else(
                    || BankDraft {
                        name: plan.map_or_else(
                            || format!("Bank {id}"),
                            |plan| plan.name().as_str().to_owned(),
                        ),
                        ..BankDraft::default()
                    },
                    |bank| BankDraft::from_record(*bank),
                );
                draft.channels = channels;
                model.banks.push(draft);
            }
            if let Some(plan) = plan {
                model.banks.push(BankDraft::from_generated(*plan));
            }
        }
        let unbanked = channels_in(None);
        if !unbanked.is_empty() {
            model.banks.push(BankDraft {
                name: "Unbanked".to_owned(),
                channels: unbanked,
                ..BankDraft::default()
            });
        }
        model
    }
}

/// Parses a megahertz string into exact integer hertz.
pub fn parse_frequency(text: &str) -> Option<Frequency> {
    Frequency::from_hz(parse_mhz(text)?).ok()
}

fn parse_mhz(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if fraction.len() > 6 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut hertz = whole.parse::<u32>().ok()?.checked_mul(1_000_000)?;
    if !fraction.is_empty() {
        let mut scaled = fraction.parse::<u32>().ok()?;
        for _ in fraction.len()..6 {
            scaled = scaled.checked_mul(10)?;
        }
        hertz = hertz.checked_add(scaled)?;
    }
    Some(hertz)
}

/// Formats integer hertz as a six-decimal megahertz string.
pub fn format_mhz(hertz: u32) -> String {
    format!("{}.{:06}", hertz / 1_000_000, hertz % 1_000_000)
}

fn parse_tenths(text: &str) -> Option<u16> {
    let text = text.trim();
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if fraction.len() > 1 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let tenths = whole.parse::<u16>().ok()?.checked_mul(10)?;
    let extra = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u16>().ok()?
    };
    tenths.checked_add(extra)
}

fn format_tenths(tenths_hz: u16) -> String {
    format!("{}.{}", tenths_hz / 10, tenths_hz % 10)
}

#[cfg(test)]
mod tests {
    use super::{
        format_mhz, parse_frequency, BankDraft, BankKind, ChannelDraft, ConfigDraft, ModelScope,
        ProjectModel, ToneDraft, ToneKind, ValidatedBank, RADIO_IMAGE_BYTES,
    };
    use radio_channel_plan::{ChannelFlags, MAX_BANKS};
    use radio_domain::{Bandwidth, BankId, PowerLevel, ScanResume, Tone, TxClass};
    use radio_programmer::RadioProject;

    fn project() -> ProjectModel {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.banks[0].name = "Amateur 2m".to_owned();
        model.add_channel(0);
        let channel = &mut model.banks[0].channels[0];
        channel.name = "GB3AB".to_owned();
        channel.receive_mhz = "145.725000".to_owned();
        channel.transmit_mhz = "145.125000".to_owned();
        channel.rx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "100.0".to_owned(),
        };
        channel.tx_tone = ToneDraft {
            kind: ToneKind::DcsInverted,
            value: "023".to_owned(),
        };
        channel.tx_class = TxClass::Amateur;
        model
    }

    fn pmr_plan(model: &mut ProjectModel) -> usize {
        model.add_generated_bank();
        let row = model.banks.len() - 1;
        model.banks[row].name = "PMR446".to_owned();
        model.banks[row].base_mhz = "446.006250".to_owned();
        model.banks[row].designator = "PMR ".to_owned();
        model.banks[row].tx_class = TxClass::LicenceFreePlan;
        row
    }

    #[test]
    fn frequencies_parse_and_format_without_rounding() {
        assert_eq!(parse_frequency("446.00625").unwrap().as_hz(), 446_006_250);
        assert_eq!(parse_frequency("145").unwrap().as_hz(), 145_000_000);
        assert_eq!(format_mhz(446_006_250), "446.006250");
        assert!(parse_frequency("").is_none());
        assert!(parse_frequency("145.1234567").is_none());
        assert!(parse_frequency("14a.5").is_none());
        assert!(parse_frequency("-145").is_none());
        assert!(parse_frequency("0").is_none());
    }

    #[test]
    fn tone_drafts_round_trip_and_reject_invalid_values() {
        let ctcss = ToneDraft::from_tone(Tone::Ctcss(1_000));
        assert_eq!(ctcss.value, "100.0");
        assert_eq!(
            ctcss.validate(ModelScope::Config, "tone").unwrap(),
            Tone::Ctcss(1_000)
        );
        let dcs = ToneDraft::from_tone(Tone::Dcs {
            code: 23,
            inverted: true,
        });
        assert_eq!(dcs.kind, ToneKind::DcsInverted);
        assert_eq!(dcs.value, "023");
        let invalid = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "10.0".to_owned(),
        };
        assert!(invalid.validate(ModelScope::Config, "tone").is_err());
        let bad_code = ToneDraft {
            kind: ToneKind::Dcs,
            value: "799".to_owned(),
        };
        assert!(bad_code.validate(ModelScope::Config, "tone").is_err());
    }

    #[test]
    fn every_channel_is_stored_in_the_bank_which_holds_it() {
        let mut model = project();
        model.add_bank();
        model.add_channel(1);
        model.add_channel(1);
        let project = model.validate().unwrap();
        let banks: Vec<_> = project
            .channels()
            .iter()
            .map(|channel| (channel.id().get(), channel.banks()))
            .collect();
        assert_eq!(banks.len(), 3);
        assert!(project.channels()[0].is_member_of(BankId::new(0)));
        assert!(project.channels()[1].is_member_of(BankId::new(1)));
        assert!(project.channels()[2].is_member_of(BankId::new(1)));
        // Numbered through the project in the radio's order.
        assert_eq!(
            banks.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(project.banks().len(), 2);
        assert!(model.to_radio_image().is_ok());
    }

    #[test]
    fn a_valid_project_round_trips_through_a_project_file() {
        let mut model = project();
        pmr_plan(&mut model);
        let image = model.to_image().unwrap();
        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!(loaded, model);
        assert_eq!(loaded.to_image().unwrap(), image);
    }

    #[test]
    fn a_radio_image_reads_back_into_the_same_banks() {
        let mut model = project();
        pmr_plan(&mut model);
        model.add_bank();
        model.banks[2].name = "Marine".to_owned();
        model.add_channel(2);
        model.banks[2].channels[0].receive_mhz = "156.800000".to_owned();
        model.banks[2].channels[0].transmit_mhz = "156.800000".to_owned();
        let image = model.to_radio_image().unwrap();
        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!(loaded, model);
        assert_eq!(loaded.to_radio_image().unwrap(), image);
    }

    #[test]
    fn an_older_project_puts_each_channel_in_one_bank() {
        // A format 4 project could list a channel in several banks, in none,
        // or in a plan's bank; here every channel goes into exactly one.
        let model = project();
        let mut legacy = RadioProject::new();
        let valid = model.validate().unwrap();
        legacy.add_bank(valid.banks()[0]);
        let channel = |id: u16, banks: u16| {
            let record = valid.channels()[0];
            radio_channel_plan::ChannelRecord::new(radio_channel_plan::ChannelDefinition {
                id: radio_domain::ChannelId::new(id),
                name: record.name(),
                receive: record.receive(),
                transmit: record.transmit(),
                rx_tone: record.rx_tone(),
                tx_tone: record.tx_tone(),
                modulation: record.modulation(),
                bandwidth: record.bandwidth(),
                power: record.power(),
                step: record.step(),
                squelch: record.squelch(),
                flags: record.flags(),
                banks: radio_channel_plan::BankMask::from_bits(banks),
                tx_class: record.tx_class(),
            })
            .unwrap()
        };
        legacy.add_channel(channel(1, 0b11));
        legacy.add_channel(channel(2, 0));
        legacy.add_channel(channel(3, 0b100));
        let mut plan = model.clone();
        pmr_plan(&mut plan);
        let ValidatedBank::Generated(pmr) = plan.banks[1].validate(2).unwrap() else {
            panic!("plan");
        };
        legacy.add_generated_bank(pmr);

        let loaded = ProjectModel::from_project(&legacy);
        let summary: Vec<_> = loaded
            .banks
            .iter()
            .map(|bank| (bank.name.as_str(), bank.kind, bank.channels.len()))
            .collect();
        assert_eq!(
            summary,
            [
                ("Amateur 2m", BankKind::Named, 1),
                ("PMR446", BankKind::Named, 1),
                ("PMR446", BankKind::Generated, 0),
                ("Unbanked", BankKind::Named, 1),
            ]
        );
        assert!(loaded.validate().is_ok());
    }

    #[test]
    fn invalid_rows_are_reported_with_their_scope_and_field() {
        let mut model = project();
        model.banks[0].channels[0].name = String::new();
        model.banks[0].name = String::new();
        model.config.scan_dwell_ms = 0;
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors
            .iter()
            .any(|failure| failure.scope == ModelScope::Bank(0) && failure.field == "name"));
        assert!(errors
            .iter()
            .any(|failure| failure.scope == ModelScope::Config));

        model.banks[0].name = "Fixed".to_owned();
        model.banks[0].channels[0].receive_mhz = "nope".to_owned();
        let errors = model.validate().unwrap_err();
        assert!(errors.iter().any(|failure| failure.scope
            == ModelScope::Channel { bank: 0, row: 0 }
            && failure.field == "name"));
        assert_eq!(
            errors[0].to_string(),
            "bank 1 channel 1: name must be 1 to 12 printable characters"
        );
        assert!(model.to_image().is_err());
        assert!(model.to_radio_image().is_err());
    }

    #[test]
    fn a_radio_holds_sixteen_banks_and_a_plan_holds_no_channel_rows() {
        let mut model = ProjectModel::new();
        for _ in 0..=MAX_BANKS {
            model.add_bank();
        }
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].scope, ModelScope::Bank(usize::from(MAX_BANKS)));

        let mut model = ProjectModel::new();
        let row = pmr_plan(&mut model);
        model.add_channel(row);
        let errors = model.validate().unwrap_err();
        assert_eq!(errors[0].field, "channels");
    }

    #[test]
    fn new_channels_inherit_the_last_one_in_their_bank() {
        let mut model = project();
        model.banks[0].channels[0].step_hz = 6_250;
        model.add_channel(0);
        assert_eq!(model.banks[0].channels[1].step_hz, 6_250);
        assert_eq!(model.banks[0].channels[1].tx_class, TxClass::Amateur);
        assert_eq!(model.banks[0].channels[1].name, "CH2");
        model.add_channel(9);
        assert_eq!(model.channel_count(), 2);
    }

    #[test]
    fn banks_move_and_channels_duplicate_within_their_bank() {
        let mut model = project();
        pmr_plan(&mut model);
        model.move_bank(1, true);
        assert_eq!(model.banks[0].name, "PMR446");
        model.move_bank(0, true);
        model.move_bank(1, false);
        assert_eq!(model.banks[0].name, "PMR446");

        model.duplicate_channel(1, 0);
        assert_eq!(model.banks[1].channels.len(), 2);
        assert_eq!(model.banks[1].channels[0], model.banks[1].channels[1]);
        model.duplicate_channel(1, 9);
        model.duplicate_channel(9, 0);
        assert_eq!(model.channel_count(), 2);
    }

    #[test]
    fn configuration_drafts_validate_their_envelope() {
        let mut draft = ConfigDraft {
            scan_resume: ScanResume::Carrier,
            battery_save_ratio: 6,
            ..ConfigDraft::default()
        };
        assert!(draft.validate().is_err());
        draft.battery_save_ratio = 5;
        assert_eq!(draft.validate().unwrap().battery_save_ratio, 5);
        draft.squelch = 10;
        assert!(draft.validate().is_err());
    }

    #[test]
    fn generated_plans_validate_their_span_and_report_it() {
        let mut draft = BankDraft {
            name: "PMR446".to_owned(),
            kind: BankKind::Generated,
            base_mhz: "446.00625".to_owned(),
            spacing_hz: 12_500,
            channel_count: 16,
            tx_class: TxClass::LicenceFreePlan,
            ..BankDraft::default()
        };
        let ValidatedBank::Generated(bank) = draft.validate(3).unwrap() else {
            panic!("a generated row must validate into a generated plan");
        };
        assert_eq!(bank.id(), BankId::new(3));
        assert_eq!(bank.channel(15).unwrap().receive.as_hz(), 446_193_750);
        assert_eq!(
            draft.generated_span(),
            Some(("446.006250".to_owned(), "446.193750".to_owned()))
        );

        draft.channel_count = 0;
        assert_eq!(draft.validate(0).unwrap_err().field, "channels");
        assert!(draft.generated_span().is_none());

        draft.channel_count = 16;
        draft.spacing_hz = 0;
        assert_eq!(draft.validate(0).unwrap_err().field, "spacing");

        draft.spacing_hz = 12_500;
        draft.base_mhz = "not a frequency".to_owned();
        assert_eq!(draft.validate(0).unwrap_err().field, "base");

        draft.base_mhz = "4294.967".to_owned();
        draft.channel_count = u16::MAX;
        assert_eq!(draft.validate(0).unwrap_err().field, "channels");
    }

    #[test]
    fn a_plan_template_is_edited_once_and_reaches_every_expanded_channel() {
        let mut model = ProjectModel::new();
        let row = pmr_plan(&mut model);
        let bank = &mut model.banks[row];
        bank.channel_count = 8;
        bank.bandwidth = Bandwidth::Wide;
        bank.power = PowerLevel::Medium;
        bank.squelch = 5;
        bank.busy_lockout = true;
        bank.rx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "100.0".to_owned(),
        };

        let ValidatedBank::Generated(plan) = model.banks[row].validate(row).unwrap() else {
            panic!("a generated row must validate into a generated plan");
        };
        for index in 0..plan.channel_count() {
            let channel = plan.channel_record(index).unwrap();
            assert_eq!(channel.bandwidth(), Bandwidth::Wide);
            assert_eq!(channel.power(), PowerLevel::Medium);
            assert_eq!(channel.squelch().get(), 5);
            assert_eq!(channel.rx_tone(), Tone::Ctcss(1_000));
            assert!(channel.flags().contains(ChannelFlags::BUSY_LOCKOUT));
        }

        let expansion = model.banks[row].expansion(4);
        assert_eq!(expansion.len(), 4);
        assert_eq!(expansion[0], ("PMR 1".to_owned(), "446.006250".to_owned()));

        let loaded = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        assert_eq!(loaded.banks[0].bandwidth, Bandwidth::Wide);
        assert!(loaded.banks[0].busy_lockout);
        assert_eq!(loaded.banks[0].rx_tone.value, "100.0");
    }

    #[test]
    fn the_storage_summary_reports_the_radio_image() {
        let mut model = project();
        let stored_only = model.storage_summary();
        assert_eq!(stored_only.stored_channels, 1);
        assert_eq!(stored_only.selectable_channels(), 1);
        let bytes = stored_only.image_bytes.unwrap();
        assert!(bytes < RADIO_IMAGE_BYTES);
        let per_mille = stored_only.used_per_mille().unwrap();
        assert!(per_mille > 0 && per_mille <= 1_000);
        assert!(per_mille * RADIO_IMAGE_BYTES >= bytes * 1_000);

        let row = pmr_plan(&mut model);
        let summary = model.storage_summary();
        assert_eq!(summary.expanded_channels, 16);
        assert_eq!(summary.selectable_channels(), 17);
        assert_eq!(summary.plans, 1);
        assert!(summary.image_bytes.unwrap() > bytes);
        assert_eq!(summary.banks, 2);

        // A row which does not validate is not counted, and no image is claimed.
        model.banks[row].base_mhz = "not a frequency".to_owned();
        let summary = model.storage_summary();
        assert_eq!(summary.expanded_channels, 0);
        assert_eq!(summary.image_bytes, None);
        assert_eq!(summary.used_per_mille(), None);
        assert_eq!(summary.banks, 2);
    }

    #[test]
    fn channel_drafts_round_trip_through_records() {
        let draft = project().banks[0].channels[0].clone();
        let record = draft
            .validate(ModelScope::Project, 7, BankId::new(2))
            .unwrap();
        assert_eq!(record.id().get(), 7);
        assert!(record.is_member_of(BankId::new(2)));
        assert_eq!(ChannelDraft::from_record(record), draft);
    }
}
