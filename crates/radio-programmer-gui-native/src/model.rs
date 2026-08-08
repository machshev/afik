//! Editable project model shared by the native editor and its tests.
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
use radio_programmer::{CompileError, ConfigurationCompiler, DeviceCapabilities, RadioProject};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_configuration_image, decode_generated_bank,
    decode_radio_config, generated_bank_encoded_len, ObjectKind, CHANNEL_BANK_ENCODED_LEN,
    CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_OBJECT_HEADER_LEN,
    MAX_GENERATED_BANK_ENCODED_LEN, MAX_OBJECT_DATA, RADIO_CONFIG_ENCODED_LEN,
    STORAGE_FORMAT_VERSION,
};

/// Maximum objects an offline host project may hold.
pub const MAX_PROJECT_OBJECTS: u16 = 1_024;
/// Maximum accepted canonical image size for an offline load.
pub const MAX_PROJECT_IMAGE_BYTES: usize = 256 * 1024;

/// Host-side capabilities used for offline compilation and image export.
///
/// These are deliberately permissive: a device session always recompiles the
/// project against the negotiated target capabilities before any write.
pub fn host_capabilities() -> DeviceCapabilities {
    DeviceCapabilities {
        protocol_version: 1,
        storage_version: STORAGE_FORMAT_VERSION,
        max_frame_payload: 128,
        max_objects: MAX_PROJECT_OBJECTS,
        max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
        plan_encodings: PlanEncoding::LinearSimplex.capability_bit()
            | PlanEncoding::LinearFixedOffset.capability_bit(),
        // Offline the host declares no stored-configuration bound; a connected
        // radio reports its own, and that is what the editor shows.
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
    /// One channel row, identified by its zero-based row index.
    Channel(usize),
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
            Self::Channel(row) => write!(formatter, "channel row {}", row + 1),
            Self::Bank(row) => write!(formatter, "bank row {}", row + 1),
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
    /// Stable channel identifier.
    pub id: u16,
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
    /// Bank membership by bank identifier.
    pub banks: [bool; MAX_BANKS as usize],
    /// Trusted transmit classification.
    pub tx_class: TxClass,
}

impl Default for ChannelDraft {
    fn default() -> Self {
        Self {
            id: 1,
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
            banks: [false; MAX_BANKS as usize],
            tx_class: TxClass::Never,
        }
    }
}

impl ChannelDraft {
    /// Builds a draft from a validated channel record.
    pub fn from_record(record: ChannelRecord) -> Self {
        let flags = record.flags();
        let mut banks = [false; MAX_BANKS as usize];
        for (bank, member) in banks.iter_mut().enumerate() {
            let id = u16::try_from(bank).unwrap_or(u16::MAX);
            *member = record.banks().contains(BankId::new(id));
        }
        Self {
            id: record.id().get(),
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
            banks,
            tx_class: record.tx_class(),
        }
    }

    /// Validates every field into one channel record.
    pub fn validate(&self, row: usize) -> Result<ChannelRecord, ModelError> {
        let scope = ModelScope::Channel(row);
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
        let mut banks = BankMask::default();
        for (bank, member) in self.banks.iter().enumerate() {
            if *member {
                let id = BankId::new(u16::try_from(bank).unwrap_or(u16::MAX));
                banks = banks
                    .with(id, true)
                    .map_err(|_| error(scope, "banks", "references an unaddressable bank"))?;
            }
        }
        if self.id >= GENERATED_CHANNEL_ID_BASE {
            return Err(error(
                scope,
                "id",
                format!(
                    "must be 0 to {}; higher identifiers belong to channels a radio \
                     expands from a generated plan",
                    GENERATED_CHANNEL_ID_BASE - 1
                ),
            ));
        }
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(self.id),
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

/// Which kind of bank one editable row defines.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BankKind {
    /// A named bank explicit channel rows join by membership.
    #[default]
    Named,
    /// A compact arithmetic simplex plan the radio expands for itself.
    Generated,
}

impl BankKind {
    /// Returns the editor label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Named => "Named channels",
            Self::Generated => "Generated plan",
        }
    }

    /// Returns a one-line explanation of what the kind stores.
    pub const fn detail(self) -> &'static str {
        match self {
            Self::Named => "Groups the explicit channel rows which claim membership of this bank.",
            Self::Generated => {
                "One stored plan the radio expands into complete channels, however many \
                 it holds, so no channel row is stored for any of them. The target must \
                 advertise the plan encoding."
            }
        }
    }

    /// Returns every selectable kind in display order.
    pub const fn all() -> [Self; 2] {
        [Self::Named, Self::Generated]
    }
}

