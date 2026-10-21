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
    BankFlags, BankGroup, BankName, BankOrigin, BankSet, BankSettings, BankSource, ChannelBank,
    ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord, ChannelTemplate, Designator,
    GeneratedBank, LocationName, Locator, PlanEncoding, SavedLocation, SharedSettings, StartPlace,
    VfoStart, GENERATED_CHANNEL_ID_BASE, MAX_BANKS, MAX_DESIGNATOR_LEN, MAX_GENERATED_CHANNELS,
    MAX_GROUPS, MAX_LOCATION_NAME_LEN, MAX_SAVED_LOCATIONS, VFO_SLOTS,
};
use radio_domain::tones::{standard_tone, CTCSS_COUNT, TONE_POSITIONS};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, Offset, PowerLevel,
    RadioConfig, RadioFlags, ScanResume, SquelchLevel, Tone, TxClass, MAX_BATTERY_SAVE_RATIO,
    MAX_SQUELCH_LEVEL, TX_POWER_LEVELS, TX_TIMEOUT_CHOICES,
};
use radio_platform::operator_shell::{
    VFO_DEFAULT_HZ, VFO_MAXIMUM_HZ, VFO_MINIMUM_HZ, VFO_STEPS_HZ,
};
use radio_programmer::{ConfigurationCompiler, DeviceCapabilities, RadioProject};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_configuration_image, decode_generated_bank,
    decode_radio_config, decode_tx_range, radio_image, upgrade, ObjectKind,
    CONFIGURATION_IMAGE_VERSION, MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
};
use radio_tx_policy::{TxRange, MAX_TX_RANGES};

use crate::locators::{SavedLocator, SavedLocators};

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
    /// One programmed TX range, by zero-based row.
    TxRange(usize),
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
            Self::TxRange(row) => write!(formatter, "TX range {}", row + 1),
            Self::Project => formatter.write_str("project"),
        }
    }
}

/// The saved locations a project file keeps, as the Map tab edits them.
fn saved_locations(project: &RadioProject) -> SavedLocators {
    SavedLocators {
        list: project
            .locations()
            .iter()
            .map(|location| SavedLocator {
                locator: location.locator.as_str().to_owned(),
                description: location
                    .name
                    .map_or_else(String::new, |name| name.as_str().to_owned()),
            })
            .collect(),
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

    /// Returns the standard values of this kind as a [`ToneDraft`] holds
    /// them, ascending: each CTCSS tone in hertz, such as `88.5`, or each DCS
    /// code's octal digits, such as `023`. The handset lists the same ones.
    pub fn standard_values(self) -> Vec<String> {
        let positions = match self {
            Self::None => 0..0,
            Self::Ctcss => 1..1 + CTCSS_COUNT,
            Self::Dcs | Self::DcsInverted => 1 + CTCSS_COUNT..TONE_POSITIONS,
        };
        positions
            .filter_map(|position| standard_tone(position, false))
            .map(|tone| ToneDraft::from_tone(tone).value)
            .collect()
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
    /// Transmit frequency in megahertz; empty, and never read, on a
    /// broadcast channel, which only receives.
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
    /// Skip this channel while scanning.
    pub scan_skip: bool,
    /// Inhibit transmission while the channel is busy.
    pub busy_lockout: bool,
    /// Exchange receive and transmit frequencies.
    pub reverse: bool,
    /// Request the audio compander.
    pub compander: bool,
    /// The calling channel of its band, which the radio can reach directly.
    pub calling: bool,
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
            power: PowerLevel::Low1,
            step_hz: 12_500,
            scan_skip: false,
            busy_lockout: false,
            reverse: false,
            compander: false,
            calling: false,
        }
    }
}

impl ChannelDraft {
    /// Builds a draft from a validated channel record.
    pub fn from_record(record: ChannelRecord) -> Self {
        let flags = record.flags();
        let receive_only = record.modulation().is_receive_only();
        Self {
            name: record.name().as_str().to_owned(),
            receive_mhz: format_mhz(record.receive().as_hz()),
            transmit_mhz: if receive_only {
                String::new()
            } else {
                format_mhz(record.transmit().as_hz())
            },
            rx_tone: ToneDraft::from_tone(record.rx_tone()),
            tx_tone: ToneDraft::from_tone(record.tx_tone()),
            modulation: record.modulation(),
            bandwidth: record.bandwidth(),
            power: record.power(),
            step_hz: record.step().as_hz(),
            scan_skip: flags.contains(ChannelFlags::SCAN_SKIP),
            busy_lockout: flags.contains(ChannelFlags::BUSY_LOCKOUT),
            reverse: flags.contains(ChannelFlags::REVERSE),
            compander: flags.contains(ChannelFlags::COMPANDER),
            calling: flags.contains(ChannelFlags::CALLING),
        }
    }

    /// Validates every field into one channel record numbered `id` and
    /// stored in `bank`.
    ///
    /// A setting the bank shares comes from the bank, whatever this row holds,
    /// so the row's own value for it is neither checked nor stored.
    pub fn validate(
        &self,
        scope: ModelScope,
        id: u16,
        bank: &ChannelBank,
    ) -> Result<ChannelRecord, ModelError> {
        let shared = bank.settings().shared;
        let name = ChannelName::new(self.name.trim())
            .map_err(|_| error(scope, "name", "must be 1 to 12 printable characters"))?;
        let receive = parse_frequency(&self.receive_mhz)
            .ok_or_else(|| error(scope, "receive", "is not a frequency in megahertz"))?;
        // Only FM transmits: whatever an AM, USB or broadcast row holds for
        // the transmit side, it has none (`ADR-105`, `ADR-107`).
        let receive_only = if shared.contains(SharedSettings::MODULATION) {
            bank.settings().modulation
        } else {
            self.modulation
        }
        .is_receive_only();
        let transmit = if receive_only || shared.contains(SharedSettings::OFFSET) {
            receive
        } else {
            parse_frequency(&self.transmit_mhz)
                .ok_or_else(|| error(scope, "transmit", "is not a frequency in megahertz"))?
        };
        let step = if shared.contains(SharedSettings::STEP) {
            bank.settings().step
        } else {
            FrequencyStep::from_hz(self.step_hz)
                .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?
        };
        let rx_tone = if receive_only || shared.contains(SharedSettings::RX_TONE) {
            Tone::None
        } else {
            self.rx_tone.validate(scope, "receive tone")?
        };
        let tx_tone = if receive_only || shared.contains(SharedSettings::TX_TONE) {
            Tone::None
        } else {
            self.tx_tone.validate(scope, "transmit tone")?
        };
        let mut flags = ChannelFlags::default()
            .with(ChannelFlags::SCAN_SKIP, self.scan_skip)
            .with(ChannelFlags::CALLING, self.calling);
        if !receive_only {
            flags = flags
                .with(ChannelFlags::BUSY_LOCKOUT, self.busy_lockout)
                .with(ChannelFlags::REVERSE, self.reverse)
                .with(ChannelFlags::COMPANDER, self.compander);
        }
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
        let definition = bank
            .settings()
            .apply(ChannelDefinition {
                id: ChannelId::new(id),
                name,
                receive,
                transmit,
                rx_tone,
                tx_tone,
                modulation: self.modulation,
                bandwidth: self.bandwidth,
                power: self.power,
                step,
                flags,
                bank: Some(bank.id()),
            })
            .map_err(|_| {
                error(
                    scope,
                    "transmit",
                    "is out of range at the offset the bank shares",
                )
            })?;
        ChannelRecord::new(definition)
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

/// What a bank is for, as the memory bar colours it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BankRole {
    /// Made for, or filed under, a location.
    Location,
    /// In [`COMMON_GROUP`], as a default set is: the same everywhere.
    DefaultSet,
    /// Any other bank.
    Other,
}

impl BankRole {
    /// Every role, in the order the memory bar draws them.
    pub const ALL: [Self; 3] = [Self::Location, Self::DefaultSet, Self::Other];

    /// The name the memory bar's key shows.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Location => "Locations",
            Self::DefaultSet => "Default sets",
            Self::Other => "Other",
        }
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
    /// Bytes of the radio image, when the whole project compiles into one,
    /// counted past the radio's memory when it does not fit.
    pub image_bytes: Option<usize>,
}

impl StorageSummary {
    /// Returns every channel the radio can select.
    pub const fn selectable_channels(&self) -> usize {
        self.stored_channels + self.expanded_channels
    }

    /// Returns the radio memory the image takes in tenths of a percent,
    /// rounded up so a nearly full radio never reads as having room; past
    /// 1000 when it does not fit.
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
    /// Which of the settings below a channel bank shares with its channels.
    ///
    /// A generated plan shares all of them, whatever this holds.
    pub shared: SharedSettings,
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
    /// Skip every generated channel while scanning.
    pub scan_skip: bool,
    /// Inhibit transmission on a busy generated channel.
    pub busy_lockout: bool,
    /// Request the audio compander on every generated channel.
    pub compander: bool,
    /// The channels a channel bank holds, in the radio's order.
    pub channels: Vec<ChannelDraft>,
    /// The groups the bank is in, by their place in the project's list
    /// (`ADR-116`); kept on the bank so it moves with it.
    pub groups: Vec<usize>,
    /// The generator that made the bank and for where (`ADR-117`); none for
    /// the operator's own. Kept in the project, never sent to a radio.
    pub origin: Option<Origin>,
}

/// Which generator made a bank, and the locator it made it for; empty for a
/// bank every location shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The generator.
    pub source: BankSource,
    /// The location's Maidenhead locator, upper case, or empty.
    pub locator: String,
}