/// What one project costs a radio, and what its plans saved.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StorageSummary {
    /// Stored objects, including the global configuration.
    pub objects: usize,
    /// Stored bytes, excluding the image container header.
    pub bytes: usize,
    /// Channels which cost one stored object each.
    pub stored_channels: usize,
    /// Channels the radio expands from stored plans at no per-channel cost.
    pub expanded_channels: usize,
    /// Stored plans which expand into channels.
    pub plans: usize,
    /// Bytes the generated plans themselves occupy.
    pub plan_bytes: usize,
}

impl StorageSummary {
    /// Returns every channel the radio can select.
    pub const fn selectable_channels(&self) -> usize {
        self.stored_channels + self.expanded_channels
    }

    /// Returns the bytes one canonical image of this project occupies.
    ///
    /// This is what a radio must find room for: every stored object plus the
    /// image container header and one envelope per object.
    pub const fn image_bytes(&self) -> usize {
        CONFIGURATION_IMAGE_HEADER_LEN
            + self.bytes
            + self.objects * CONFIGURATION_IMAGE_OBJECT_HEADER_LEN
    }

    /// Returns the bytes saved by expanding plans instead of storing channels.
    ///
    /// The comparison is against the same configuration written as explicit
    /// channel rows, which would need no plan objects at all.
    pub const fn bytes_saved(&self) -> usize {
        self.expanded_channels * CHANNEL_ENCODED_LEN - self.plan_bytes
    }
}

/// Returns the bytes one validated plan occupies in a radio's store.
///
/// Each encoding family is stored at its own length, so a simplex band costs
/// less than a repeater sub-band and the editor says so.
fn plan_bytes(plan: GeneratedBank) -> usize {
    generated_bank_encoded_len(plan.encoding()).unwrap_or(MAX_GENERATED_BANK_ENCODED_LEN)
}

/// One validated bank, which is either named or a compact generated plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatedBank {
    /// A named bank addressed by channel membership.
    Named(ChannelBank),
    /// A compact arithmetic simplex plan.
    Generated(GeneratedBank),
}

/// One editable bank row.
// Each flag mirrors one stored device flag bit, so they stay separate fields.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BankDraft {
    /// Bank identifier from zero to fifteen.
    pub id: u16,
    /// Display name.
    pub name: String,
    /// Which kind of bank this row defines.
    pub kind: BankKind,
    /// Whether a named bank participates in scanning.
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
}

impl Default for BankDraft {
    fn default() -> Self {
        Self {
            id: 0,
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
        }
    }
}