impl Origin {
    /// Made by `source` for `locator`, or for every location when it is
    /// empty.
    pub fn new(source: BankSource, locator: &str) -> Self {
        Self {
            source,
            locator: locator.trim().to_ascii_uppercase(),
        }
    }
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
            shared: SharedSettings::none(),
            rx_tone: ToneDraft::default(),
            tx_tone: ToneDraft::default(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low1,
            step_hz: 12_500,
            scan_skip: false,
            busy_lockout: false,
            compander: false,
            channels: Vec::new(),
            groups: Vec::new(),
            origin: None,
        }
    }
}

impl BankDraft {
    /// Whether the bank is airband: channels, all AM in the VHF airband.
    pub fn is_airband(&self) -> bool {
        self.kind == BankKind::Named
            && !self.channels.is_empty()
            && self.channels.iter().all(|channel| {
                channel.modulation == Modulation::Am
                    && parse_mhz(&channel.receive_mhz).is_some_and(|hz| AIRBAND_HZ.contains(&hz))
            })
    }

    /// Whether every channel is FM in the marine VHF band, as the marine
    /// generator's banks are.
    pub fn is_marine(&self) -> bool {
        self.kind == BankKind::Named
            && !self.channels.is_empty()
            && self.channels.iter().all(|channel| {
                channel.modulation == Modulation::Fm
                    && parse_mhz(&channel.receive_mhz).is_some_and(|hz| MARINE_HZ.contains(&hz))
            })
    }

    /// Builds a draft from a validated named bank.
    pub fn from_record(bank: ChannelBank) -> Self {
        let settings = bank.settings();
        Self {
            name: bank.name().as_str().to_owned(),
            kind: BankKind::Named,
            scan_enabled: bank.is_scan_enabled(),
            shared: settings.shared,
            modulation: settings.modulation,
            bandwidth: settings.bandwidth,
            power: settings.power,
            step_hz: settings.step.as_hz(),
            offset_hz: settings.offset.as_hz(),
            rx_tone: ToneDraft::from_tone(settings.rx_tone),
            tx_tone: ToneDraft::from_tone(settings.tx_tone),
            ..Self::default()
        }
    }

    /// Validates the settings a channel bank shares with its channels.
    ///
    /// Only a shared value is checked: one the bank does not share is kept for
    /// when the operator shares it again, but nothing uses it.
    fn settings(&self, scope: ModelScope) -> Result<BankSettings, ModelError> {
        let shared = self.shared;
        let unshared = BankSettings::unshared();
        if self.is_receive_only() {
            // Nothing on the transmit side is shown or stored.
            return Ok(BankSettings {
                shared,
                modulation: self.modulation,
                bandwidth: self.bandwidth,
                power: self.power,
                step: FrequencyStep::from_hz(self.step_hz).or_else(|_| {
                    if shared.contains(SharedSettings::STEP) {
                        Err(error(scope, "step", "must be a non-zero number of hertz"))
                    } else {
                        Ok(unshared.step)
                    }
                })?,
                offset: Offset::from_hz(0),
                rx_tone: Tone::None,
                tx_tone: Tone::None,
            });
        }
        Ok(BankSettings {
            shared,
            modulation: self.modulation,
            bandwidth: self.bandwidth,
            power: self.power,
            step: if shared.contains(SharedSettings::STEP) {
                FrequencyStep::from_hz(self.step_hz)
                    .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?
            } else {
                FrequencyStep::from_hz(self.step_hz).unwrap_or(unshared.step)
            },
            offset: Offset::from_hz(self.offset_hz),
            rx_tone: if shared.contains(SharedSettings::RX_TONE) {
                self.rx_tone.validate(scope, "receive tone")?
            } else {
                self.rx_tone
                    .validate(scope, "receive tone")
                    .unwrap_or(Tone::None)
            },
            tx_tone: if shared.contains(SharedSettings::TX_TONE) {
                self.tx_tone.validate(scope, "transmit tone")?
            } else {
                self.tx_tone
                    .validate(scope, "transmit tone")
                    .unwrap_or(Tone::None)
            },
        })
    }

    /// Whether every channel in the bank only receives, being AM, USB or
    /// broadcast: by the modulation the bank shares or, for a plan, its
    /// template.
    pub fn is_receive_only(&self) -> bool {
        self.modulation.is_receive_only()
            && (self.kind == BankKind::Generated
                || self.shared.contains(SharedSettings::MODULATION))
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
            rx_tone: ToneDraft::from_tone(template.rx_tone),
            tx_tone: ToneDraft::from_tone(template.tx_tone),
            modulation: template.modulation,
            bandwidth: template.bandwidth,
            power: template.power,
            step_hz: template.step.as_hz(),
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
            BankKind::Named => {
                let settings = self.settings(scope)?;
                ChannelBank::new(
                    BankId::new(id),
                    name,
                    BankFlags::default().with(BankFlags::SCAN_ENABLED, self.scan_enabled),
                )
                .map_err(|_| {
                    error(
                        scope,
                        "bank",
                        format!("a radio holds {MAX_BANKS} banks at most"),
                    )
                })?
                .with_settings(settings)
                .map(ValidatedBank::Named)
                .map_err(|failure| error(scope, "shared settings", failure.to_string()))
            }
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
                    self.template(scope)?,
                    Offset::from_hz(if self.is_receive_only() {
                        0
                    } else {
                        self.offset_hz
                    }),
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
        if self.is_receive_only() {
            return Ok(ChannelTemplate {
                rx_tone: Tone::None,
                tx_tone: Tone::None,
                modulation: self.modulation,
                bandwidth: self.bandwidth,
                power: self.power,
                step: FrequencyStep::from_hz(self.step_hz)
                    .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?,
                flags: ChannelFlags::default().with(ChannelFlags::SCAN_SKIP, self.scan_skip),
            });
        }
        Ok(ChannelTemplate {
            rx_tone: self.rx_tone.validate(scope, "receive tone")?,
            tx_tone: self.tx_tone.validate(scope, "transmit tone")?,
            modulation: self.modulation,
            bandwidth: self.bandwidth,
            power: self.power,
            step: FrequencyStep::from_hz(self.step_hz)
                .map_err(|_| error(scope, "step", "must be a non-zero number of hertz"))?,
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

    /// Returns the name the radio shows for the channel at `position`, or
    /// `None` past the bank's end or in a plan which does not yet expand.
    pub fn channel_name(&self, position: usize) -> Option<String> {
        match self.kind {
            BankKind::Named => self
                .channels
                .get(position)
                .map(|channel| channel.name.trim().to_owned()),
            BankKind::Generated => {
                let Ok(ValidatedBank::Generated(plan)) = self.validate(0) else {
                    return None;
                };
                let channel = plan.channel_record(u16::try_from(position).ok()?).ok()?;
                Some(channel.name().as_str().trim().to_owned())
            }
        }
    }

    /// Returns the lowest and highest receive frequency the bank covers in
    /// megahertz: a plan's span, or the span of its channels which parse.
    pub fn frequency_span(&self) -> Option<(String, String)> {
        match self.kind {
            BankKind::Generated => self.generated_span(),
            BankKind::Named => {
                let hertz = self
                    .channels
                    .iter()
                    .filter_map(|channel| parse_frequency(&channel.receive_mhz))
                    .map(Frequency::as_hz);
                let (low, high) = hertz.fold(None, |span, hz| match span {
                    None => Some((hz, hz)),
                    Some((low, high)) => Some((u32::min(low, hz), u32::max(high, hz))),
                })?;
                Some((format_mhz(low), format_mhz(high)))
            }
        }
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
    /// New channels default to busy lockout.
    pub busy_lockout_default: bool,
    /// Apply the AM gain-compensation workaround.
    pub am_fix: bool,
    /// Request squelch tail elimination on tone-coded channels.
    pub tone_tail_elimination: bool,
    /// TX power level the radio starts on, zero-based.
    pub tx_power: u8,
    /// TX timeout index the radio starts on; the timeout is one more minute.
    pub tx_timeout: u8,
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
            busy_lockout_default: config.flags.contains(RadioFlags::BUSY_LOCKOUT_DEFAULT),
            am_fix: config.flags.contains(RadioFlags::AM_FIX),
            tone_tail_elimination: config.flags.contains(RadioFlags::TONE_TAIL_ELIMINATION),
            tx_power: config.tx_power,
            tx_timeout: config.tx_timeout,
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
        if self.tx_power >= TX_POWER_LEVELS {
            return Err(error(
                scope,
                "TX power",
                format!("must be 0 to {}", TX_POWER_LEVELS - 1),
            ));
        }
        if self.tx_timeout >= TX_TIMEOUT_CHOICES {
            return Err(error(
                scope,
                "TX timeout",
                format!("must be 1 to {TX_TIMEOUT_CHOICES} minutes"),
            ));
        }
        let flags = RadioFlags::default()
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
            tx_power: self.tx_power,
            tx_timeout: self.tx_timeout,
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
    /// The programmed TX ranges, in order (`ADR-101`).
    pub tx_ranges: Vec<TxRangeDraft>,
    /// The bank groups, in the radio's order (`ADR-116`); a group with a
    /// locator is a location.
    pub groups: Vec<GroupDraft>,
    /// Where each slot starts, A first, written in the radio's place with the
    /// image; `None` starts slot A on every channel and slot B on its VFO.
    pub starts: [Option<StartDraft>; VFO_SLOTS],
    /// Each slot's programmed VFO, A first, written in the radio's place with
    /// the start places; `None` leaves that slot's VFO at the radio's defaults.
    pub vfos: [Option<VfoDraft>; VFO_SLOTS],
    /// The locations the operator keeps for the generators (`ADR-120`).
    pub locations: SavedLocators,
}

/// One slot's VFO as the Radio tab edits it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VfoDraft {
    /// The VFO frequency in megahertz.
    pub mhz: String,
    /// The tuning step in hertz, one of [`VFO_STEPS_HZ`].
    pub step_hz: u32,
    /// The tone that opens its squelch, or none for any carrier.
    pub rx_tone: Tone,
    /// The tone it transmits.
    pub tx_tone: Tone,
    /// Its TX power, or `None` for the Transmit section's default.
    pub power: Option<PowerLevel>,
}

impl Default for VfoDraft {
    fn default() -> Self {
        Self {
            mhz: format_mhz(VFO_DEFAULT_HZ),
            step_hz: VFO_STEPS_HZ[1],
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            power: None,
        }
    }
}

impl VfoDraft {
    /// The draft of a programmed VFO.
    #[must_use]
    pub fn from_vfo(vfo: VfoStart) -> Self {
        Self {
            mhz: format_mhz(vfo.hz),
            step_hz: vfo.step_hz,
            rx_tone: vfo.rx_tone,
            tx_tone: vfo.tx_tone,
            power: vfo.power,
        }
    }

    /// Checks the draft for slot `slot` (A 0, B 1).
    ///
    /// # Errors
    ///
    /// Returns a frequency the radio's VFO cannot tune, or a step it does not
    /// offer.
    pub fn validate(&self, slot: usize) -> Result<VfoStart, ModelError> {
        let field = if slot == 0 { "VFO A" } else { "VFO B" };
        let failed = |message: &str| error(ModelScope::Project, field, message);
        let hz = parse_mhz(&self.mhz)
            .filter(|hz| (VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(hz))
            .ok_or_else(|| failed("enter a frequency from 1 to 999.999 MHz"))?;
        if !VFO_STEPS_HZ.contains(&self.step_hz) {
            return Err(failed("choose a step the radio offers"));
        }
        Ok(VfoStart {
            hz,
            step_hz: self.step_hz,
            rx_tone: self.rx_tone,
            tx_tone: self.tx_tone,
            power: self.power,
        })
    }
}

/// Where one slot starts (`ADR-116`), by names so moving banks or groups
/// cannot point it elsewhere.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartDraft {
    /// The group the list is narrowed to; empty for every bank.
    pub group: String,
    /// The bank filtering the channels; empty for all of them.
    pub bank: String,
    /// The channel in that bank, from one; zero is the first of the view.
    pub channel: u16,
    /// Whether the radio lists the whole group, or every bank without one,
    /// rather than only the bank; the channel is still the bank's.
    pub whole_group: bool,
}

/// One editable bank group; its banks are those listing it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GroupDraft {
    /// The name the radio shows, up to 16 characters.
    pub name: String,
    /// A Maidenhead locator when the group is a location, else empty.
    pub locator: String,
}

/// The group the studio files airband banks under.
pub const AIRBAND_GROUP: &str = "AIRBAND";

/// The 118–137 MHz VHF airband, in hertz.
const AIRBAND_HZ: core::ops::RangeInclusive<u32> = 118_000_000..=137_000_000;

/// The group the studio files marine banks under.
pub const MARINE_GROUP: &str = "MARINE";

/// The group whose banks belong to every group on the radio, such as
/// PMR446, which is the same wherever the operator is. The project keeps
/// its members as they are; [`ProjectModel::validate_for_radio`] spreads
/// them.
pub const COMMON_GROUP: &str = "COMMON";

/// The marine VHF band, ship and coast sides, 156.000–162.025 MHz, in
/// hertz.
const MARINE_HZ: core::ops::RangeInclusive<u32> = 156_000_000..=162_025_000;

/// One editable programmed TX range (`ADR-101`): where this radio may
/// transmit, as which class, and how loud.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxRangeDraft {
    /// Service the range belongs to; the handset can switch it off.
    pub class: TxClass,
    /// Lowest permitted frequency in megahertz, inclusive.
    pub first_mhz: String,
    /// Highest permitted frequency in megahertz, inclusive.
    pub last_mhz: String,
    /// Highest permitted power in milliwatts.
    pub max_mw: u16,
    /// Whether wide FM is permitted.
    pub wide_fm: bool,
    /// Channel raster in hertz from the first frequency; zero permits any.
    pub raster_hz: u16,
}

impl Default for TxRangeDraft {
    fn default() -> Self {
        Self {
            class: TxClass::Amateur,
            first_mhz: String::new(),
            last_mhz: String::new(),
            max_mw: 5_000,
            wide_fm: true,
            raster_hz: 0,
        }
    }
}

impl TxRangeDraft {
    /// Copies a validated range into editable fields.
    pub fn from_range(range: TxRange) -> Self {
        Self {
            class: range.class,
            first_mhz: format_mhz(range.first_hz),
            last_mhz: format_mhz(range.last_hz),
            max_mw: range.max_mw,
            wide_fm: range.wide_fm,
            raster_hz: range.raster_hz,
        }
    }

    /// Validates the fields as range `row`.
    ///
    /// # Errors
    ///
    /// Returns the first field that does not hold.
    pub fn validate(&self, row: usize) -> Result<TxRange, ModelError> {
        let scope = ModelScope::TxRange(row);
        let first = parse_frequency(&self.first_mhz)
            .ok_or_else(|| error(scope, "first", "enter a frequency in MHz"))?;
        let last = parse_frequency(&self.last_mhz)
            .ok_or_else(|| error(scope, "last", "enter a frequency in MHz"))?;
        let range = TxRange {
            class: self.class,
            first_hz: first.as_hz(),
            last_hz: last.as_hz(),
            max_mw: self.max_mw,
            wide_fm: self.wide_fm,
            raster_hz: self.raster_hz,
        };
        range
            .validate()
            .map_err(|failure| error(scope, "range", failure.to_string()))?;
        Ok(range)
    }

    /// Whether this is the same range as `other`, whatever its settings.
    pub fn same_span(&self, other: &Self) -> bool {
        self.class == other.class
            && parse_frequency(&self.first_mhz) == parse_frequency(&other.first_mhz)
            && parse_frequency(&self.last_mhz) == parse_frequency(&other.last_mhz)
    }
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

    /// Moves bank `from` to just before the bank now at `slot`; `slot` equal
    /// to the bank count moves it last.
    pub fn move_bank_to(&mut self, from: usize, slot: usize) {
        move_to(&mut self.banks, from, slot);
    }

    /// The group named `name`, ignoring case.
    pub fn group_named(&self, name: &str) -> Option<usize> {
        self.groups
            .iter()
            .position(|group| group.name.trim().eq_ignore_ascii_case(name.trim()))
    }

    /// The location at `locator`, ignoring case.
    pub fn location(&self, locator: &str) -> Option<usize> {
        self.groups.iter().position(|group| {
            !group.locator.is_empty() && group.locator.eq_ignore_ascii_case(locator.trim())
        })
    }

    /// The group named `name`, or the location at `locator` when one is
    /// given, added if the project has none; `None` once the radio's limit of
    /// groups is reached.
    pub fn ensure_group(&mut self, name: &str, locator: Option<&str>) -> Option<usize> {
        let found = match locator {
            Some(locator) => self.location(locator),
            None => self.group_named(name),
        };
        if found.is_some() {
            return found;
        }
        if self.groups.len() >= usize::from(MAX_GROUPS) {
            return None;
        }
        self.groups.push(GroupDraft {
            name: name.trim().to_ascii_uppercase(),
            locator: locator
                .map_or_else(String::new, |locator| locator.trim().to_ascii_uppercase()),
        });
        Some(self.groups.len() - 1)
    }

    /// Puts bank `bank` in group `group`, or takes it out.
    pub fn set_member(&mut self, bank: usize, group: usize, member: bool) {
        let Some(bank) = self.banks.get_mut(bank) else {
            return;
        };
        bank.groups.retain(|other| *other != group);
        if member && group < self.groups.len() {
            bank.groups.push(group);
            bank.groups.sort_unstable();
        }
    }

    /// Removes group `group`; every bank stays, outside it.
    pub fn remove_group(&mut self, group: usize) {
        if group >= self.groups.len() {
            return;
        }
        self.groups.remove(group);
        for bank in &mut self.banks {
            bank.groups.retain(|other| *other != group);
            for other in &mut bank.groups {
                if *other > group {
                    *other -= 1;
                }
            }
        }
    }

    /// Files the bank named `bank` under the location at `locator`, added
    /// as `location` if the project has none, and under [`AIRBAND_GROUP`]
    /// when `airband`. Returns whether every group could be had.
    pub fn file_bank(&mut self, bank: &str, location: &str, locator: &str, airband: bool) -> bool {
        let Some(row) = self.banks.iter().position(|draft| draft.name == bank) else {
            return false;
        };
        let mut filed = true;
        match self.ensure_group(location, Some(locator)) {
            Some(group) => self.set_member(row, group, true),
            None => filed = false,
        }
        if airband {
            filed &= self.file_in(bank, AIRBAND_GROUP);
        }
        filed
    }

    /// Files the bank named `bank` under the group `name`, added if the
    /// project has none. Returns whether the group could be had.
    pub fn file_in(&mut self, bank: &str, name: &str) -> bool {
        let Some(row) = self.banks.iter().position(|draft| draft.name == bank) else {
            return false;
        };
        let Some(group) = self.ensure_group(name, None) else {
            return false;
        };
        self.set_member(row, group, true);
        true
    }

    /// Files every bank whose name ends in a Maidenhead locator, as each
    /// generator names its bank (`RPT JO02EH`), under that location.
    /// Returns how many banks were filed.
    pub fn group_by_locator(&mut self) -> usize {
        let mut filed = 0;
        for row in 0..self.banks.len() {
            let Some(locator) = trailing_locator(&self.banks[row].name) else {
                continue;
            };
            if let Some(group) = self.ensure_group(&locator, Some(&locator)) {
                if !self.banks[row].groups.contains(&group) {
                    self.set_member(row, group, true);
                    filed += 1;
                }
            }
        }
        filed
    }