impl BankDraft {
    /// Builds a draft from a validated named bank.
    pub fn from_record(bank: ChannelBank) -> Self {
        Self {
            id: bank.id().get(),
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
            id: bank.id().get(),
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

    /// Validates every field of this row into one bank.
    pub fn validate(&self, row: usize) -> Result<ValidatedBank, ModelError> {
        let scope = ModelScope::Bank(row);
        let name = BankName::new(self.name.trim())
            .map_err(|_| error(scope, "name", "must be 1 to 16 printable characters"))?;
        if self.id >= MAX_BANKS {
            return Err(error(
                scope,
                "id",
                format!("must be 0 to {}", MAX_BANKS - 1),
            ));
        }
        match self.kind {
            BankKind::Named => ChannelBank::new(
                BankId::new(self.id),
                name,
                BankFlags::default().with(BankFlags::SCAN_ENABLED, self.scan_enabled),
            )
            .map(ValidatedBank::Named)
            .map_err(|_| error(scope, "id", format!("must be 0 to {}", MAX_BANKS - 1))),
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
                    BankId::new(self.id),
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

    /// Returns the bytes this plan occupies in a radio's store.
    ///
    /// A row which does not yet validate stores nothing, so it reports zero
    /// rather than a length no radio would ever be asked for.
    pub fn stored_bytes(&self) -> usize {
        match self.validate(0) {
            Ok(ValidatedBank::Generated(plan)) => plan_bytes(plan),
            _ => 0,
        }
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
    /// Editable channel rows in display order.
    pub channels: Vec<ChannelDraft>,
    /// Editable bank rows in display order.
    pub banks: Vec<BankDraft>,
    /// The editable global radio configuration.
    pub config: ConfigDraft,
}

impl ProjectModel {
    /// Returns an empty project holding the conservative configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one channel row using the next free identifier.
    pub fn add_channel(&mut self) {
        let id = self
            .channels
            .iter()
            .map(|channel| channel.id)
            .max()
            .map_or(1, |highest| highest.saturating_add(1));
        let mut draft = ChannelDraft {
            id,
            name: format!("CH{id}"),
            ..ChannelDraft::default()
        };
        if let Some(previous) = self.channels.last() {
            draft.step_hz = previous.step_hz;
            draft.modulation = previous.modulation;
            draft.bandwidth = previous.bandwidth;
            draft.banks = previous.banks;
        }
        self.channels.push(draft);
    }

    /// Appends one channel row copied from an existing row.
    ///
    /// Copying is how an operator enters a repeater pair or a group of similar
    /// channels without retyping every field, so only the identity changes.
    pub fn duplicate_channel(&mut self, row: usize) {
        let Some(source) = self.channels.get(row) else {
            return;
        };
        let id = self
            .channels
            .iter()
            .map(|channel| channel.id)
            .max()
            .map_or(1, |highest| highest.saturating_add(1));
        let mut draft = source.clone();
        draft.id = id;
        self.channels.insert(row + 1, draft);
    }

    /// Appends one named bank row using the lowest free identifier.
    pub fn add_bank(&mut self) {
        self.push_bank(BankKind::Named);
    }

    /// Appends one generated-plan bank row using the lowest free identifier.
    pub fn add_generated_bank(&mut self) {
        self.push_bank(BankKind::Generated);
    }

    /// Appends one bank row of the requested kind.
    ///
    /// Identifiers are taken from the whole bank list rather than one kind, so
    /// the number an operator reads always names exactly one row.
    fn push_bank(&mut self, kind: BankKind) {
        let id = (0..MAX_BANKS)
            .find(|candidate| !self.banks.iter().any(|bank| bank.id == *candidate))
            .unwrap_or(0);
        self.banks.push(BankDraft {
            id,
            name: format!("Bank {id}"),
            kind,
            ..BankDraft::default()
        });
    }

    /// Returns what each addressable bank identifier is defined as.
    ///
    /// Both kinds define an identifier and a channel may join either. A radio
    /// tests one membership mask for stored and expanded channels alike, so a
    /// stored channel ticked into a plan's identifier is selected beside that
    /// plan's channels under the same bank filter. That is the useful case: a
    /// calling channel or a repeater filed with the simplex band it belongs to.
    /// A named bank wins the label only because it is the one an operator
    /// chose; it does not exclude the plan.
    pub fn bank_slots(&self) -> Vec<Option<(BankKind, String)>> {
        let mut slots = vec![None; MAX_BANKS as usize];
        for bank in &self.banks {
            if let Some(slot) = slots.get_mut(usize::from(bank.id)) {
                if slot.is_none() || matches!(bank.kind, BankKind::Named) {
                    *slot = Some((bank.kind, bank.name.trim().to_owned()));
                }
            }
        }
        slots
    }

    /// Returns what this project costs a radio, and what the plans saved.
    ///
    /// Only rows which validate are counted, so the figures always describe a
    /// configuration which could actually be written.
    pub fn storage_summary(&self) -> StorageSummary {
        let mut summary = StorageSummary::default();
        for (row, draft) in self.banks.iter().enumerate() {
            match draft.validate(row) {
                Ok(ValidatedBank::Named(_)) => {
                    summary.objects += 1;
                    summary.bytes += CHANNEL_BANK_ENCODED_LEN;
                }
                Ok(ValidatedBank::Generated(plan)) => {
                    let stored = plan_bytes(plan);
                    summary.objects += 1;
                    summary.bytes += stored;
                    summary.plan_bytes += stored;
                    summary.plans += 1;
                    summary.expanded_channels += usize::from(plan.channel_count());
                }
                Err(_) => {}
            }
        }
        for (row, draft) in self.channels.iter().enumerate() {
            if draft.validate(row).is_ok() {
                summary.objects += 1;
                summary.bytes += CHANNEL_ENCODED_LEN;
                summary.stored_channels += 1;
            }
        }
        if self.config.validate().is_ok() {
            summary.objects += 1;
            summary.bytes += RADIO_CONFIG_ENCODED_LEN;
        }
        summary
    }

    /// Validates every row and builds one compilable project.
    pub fn validate(&self) -> Result<RadioProject, Vec<ModelError>> {
        let mut errors = Vec::new();
        let mut project = RadioProject::new();

        for (row, draft) in self.banks.iter().enumerate() {
            match draft.validate(row) {
                Ok(ValidatedBank::Named(bank)) => project.add_bank(bank),
                Ok(ValidatedBank::Generated(bank)) => project.add_generated_bank(bank),
                Err(failure) => errors.push(failure),
            }
        }
        for (row, draft) in self.channels.iter().enumerate() {
            match draft.validate(row) {
                Ok(channel) => project.add_channel(channel),
                Err(failure) => errors.push(failure),
            }
        }
        match self.config.validate() {
            Ok(config) => project.set_config(config),
            Err(failure) => errors.push(failure),
        }

        for (row, draft) in self.channels.iter().enumerate() {
            if self
                .channels
                .iter()
                .filter(|other| other.id == draft.id)
                .count()
                > 1
            {
                errors.push(error(
                    ModelScope::Channel(row),
                    "id",
                    format!("identifier {} is used by another channel", draft.id),
                ));
            }
        }
        // A named bank and a generated plan are separate stored objects, so only
        // two rows of the same kind can collide.
        for (row, draft) in self.banks.iter().enumerate() {
            if self
                .banks
                .iter()
                .filter(|other| other.id == draft.id && other.kind == draft.kind)
                .count()
                > 1
            {
                errors.push(error(
                    ModelScope::Bank(row),
                    "id",
                    format!("identifier {} is used by another bank", draft.id),
                ));
            }
        }

        if errors.is_empty() {
            Ok(project)
        } else {
            Err(errors)
        }
    }

    /// Validates and compiles the project into a canonical configuration image.
    pub fn to_image(&self) -> Result<Vec<u8>, Vec<ModelError>> {
        let project = self.validate()?;
        let compiled = ConfigurationCompiler::new(host_capabilities())
            .compile(&project)
            .map_err(|failure| vec![compile_error(failure)])?;
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

    /// Replaces the project with the contents of one canonical image.
    pub fn from_image(bytes: &[u8]) -> Result<Self, ModelError> {
        if bytes.len() > MAX_PROJECT_IMAGE_BYTES {
            return Err(error(
                ModelScope::Project,
                "image",
                format!("exceeds {MAX_PROJECT_IMAGE_BYTES} bytes"),
            ));
        }
        let image = decode_configuration_image(bytes)
            .map_err(|failure| error(ModelScope::Project, "image", failure.to_string()))?;
        let mut model = Self {
            channels: Vec::new(),
            banks: Vec::new(),
            config: ConfigDraft::default(),
        };
        for object in image.objects() {
            match object.key().kind {
                ObjectKind::Channel => {
                    let record = decode_channel(&object).map_err(|failure| {
                        error(ModelScope::Project, "channel", failure.to_string())
                    })?;
                    model.channels.push(ChannelDraft::from_record(record));
                }
                ObjectKind::ChannelBank => {
                    let bank = decode_channel_bank(&object).map_err(|failure| {
                        error(ModelScope::Project, "bank", failure.to_string())
                    })?;
                    model.banks.push(BankDraft::from_record(bank));
                }
                ObjectKind::RadioConfig => {
                    let config = decode_radio_config(&object).map_err(|failure| {
                        error(ModelScope::Project, "configuration", failure.to_string())
                    })?;
                    model.config = ConfigDraft::from_config(config);
                }
                ObjectKind::GeneratedBank => {
                    let bank = decode_generated_bank(&object).map_err(|failure| {
                        error(ModelScope::Project, "generated bank", failure.to_string())
                    })?;
                    model.banks.push(BankDraft::from_generated(bank));
                }
            }
        }
        Ok(model)
    }
}

fn compile_error(failure: CompileError) -> ModelError {
    error(ModelScope::Project, "compile", failure.to_string())
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
        ProjectModel, ToneDraft, ToneKind, ValidatedBank, CHANNEL_ENCODED_LEN,
        GENERATED_CHANNEL_ID_BASE,
    };
    use radio_channel_control::{ChannelSource, ProgrammedMemory};
    use radio_channel_plan::{ChannelFlags, MAX_BANKS};
    use radio_domain::{Bandwidth, BankId, PowerLevel, ScanResume, Tone, TxClass};

    fn project() -> ProjectModel {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.banks[0].name = "Amateur 2m".to_owned();
        model.add_channel();
        model.channels[0].name = "GB3AB".to_owned();
        model.channels[0].receive_mhz = "145.725".to_owned();
        model.channels[0].transmit_mhz = "145.125".to_owned();
        model.channels[0].rx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "100.0".to_owned(),
        };
        model.channels[0].tx_tone = ToneDraft {
            kind: ToneKind::DcsInverted,
            value: "023".to_owned(),
        };
        model.channels[0].tx_class = TxClass::Amateur;
        model.channels[0].banks[0] = true;
        model
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
        assert_eq!(
            dcs.validate(ModelScope::Config, "tone").unwrap(),
            Tone::Dcs {
                code: 23,
                inverted: true
            }
        );
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
    fn a_valid_project_round_trips_through_a_canonical_image() {
        let model = project();
        let image = model.to_image().unwrap();
        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!(loaded.channels.len(), 1);
        assert_eq!(loaded.banks.len(), 1);
        assert_eq!(loaded.channels[0].name, "GB3AB");
        assert_eq!(loaded.channels[0].receive_mhz, "145.725000");
        assert_eq!(loaded.channels[0].rx_tone.value, "100.0");
        assert_eq!(loaded.channels[0].tx_tone.kind, ToneKind::DcsInverted);
        assert!(loaded.channels[0].banks[0]);
        assert_eq!(loaded.banks[0].name, "Amateur 2m");
        assert_eq!(loaded.config, model.config);
        assert_eq!(loaded.to_image().unwrap(), image);
    }

    #[test]
    fn invalid_rows_are_reported_with_their_scope_and_field() {
        let mut model = project();
        model.channels[0].name = String::new();
        model.channels[0].receive_mhz = "not a frequency".to_owned();
        model.banks[0].name = String::new();
        model.config.scan_dwell_ms = 0;
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors
            .iter()
            .any(|failure| failure.scope == ModelScope::Bank(0) && failure.field == "name"));
        assert!(errors
            .iter()
            .any(|failure| failure.scope == ModelScope::Channel(0) && failure.field == "name"));
        assert!(errors
            .iter()
            .any(|failure| failure.scope == ModelScope::Config));
        assert!(model.to_image().is_err());
    }

    #[test]
    fn duplicate_identifiers_are_rejected() {
        let mut model = project();
        model.add_channel();
        model.channels[1].id = model.channels[0].id;
        model.add_bank();
        model.banks[1].id = model.banks[0].id;
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.iter().filter(|e| e.field == "id").count(), 4);
    }

    #[test]
    fn channels_cannot_reference_an_undefined_bank() {
        let mut model = project();
        model.channels[0].banks[4] = true;
        let errors = model.to_image().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].scope, ModelScope::Project);
        assert!(errors[0].detail.contains("undefined bank 4"));
    }

    #[test]
    fn new_rows_inherit_the_previous_row_and_use_free_identifiers() {
        let mut model = project();
        model.channels[0].step_hz = 6_250;
        model.add_channel();
        assert_eq!(model.channels[1].id, 2);
        assert_eq!(model.channels[1].step_hz, 6_250);
        assert!(model.channels[1].banks[0]);
        model.add_bank();
        assert_eq!(model.banks[1].id, 1);
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
    fn bank_identifiers_are_bounded() {
        for kind in BankKind::all() {
            let draft = BankDraft {
                id: 16,
                name: "Too high".to_owned(),
                kind,
                ..BankDraft::default()
            };
            assert_eq!(draft.validate(0).unwrap_err().field, "id");
        }
    }

    #[test]
    fn generated_plans_validate_their_span_and_report_it() {
        let mut draft = BankDraft {
            id: 3,
            name: "PMR446".to_owned(),
            kind: BankKind::Generated,
            base_mhz: "446.00625".to_owned(),
            spacing_hz: 12_500,
            channel_count: 16,
            tx_class: TxClass::LicenceFreePlan,
            ..BankDraft::default()
        };
        let ValidatedBank::Generated(bank) = draft.validate(0).unwrap() else {
            panic!("a generated row must validate into a generated plan");
        };
        assert_eq!(bank.channel_count(), 16);
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
        assert!(draft.generated_span().is_none());

        draft.spacing_hz = 12_500;
        draft.base_mhz = "not a frequency".to_owned();
        assert_eq!(draft.validate(0).unwrap_err().field, "base");

        // A plan which runs off the end of the frequency representation is
        // rejected before it can reach an image.
        draft.base_mhz = "4294.967".to_owned();
        draft.channel_count = u16::MAX;
        assert_eq!(draft.validate(0).unwrap_err().field, "channels");
        assert!(draft.generated_span().is_none());
    }

    #[test]
    fn generated_plans_round_trip_through_a_canonical_image() {
        let mut model = ProjectModel::new();
        model.add_generated_bank();
        model.banks[0].name = "PMR446".to_owned();
        model.banks[0].base_mhz = "446.00625".to_owned();
        model.banks[0].tx_class = TxClass::LicenceFreePlan;
        let image = model.to_image().unwrap();

        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!(loaded.banks.len(), 1);
        assert_eq!(loaded.banks[0].kind, BankKind::Generated);
        assert_eq!(loaded.banks[0].name, "PMR446");
        assert_eq!(loaded.banks[0].base_mhz, "446.006250");
        assert_eq!(loaded.banks[0].spacing_hz, 12_500);
        assert_eq!(loaded.banks[0].channel_count, 16);
        assert_eq!(loaded.banks[0].tx_class, TxClass::LicenceFreePlan);
        assert!(loaded.channels.is_empty());
        assert_eq!(loaded.to_image().unwrap(), image);
    }

    #[test]
    fn a_plan_template_is_edited_once_and_reaches_every_expanded_channel() {
        let mut model = ProjectModel::new();
        model.add_generated_bank();
        model.banks[0].id = 2;
        model.banks[0].name = "PMR446".to_owned();
        model.banks[0].base_mhz = "446.00625".to_owned();
        model.banks[0].channel_count = 8;
        model.banks[0].bandwidth = Bandwidth::Wide;
        model.banks[0].power = PowerLevel::Medium;
        model.banks[0].squelch = 5;
        model.banks[0].busy_lockout = true;
        model.banks[0].rx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "100.0".to_owned(),
        };

        let ValidatedBank::Generated(plan) = model.banks[0].validate(0).unwrap() else {
            panic!("a generated row must validate into a generated plan");
        };
        for index in 0..plan.channel_count() {
            let channel = plan.channel_record(index).unwrap();
            assert_eq!(channel.bandwidth(), Bandwidth::Wide);
            assert_eq!(channel.power(), PowerLevel::Medium);
            assert_eq!(channel.squelch().get(), 5);
            assert_eq!(channel.rx_tone(), Tone::Ctcss(1_000));
            assert!(channel.flags().contains(ChannelFlags::BUSY_LOCKOUT));
            assert!(channel.is_member_of(BankId::new(2)));
        }

        // The editor shows those channels, and the template survives storage.
        let expansion = model.banks[0].expansion(4);
        assert_eq!(expansion.len(), 4);
        assert_eq!(expansion[0], ("PMR 1".to_owned(), "446.006250".to_owned()));
        assert_eq!(expansion[3], ("PMR 4".to_owned(), "446.043750".to_owned()));

        let image = model.to_image().unwrap();
        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!(loaded.banks[0].bandwidth, Bandwidth::Wide);
        assert_eq!(loaded.banks[0].power, PowerLevel::Medium);
        assert_eq!(loaded.banks[0].squelch, 5);
        assert!(loaded.banks[0].busy_lockout);
        assert_eq!(loaded.banks[0].rx_tone.value, "100.0");
    }

    #[test]
    fn the_storage_summary_reports_what_the_plans_saved() {
        let mut model = project();
        let stored_only = model.storage_summary();
        assert_eq!(stored_only.stored_channels, 1);
        assert_eq!(stored_only.expanded_channels, 0);
        assert_eq!(stored_only.selectable_channels(), 1);

        model.add_generated_bank();
        model.banks[1].name = "PMR446".to_owned();
        model.banks[1].base_mhz = "446.00625".to_owned();
        model.banks[1].channel_count = 16;
        let summary = model.storage_summary();
        assert_eq!(summary.stored_channels, 1);
        assert_eq!(summary.expanded_channels, 16);
        assert_eq!(summary.selectable_channels(), 17);
        assert_eq!(summary.plans, 1);
        assert_eq!(
            summary.bytes_saved(),
            16 * CHANNEL_ENCODED_LEN - summary.plan_bytes
        );
        // Sixteen channels for one object: the whole point of the plan.
        assert_eq!(summary.objects, stored_only.objects + 1);

        // An invalid row is not counted, so the figures always describe a
        // configuration which could be written.
        model.banks[1].base_mhz = "not a frequency".to_owned();
        assert_eq!(model.storage_summary().expanded_channels, 0);
    }

    #[test]
    fn a_stored_channel_cannot_claim_an_expanded_identifier() {
        let mut model = project();
        model.channels[0].id = GENERATED_CHANNEL_ID_BASE;
        let failure = model.channels[0].validate(0).unwrap_err();
        assert_eq!(failure.field, "id");
        assert!(failure.detail.contains("generated plan"));
    }

    #[test]
    fn bank_slots_name_the_plan_which_owns_an_identifier() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.banks[0].id = 0;
        model.banks[0].name = "2M".to_owned();
        model.add_generated_bank();
        model.banks[1].id = 4;
        model.banks[1].name = "PMR446".to_owned();

        let slots = model.bank_slots();
        assert_eq!(slots[0], Some((BankKind::Named, "2M".to_owned())));
        assert_eq!(slots[4], Some((BankKind::Generated, "PMR446".to_owned())));
        assert_eq!(slots[5], None);
    }