    /// Files every bank whose channels are all AM in the VHF airband under
    /// [`AIRBAND_GROUP`]. Returns how many banks were filed.
    pub fn group_airband(&mut self) -> usize {
        self.group_where(AIRBAND_GROUP, BankDraft::is_airband)
    }

    /// Files every bank whose channels are all FM in the marine VHF band
    /// under [`MARINE_GROUP`]. Returns how many banks were filed.
    pub fn group_marine(&mut self) -> usize {
        self.group_where(MARINE_GROUP, BankDraft::is_marine)
    }

    /// Files every bank `belongs` accepts under the group `name`, added if
    /// the project has none. Returns how many banks were filed.
    fn group_where(&mut self, name: &str, belongs: fn(&BankDraft) -> bool) -> usize {
        let members: Vec<usize> = (0..self.banks.len())
            .filter(|row| belongs(&self.banks[*row]))
            .collect();
        if members.is_empty() {
            return 0;
        }
        let Some(group) = self.ensure_group(name, None) else {
            return 0;
        };
        let mut filed = 0;
        for row in members {
            if !self.banks[row].groups.contains(&group) {
                self.set_member(row, group, true);
                filed += 1;
            }
        }
        filed
    }

    /// Turns the generated plan in row `row` into a channel bank holding the
    /// channels it expands to, so each can be edited, and returns how many.
    ///
    /// The bank shares every setting the plan held except the tones, which
    /// each channel carries so one can differ; the calling channel stays
    /// marked. A plan whose channels would not fit the radio is refused and
    /// left as it was, since each channel costs a record where the plan cost
    /// one.
    pub fn make_channels_editable(&mut self, row: usize) -> Result<usize, String> {
        let Some(bank) = self.banks.get(row) else {
            return Err("there is no such bank".to_owned());
        };
        let Ok(ValidatedBank::Generated(plan)) = bank.validate(row) else {
            return Err(format!("{} is not a complete generated plan", bank.name));
        };
        let channels: Vec<_> = (0..plan.channel_count())
            .filter_map(|index| plan.channel_record(index).ok())
            .map(ChannelDraft::from_record)
            .collect();
        let count = channels.len();
        let mut converted = self.clone();
        let bank = &mut converted.banks[row];
        bank.kind = BankKind::Named;
        bank.shared = SharedSettings::from_bits(
            SharedSettings::MODULATION
                | SharedSettings::BANDWIDTH
                | SharedSettings::STEP
                | SharedSettings::POWER
                | SharedSettings::OFFSET,
        )
        .unwrap_or_default();
        bank.calling_index = None;
        bank.channels = channels;
        let before = self.radio_image_len();
        match converted.radio_image_len() {
            Some(bytes) if bytes > RADIO_IMAGE_BYTES => {
                return Err(format!(
                    "its {count} channels would take the image to {bytes} bytes; \
                     a radio holds {RADIO_IMAGE_BYTES}"
                ));
            }
            None if before.is_some() => {
                return Err(format!("its {count} channels would not fit the radio"));
            }
            _ => {}
        }
        *self = converted;
        Ok(count)
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
                        if channel.validate(scope, id, &bank).is_ok() {
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
        summary.image_bytes = self.radio_image_len();
        summary
    }

    /// What `bank` is for: COMMON's banks are default sets, then a bank a
    /// generator made for a location, or filed under one, is a location's.
    pub fn bank_role(&self, bank: &BankDraft) -> BankRole {
        let common = self.group_named(COMMON_GROUP);
        if common.is_some_and(|common| bank.groups.contains(&common)) {
            return BankRole::DefaultSet;
        }
        let made_for_one = bank
            .origin
            .as_ref()
            .is_some_and(|origin| !origin.locator.is_empty());
        let filed_under_one = bank.groups.iter().any(|&group| {
            self.groups
                .get(group)
                .is_some_and(|group| !group.locator.is_empty())
        });
        if made_for_one || filed_under_one {
            BankRole::Location
        } else {
            BankRole::Other
        }
    }

    /// The radio image bytes each [`BankRole`]'s banks take, in
    /// [`BankRole::ALL`]'s order: each one's banks alone, less an empty
    /// project. What the groups and configuration take is in none of them.
    pub fn role_bytes(&self) -> Option<[usize; 3]> {
        let empty = Self::new().radio_image_len()?;
        let mut bytes = [0; 3];
        for (role, total) in BankRole::ALL.into_iter().zip(&mut bytes) {
            let banks: Vec<_> = self
                .banks
                .iter()
                .filter(|bank| self.bank_role(bank) == role)
                .map(|bank| BankDraft {
                    groups: Vec::new(),
                    ..bank.clone()
                })
                .collect();
            if banks.is_empty() {
                continue;
            }
            let alone = Self {
                banks,
                ..Self::new()
            };
            *total = alone.radio_image_len()?.saturating_sub(empty);
        }
        Some(bytes)
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
                        match channel.validate(scope, id, &bank) {
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
        if self.tx_ranges.len() > MAX_TX_RANGES {
            errors.push(error(
                ModelScope::Project,
                "TX ranges",
                format!("a radio holds at most {MAX_TX_RANGES}"),
            ));
        }
        for (row, range) in self.tx_ranges.iter().enumerate() {
            match range.validate(row) {
                Ok(range) => project.add_tx_range(range),
                Err(failure) => errors.push(failure),
            }
        }

        self.validate_place(&mut project, &mut errors);
        if self.groups.len() > usize::from(MAX_GROUPS) {
            errors.push(error(
                ModelScope::Project,
                "groups",
                format!("a radio holds at most {MAX_GROUPS}"),
            ));
        }
        for (row, group) in self.groups.iter().enumerate() {
            match self.validate_group(row, group) {
                Ok(group) => project.add_group(group),
                Err(failure) => errors.push(failure),
            }
        }
        for (row, bank) in self.banks.iter().enumerate() {
            let Some(origin) = &bank.origin else {
                continue;
            };
            let failed = |reason: &str| error(ModelScope::Bank(row), "origin", reason);
            let locator = if origin.locator.is_empty() {
                Ok(None)
            } else {
                Locator::new(&origin.locator)
                    .map(Some)
                    .map_err(|_| failed("has a locator that is too long"))
            };
            let made = locator.and_then(|locator| {
                let id = u16::try_from(row).map_err(|_| failed("is past the radio's limit"))?;
                BankOrigin::new(BankId::new(id), origin.source, locator)
                    .map_err(|_| failed("is past the radio's limit"))
            });
            match made {
                Ok(origin) => project.add_origin(origin),
                Err(failure) => errors.push(failure),
            }
        }
        self.validate_locations(&mut project, &mut errors);

        if errors.is_empty() {
            Ok(project)
        } else {
            Err(errors)
        }
    }

    /// Checks the saved locations the project file keeps (`ADR-120`).
    fn validate_locations(&self, project: &mut RadioProject, errors: &mut Vec<ModelError>) {
        for saved in &self.locations.list {
            let failed = |reason: &str| {
                error(
                    ModelScope::Project,
                    "saved location",
                    format!("{}: {reason}", saved.label()),
                )
            };
            let locator = Locator::new(&saved.locator).map_err(|_| failed("not a locator"));
            let name = if saved.description.is_empty() {
                Ok(None)
            } else {
                LocationName::new(&saved.description)
                    .map(Some)
                    .map_err(|_| {
                        failed(&format!(
                            "a name is up to {MAX_LOCATION_NAME_LEN} letters, digits and \
                         punctuation"
                        ))
                    })
            };
            match (locator, name) {
                (Ok(locator), Ok(name)) => project.add_location(SavedLocation { locator, name }),
                (Err(failure), _) | (_, Err(failure)) => errors.push(failure),
            }
        }
        if self.locations.list.len() > MAX_SAVED_LOCATIONS {
            errors.push(error(
                ModelScope::Project,
                "saved locations",
                format!("a project keeps at most {MAX_SAVED_LOCATIONS}"),
            ));
        }
    }

    /// Checks what a write sends as the radio's place: each slot's start
    /// place and programmed VFO. The radio has one group, so two start places
    /// must name the same one.
    fn validate_place(&self, project: &mut RadioProject, errors: &mut Vec<ModelError>) {
        for (slot, start) in self.starts.iter().enumerate() {
            if let Some(start) = start {
                match self.validate_start(slot, start) {
                    Ok(start) => project.set_start(slot, Some(start)),
                    Err(failure) => errors.push(failure),
                }
            }
        }
        if let [Some(a), Some(b)] = project.starts() {
            if a.group != b.group {
                errors.push(error(
                    ModelScope::Project,
                    "start",
                    "slots A and B start in different groups; the radio has one group",
                ));
            }
        }
        for (slot, vfo) in self.vfos.iter().enumerate() {
            if let Some(vfo) = vfo {
                match vfo.validate(slot) {
                    Ok(vfo) => project.set_vfo(slot, Some(vfo)),
                    Err(failure) => errors.push(failure),
                }
            }
        }
    }

    /// Validates slot `slot`'s start place: its group and bank exist, the
    /// bank is in the group, and the channel is in the bank.
    fn validate_start(&self, slot: usize, start: &StartDraft) -> Result<StartPlace, ModelError> {
        let field = if slot == 0 { "start A" } else { "start B" };
        let failed = |message: String| error(ModelScope::Project, field, message);
        let group = if start.group.trim().is_empty() {
            None
        } else {
            Some(
                self.group_named(&start.group)
                    .ok_or_else(|| failed(format!("names no group {:?}", start.group)))?,
            )
        };
        let bank = if start.bank.trim().is_empty() {
            None
        } else {
            let row = self
                .banks
                .iter()
                .position(|bank| bank.name.trim() == start.bank.trim())
                .ok_or_else(|| failed(format!("names no bank {:?}", start.bank)))?;
            // A bank in COMMON is in every group on the radio.
            let common = self.group_named(COMMON_GROUP);
            let groups = &self.banks[row].groups;
            let in_group = |group: usize| {
                groups.contains(&group) || common.is_some_and(|common| groups.contains(&common))
            };
            if group.is_some_and(|group| !in_group(group)) {
                return Err(failed(format!(
                    "bank {:?} is not in group {:?}",
                    start.bank, start.group
                )));
            }
            Some(row)
        };
        let channel = match (bank, start.channel) {
            (Some(row), number @ 1..) => {
                let bank = &self.banks[row];
                let count = match bank.kind {
                    BankKind::Named => bank.channels.len(),
                    BankKind::Generated => usize::from(bank.channel_count),
                };
                if usize::from(number) > count {
                    return Err(failed(format!(
                        "bank {:?} has {count} channels, not {number}",
                        start.bank
                    )));
                }
                Some((bank_id(row)?, number - 1))
            }
            _ => None,
        };
        Ok(StartPlace {
            group: group.and_then(|group| u8::try_from(group).ok()),
            bank: bank
                .filter(|_| !start.whole_group)
                .map(bank_id)
                .transpose()?,
            channel,
        })
    }

    /// Validates group `row`: its name, its locator, and the banks listing
    /// it, by their identifiers.
    fn validate_group(&self, row: usize, group: &GroupDraft) -> Result<BankGroup, ModelError> {
        let failed = |message: &str| {
            error(
                ModelScope::Project,
                "groups",
                format!("group {} {message}", row + 1),
            )
        };
        let name = BankName::new(group.name.trim())
            .map_err(|_| failed("needs a name of 1 to 16 printable characters"))?;
        let locator = if group.locator.trim().is_empty() {
            None
        } else {
            let text = group.locator.trim().to_ascii_uppercase();
            crate::repeaters::locator_centre(&text).map_err(|failure| failed(&failure))?;
            Some(Locator::new(&text).map_err(|_| failed("has a locator that is too long"))?)
        };
        let banks = (0..self.banks.len())
            .filter(|bank| self.banks[*bank].groups.contains(&row))
            .filter_map(|bank| u16::try_from(bank).ok())
            .fold(BankSet::EMPTY, |set, bank| set.with(BankId::new(bank)));
        let id = u8::try_from(row).map_err(|_| failed("is past the radio's limit"))?;
        BankGroup::new(id, name, banks)
            .map(|group| group.with_locator(locator))
            .map_err(|failure| failed(&failure.to_string()))
    }

    /// Validates the project as a radio gets it: each bank in the
    /// [`COMMON_GROUP`] is also in every other group.
    pub fn validate_for_radio(&self) -> Result<RadioProject, Vec<ModelError>> {
        let Some(common) = self.group_named(COMMON_GROUP) else {
            return self.validate();
        };
        let mut spread = self.clone();
        for bank in &mut spread.banks {
            if bank.groups.contains(&common) {
                bank.groups = (0..self.groups.len()).collect();
            }
        }
        spread.validate()
    }

    /// Validates and compiles the image a radio runs (format 5).
    pub fn to_radio_image(&self) -> Result<Vec<u8>, Vec<ModelError>> {
        self.validate_for_radio()?
            .compile_radio_image()
            .map_err(|failure| {
                vec![error(
                    ModelScope::Project,
                    "radio image",
                    failure.to_string(),
                )]
            })
    }

    /// How long the radio image is, even past the radio's memory, when the
    /// project validates and compiles.
    pub fn radio_image_len(&self) -> Option<usize> {
        self.validate_for_radio().ok()?.radio_image_len().ok()
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

    /// Whether `bytes` is a project file an older studio saved, which
    /// [`Self::from_image`] converts (`ADR-115`).
    pub fn is_older_project(bytes: &[u8]) -> bool {
        bytes.get(4..6) == Some(&[CONFIGURATION_IMAGE_VERSION, upgrade::MASK_FORMAT_VERSION])
    }

    /// Builds a project from a project file or an image read from a radio.
    ///
    /// The format 5 object set and the format 7 radio image are read, and a
    /// format 4 project file is converted to format 5 first; saving then
    /// writes format 5, so the conversion is one way.
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
        let mut upgraded = bytes.to_vec();
        upgrade::project(&mut upgraded).map_err(invalid)?;
        let image = decode_configuration_image(&upgraded).map_err(invalid)?;
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
                // Objects come in key order, so ranges keep their positions.
                ObjectKind::TxRange => {
                    project.add_tx_range(decode_tx_range(&object).map_err(invalid)?);
                }
                ObjectKind::BankGroup => {
                    project.add_group(radio_storage::decode_bank_group(&object).map_err(invalid)?);
                }
                ObjectKind::StartPlace => {
                    let (slot, start) =
                        radio_storage::decode_start_place(&object).map_err(invalid)?;
                    project.set_start(usize::from(slot), Some(start));
                }
                ObjectKind::BankOrigin => {
                    project
                        .add_origin(radio_storage::decode_bank_origin(&object).map_err(invalid)?);
                }
                ObjectKind::SavedLocation => {
                    project.add_location(
                        radio_storage::decode_saved_location(&object).map_err(invalid)?,
                    );
                }
                ObjectKind::VfoStart => {
                    let (slot, vfo) = radio_storage::decode_vfo_start(&object).map_err(invalid)?;
                    project.set_vfo(usize::from(slot), Some(vfo));
                }
            }
        }
        Ok(Self::from_project(&project))
    }