    #[test]
    fn a_named_bank_and_a_generated_plan_may_share_one_identifier() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_generated_bank();
        model.banks[1].id = model.banks[0].id;
        model
            .validate()
            .expect("separate object kinds do not collide");

        model.add_generated_bank();
        model.banks[2].id = model.banks[1].id;
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.iter().filter(|e| e.field == "id").count(), 2);
    }

    #[test]
    fn a_stored_channel_joins_a_plans_bank_exactly_as_the_radio_selects_it() {
        // A calling channel filed with the simplex band it belongs to. The
        // editor must not claim this is impossible: the radio tests one
        // membership mask for both channel kinds, and this proves the answer
        // the editor gives is the answer the radio's own filter gives.
        let mut model = ProjectModel::new();
        model.add_generated_bank();
        model.banks[0].id = 3;
        model.banks[0].name = "2M SIMPLEX".to_owned();
        model.banks[0].designator = "S".to_owned();
        model.banks[0].first_number = 8;
        model.banks[0].base_mhz = "145.200000".to_owned();
        model.banks[0].spacing_hz = 25_000;
        model.banks[0].channel_count = 16;
        model.banks[0].calling_index = Some(12);

        model.channels.push(ChannelDraft {
            id: 1,
            name: "GB3XX".to_owned(),
            receive_mhz: "145.725000".to_owned(),
            transmit_mhz: "145.125000".to_owned(),
            banks: {
                let mut banks = [false; MAX_BANKS as usize];
                banks[3] = true;
                banks
            },
            ..ChannelDraft::default()
        });

        let slots = model.bank_slots();
        assert_eq!(
            slots[3],
            Some((BankKind::Generated, "2M SIMPLEX".to_owned()))
        );

        // Build the radio's own store from the same validated objects and ask
        // it which channels bank 3 selects.
        let project = model.validate().expect("project");
        let mut memory = ProgrammedMemory::<4, 2>::new();
        for plan in project.generated_banks() {
            memory.install(*plan).expect("plan installs");
        }
        for channel in project.channels() {
            memory.insert(*channel).expect("channel inserts");
        }

        let bank = BankId::new(3);
        let selected: Vec<String> = (0..memory.len())
            .filter(|index| memory.member_at(*index, bank))
            .filter_map(|index| memory.get(index))
            .map(|channel| channel.name().as_str().to_owned())
            .collect();

        assert_eq!(selected.len(), 17, "the stored channel joins the plan's 16");
        assert_eq!(selected[0], "GB3XX");
        assert_eq!(selected[1], "S8");
        assert_eq!(selected[13], "S20 CALL");
        assert_eq!(selected[16], "S23");
    }

    #[test]
    fn duplicating_a_channel_keeps_every_field_but_the_identifier() {
        let mut model = project();
        model.duplicate_channel(0);
        assert_eq!(model.channels.len(), 2);
        assert_eq!(model.channels[1].id, 2);
        assert_eq!(model.channels[1].name, model.channels[0].name);
        assert_eq!(model.channels[1].receive_mhz, model.channels[0].receive_mhz);
        assert_eq!(model.channels[1].banks, model.channels[0].banks);
        model.duplicate_channel(9);
        assert_eq!(model.channels.len(), 2);
    }

    #[test]
    fn channel_drafts_round_trip_through_records() {
        let mut draft = project().channels[0].clone();
        draft.receive_mhz = "145.725000".to_owned();
        draft.transmit_mhz = "145.125000".to_owned();
        let record = draft.validate(0).unwrap();
        assert_eq!(ChannelDraft::from_record(record), draft);
    }
}