    /// Groups a project's objects into banks.
    ///
    /// Banks keep their identifier order, a bank's channels before its plan.
    /// A channel goes into the bank it names; a channel in none goes into
    /// a last bank of its own, because every channel here is in a bank.
    pub fn from_project(project: &RadioProject) -> Self {
        let first_bank = |channel: &ChannelRecord| channel.bank().map(BankId::get);
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
            tx_ranges: project
                .tx_ranges()
                .iter()
                .map(|range| TxRangeDraft::from_range(*range))
                .collect(),
            groups: project
                .groups()
                .iter()
                .map(|group| GroupDraft {
                    name: group.name().as_str().to_owned(),
                    locator: group
                        .locator()
                        .map_or_else(String::new, |locator| locator.as_str().to_owned()),
                })
                .collect(),
            starts: [None, None],
            vfos: project.vfos().map(|vfo| vfo.map(VfoDraft::from_vfo)),
            locations: saved_locations(project),
        };
        // A group names banks by identifier; each row below remembers its.
        let in_groups = |id: u16| -> Vec<usize> {
            (0..project.groups().len())
                .filter(|row| project.groups()[*row].banks().contains(BankId::new(id)))
                .collect()
        };
        let origin_of = |id: u16| {
            project
                .origins()
                .iter()
                .find(|origin| origin.bank().get() == id)
                .map(|origin| Origin {
                    source: origin.source(),
                    locator: origin
                        .locator()
                        .map_or_else(String::new, |locator| locator.as_str().to_owned()),
                })
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
                draft.groups = in_groups(id);
                draft.origin = origin_of(id);
                model.banks.push(draft);
            }
            if let Some(plan) = plan {
                let mut draft = BankDraft::from_generated(*plan);
                draft.groups = in_groups(id);
                draft.origin = origin_of(id);
                model.banks.push(draft);
            }
        }
        model.starts = project
            .starts()
            .map(|start| start.map(|start| start_draft(project, &model.groups, start)));
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

/// The studio's start place for a project's, naming its group and bank.
fn start_draft(project: &RadioProject, groups: &[GroupDraft], start: StartPlace) -> StartDraft {
    let name_of = |id: BankId| {
        project
            .banks()
            .iter()
            .find(|bank| bank.id() == id)
            .map(|bank| bank.name().as_str().to_owned())
            .or_else(|| {
                project
                    .generated_banks()
                    .iter()
                    .find(|plan| plan.id() == id)
                    .map(|plan| plan.name().as_str().to_owned())
            })
            .unwrap_or_default()
    };
    StartDraft {
        group: start
            .group
            .and_then(|group| groups.get(usize::from(group)))
            .map(|group| group.name.clone())
            .unwrap_or_default(),
        // A channel with no bank filter is one in the whole group's view.
        bank: start
            .bank
            .or(start.channel.map(|(bank, _)| bank))
            .map(name_of)
            .unwrap_or_default(),
        channel: start.channel.map_or(0, |(_, position)| position + 1),
        whole_group: start.bank.is_none() && start.channel.is_some(),
    }
}

/// The identifier of the bank in row `row`.
fn bank_id(row: usize) -> Result<BankId, ModelError> {
    u16::try_from(row)
        .ok()
        .filter(|id| *id < MAX_BANKS)
        .map(BankId::new)
        .ok_or_else(|| {
            error(
                ModelScope::Project,
                "start",
                "names a bank past the radio's limit",
            )
        })
}

/// The Maidenhead locator a bank name ends in, as a generator names its
/// bank (`RPT JO02EH`), upper-cased.
pub fn trailing_locator(name: &str) -> Option<String> {
    let word = name.split_whitespace().last()?.to_ascii_uppercase();
    (name.split_whitespace().count() > 1
        && matches!(word.len(), 4 | 6 | 8)
        && crate::repeaters::locator_centre(&word).is_ok())
    .then_some(word)
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

/// Moves `items[from]` to just before what is now at `slot`, or last when
/// `slot` is the length. Anything out of range changes nothing.
pub(crate) fn move_to<T>(items: &mut [T], from: usize, slot: usize) {
    if from >= items.len() || slot > items.len() {
        return;
    }
    if slot > from + 1 {
        items[from..slot].rotate_left(1);
    } else if slot < from {
        items[slot..=from].rotate_right(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_mhz, parse_frequency, BankDraft, BankKind, BankRole, BankSource, ChannelDraft,
        ConfigDraft, ModelScope, Origin, ProjectModel, StartDraft, ToneDraft, ToneKind,
        ValidatedBank, COMMON_GROUP, MARINE_GROUP, RADIO_IMAGE_BYTES,
    };
    use radio_channel_plan::{ChannelFlags, SharedSettings, MAX_BANKS};
    use radio_domain::{Bandwidth, BankId, Modulation, PowerLevel, ScanResume, Tone};
    use radio_programmer::RadioProject;

    /// The tone editor offers the handset's tones, each one a draft takes.
    #[test]
    fn each_tone_kind_offers_the_standard_values_and_each_validates() {
        let ctcss = ToneKind::Ctcss.standard_values();
        assert_eq!(ctcss.len(), 50);
        assert_eq!(ctcss.first().unwrap(), "67.0");
        assert!(ctcss.contains(&"88.5".to_owned()));
        assert_eq!(ctcss.last().unwrap(), "254.1");
        let dcs = ToneKind::Dcs.standard_values();
        assert_eq!(dcs.len(), 104);
        assert_eq!(dcs.first().unwrap(), "023");
        assert_eq!(dcs.last().unwrap(), "754");
        assert_eq!(ToneKind::DcsInverted.standard_values(), dcs);
        assert!(ToneKind::None.standard_values().is_empty());
        for kind in ToneKind::all() {
            for value in kind.standard_values() {
                let draft = ToneDraft { kind, value };
                assert!(
                    draft.validate(ModelScope::Project, "tone").is_ok(),
                    "{draft:?}"
                );
            }
        }
    }

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
        model
    }

    fn pmr_plan(model: &mut ProjectModel) -> usize {
        model.add_generated_bank();
        let row = model.banks.len() - 1;
        model.banks[row].name = "PMR446".to_owned();
        model.banks[row].base_mhz = "446.006250".to_owned();
        model.banks[row].designator = "PMR ".to_owned();
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
            .map(|channel| (channel.id().get(), channel.bank()))
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

    /// Rewrites a current project file as a format 4 studio saved it:
    /// format 4 in the header and every object, each channel's bank as a
    /// membership mask, and the checksum resealed.
    fn as_format_four(mut bytes: Vec<u8>) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = u32::MAX;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xEDB8_8320 & 0_u32.wrapping_sub(crc & 1));
                }
            }
            !crc
        }
        bytes[5] = 4;
        let mut offset = 16;
        while offset < bytes.len() {
            let len = usize::from(u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]));
            let data = offset + 5;
            bytes[data] = 4;
            if bytes[offset] == radio_storage::ObjectKind::Channel as u8 {
                let bank = u16::from_le_bytes([bytes[data + 40], bytes[data + 41]]);
                let mask = if bank == u16::MAX { 0 } else { 1_u16 << bank };
                bytes[data + 40..data + 42].copy_from_slice(&mask.to_le_bytes());
            }
            offset = data + len;
        }
        let mut covered = bytes[..12].to_vec();
        covered.extend_from_slice(&bytes[16..]);
        let checksum = crc32(&covered);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    /// A named AM bank `name` of one channel at `mhz`.
    fn am_bank(model: &mut ProjectModel, name: &str, mhz: &str) -> usize {
        model.add_bank();
        let row = model.banks.len() - 1;
        model.banks[row].name = name.to_owned();
        model.add_channel(row);
        let channel = &mut model.banks[row].channels[0];
        channel.receive_mhz = mhz.to_owned();
        // AM only receives (`ADR-107`).
        channel.transmit_mhz = String::new();
        channel.modulation = Modulation::Am;
        row
    }

    #[test]
    fn banks_are_grouped_as_marine() {
        let mut model = project();
        for (name, mhz) in [("SEA JO02EH", "156.800000"), ("COAST", "161.975000")] {
            let row = am_bank(&mut model, name, mhz);
            model.banks[row].channels[0].modulation = Modulation::Fm;
        }
        // Not marine: AM in the band, and FM outside it.
        am_bank(&mut model, "AM SEA", "156.800000");
        let row = am_bank(&mut model, "TWO METRES", "145.500000");
        model.banks[row].channels[0].modulation = Modulation::Fm;

        assert_eq!(model.group_marine(), 2);
        assert_eq!(model.group_marine(), 0, "filing again changes nothing");
        let marine = model.group_named(MARINE_GROUP).unwrap();
        let members: Vec<_> = model
            .banks
            .iter()
            .filter(|bank| bank.groups.contains(&marine))
            .map(|bank| bank.name.as_str())
            .collect();
        assert_eq!(members, ["SEA JO02EH", "COAST"]);
    }

    #[test]
    fn banks_are_grouped_by_their_locator_and_as_airband() {
        let mut model = project();
        model.banks[0].name = "RPT JO02EH".to_owned();
        am_bank(&mut model, "AIR JO02EH", "124.975000");
        am_bank(&mut model, "AIR IO91WM", "118.500000");
        am_bank(&mut model, "EGSC CAMBRIDGE", "122.200000");
        // Not airband: AM outside it, and FM inside it.
        am_bank(&mut model, "CB AM", "27.405000");

        assert_eq!(model.group_by_locator(), 3);
        assert_eq!(model.group_by_locator(), 0, "filing again changes nothing");
        assert_eq!(model.group_airband(), 3);
        let names: Vec<_> = model
            .groups
            .iter()
            .map(|group| (group.name.as_str(), group.locator.as_str()))
            .collect();
        assert_eq!(
            names,
            [("JO02EH", "JO02EH"), ("IO91WM", "IO91WM"), ("AIRBAND", "")]
        );
        assert_eq!(model.banks[1].groups, [0, 2], "AIR JO02EH is in both");
        assert!(model.banks[4].groups.is_empty());

        // The groups survive the project file, and reach the radio.
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap(), model);
        let image = model.to_radio_image().unwrap();
        let read = RadioProject::from_radio_image(&image).unwrap();
        assert_eq!(read.groups().len(), 3);
        assert_eq!(read.groups()[2].banks().len(), 3);
        assert_eq!(read.groups()[0].locator(), None, "the radio never gets it");
    }

    #[test]
    fn a_banks_origin_survives_the_project_file_but_not_the_radio() {
        use super::Origin;
        use radio_channel_plan::BankSource;

        let mut model = project();
        model.banks[0].name = "RPT JO02EH".to_owned();
        am_bank(&mut model, "SEA CORE", "124.975000");
        let image = model.to_radio_image().unwrap();
        model.banks[0].origin = Some(Origin::new(BankSource::Repeaters, "jo02eh"));
        model.banks[1].origin = Some(Origin::new(BankSource::Marine, ""));
        let file = model.to_image().unwrap();
        let read = ProjectModel::from_image(&file).unwrap();
        assert_eq!(read, model);
        assert_eq!(read.banks[0].origin.as_ref().unwrap().locator, "JO02EH");
        assert_eq!(
            model.to_radio_image().unwrap(),
            image,
            "never sent to a radio"
        );
        let from_radio = ProjectModel::from_image(&image).unwrap();
        assert!(from_radio.banks.iter().all(|bank| bank.origin.is_none()));

        model.banks[0].origin = Some(Origin::new(BankSource::Repeaters, "JO02EH12AB"));
        assert!(model.validate().is_err(), "a locator too long is refused");
    }

    #[test]
    fn common_banks_reach_every_group_on_the_radio_but_stay_put_in_the_project() {
        use super::COMMON_GROUP;

        let mut model = project();
        model.banks[0].name = "PMR446".to_owned();
        am_bank(&mut model, "AIR JO02EH", "124.975000");
        am_bank(&mut model, "AIR JO01PX", "125.900000");
        assert!(model.file_bank("AIR JO02EH", "HOME", "JO02EH", true));
        assert!(model.file_bank("AIR JO01PX", "FELIXSTOWE", "JO01PX", true));
        assert!(model.file_in("PMR446", COMMON_GROUP));
        let common = model.group_named(COMMON_GROUP).unwrap();
        assert_eq!(model.banks[0].groups, [common]);

        // On the radio PMR446 is in every group; the rest are as filed.
        let radio = model.validate_for_radio().unwrap();
        let pmr = BankId::new(0);
        assert!(radio
            .groups()
            .iter()
            .all(|group| group.banks().contains(pmr)));
        let home = model.location("JO02EH").unwrap();
        let holds = |bank: u16| radio.groups()[home].banks().contains(BankId::new(bank));
        assert!(holds(1) && !holds(2));
        let read = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        assert!(read.banks[0].groups.contains(&home));

        // The project file keeps PMR446 in COMMON alone, so taking it out
        // there takes it out everywhere.
        let file = ProjectModel::from_image(&model.to_image().unwrap()).unwrap();
        assert_eq!(file.banks[0].groups, [common]);
        let plain = model.validate().unwrap();
        assert!(!plain.groups()[home].banks().contains(pmr));

        // The radio may start on PMR446 within HOME.
        model.starts[0] = Some(StartDraft {
            group: "HOME".to_owned(),
            bank: "PMR446".to_owned(),
            channel: 1,
            whole_group: false,
        });
        assert!(model.validate().is_ok() && model.validate_for_radio().is_ok());
    }

    #[test]
    fn programmed_vfos_survive_the_project_file_and_are_checked() {
        use super::VfoDraft;
        use radio_channel_plan::VfoStart;

        let mut model = project();
        assert!(!model.validate().unwrap().writes_place());
        model.vfos[1] = Some(VfoDraft {
            mhz: "446.068750".to_owned(),
            step_hz: 6_250,
            rx_tone: Tone::Ctcss(885),
            tx_tone: Tone::Dcs {
                code: 23,
                inverted: true,
            },
            power: Some(PowerLevel::Low4),
        });
        let project = model.validate().unwrap();
        assert_eq!(
            project.vfos(),
            [
                None,
                Some(VfoStart {
                    hz: 446_068_750,
                    step_hz: 6_250,
                    rx_tone: Tone::Ctcss(885),
                    tx_tone: Tone::Dcs {
                        code: 23,
                        inverted: true,
                    },
                    power: Some(PowerLevel::Low4),
                }),
            ]
        );
        assert!(project.writes_place());
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap(), model);

        // A frequency the VFO cannot tune, or a step it does not offer.
        for (mhz, step_hz) in [
            ("0.5", 6_250),
            ("1000", 6_250),
            ("", 6_250),
            ("145.5", 7_000),
        ] {
            model.vfos[0] = Some(VfoDraft {
                mhz: mhz.to_owned(),
                step_hz,
                ..VfoDraft::default()
            });
            assert!(model.validate().is_err(), "{mhz} {step_hz}");
        }
        model.vfos[0] = Some(VfoDraft::default());
        assert_eq!(
            model.validate().unwrap().vfos()[0].map(|vfo| (vfo.hz, vfo.step_hz)),
            Some((145_500_000, 12_500))
        );
    }

    #[test]
    fn a_start_place_survives_the_project_file_and_is_checked() {
        let mut model = project();
        model.banks[0].name = "RPT JO02EH".to_owned();
        model.add_channel(0);
        am_bank(&mut model, "AIR JO02EH", "124.975000");
        model.group_by_locator();
        model.starts[0] = Some(StartDraft {
            group: "JO02EH".to_owned(),
            bank: "RPT JO02EH".to_owned(),
            channel: 2,
            whole_group: false,
        });
        // Slot B starts on the airband bank in the same group.
        model.starts[1] = Some(StartDraft {
            group: "JO02EH".to_owned(),
            bank: "AIR JO02EH".to_owned(),
            channel: 1,
            whole_group: false,
        });
        let project = model.validate().unwrap();
        assert_eq!(
            project.starts(),
            [
                Some(radio_channel_plan::StartPlace {
                    group: Some(0),
                    bank: Some(BankId::new(0)),
                    channel: Some((BankId::new(0), 1)),
                }),
                Some(radio_channel_plan::StartPlace {
                    group: Some(0),
                    bank: Some(BankId::new(1)),
                    channel: Some((BankId::new(1), 0)),
                }),
            ]
        );
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap(), model);

        // Slot A may list the whole group while starting on that channel.
        model.starts[0].as_mut().unwrap().whole_group = true;
        assert_eq!(
            model.validate().unwrap().starts()[0],
            Some(radio_channel_plan::StartPlace {
                group: Some(0),
                bank: None,
                channel: Some((BankId::new(0), 1)),
            })
        );
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap(), model);
        model.starts[0].as_mut().unwrap().whole_group = false;

        // The radio has one group, so the slots cannot start in two.
        model.starts[1] = Some(StartDraft::default());
        assert!(model.validate().is_err(), "B in every bank, A in JO02EH");
        model.starts[1] = None;

        // A missing group or bank, a bank outside the group, or a channel
        // past the bank is refused.
        for (group, bank, channel) in [
            ("NOWHERE", "", 0),
            ("", "NO BANK", 0),
            ("", "RPT JO02EH", 3),
        ] {
            model.starts[0] = Some(StartDraft {
                group: group.to_owned(),
                bank: bank.to_owned(),
                channel,
                whole_group: false,
            });
            assert!(model.validate().is_err(), "{group} {bank} {channel}");
        }
        model.ensure_group("OTHER", None);
        model.starts[0] = Some(StartDraft {
            group: "OTHER".to_owned(),
            bank: "RPT JO02EH".to_owned(),
            channel: 0,
            whole_group: false,
        });
        assert!(model.validate().is_err(), "the bank is not in the group");
        // Every bank, first channel.
        model.starts[0] = Some(StartDraft::default());
        assert_eq!(
            model.validate().unwrap().starts()[0],
            Some(radio_channel_plan::StartPlace::default())
        );
    }

    #[test]
    fn membership_moves_with_its_bank_and_a_removed_group_renumbers() {
        let mut model = project();
        am_bank(&mut model, "AIR JO02EH", "124.975000");
        model.ensure_group("HOME", Some("jo02eh")).unwrap();
        assert_eq!(model.groups[0].locator, "JO02EH");
        assert!(model.file_bank("AIR JO02EH", "ELSEWHERE", "JO02EH", true));
        assert_eq!(model.groups[0].name, "HOME", "the location keeps its name");
        assert_eq!(model.banks[1].groups, [0, 1]);
        assert_eq!(model.location("jo02eh"), Some(0));
        assert_eq!(model.group_named("airband"), Some(1));

        model.move_bank_to(1, 0);
        assert_eq!(model.banks[0].groups, [0, 1], "it moved with the bank");
        model.remove_group(0);
        assert_eq!(model.banks[0].groups, [0], "AIRBAND is now the first");
        assert_eq!(model.groups.len(), 1);
        model.set_member(0, 0, false);
        assert!(model.banks[0].groups.is_empty());

        // A location with nothing in it yet is kept in the project file.
        model.ensure_group("TRIP", Some("IO91WM")).unwrap();
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap().groups.len(), 2);
        model.groups[1].locator = "ZZ99".to_owned();
        assert!(model.validate().is_err(), "not a locator");
    }

    #[test]
    fn more_than_sixteen_banks_reach_the_radio_and_come_back() {
        let mut model = ProjectModel::new();
        for row in 0..40 {
            model.add_bank();
            model.banks[row].name = format!("B{row}");
            model.add_channel(row);
            model.banks[row].channels[0].receive_mhz = format!("{}.000000", 430 + row);
            model.banks[row].channels[0].transmit_mhz = format!("{}.000000", 430 + row);
        }
        let image = model.to_radio_image().expect("40 banks of one channel fit");
        assert_eq!(ProjectModel::from_image(&image).unwrap(), model);
        let file = model.to_image().unwrap();
        assert_eq!(ProjectModel::from_image(&file).unwrap(), model);
    }

    #[test]
    fn a_project_an_older_studio_saved_opens_and_saves_in_the_new_format() {
        let mut model = project();
        pmr_plan(&mut model);
        model.add_bank();
        model.banks[2].name = "Marine".to_owned();
        model.add_channel(2);
        model.banks[2].channels[0].receive_mhz = "156.800000".to_owned();
        model.banks[2].channels[0].transmit_mhz = "156.800000".to_owned();
        let current = model.to_image().unwrap();
        assert!(!ProjectModel::is_older_project(&current));
        let older = as_format_four(current.clone());
        assert!(ProjectModel::is_older_project(&older));

        let loaded = ProjectModel::from_image(&older).unwrap();
        assert_eq!(loaded, model);
        // Saving writes the new format only.
        assert_eq!(loaded.to_image().unwrap(), current);
    }

    #[test]
    fn an_older_project_puts_each_channel_in_one_bank() {
        // A format 4 project could put a channel in no bank or in a plan's
        // bank; here every channel goes into exactly one. (One naming several
        // banks is read into its first when the file is converted.)
        let model = project();
        let mut legacy = RadioProject::new();
        let valid = model.validate().unwrap();
        legacy.add_bank(valid.banks()[0]);
        let channel = |id: u16, bank: Option<u16>| {
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
                flags: record.flags(),
                bank: bank.map(BankId::new),
            })
            .unwrap()
        };
        legacy.add_channel(channel(1, Some(0)));
        legacy.add_channel(channel(2, None));
        legacy.add_channel(channel(3, Some(2)));
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
        assert_eq!(model.banks[0].channels[1].name, "CH2");
        model.add_channel(9);
        assert_eq!(model.channel_count(), 2);
    }

    #[test]
    fn banks_move_and_channels_duplicate_within_their_bank() {
        let mut model = project();
        pmr_plan(&mut model);
        model.move_bank_to(1, 0);
        assert_eq!(model.banks[0].name, "PMR446");

        // Dragged: before the bank at a slot, or last at the count.
        model.add_bank();
        let names = |model: &ProjectModel| -> Vec<String> {
            model.banks.iter().map(|b| b.name.clone()).collect()
        };
        let order = names(&model);
        model.move_bank_to(0, 3);
        assert_eq!(names(&model), [&*order[1], &*order[2], &*order[0]]);
        model.move_bank_to(2, 0);
        assert_eq!(names(&model), order);
        model.move_bank_to(0, 1);
        model.move_bank_to(1, 1);
        model.move_bank_to(9, 0);
        model.move_bank_to(0, 9);
        assert_eq!(names(&model), order, "no move, or out of range");
        model.move_bank_to(1, 0);
        assert_eq!(names(&model), [&*order[1], &*order[0], &*order[2]]);
        model.move_bank_to(1, 0);

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
        draft.squelch = 3;
        draft.tx_power = radio_domain::TX_POWER_LEVELS;
        assert!(draft.validate().is_err());
        draft.tx_power = radio_domain::TX_POWER_LEVELS - 1;
        draft.tx_timeout = radio_domain::TX_TIMEOUT_CHOICES;
        assert!(draft.validate().is_err());
        draft.tx_timeout = radio_domain::TX_TIMEOUT_CHOICES - 1;
        let config = draft.validate().unwrap();
        assert_eq!((config.tx_power, config.tx_timeout), (5, 4));
    }

    /// The handset's TX settings go to the radio and come back from it.
    #[test]
    fn tx_power_and_timeout_round_trip_through_the_radio_image() {
        let mut model = project();
        model.config.tx_power = 2;
        model.config.tx_timeout = 3;
        let image = model.to_radio_image().unwrap();
        let loaded = ProjectModel::from_image(&image).unwrap();
        assert_eq!((loaded.config.tx_power, loaded.config.tx_timeout), (2, 3));
    }

    #[test]
    fn a_channel_bank_spans_the_receive_frequencies_which_parse() {
        let mut model = project();
        assert_eq!(
            model.banks[0].frequency_span(),
            Some(("145.725000".to_owned(), "145.725000".to_owned()))
        );
        model.add_channel(0);
        model.banks[0].channels[1].receive_mhz = "144.800".to_owned();
        model.add_channel(0);
        model.banks[0].channels[2].receive_mhz = "half typed.".to_owned();
        assert_eq!(
            model.banks[0].frequency_span(),
            Some(("144.800000".to_owned(), "145.725000".to_owned()))
        );
        model.banks[0].channels.clear();
        assert_eq!(model.banks[0].frequency_span(), None);
    }

    #[test]
    fn generated_plans_validate_their_span_and_report_it() {
        let mut draft = BankDraft {
            name: "PMR446".to_owned(),
            kind: BankKind::Generated,
            base_mhz: "446.00625".to_owned(),
            spacing_hz: 12_500,
            channel_count: 16,
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
        bank.power = PowerLevel::Mid;
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
            assert_eq!(channel.power(), PowerLevel::Mid);
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
    fn the_memory_bar_splits_banks_by_what_they_are_for() {
        let mut model = project();
        pmr_plan(&mut model);
        model.add_bank();
        model.add_channel(2);
        model.banks[2].channels[0].receive_mhz = "145.500".to_owned();
        assert!(model.file_in(&model.banks[1].name.clone(), COMMON_GROUP));
        let home = model.ensure_group("JO02EH", Some("JO02EH")).unwrap();
        model.banks[2].groups.push(home);
        let roles: Vec<_> = model.banks.iter().map(|b| model.bank_role(b)).collect();
        assert_eq!(
            roles,
            [BankRole::Other, BankRole::DefaultSet, BankRole::Location]
        );

        // Made for a location, a bank is that location's wherever it is filed.
        model.banks[0].origin = Some(Origin::new(BankSource::Repeaters, "JO02EH"));
        assert_eq!(model.bank_role(&model.banks[0]), BankRole::Location);
        model.banks[0].origin = None;

        let [location, common, other] = model.role_bytes().unwrap();
        assert!(location > 0 && common > 0 && other > 0);
        let total = model.storage_summary().image_bytes.unwrap();
        let empty = ProjectModel::new().radio_image_len().unwrap();
        assert!(location + common + other <= total - empty);

        // With no project image there is nothing to split.
        model.banks[1].base_mhz = "not a frequency".to_owned();
        assert_eq!(model.role_bytes(), None);
    }

    #[test]
    fn a_broadcast_channel_only_receives_whatever_its_transmit_fields_hold() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        // A channel switched from FM to broadcast keeps what it had as FM:
        // a split, tones and the transmit flags. None of it is used or checked.
        let channel = &mut model.banks[0].channels[0];
        channel.receive_mhz = "101.5".to_owned();
        channel.transmit_mhz = "not a frequency".to_owned();
        channel.modulation = Modulation::Broadcast;
        channel.step_hz = 100_000;
        channel.tx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "77.0".to_owned(),
        };
        channel.busy_lockout = true;
        channel.scan_skip = true;
        let loaded = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        let station = &loaded.banks[0].channels[0];
        assert_eq!(station.receive_mhz, "101.500000");
        assert!(station.transmit_mhz.is_empty(), "no transmit side");
        assert_eq!(station.tx_tone.kind, ToneKind::None);
        assert!(station.scan_skip && !station.busy_lockout);

        // A broadcast bank sharing an offset and tones takes none of them.
        let bank = &mut model.banks[0];
        bank.shared = SharedSettings::none()
            .with(SharedSettings::MODULATION, true)
            .with(SharedSettings::OFFSET, true)
            .with(SharedSettings::RX_TONE, true);
        bank.modulation = Modulation::Broadcast;
        bank.offset_hz = -600_000;
        bank.rx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "88.5".to_owned(),
        };
        assert!(bank.is_receive_only());
        bank.channels[0].modulation = Modulation::Fm;
        let loaded = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        assert_eq!(loaded.banks[0].offset_hz, 0);
        assert!(loaded.banks[0].channels[0].transmit_mhz.is_empty());

        // So does a broadcast plan.
        let mut plan = BankDraft {
            name: "FM".to_owned(),
            kind: BankKind::Generated,
            base_mhz: "87.5".to_owned(),
            spacing_hz: 100_000,
            channel_count: 3,
            modulation: Modulation::Broadcast,
            step_hz: 100_000,
            offset_hz: -600_000,
            ..BankDraft::default()
        };
        plan.tx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "77.0".to_owned(),
        };
        let model = ProjectModel {
            banks: vec![plan],
            ..ProjectModel::new()
        };
        let loaded = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        assert_eq!(loaded.banks[0].offset_hz, 0);
        assert_eq!(loaded.banks[0].tx_tone.kind, ToneKind::None);
    }

    #[test]
    fn only_fm_channels_have_a_transmit_side() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        model.add_channel(0);
        model.add_channel(0);
        for (channel, modulation) in model.banks[0].channels.iter_mut().zip([
            Modulation::Am,
            Modulation::Usb,
            Modulation::Fm,
        ]) {
            channel.receive_mhz = "121.5".to_owned();
            channel.transmit_mhz = String::new();
            channel.modulation = modulation;
        }
        // AM and USB need no transmit frequency; FM still does.
        let errors = model.to_radio_image().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "transmit");
        assert_eq!(errors[0].scope, ModelScope::Channel { bank: 0, row: 2 });
        model.banks[0].channels.truncate(2);
        let loaded = ProjectModel::from_image(&model.to_radio_image().unwrap()).unwrap();
        for channel in &loaded.banks[0].channels {
            assert_eq!(channel.receive_mhz, "121.500000");
            assert!(channel.transmit_mhz.is_empty(), "no transmit side");
        }
        // A bank sharing AM is receive-only too.
        let bank = &mut model.banks[0];
        bank.shared = SharedSettings::none().with(SharedSettings::MODULATION, true);
        bank.modulation = Modulation::Am;
        assert!(bank.is_receive_only());
        bank.modulation = Modulation::Fm;
        assert!(!bank.is_receive_only());
    }

    #[test]
    fn a_bank_shares_its_settings_with_every_channel_and_survives_a_save() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        model.add_channel(0);
        let bank = &mut model.banks[0];
        bank.shared = SharedSettings::none()
            .with(SharedSettings::BANDWIDTH, true)
            .with(SharedSettings::POWER, true)
            .with(SharedSettings::OFFSET, true)
            .with(SharedSettings::TX_TONE, true);
        bank.bandwidth = Bandwidth::Wide;
        bank.power = PowerLevel::High;
        bank.offset_hz = -600_000;
        bank.tx_tone = ToneDraft {
            kind: ToneKind::Ctcss,
            value: "77.0".to_owned(),
        };
        // A row's own value for a shared setting is neither used nor checked.
        bank.channels[0].transmit_mhz = "not a frequency".to_owned();
        bank.channels[0].power = PowerLevel::Low1;
        bank.channels[1].receive_mhz = "145.725000".to_owned();
        bank.channels[1].modulation = Modulation::Am;

        let project = model.validate().unwrap();
        let channels = project.channels();
        for channel in channels {
            assert_eq!(channel.bandwidth(), Bandwidth::Wide);
            assert_eq!(channel.power(), PowerLevel::High);
            assert_eq!(channel.tx_tone(), Tone::Ctcss(770));
        }
        assert_eq!(channels[1].transmit().as_hz(), 145_125_000);
        // Modulation is not shared, so each channel keeps its own.
        assert_eq!(channels[0].modulation(), Modulation::Fm);
        assert_eq!(channels[1].modulation(), Modulation::Am);

        let saved = model.to_image().unwrap();
        let loaded = ProjectModel::from_image(&saved).unwrap();
        assert_eq!(loaded.banks[0].shared, model.banks[0].shared);
        assert_eq!(loaded.banks[0].offset_hz, -600_000);
        assert_eq!(loaded.validate().unwrap().channels(), channels);
        assert!(model.to_radio_image().is_ok());
    }

    #[test]
    fn a_shared_offset_out_of_range_is_reported_on_the_channel() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        model.banks[0].shared = SharedSettings::none().with(SharedSettings::OFFSET, true);
        model.banks[0].offset_hz = -200_000_000;
        let errors = model.validate().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].scope, ModelScope::Channel { bank: 0, row: 0 });
        assert_eq!(errors[0].field, "transmit");
    }

    #[test]
    fn channel_drafts_round_trip_through_records() {
        let draft = project().banks[0].channels[0].clone();
        let ValidatedBank::Named(bank) = BankDraft::default().validate(2).unwrap() else {
            panic!("a channel bank");
        };
        let record = draft.validate(ModelScope::Project, 7, &bank).unwrap();
        assert_eq!(record.id().get(), 7);
        assert!(record.is_member_of(BankId::new(2)));
        assert_eq!(ChannelDraft::from_record(record), draft);
    }
}
