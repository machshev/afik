//! Receive-only operator shell: screens, channel and VFO selection, and banks.
//!
//! The shell is pure. It consumes debounced key presses and explicit
//! milliseconds and returns the intent the caller should apply to the shared
//! receive controller, so every screen transition, numeric entry, bank filter
//! step, and VFO tuning step is host-testable without a display, a keypad, or a
//! radio.
//!
//! The operator listens to one of two sources: a programmed memory channel or
//! the VFO, which is a directly tuned frequency. A radio nobody has programmed
//! is simply in VFO, so there is no separate unprogrammed mode.
//!
//! The radio has two slots, A and B, as the stock and Armel firmware do. Each
//! is a source of its own: a channel in any bank, or the VFO. One is selected,
//! and it is the one keys act on and PTT transmits on; dual watch listens to
//! the other between signals. F then 2 (A/B) selects the other slot and F then
//! 3 (VFO/MR) switches the selected one between its channels and the VFO.
//!
//! No intent can transmit. The set deliberately contains selection, bank
//! filtering, VFO tuning, and monitoring only.

use radio_domain::{BankId, SquelchLevel, Tone, TxClass, MAX_SQUELCH_LEVEL};
use radio_tx_policy::ClassSet;

use crate::receive_app::Key;
use radio_channel_plan::BankSet;

/// Digits a channel-number entry accepts.
pub const ENTRY_DIGITS: usize = 2;

/// Digits a VFO frequency entry accepts, in kilohertz.
///
/// Six digits reach 999.999 MHz in whole kilohertz. A finer offset, such as the
/// 6.25 kHz PMR446 raster, is reached from there with the tuning step, so the
/// keypad needs no more digits than an operator will actually type.
pub const VFO_ENTRY_DIGITS: usize = 6;

/// Milliseconds an incomplete channel-number entry waits before it commits.
pub const ENTRY_TIMEOUT_MILLISECONDS: u32 = 1_200;

/// Milliseconds the star key must be held before it starts a scan.
///
/// Long enough that an operator reaching for the source list never trips it,
/// short enough that holding a key for it does not feel like a stuck radio.
pub const HOLD_MILLISECONDS: u32 = 600;

/// Frequency the VFO starts on.
pub const VFO_DEFAULT_HZ: u32 = 145_500_000;

/// Selectable VFO tuning steps in hertz.
pub const VFO_STEPS_HZ: [u32; 6] = [6_250, 12_500, 25_000, 50_000, 100_000, 1_000_000];

/// Lowest frequency the VFO will tune to.
///
/// This is a representation bound, not a claim about the radio: the published
/// BK4819 ranges conflict and the board's own filters are unknown, so tuning
/// inside this range is not a promise that the radio can hear anything there.
/// See `EVID-BK4819-007`.
pub const VFO_MINIMUM_HZ: u32 = 1_000_000;

/// Highest frequency the VFO will tune to, which six kilohertz digits can name.
pub const VFO_MAXIMUM_HZ: u32 = 999_999_000;

/// Which way an arrow key points.
///
/// Every screen this shell draws lists its rows downwards, so Up always moves
/// towards the top of what the operator is looking at: the previous list row and
/// the previous channel position. The VFO is the one exception, because a
/// frequency is not a list and the up key tunes upwards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    /// Towards the top of the screen, or upwards in frequency.
    Up,
    /// Towards the bottom of the screen, or downwards in frequency.
    Down,
}

/// Which bounded list cursor one arrow key press moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cursor {
    /// The receive-source list.
    Source,
    /// The VFO tuning-step list.
    Step,
    /// The settings menu.
    Settings,
    /// The squelch-level list.
    Squelch,
    /// The TX power-level list.
    TxPower,
    /// The TX timeout list.
    TxTimeout,
    /// The programmed TX class list.
    TxClass,
}

/// Which receive source the operator is listening to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    /// A programmed memory channel, filtered by the active bank.
    #[default]
    Memory,
    /// A directly tuned frequency.
    Vfo,
}

/// One of the radio's two slots.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SlotId {
    /// The upper slot.
    #[default]
    A,
    /// The lower slot.
    B,
}

impl SlotId {
    /// Returns the other slot.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// What one slot listens to: its source, bank filter and VFO.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
    /// Programmed channels or the VFO.
    pub mode: Mode,
    /// The bank filtering the channels, if one is.
    pub bank_filter: Option<BankId>,
    /// The VFO frequency in hertz.
    pub vfo_hz: u32,
    /// Index into [`VFO_STEPS_HZ`].
    pub step_index: usize,
    /// The tone the VFO transmits with: none, or one the F 4 catch heard.
    pub vfo_tx_tone: Tone,
}

impl Slot {
    /// A slot on the default VFO frequency and step.
    pub const VFO: Self = Self {
        mode: Mode::Vfo,
        bank_filter: None,
        vfo_hz: VFO_DEFAULT_HZ,
        step_index: 1,
        vfo_tx_tone: Tone::None,
    };

    /// Returns the VFO tuning step in hertz.
    #[must_use]
    pub fn vfo_step_hz(&self) -> u32 {
        VFO_STEPS_HZ
            .get(self.step_index)
            .copied()
            .unwrap_or(VFO_STEPS_HZ[1])
    }
}

/// One row of the source list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// The directly tuned VFO.
    Vfo,
    /// Every programmed channel, with no bank filter.
    AllChannels,
    /// Only the channels in one bank.
    Bank(BankId),
    /// A group to narrow the list to, or `None` for every bank (`ADR-116`).
    Group(Option<u8>),
}

/// Which screen the operator is looking at.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screen {
    /// The active channel or VFO frequency, and the receive state.
    #[default]
    Operating,
    /// The VFO and every populated bank.
    SourceList,
    /// The selectable VFO tuning steps.
    StepList,
    /// The radio-wide settings an operator can change from the handset.
    Settings,
    /// The selectable squelch levels.
    SquelchList,
    /// The selectable TX power levels, where the target can transmit.
    TxPowerList,
    /// The selectable TX timeouts, where the target can transmit.
    TxTimeoutList,
    /// The programmed TX classes, each switched on or off on the handset.
    TxClassList,
    /// Image identity and storage state.
    Info,
    /// The frequency and tone catch (F then 4, `FC`).
    Catch,
}

/// One row of the settings menu.
///
/// The menu exists so a setting can be changed on the radio rather than only
/// from a host, so every row here is a value the operator can reach in the
/// field. It is deliberately a list rather than a numbered menu: the same Up,
/// Down, Menu, and Exit keys work on it as on every other screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Setting {
    /// The radio-wide squelch level applied when no channel overrides it.
    Squelch,
    /// The radio-wide TX power level, where the target can transmit.
    TxPower,
    /// How long one press may transmit, where the target can transmit.
    TxTimeout,
    /// Which programmed TX classes this handset may use (`ADR-101`).
    TxClasses,
    /// Whether the radio listens to the other slot between signals.
    DualWatch,
    /// The selected slot's VFO tuning step.
    VfoStep,
    /// Whether channels without a receive tone show the tone heard, which
    /// runs the reference's tone scan during ordinary receive; an operator
    /// experiment, off at start-up (`RISK-046`).
    ToneHeard,
}

/// Every settings row, in the order the menu lists them.
///
/// This is deliberately short. A row earns its place by being something an
/// operator changes in the field, with the radio in their hand and no host in
/// reach, and squelch qualifies because the right level depends on where they
/// are standing. The scan dwell did not: it is set once for a unit — the K1
/// wanted 90 ms, which `EVID-K1-071` records — so it belongs in the
/// configuration a programmer writes, where it can be given whole
/// milliseconds instead of a menu's worth of rows.
///
/// Dual watch qualifies because whether a second channel is worth hearing
/// depends on what the operator is doing at the time. Tone heard is an
/// experiment the operator switches while comparing receive against Armel.
///
/// The VFO step moved here from side key 1, which now switches tone squelch.
pub const SETTINGS: [Setting; 4] = [
    Setting::Squelch,
    Setting::DualWatch,
    Setting::VfoStep,
    Setting::ToneHeard,
];

/// Settings rows on a target that can transmit. TX power qualifies for the
/// same reason squelch does: the right level depends on where the operator is.
/// The TX timeout does too: a net with long overs needs a longer one. The TX
/// classes switch lets the owner narrow what a borrowed radio may transmit.
pub const SETTINGS_WITH_TX: [Setting; 7] = [
    Setting::Squelch,
    Setting::TxPower,
    Setting::TxTimeout,
    Setting::TxClasses,
    Setting::DualWatch,
    Setting::VfoStep,
    Setting::ToneHeard,
];

/// Every TX class, in the order the class list shows them.
pub const TX_CLASSES: [TxClass; 7] = [
    TxClass::LicenceFreePlan,
    TxClass::Amateur,
    TxClass::Marine,
    TxClass::Aeronautical,
    TxClass::Business,
    TxClass::Experimental,
    TxClass::CitizensBand,
];

/// Selectable TX power levels. The programmed configuration names one of
/// these, so the count lives with it.
pub use radio_domain::TX_POWER_LEVELS;
/// Rows of the TX power list: CHANNEL, then each level.
pub const TX_POWER_ROWS: u8 = TX_POWER_LEVELS + 1;

/// Nominal output of each TX power level, in milliwatts: Armel F4HWN Low1 to
/// Low4, Mid and High as its README documents them. Nominal, not measured by
/// AFIK. A programmed TX range's ceiling refuses any level above it.
pub const TX_POWER_NOMINAL_MW: [u32; TX_POWER_LEVELS as usize] = [20, 125, 250, 500, 2_000, 5_000];

/// The low levels, each the calibration's low row scaled by a target divider;
/// the levels above are the mid and high rows as the reference reads them.
pub const LOW_POWER_LEVELS: u8 = 4;

/// Selectable TX timeouts: one to five whole minutes, the last being the TX
/// policy's ceiling; and the one a radio arrives with.
pub use radio_domain::{TX_TIMEOUT_CHOICES, TX_TIMEOUT_DEFAULT};

/// Selectable squelch levels, from permanently open to tightest.
pub const SQUELCH_LEVELS: u8 = MAX_SQUELCH_LEVEL + 1;

/// What the caller should do to the receive controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intent {
    /// Nothing changed; the display does not need redrawing.
    Idle,
    /// Only the screen changed.
    Redraw,
    /// Select the next channel in the active view.
    SelectNext,
    /// Select the previous channel in the active view.
    SelectPrevious,
    /// Select one zero-based index in the active view.
    SelectIndex(u16),
    /// Listen to a different source, which may be the VFO.
    SetSource(Source),
    /// Rebuild the receive source because the VFO frequency changed.
    TuneVfo,
    /// Open or close the squelch override.
    ToggleMonitor,
    /// Apply and store a new radio-wide squelch level.
    SetSquelch(SquelchLevel),
    /// Walk the channels of the active view, stopping on a busy one.
    StartScan,
    /// Stop walking and stay on the channel the scan reached.
    StopScan,
    /// The other slot is now the selected one (F then 2, A/B).
    SwapSlot,
    /// The selected slot switched between its channels and the VFO (F then
    /// 3, VFO/MR); its memory selection is kept.
    ToggleVfo,
    /// Select the calling channel of the active view, if it has one (F then
    /// 9, Call).
    SelectCalling,
    /// Switch tone squelch on the selected slot (side key 1): on, it opens
    /// only for the channel's receive tone, or its transmit tone when it has
    /// none; off, on any carrier.
    ToggleRxTone,
    /// Start catching the strongest carrier and its tone (F then 4, `FC`).
    StartCatch,
    /// Leave the catch and return to the selected slot.
    StopCatch,
    /// Put what was caught in the VFO ([`Shell::catch_to_vfo`]).
    UseCatch,
}

/// Target capabilities which gate operator screens without changing their
/// shared navigation model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    /// Whether the target can tune a directly entered VFO frequency.
    pub vfo: bool,
    /// Whether a handset squelch setting has a target-owned apply path.
    pub settings: bool,
    /// Whether a source selector is meaningful on the target.
    pub sources: bool,
    /// Whether the target has a bounded scan adapter.
    pub scan: bool,
    /// Whether the target exposes an identity/status information screen.
    pub info: bool,
    /// Whether the target can transmit, so a TX power setting means something.
    pub tx_power: bool,
}

impl Capabilities {
    /// The complete K1 capability set.
    pub const K1: Self = Self {
        vfo: true,
        settings: true,
        sources: true,
        scan: true,
        info: true,
        tx_power: true,
    };

    /// The currently evidenced K5 capability set, including PMR446 TX. The VFO
    /// is one more channel record on the same receive path the programmed
    /// channels already use.
    pub const K5: Self = Self {
        vfo: true,
        settings: true,
        sources: true,
        scan: false,
        info: true,
        tx_power: true,
    };
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::K1
    }
}

/// The receive state the shell needs to bound its own decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Context {
    /// Channels selectable in the active bank view.
    pub visible_channels: u16,
    /// Zero-based index of the active channel in that view.
    pub active_index: u16,
    /// Whether the receive controller is currently scanning.
    pub scanning: bool,
}

/// A pending channel-number entry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Entry {
    digits: u8,
    /// Digits typed so far: a channel position, or a VFO frequency in kilohertz.
    value: u32,
    started_ms: u32,
}

/// Operator shell state.
#[derive(Clone, Copy, Debug)]
// Each flag is an independent operator switch or key state.
#[allow(clippy::struct_excessive_bools)]
pub struct Shell {
    screen: Screen,
    mode: Mode,
    entry: Option<Entry>,
    bank_filter: Option<BankId>,
    banks: BankSet,
    /// The group the source list is narrowed to, if one is (`ADR-116`).
    group: Option<u8>,
    /// How many groups the configuration holds.
    group_count: u8,
    /// The banks of `group`, which the interface reads from the image.
    group_banks: BankSet,
    source_cursor: usize,
    vfo_hz: u32,
    vfo_tx_tone: Tone,
    step_index: usize,
    step_cursor: usize,
    settings_cursor: usize,
    squelch: SquelchLevel,
    squelch_cursor: u8,
    tx_power: u8,
    /// The level the operator chose on the handset in place of the
    /// channel's; never stored, so a power cycle clears it.
    tx_override: Option<u8>,
    tx_power_cursor: u8,
    tx_timeout: u8,
    tx_timeout_cursor: u8,
    /// Classes of the programmed TX ranges.
    tx_programmed: ClassSet,
    /// Classes the operator switched off on the handset.
    tx_disabled: ClassSet,
    tx_class_cursor: u8,
    /// Whether a star press is still waiting to become a hold or a release.
    star_pending: bool,
    /// Whether F was pressed and the next key takes its F meaning.
    function: bool,
    /// Which slot the fields above describe.
    active: SlotId,
    /// The slot not selected.
    other: Slot,
    /// Whether the radio listens to the other slot between signals.
    dual_watch: bool,
    /// Whether untoned channels show the tone heard; never stored.
    tone_heard: bool,
    capabilities: Capabilities,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    /// Constructs a shell showing the operating screen with no bank filter.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            screen: Screen::Operating,
            // Nothing is programmed until a host says otherwise, and the VFO is
            // the source which always has something to tune.
            mode: Mode::Vfo,
            entry: None,
            bank_filter: None,
            banks: BankSet::EMPTY,
            group: None,
            group_count: 0,
            group_banks: BankSet::EMPTY,
            source_cursor: 0,
            vfo_hz: VFO_DEFAULT_HZ,
            vfo_tx_tone: Tone::None,
            step_index: 1,
            step_cursor: 1,
            settings_cursor: 0,
            squelch: SquelchLevel::CONSERVATIVE,
            squelch_cursor: SquelchLevel::CONSERVATIVE.get(),
            tx_power: 0,
            tx_override: None,
            tx_power_cursor: 0,
            tx_timeout: TX_TIMEOUT_DEFAULT,
            tx_timeout_cursor: TX_TIMEOUT_DEFAULT,
            tx_programmed: ClassSet::NONE,
            tx_disabled: ClassSet::NONE,
            tx_class_cursor: 0,
            star_pending: false,
            function: false,
            active: SlotId::A,
            other: Slot::VFO,
            dual_watch: false,
            tone_heard: false,
            capabilities: Capabilities::K1,
        }
    }

    /// Applies target capability gates while retaining operator selection.
    pub fn set_capabilities(&mut self, capabilities: Capabilities) {
        self.capabilities = capabilities;
        if !capabilities.vfo && self.mode == Mode::Vfo {
            self.mode = Mode::Memory;
            self.bank_filter = None;
            self.source_cursor = self.source_row();
        }
        if !capabilities.vfo && self.other.mode == Mode::Vfo {
            self.other.mode = Mode::Memory;
            self.other.bank_filter = None;
        }
        if !capabilities.sources && self.screen == Screen::SourceList {
            self.screen = Screen::Operating;
        }
        if !capabilities.settings
            && matches!(
                self.screen,
                Screen::Settings
                    | Screen::SquelchList
                    | Screen::TxPowerList
                    | Screen::TxTimeoutList
                    | Screen::TxClassList
            )
        {
            self.screen = Screen::Operating;
        }
        if !capabilities.tx_power
            && matches!(
                self.screen,
                Screen::TxPowerList | Screen::TxTimeoutList | Screen::TxClassList
            )
        {
            self.screen = Screen::Operating;
        }
        self.settings_cursor = self
            .settings_cursor
            .min(self.settings().len().saturating_sub(1));
    }

    /// Returns the settings rows this target shows, in menu order.
    #[must_use]
    pub const fn settings(&self) -> &'static [Setting] {
        if self.capabilities.tx_power {
            &SETTINGS_WITH_TX
        } else {
            &SETTINGS
        }
    }

    /// Returns the programmed TX power level, zero-based: the level of a
    /// source with none of its own, such as the VFO.
    #[must_use]
    pub const fn tx_power(&self) -> u8 {
        self.tx_power
    }

    /// Returns the level the operator chose in place of the channel's, if any.
    #[must_use]
    pub const fn tx_override(&self) -> Option<u8> {
        self.tx_override
    }

    /// Returns the level to transmit at on a source programmed with `own`:
    /// the handset override if one is chosen, otherwise `own`.
    #[must_use]
    pub const fn tx_level(&self, own: u8) -> u8 {
        match self.tx_override {
            Some(level) => level,
            None => own,
        }
    }

    /// Returns the TX power-list cursor row: row 0 is CHANNEL, row `n` is
    /// level `n - 1`.
    #[must_use]
    pub const fn tx_power_cursor(&self) -> u8 {
        self.tx_power_cursor
    }

    /// Returns the selected TX timeout index; the timeout is one more minute.
    #[must_use]
    pub const fn tx_timeout(&self) -> u8 {
        self.tx_timeout
    }

    /// Returns the selected TX timeout in milliseconds.
    #[must_use]
    pub const fn tx_timeout_ms(&self) -> u32 {
        (self.tx_timeout as u32 + 1) * 60_000
    }

    /// Returns the TX timeout-list cursor row.
    #[must_use]
    pub const fn tx_timeout_cursor(&self) -> u8 {
        self.tx_timeout_cursor
    }

    /// Returns the active target capability gates.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Returns the radio-wide squelch level in force.
    #[must_use]
    pub const fn squelch(&self) -> SquelchLevel {
        self.squelch
    }

    /// Adopts the squelch level a programmed configuration carries.
    ///
    /// A host write is the authority on this, so the handset menu shows what
    /// the radio was last programmed with rather than its own stale copy.
    pub fn set_squelch(&mut self, squelch: SquelchLevel) {
        self.squelch = squelch;
        self.squelch_cursor = squelch.get();
    }

    /// Adopts the TX power level and timeout a programmed configuration
    /// carries, as [`Self::set_squelch`] does the squelch.
    ///
    /// The power is the level of a source with none of its own; the operator's
    /// override is kept. Values outside the selectable range leave the current
    /// choice alone; a validated configuration never carries one.
    pub fn set_tx(&mut self, power: u8, timeout: u8) {
        if power < TX_POWER_LEVELS {
            self.tx_power = power;
        }
        if timeout < TX_TIMEOUT_CHOICES {
            self.tx_timeout = timeout;
            self.tx_timeout_cursor = timeout;
        }
    }

    /// Adopts the classes of the programmed TX ranges, which the class list
    /// shows; the operator's switches are kept.
    pub fn set_tx_classes(&mut self, programmed: ClassSet) {
        self.tx_programmed = programmed;
        let rows = self.tx_class_rows();
        if usize::from(self.tx_class_cursor) >= rows {
            self.tx_class_cursor = 0;
        }
    }

    /// Restores the classes the operator switched off, as the radio's own
    /// record kept them.
    pub fn set_tx_disabled(&mut self, disabled: ClassSet) {
        self.tx_disabled = disabled;
    }

    /// Returns the classes the operator switched off.
    #[must_use]
    pub const fn tx_disabled(&self) -> ClassSet {
        self.tx_disabled
    }

    /// Returns the classes the operator has not switched off, for
    /// [`radio_tx_policy::TxPolicy::restricted`].
    #[must_use]
    pub const fn tx_enabled(&self) -> ClassSet {
        ClassSet::from_bits(!self.tx_disabled.bits())
    }

    /// Returns the programmed classes in list order, and how many there are.
    #[must_use]
    pub fn tx_class_list(&self) -> ([Option<TxClass>; TX_CLASSES.len()], usize) {
        let mut list = [None; TX_CLASSES.len()];
        let mut count = 0;
        for class in TX_CLASSES {
            if self.tx_programmed.contains(class) {
                list[count] = Some(class);
                count += 1;
            }
        }
        (list, count)
    }

    fn tx_class_rows(&self) -> usize {
        self.tx_class_list().1
    }

    /// Returns the TX class-list cursor row.
    #[must_use]
    pub const fn tx_class_cursor(&self) -> u8 {
        self.tx_class_cursor
    }

    /// Returns the settings-menu cursor row.
    #[must_use]
    pub const fn settings_cursor(&self) -> usize {
        self.settings_cursor
    }

    /// Returns the squelch-list cursor row.
    #[must_use]
    pub const fn squelch_cursor(&self) -> u8 {
        self.squelch_cursor
    }

    /// Returns the source the operator is listening to.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Returns which slot is selected.
    #[must_use]
    pub const fn active_slot(&self) -> SlotId {
        self.active
    }

    /// Returns what the selected slot listens to.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        Slot {
            mode: self.mode,
            bank_filter: self.bank_filter,
            vfo_hz: self.vfo_hz,
            step_index: self.step_index,
            vfo_tx_tone: self.vfo_tx_tone,
        }
    }

    /// Returns what the slot not selected listens to.
    #[must_use]
    pub const fn other_slot(&self) -> Slot {
        self.other
    }

    /// Reports whether dual watch is on.
    #[must_use]
    pub const fn dual_watch(&self) -> bool {
        self.dual_watch
    }

    /// Switches dual watch, as a configuration or the retained place says.
    pub fn set_dual_watch(&mut self, on: bool) {
        self.dual_watch = on;
    }

    /// Returns the tone the selected slot's VFO transmits with.
    #[must_use]
    pub const fn vfo_tx_tone(&self) -> Tone {
        self.vfo_tx_tone
    }

    /// Puts a caught frequency in the selected slot's VFO, with the caught
    /// tone (or none) to transmit, and shows the operating screen; the caller
    /// rebuilds the slot as for VFO/MR. Refused, leaving the catch open,
    /// without a VFO or off its range.
    pub fn catch_to_vfo(&mut self, hertz: u32, tone: Option<Tone>) -> bool {
        if self.screen != Screen::Catch
            || !self.capabilities.vfo
            || !(VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(&hertz)
        {
            return false;
        }
        self.mode = Mode::Vfo;
        self.vfo_hz = hertz;
        self.vfo_tx_tone = tone.unwrap_or(Tone::None);
        self.entry = None;
        self.screen = Screen::Operating;
        true
    }

    /// Reports whether untoned channels show the tone heard.
    #[must_use]
    pub const fn tone_heard(&self) -> bool {
        self.tone_heard
    }

    /// Reports whether F was pressed and waits for the key it modifies.
    #[must_use]
    pub const fn function_pending(&self) -> bool {
        self.function
    }

    /// Makes the other slot the selected one.
    // `#[inline(never)]` here and on the functions below keeps one copy of a
    // function the K5 build otherwise inlines at every caller (`RISK-049`).
    #[inline(never)]
    fn swap_slots(&mut self) {
        let selected = self.slot();
        self.mode = self.other.mode;
        self.bank_filter = self.other.bank_filter;
        self.vfo_hz = self.other.vfo_hz;
        self.vfo_tx_tone = self.other.vfo_tx_tone;
        self.step_index = self.other.step_index;
        self.step_cursor = self.step_index;
        self.other = selected;
        self.active = self.active.other();
        self.entry = None;
        self.source_cursor = self.source_row();
    }

    /// Returns the VFO frequency in hertz.
    #[must_use]
    pub const fn vfo_hz(&self) -> u32 {
        self.vfo_hz
    }

    /// Returns the VFO tuning step in hertz.
    #[must_use]
    #[inline(never)]
    pub fn vfo_step_hz(&self) -> u32 {
        VFO_STEPS_HZ
            .get(self.step_index)
            .copied()
            .unwrap_or(VFO_STEPS_HZ[1])
    }

    /// Returns the step-list cursor row.
    #[must_use]
    pub const fn step_cursor(&self) -> usize {
        self.step_cursor
    }

    /// Returns the selected step row.
    #[must_use]
    pub const fn step_index(&self) -> usize {
        self.step_index
    }

    /// Returns the visible screen.
    #[must_use]
    pub const fn screen(&self) -> Screen {
        self.screen
    }

    /// Returns the active bank filter.
    #[must_use]
    pub const fn bank_filter(&self) -> Option<BankId> {
        self.bank_filter
    }

    /// Returns the digits typed so far, if a number is being entered.
    #[must_use]
    pub fn entry(&self) -> Option<u32> {
        self.entry.map(|entry| entry.value)
    }

    /// Returns the populated banks.
    #[must_use]
    pub const fn banks(&self) -> BankSet {
        self.banks
    }

    /// Returns the source-list cursor row.
    #[must_use]
    pub const fn source_cursor(&self) -> usize {
        self.source_cursor
    }

    /// Returns the group the source list is narrowed to, if one is.
    #[must_use]
    pub const fn group(&self) -> Option<u8> {
        self.group
    }

    /// Returns the banks of the group the list is narrowed to.
    #[must_use]
    pub const fn group_banks(&self) -> BankSet {
        self.group_banks
    }

    /// The banks the source list offers: the group's, or every bank.
    const fn listed(&self) -> BankSet {
        if self.group.is_some() {
            self.group_banks
        } else {
            self.banks
        }
    }

    /// Returns the number of source-list rows: the VFO, every channel, the
    /// banks, then every bank and each group when there are groups.
    #[must_use]
    pub fn source_rows(&self) -> usize {
        let groups = if self.group_count > 0 {
            usize::from(self.group_count) + 1
        } else {
            0
        };
        self.listed().len() + groups + if self.capabilities.vfo { 2 } else { 1 }
    }

    /// Returns the source one row selects.
    #[must_use]
    pub fn source_at(&self, row: usize) -> Option<Source> {
        let vfo = usize::from(self.capabilities.vfo);
        if row < vfo {
            return Some(Source::Vfo);
        }
        let Some(row) = (row - vfo).checked_sub(1) else {
            return Some(Source::AllChannels);
        };
        let listed = self.listed();
        if let Some(bank) = listed.nth(row) {
            return Some(Source::Bank(bank));
        }
        match row - listed.len() {
            0 if self.group_count > 0 => Some(Source::Group(None)),
            group => group
                .checked_sub(1)
                .and_then(|group| u8::try_from(group).ok())
                .filter(|group| *group < self.group_count)
                .map(|group| Source::Group(Some(group))),
        }
    }

    /// Reports whether one source row is the source in use.
    #[must_use]
    pub fn is_active_source(&self, row: usize) -> bool {
        match (self.source_at(row), self.mode) {
            (Some(Source::Vfo), Mode::Vfo) => true,
            (Some(Source::AllChannels), Mode::Memory) => self.bank_filter.is_none(),
            (Some(Source::Bank(bank)), Mode::Memory) => self.bank_filter == Some(bank),
            (Some(Source::Group(group)), _) => self.group == group,
            _ => false,
        }
    }

    /// Selects the programmed memory as the source.
    ///
    /// A host write is the operator asking for their channels, so the radio
    /// leaves the VFO for them rather than waiting to be told twice.
    pub fn select_memory(&mut self) {
        self.mode = Mode::Memory;
        self.other.mode = Mode::Memory;
        self.entry = None;
        self.source_cursor = self.source_row();
    }

    /// Restores the source and bank filter the operator last listened to.
    ///
    /// [`Shell::set_banks`] must have run first: a bank the current
    /// configuration does not populate is dropped rather than restored, so a
    /// radio reprogrammed since it was last switched off cannot come back
    /// filtered to a view with nothing in it.
    pub fn restore_source(&mut self, memory_mode: bool, bank: Option<BankId>) {
        self.screen = Screen::Operating;
        self.entry = None;
        self.mode = if memory_mode { Mode::Memory } else { Mode::Vfo };
        self.bank_filter = bank.filter(|bank| self.listed().contains(*bank));
        self.source_cursor = self.source_row();
    }

    /// Restores the group the list was narrowed to; one the configuration
    /// does not hold is dropped. [`Shell::set_groups`] must have run first,
    /// and the interface then gives the group's banks.
    pub fn restore_group(&mut self, group: Option<u8>) {
        self.group = group.filter(|group| *group < self.group_count);
    }

    /// Takes the number of groups a configuration holds; a group past them is
    /// left.
    pub fn set_groups(&mut self, count: u8) {
        self.group_count = count;
        if self.group.is_some_and(|group| group >= count) {
            self.group = None;
        }
    }

    /// Takes the banks of the group the list is narrowed to, as read from the
    /// image. A bank filter outside them is cleared, so the view is always in
    /// the group; a group with none is left.
    pub fn set_group_banks(&mut self, banks: BankSet) {
        self.group_banks = banks;
        if banks.is_empty() {
            self.group = None;
        }
        if self
            .bank_filter
            .is_some_and(|bank| !self.listed().contains(bank))
        {
            self.bank_filter = None;
        }
        self.source_cursor = self.source_row();
    }

    /// Restores the slot that was not selected, and which slot was.
    ///
    /// Checked as [`Shell::restore_source`] and [`Shell::restore_vfo`] check
    /// theirs; [`Shell::set_banks`] must have run first.
    pub fn restore_other(&mut self, active: SlotId, other: Slot) {
        self.active = active;
        self.other.mode = other.mode;
        self.other.bank_filter = other.bank_filter.filter(|bank| self.banks.contains(*bank));
        if (VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(&other.vfo_hz) {
            self.other.vfo_hz = other.vfo_hz;
        }
        if other.step_index < VFO_STEPS_HZ.len() {
            self.other.step_index = other.step_index;
        }
        if !self.capabilities.vfo && self.other.mode == Mode::Vfo {
            self.other.mode = Mode::Memory;
        }
    }

    /// Restores the VFO frequency and tuning step the operator last used.
    ///
    /// Both are checked rather than trusted: this comes from external memory,
    /// and a record from another image or a damaged one must leave the radio on
    /// its defaults instead of on a frequency it cannot represent.
    pub fn restore_vfo(&mut self, vfo_hz: u32, step_index: usize) {
        if (VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(&vfo_hz) {
            self.vfo_hz = vfo_hz;
        }
        if step_index < VFO_STEPS_HZ.len() {
            self.step_index = step_index;
            self.step_cursor = step_index;
        }
    }

    /// Replaces the selectable banks after a configuration is programmed.
    ///
    /// A filter which the new configuration does not populate is cleared, so
    /// the operator can never be left looking at an empty view.
    pub fn set_banks(&mut self, banks: BankSet) -> bool {
        self.banks = banks;
        self.entry = None;
        if self
            .other
            .bank_filter
            .is_some_and(|filter| !self.banks.contains(filter))
        {
            self.other.bank_filter = None;
        }
        if self
            .bank_filter
            .is_some_and(|filter| !self.banks.contains(filter))
        {
            self.bank_filter = None;
            self.source_cursor = self.source_row();
            return true;
        }
        self.source_cursor = self.source_row();
        false
    }

    /// Returns the source-list row the source in use occupies.
    fn source_row(&self) -> usize {
        if self.capabilities.vfo && matches!(self.mode, Mode::Vfo) {
            return 0;
        }
        let offset = usize::from(self.capabilities.vfo);
        self.bank_filter
            .and_then(|filter| self.listed().position(filter))
            .map_or(offset, |position| position + offset + 1)
    }

    /// Applies one debounced key press.
    ///
    /// A running scan is what the operator stops first: while one is scanning
    /// every key stops it and does nothing else, so there is no key which both
    /// abandons the scan and acts on the channel it happened to be sitting on.
    pub fn press(&mut self, key: Key, now_ms: u32, context: Context) -> Intent {
        if key != Key::Star {
            // A second key press supersedes an unreleased star, and the
            // debouncer reports it without a release edge in between.
            self.star_pending = false;
        }
        if context.scanning {
            self.entry = None;
            self.star_pending = false;
            self.function = false;
            return Intent::StopScan;
        }
        if self.function && key != Key::Function {
            return self.function_key(key);
        }
        match key {
            // Menu opens settings from the operating screen, as on the stock
            // and Armel firmware, so side key one opens the channel or step
            // list that Menu used to.
            Key::Side1 if self.screen == Screen::Operating => {
                self.entry = None;
                Intent::ToggleRxTone
            }
            Key::Side2 => Intent::ToggleMonitor,
            // Receive-only: the image constructs no transmit path, so the
            // push-to-talk input cannot reach the radio.
            // F gives the next key the meaning printed beneath it, as on the
            // stock and Armel firmware. Pressed twice it opens the Info screen,
            // and on that screen it closes it again.
            Key::Function => self.press_function(),
            // The star key does two things, so it commits on release rather
            // than on the way down: a tap opens the source list and a hold
            // scans it. Deciding on the press would either open a list the
            // hold then has to close again, or lose the tap altogether.
            Key::Star if self.capabilities.sources || self.can_scan() => {
                if self.can_scan() {
                    self.entry = None;
                    self.star_pending = true;
                    Intent::Idle
                } else {
                    self.open_sources()
                }
            }
            Key::Menu => self.confirm(context),
            Key::Exit => self.cancel(),
            Key::Up => self.step(Direction::Up),
            Key::Down => self.step(Direction::Down),
            Key::Digit(0) => self.digit(0, now_ms, context),
            Key::Digit(1) => self.digit(1, now_ms, context),
            Key::Digit(2) => self.digit(2, now_ms, context),
            Key::Digit(3) => self.digit(3, now_ms, context),
            Key::Digit(4) => self.digit(4, now_ms, context),
            Key::Digit(5) => self.digit(5, now_ms, context),
            Key::Digit(6) => self.digit(6, now_ms, context),
            Key::Digit(7) => self.digit(7, now_ms, context),
            Key::Digit(8) => self.digit(8, now_ms, context),
            Key::Digit(9) => self.digit(9, now_ms, context),
            Key::Ptt | Key::Side1 | Key::Star | Key::Digit(_) => Intent::Idle,
        }
    }

    /// Arms F on the operating screen, or opens and closes Info.
    fn press_function(&mut self) -> Intent {
        match self.screen {
            Screen::Info => {
                self.screen = Screen::Operating;
                Intent::Redraw
            }
            Screen::Operating if self.function => {
                self.function = false;
                if self.capabilities.info {
                    self.screen = Screen::Info;
                }
                Intent::Redraw
            }
            Screen::Operating => {
                self.entry = None;
                self.function = true;
                Intent::Redraw
            }
            _ => Intent::Idle,
        }
    }

    /// Applies the key pressed after F. A key with no F meaning only clears F.
    fn function_key(&mut self, key: Key) -> Intent {
        self.function = false;
        self.star_pending = false;
        match key {
            Key::Digit(2) => {
                self.swap_slots();
                Intent::SwapSlot
            }
            Key::Digit(3) if self.capabilities.vfo => {
                self.mode = match self.mode {
                    Mode::Memory => Mode::Vfo,
                    Mode::Vfo => Mode::Memory,
                };
                self.entry = None;
                self.source_cursor = self.source_row();
                Intent::ToggleVfo
            }
            Key::Digit(9) if matches!(self.mode, Mode::Memory) => {
                self.entry = None;
                Intent::SelectCalling
            }
            Key::Digit(4) => {
                self.entry = None;
                self.screen = Screen::Catch;
                Intent::StartCatch
            }
            _ => Intent::Redraw,
        }
    }

    /// Applies one key which has now been held past [`HOLD_MILLISECONDS`].
    ///
    /// Holding star starts a scan of whatever the operator is already listening
    /// to: every channel of the active bank, or every programmed channel when
    /// no bank filters the view.
    pub fn hold(&mut self, key: Key, context: Context) -> Intent {
        if key != Key::Star || !self.star_pending || context.scanning {
            return Intent::Idle;
        }
        self.star_pending = false;
        if !self.can_scan() {
            return Intent::Idle;
        }
        Intent::StartScan
    }

    /// Applies one debounced key release.
    ///
    /// Only a star press held for less than [`HOLD_MILLISECONDS`] reaches this
    /// with anything to do; every other key has already acted on the way down.
    pub fn release(&mut self, key: Key) -> Intent {
        if key != Key::Star || !self.star_pending {
            return Intent::Idle;
        }
        self.star_pending = false;
        self.open_sources()
    }

    /// Reports whether a hold here would start a scan.
    ///
    /// Only the operating screen scans, and only over programmed channels: the
    /// VFO is one frequency and a list screen is a choice the operator is in
    /// the middle of making.
    const fn can_scan(&self) -> bool {
        self.capabilities.scan
            && matches!(self.screen, Screen::Operating)
            && matches!(self.mode, Mode::Memory)
    }

    /// Commits or discards a timed-out channel-number entry.
    pub fn tick(&mut self, now_ms: u32, context: Context) -> Intent {
        let Some(entry) = self.entry else {
            return Intent::Idle;
        };
        if now_ms.wrapping_sub(entry.started_ms) < ENTRY_TIMEOUT_MILLISECONDS {
            return Intent::Idle;
        }
        self.commit_entry(entry, context)
    }

    fn step(&mut self, direction: Direction) -> Intent {
        self.entry = None;
        match self.screen {
            Screen::Operating => match self.mode {
                // In the VFO the operating screen tunes by one step, which is
                // what Up and Down mean when there is no channel list to walk.
                // This is the one place Up means "larger": a frequency is not a
                // list, and every radio tunes upwards on the up key.
                Mode::Vfo => self.tune(matches!(direction, Direction::Up)),
                // Positions count downwards on the channel list, so Up moves
                // towards position one exactly as it does with the list open.
                Mode::Memory => match direction {
                    Direction::Up => Intent::SelectPrevious,
                    Direction::Down => Intent::SelectNext,
                },
            },
            Screen::SourceList => self.step_row(direction, self.source_rows(), Cursor::Source),
            Screen::StepList => self.step_row(direction, VFO_STEPS_HZ.len(), Cursor::Step),
            Screen::Settings => self.step_row(direction, self.settings().len(), Cursor::Settings),
            Screen::SquelchList => {
                self.step_row(direction, usize::from(SQUELCH_LEVELS), Cursor::Squelch)
            }
            Screen::TxPowerList => {
                self.step_row(direction, usize::from(TX_POWER_ROWS), Cursor::TxPower)
            }
            Screen::TxTimeoutList => self.step_row(
                direction,
                usize::from(TX_TIMEOUT_CHOICES),
                Cursor::TxTimeout,
            ),
            Screen::TxClassList => self.step_row(direction, self.tx_class_rows(), Cursor::TxClass),
            Screen::Info => Intent::Redraw,
            Screen::Catch => Intent::Idle,
        }
    }

    /// Moves the cursor of one bounded list, wrapping at both ends.
    ///
    /// Rows are drawn top to bottom in index order, so Up decrements: the
    /// highlight moves the way the key points.
    fn step_row(&mut self, direction: Direction, rows: usize, which: Cursor) -> Intent {
        let last = rows.saturating_sub(1);
        let mut squelch_cursor = usize::from(self.squelch_cursor);
        let mut tx_power_cursor = usize::from(self.tx_power_cursor);
        let mut tx_timeout_cursor = usize::from(self.tx_timeout_cursor);
        let mut tx_class_cursor = usize::from(self.tx_class_cursor);
        let cursor = match which {
            Cursor::Source => &mut self.source_cursor,
            Cursor::Step => &mut self.step_cursor,
            Cursor::Settings => &mut self.settings_cursor,
            Cursor::Squelch => &mut squelch_cursor,
            Cursor::TxPower => &mut tx_power_cursor,
            Cursor::TxTimeout => &mut tx_timeout_cursor,
            Cursor::TxClass => &mut tx_class_cursor,
        };
        *cursor = match direction {
            Direction::Up => {
                if *cursor == 0 {
                    last
                } else {
                    *cursor - 1
                }
            }
            Direction::Down => {
                if *cursor >= last {
                    0
                } else {
                    *cursor + 1
                }
            }
        };
        if matches!(which, Cursor::Squelch) {
            self.squelch_cursor = u8::try_from(squelch_cursor).unwrap_or(0);
        }
        if matches!(which, Cursor::TxPower) {
            self.tx_power_cursor = u8::try_from(tx_power_cursor).unwrap_or(0);
        }
        if matches!(which, Cursor::TxTimeout) {
            self.tx_timeout_cursor = u8::try_from(tx_timeout_cursor).unwrap_or(0);
        }
        if matches!(which, Cursor::TxClass) {
            self.tx_class_cursor = u8::try_from(tx_class_cursor).unwrap_or(0);
        }
        Intent::Redraw
    }

    /// Tunes the VFO by one step, refusing to leave the representable range.
    fn tune(&mut self, upwards: bool) -> Intent {
        let step = self.vfo_step_hz();
        let tuned = if upwards {
            self.vfo_hz.saturating_add(step)
        } else {
            self.vfo_hz.saturating_sub(step)
        };
        if !(VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(&tuned) {
            return Intent::Redraw;
        }
        self.vfo_hz = tuned;
        Intent::TuneVfo
    }

    fn confirm(&mut self, context: Context) -> Intent {
        if let Some(entry) = self.entry.take() {
            return self.commit_entry(entry, context);
        }
        match self.screen {
            Screen::SourceList => self.commit_source(),
            Screen::StepList => {
                self.screen = Screen::Operating;
                self.step_index = self.step_cursor;
                Intent::Redraw
            }
            Screen::Settings => {
                match self.settings().get(self.settings_cursor) {
                    Some(Setting::Squelch) => {
                        self.screen = Screen::SquelchList;
                        self.squelch_cursor = self.squelch.get();
                    }
                    Some(Setting::TxPower) => {
                        self.screen = Screen::TxPowerList;
                        self.tx_power_cursor = self.tx_override.map_or(0, |level| level + 1);
                    }
                    Some(Setting::TxTimeout) => {
                        self.screen = Screen::TxTimeoutList;
                        self.tx_timeout_cursor = self.tx_timeout;
                    }
                    Some(Setting::TxClasses) => {
                        self.screen = Screen::TxClassList;
                        self.tx_class_cursor = 0;
                    }
                    // Menu switches it and shows the operating screen, where
                    // both slots are.
                    Some(Setting::DualWatch) => {
                        self.dual_watch = !self.dual_watch;
                        self.screen = Screen::Operating;
                    }
                    // As dual watch: the operating screen is where the
                    // heard tone shows.
                    Some(Setting::VfoStep) => {
                        self.screen = Screen::StepList;
                        self.step_cursor = self.step_index;
                    }
                    Some(Setting::ToneHeard) => {
                        self.tone_heard = !self.tone_heard;
                        self.screen = Screen::Operating;
                    }
                    // A menu row with nothing behind it cannot open a screen the
                    // operator would then be stuck on.
                    None => self.screen = Screen::Operating,
                }
                Intent::Redraw
            }
            Screen::SquelchList => {
                // Back to the operating screen rather than the menu: the point
                // of changing squelch is to hear what it did.
                self.screen = Screen::Operating;
                let Ok(level) = SquelchLevel::new(self.squelch_cursor) else {
                    return Intent::Redraw;
                };
                if level == self.squelch {
                    return Intent::Redraw;
                }
                Intent::SetSquelch(level)
            }
            // TX power and timeout are held by the shell and read by the target
            // before each transmission, so choosing one needs no target
            // acknowledgement.
            Screen::TxPowerList => {
                self.screen = Screen::Operating;
                if self.tx_power_cursor < TX_POWER_ROWS {
                    self.tx_override = self.tx_power_cursor.checked_sub(1);
                }
                Intent::Redraw
            }
            Screen::TxTimeoutList => {
                self.screen = Screen::Operating;
                if self.tx_timeout_cursor < TX_TIMEOUT_CHOICES {
                    self.tx_timeout = self.tx_timeout_cursor;
                }
                Intent::Redraw
            }
            // Menu switches the class under the cursor and stays on the list,
            // so several can be changed at once. The interface applies the
            // switches to the programmed policy on every pass and keeps them in
            // the radio's own record, so they outlast a power cycle.
            Screen::TxClassList => {
                let (list, _) = self.tx_class_list();
                if let Some(Some(class)) = list.get(usize::from(self.tx_class_cursor)).copied() {
                    let off = self.tx_disabled.contains(class);
                    self.tx_disabled = self.tx_disabled.with(class, !off);
                }
                Intent::Redraw
            }
            Screen::Operating => self.open_settings(),
            Screen::Info => {
                self.screen = Screen::Operating;
                Intent::Redraw
            }
            // The target knows what was caught; it calls `catch_to_vfo`.
            Screen::Catch => Intent::UseCatch,
        }
    }

    /// Commits a squelch choice after the receive application and target
    /// adapter have acknowledged the physical threshold change.
    pub fn accept_squelch(&mut self, level: SquelchLevel) -> bool {
        if self.squelch_cursor != level.get() || level == self.squelch {
            return false;
        }
        self.squelch = level;
        true
    }

    /// Discards a rejected or unavailable squelch choice.
    #[inline(never)]
    pub fn reject_squelch(&mut self) {
        self.squelch_cursor = self.squelch.get();
    }

    #[inline(never)]
    fn cancel(&mut self) -> Intent {
        if self.entry.take().is_some() {
            return Intent::Redraw;
        }
        if self.screen == Screen::Operating {
            return Intent::Idle;
        }
        if self.screen == Screen::Catch {
            self.screen = Screen::Operating;
            return Intent::StopCatch;
        }
        // Exit unwinds one step, so a value list returns to the menu that opened
        // it rather than throwing the operator all the way out.
        self.screen = if matches!(
            self.screen,
            Screen::SquelchList
                | Screen::TxPowerList
                | Screen::TxTimeoutList
                | Screen::TxClassList
                | Screen::StepList
        ) {
            Screen::Settings
        } else {
            Screen::Operating
        };
        Intent::Redraw
    }

    fn digit(&mut self, digit: u32, now_ms: u32, context: Context) -> Intent {
        // On a settings screen a digit is a choice, not the start of a channel
        // number or a frequency. The squelch levels are exactly the ten digits,
        // so typing one picks it outright.
        match self.screen {
            Screen::SquelchList => {
                self.squelch_cursor = u8::try_from(digit).unwrap_or(0);
                return self.confirm(context);
            }
            // Power levels are numbered from one on screen; 0 is CHANNEL.
            Screen::TxPowerList => {
                let Some(row) = u8::try_from(digit).ok().filter(|row| *row < TX_POWER_ROWS) else {
                    return Intent::Idle;
                };
                self.tx_power_cursor = row;
                return self.confirm(context);
            }
            // Timeouts are whole minutes, so the digit is the minutes.
            Screen::TxTimeoutList => {
                let Some(choice) = u8::try_from(digit)
                    .ok()
                    .and_then(|digit| digit.checked_sub(1))
                    .filter(|choice| *choice < TX_TIMEOUT_CHOICES)
                else {
                    return Intent::Idle;
                };
                self.tx_timeout_cursor = choice;
                return self.confirm(context);
            }
            Screen::Settings | Screen::TxClassList | Screen::Catch => return Intent::Idle,
            _ => {}
        }
        let mut entry = self.entry.unwrap_or(Entry {
            digits: 0,
            value: 0,
            started_ms: now_ms,
        });
        entry.value = entry.value * 10 + digit;
        entry.digits += 1;
        entry.started_ms = now_ms;
        if usize::from(entry.digits) >= self.entry_digits() {
            self.entry = None;
            return self.commit_entry(entry, context);
        }
        self.entry = Some(entry);
        Intent::Redraw
    }

    /// Returns how many digits the current mode's entry accepts.
    const fn entry_digits(&self) -> usize {
        match self.mode {
            Mode::Memory => ENTRY_DIGITS,
            Mode::Vfo => VFO_ENTRY_DIGITS,
        }
    }

    /// Resolves a completed entry into a selection or a VFO frequency.
    ///
    /// In memory mode the typed number is the one-based position shown on the
    /// operating screen, so what the operator reads is what the operator can
    /// type. In the VFO it is a frequency in whole kilohertz. Either way a value
    /// outside range is discarded rather than clamped onto something the
    /// operator did not ask for.
    fn commit_entry(&mut self, entry: Entry, context: Context) -> Intent {
        self.entry = None;
        self.screen = Screen::Operating;
        match self.mode {
            Mode::Vfo => {
                // Digits fill from the megahertz side, so "145" and a pause is
                // 145.000 MHz rather than 145 kHz: what the operator typed is
                // what the screen already showed them.
                let missing = VFO_ENTRY_DIGITS.saturating_sub(usize::from(entry.digits));
                let Some(scale) = 10_u32.checked_pow(u32::try_from(missing).unwrap_or(0)) else {
                    return Intent::Redraw;
                };
                let Some(hertz) = entry
                    .value
                    .checked_mul(scale)
                    .and_then(|kilohertz| kilohertz.checked_mul(1_000))
                else {
                    return Intent::Redraw;
                };
                if !(VFO_MINIMUM_HZ..=VFO_MAXIMUM_HZ).contains(&hertz) {
                    return Intent::Redraw;
                }
                self.vfo_hz = hertz;
                Intent::TuneVfo
            }
            Mode::Memory => {
                let position = u16::try_from(entry.value).unwrap_or(u16::MAX);
                if position == 0 || position > context.visible_channels {
                    return Intent::Redraw;
                }
                Intent::SelectIndex(position - 1)
            }
        }
    }

    /// Opens the settings menu, or closes it if it is already open.
    ///
    /// The menu opens on its first row every time. It is short and the operator
    /// reached it from a dedicated key, so remembering where they were last
    /// would hide the rest of it rather than save them a press.
    fn open_settings(&mut self) -> Intent {
        if !self.capabilities.settings {
            return Intent::Idle;
        }
        self.entry = None;
        if matches!(
            self.screen,
            Screen::Settings
                | Screen::SquelchList
                | Screen::TxPowerList
                | Screen::TxTimeoutList
                | Screen::TxClassList
        ) {
            self.screen = Screen::Operating;
            return Intent::Redraw;
        }
        self.screen = Screen::Settings;
        self.settings_cursor = 0;
        Intent::Redraw
    }

    /// Opens the source list, or closes it if it is already open.
    ///
    /// The list opens on the source in use, so the operator can see whether the
    /// radio is on the VFO, on every channel, or on one bank.
    fn open_sources(&mut self) -> Intent {
        if !self.capabilities.sources {
            return Intent::Idle;
        }
        self.entry = None;
        if self.screen == Screen::SourceList {
            self.screen = Screen::Operating;
            return Intent::Redraw;
        }
        self.screen = Screen::SourceList;
        self.source_cursor = self.source_row();
        Intent::Redraw
    }

    /// Applies the source the cursor names and returns to the operating screen.
    fn commit_source(&mut self) -> Intent {
        self.screen = Screen::Operating;
        let Some(selected) = self.source_at(self.source_cursor) else {
            return Intent::Redraw;
        };
        if self.is_active_source(self.source_cursor) {
            return Intent::Redraw;
        }
        match selected {
            Source::Vfo => self.mode = Mode::Vfo,
            Source::AllChannels => {
                self.mode = Mode::Memory;
                self.bank_filter = None;
            }
            Source::Bank(bank) => {
                self.mode = Mode::Memory;
                self.bank_filter = Some(bank);
            }
            // Narrowing to a group selects all of it; the interface reads
            // its banks before rebuilding the view.
            Source::Group(group) => {
                self.group = group;
                self.mode = Mode::Memory;
                self.bank_filter = None;
            }
        }
        Intent::SetSource(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Capabilities, Context, Intent, Mode, Screen, Setting, Shell, SlotId, Source,
        ENTRY_TIMEOUT_MILLISECONDS, SETTINGS, SETTINGS_WITH_TX, VFO_DEFAULT_HZ, VFO_MAXIMUM_HZ,
        VFO_STEPS_HZ,
    };
    use crate::receive_app::Key;
    use radio_channel_plan::BankSet;
    use radio_domain::{BankId, SquelchLevel};

    fn context(visible: u16, active: u16) -> Context {
        Context {
            visible_channels: visible,
            active_index: active,
            scanning: false,
        }
    }

    /// Presses and releases star before the hold deadline.
    ///
    /// Star commits on release where a hold would scan instead, so a test which
    /// means "the operator tapped star" has to release it. Whichever half of
    /// the tap acted is what it returns.
    fn tap_star(shell: &mut Shell, now_ms: u32, context: Context) -> Intent {
        match shell.press(Key::Star, now_ms, context) {
            Intent::Idle => shell.release(Key::Star),
            acted => acted,
        }
    }

    fn scanning(visible: u16, active: u16) -> Context {
        Context {
            scanning: true,
            ..context(visible, active)
        }
    }

    fn bank_table(ids: &[u16]) -> BankSet {
        ids.iter()
            .fold(BankSet::EMPTY, |banks, id| banks.with(BankId::new(*id)))
    }

    /// Opens the VFO step list the way an operator does: Menu, down to the
    /// VFO STEP row, Menu.
    fn open_step_list(shell: &mut Shell, at: u32) {
        shell.press(Key::Menu, at, context(0, 0));
        let row = shell
            .settings()
            .iter()
            .position(|setting| *setting == Setting::VfoStep)
            .expect("VFO step row");
        let mut time = at;
        while shell.settings_cursor() != row {
            time += 1;
            shell.press(Key::Down, time, context(0, 0));
        }
        shell.press(Key::Menu, time + 1, context(0, 0));
        assert_eq!(shell.screen(), Screen::StepList);
    }

    /// Opens the source list and walks the cursor to `row`, then picks it.
    fn choose_source(shell: &mut Shell, row: usize, at: u32) -> Intent {
        tap_star(shell, at, context(8, 0));
        let mut time = at + 1;
        while shell.source_cursor() != row {
            shell.press(Key::Down, time, context(8, 0));
            time += 1;
        }
        shell.press(Key::Menu, time, context(8, 0))
    }

    #[test]
    fn a_group_narrows_the_source_list_to_its_banks() {
        let mut shell = memory_shell(&[1, 3, 5]);
        shell.set_groups(2);
        // VFO, all channels, three banks, every bank, two groups.
        assert_eq!(shell.source_rows(), 8);
        assert_eq!(shell.source_at(5), Some(Source::Group(None)));
        assert_eq!(shell.source_at(6), Some(Source::Group(Some(0))));
        assert_eq!(shell.source_at(7), Some(Source::Group(Some(1))));
        assert_eq!(shell.source_at(8), None);
        assert!(shell.is_active_source(5), "no group is in force");

        assert_eq!(
            choose_source(&mut shell, 7, 10),
            Intent::SetSource(Source::Group(Some(1)))
        );
        assert_eq!(shell.group(), Some(1));
        assert_eq!(shell.bank_filter(), None, "all of the group");
        // The interface reads the group's banks: 3 and 5.
        shell.set_group_banks(bank_table(&[3, 5]));
        assert_eq!(shell.source_rows(), 2 + 2 + 3);
        assert_eq!(shell.source_at(2), Some(Source::Bank(BankId::new(3))));
        assert_eq!(shell.source_at(3), Some(Source::Bank(BankId::new(5))));
        assert_eq!(shell.source_at(4), Some(Source::Group(None)));
        assert!(shell.is_active_source(1), "ALL is the group's");
        assert!(shell.is_active_source(6));

        // A bank in the group filters as before; one outside it is dropped
        // when the group's banks arrive.
        choose_source(&mut shell, 3, 40);
        assert_eq!(shell.bank_filter(), Some(BankId::new(5)));
        shell.set_group_banks(bank_table(&[3]));
        assert_eq!(shell.bank_filter(), None);

        // Every bank again.
        assert_eq!(
            choose_source(&mut shell, 3, 80),
            Intent::SetSource(Source::Group(None))
        );
        assert_eq!(shell.group(), None);
        assert_eq!(shell.source_rows(), 8);
    }

    #[test]
    fn a_group_past_the_configuration_is_left() {
        let mut shell = memory_shell(&[1]);
        shell.set_groups(3);
        shell.restore_group(Some(2));
        assert_eq!(shell.group(), Some(2));
        shell.set_groups(2);
        assert_eq!(shell.group(), None);
        shell.restore_group(Some(5));
        assert_eq!(shell.group(), None);
        // A group with no bank the image holds is left too.
        shell.restore_group(Some(1));
        shell.set_group_banks(BankSet::EMPTY);
        assert_eq!(shell.group(), None);
        // Without groups the list has no group rows.
        shell.set_groups(0);
        assert_eq!(shell.source_rows(), 3);
        assert_eq!(shell.source_at(3), None);
    }

    /// Returns a shell listening to every programmed channel, as an operator
    /// with a programmed radio would leave it.
    fn memory_shell(ids: &[u16]) -> Shell {
        let mut shell = Shell::new();
        shell.set_banks(bank_table(ids));
        tap_star(&mut shell, 0, context(8, 0));
        shell.press(Key::Down, 1, context(8, 0));
        assert_eq!(
            shell.press(Key::Menu, 2, context(8, 0)),
            Intent::SetSource(Source::AllChannels)
        );
        assert_eq!(shell.mode(), Mode::Memory);
        shell
    }

    #[test]
    fn a_radio_starts_in_the_vfo_because_nothing_is_programmed_yet() {
        let shell = Shell::new();
        assert_eq!(shell.mode(), Mode::Vfo);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.vfo_hz(), VFO_DEFAULT_HZ);
        assert_eq!(shell.vfo_step_hz(), 12_500);
    }

    #[test]
    fn the_vfo_tunes_by_its_step_and_stops_at_the_representable_edge() {
        let mut shell = Shell::new();
        assert_eq!(shell.press(Key::Up, 0, context(0, 0)), Intent::TuneVfo);
        assert_eq!(shell.vfo_hz(), 145_512_500);
        assert_eq!(shell.press(Key::Down, 10, context(0, 0)), Intent::TuneVfo);
        assert_eq!(shell.vfo_hz(), VFO_DEFAULT_HZ);

        // The top of the range is a representation bound, not a claim about what
        // the radio can hear, and tuning past it changes nothing.
        shell.press(Key::Digit(9), 20, context(0, 0));
        shell.press(Key::Digit(9), 21, context(0, 0));
        shell.press(Key::Digit(9), 22, context(0, 0));
        shell.press(Key::Digit(9), 23, context(0, 0));
        shell.press(Key::Digit(9), 24, context(0, 0));
        assert_eq!(
            shell.press(Key::Digit(9), 25, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), VFO_MAXIMUM_HZ);
        assert_eq!(shell.press(Key::Up, 30, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.vfo_hz(), VFO_MAXIMUM_HZ);
    }

    #[test]
    fn a_typed_vfo_frequency_is_kilohertz_and_commits_on_the_sixth_digit() {
        let mut shell = Shell::new();
        for (index, digit) in [
            Key::Digit(4),
            Key::Digit(3),
            Key::Digit(3),
            Key::Digit(5),
            Key::Digit(0),
        ]
        .into_iter()
        .enumerate()
        {
            let elapsed = u32::try_from(index).unwrap_or(0) * 10;
            assert_eq!(shell.press(digit, elapsed, context(0, 0)), Intent::Redraw);
        }
        assert_eq!(shell.entry(), Some(43_350));
        assert_eq!(
            shell.press(Key::Digit(0), 100, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 433_500_000);
        assert_eq!(shell.entry(), None);

        // Digits fill from the megahertz side, so a partial entry needs no
        // padding: "145" and a pause is 145.000 MHz.
        shell.press(Key::Digit(1), 200, context(0, 0));
        shell.press(Key::Digit(4), 210, context(0, 0));
        shell.press(Key::Digit(5), 220, context(0, 0));
        assert_eq!(
            shell.tick(220 + ENTRY_TIMEOUT_MILLISECONDS, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 145_000_000);

        // Four digits are the same rule: 433.5 MHz.
        shell.press(Key::Digit(4), 300, context(0, 0));
        shell.press(Key::Digit(3), 310, context(0, 0));
        shell.press(Key::Digit(3), 320, context(0, 0));
        shell.press(Key::Digit(5), 330, context(0, 0));
        assert_eq!(
            shell.tick(330 + ENTRY_TIMEOUT_MILLISECONDS, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 433_500_000);
    }

    #[test]
    fn an_out_of_range_vfo_frequency_is_discarded() {
        let mut shell = Shell::new();
        shell.press(Key::Digit(0), 0, context(0, 0));
        assert_eq!(
            shell.tick(ENTRY_TIMEOUT_MILLISECONDS, context(0, 0)),
            Intent::Redraw,
            "zero is not a frequency"
        );
        assert_eq!(shell.vfo_hz(), VFO_DEFAULT_HZ);
    }

    #[test]
    fn the_step_list_opens_on_the_step_in_force_and_applies_the_chosen_one() {
        let mut shell = Shell::new();
        open_step_list(&mut shell, 0);
        assert_eq!(shell.step_cursor(), shell.step_index());

        shell.press(Key::Up, 10, context(0, 0));
        assert_eq!(shell.step_cursor(), 0, "up moves towards the first row");
        assert_eq!(shell.press(Key::Menu, 20, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.vfo_step_hz(), VFO_STEPS_HZ[0]);

        // The finest step reaches the PMR446 raster from a whole megahertz.
        shell.press(Key::Digit(4), 30, context(0, 0));
        shell.press(Key::Digit(4), 31, context(0, 0));
        shell.press(Key::Digit(6), 32, context(0, 0));
        shell.press(Key::Digit(0), 33, context(0, 0));
        shell.press(Key::Digit(0), 34, context(0, 0));
        shell.press(Key::Digit(0), 35, context(0, 0));
        assert_eq!(shell.vfo_hz(), 446_000_000);
        shell.press(Key::Up, 40, context(0, 0));
        assert_eq!(shell.vfo_hz(), 446_006_250);
    }

    #[test]
    fn the_source_list_offers_the_vfo_every_channel_and_each_bank() {
        let mut shell = Shell::new();
        assert!(!shell.set_banks(bank_table(&[1, 3])));

        assert_eq!(tap_star(&mut shell, 0, context(8, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SourceList);
        assert_eq!(shell.source_rows(), 4);
        assert_eq!(shell.source_at(0), Some(Source::Vfo));
        assert_eq!(shell.source_at(1), Some(Source::AllChannels));
        assert_eq!(shell.source_at(2), Some(Source::Bank(BankId::new(1))));
        assert_eq!(shell.source_at(3), Some(Source::Bank(BankId::new(3))));
        assert_eq!(shell.source_cursor(), 0, "the VFO is the source in use");
        assert!(shell.is_active_source(0));

        shell.press(Key::Down, 10, context(8, 0));
        shell.press(Key::Down, 11, context(8, 0));
        assert_eq!(
            shell.press(Key::Menu, 20, context(8, 0)),
            Intent::SetSource(Source::Bank(BankId::new(1)))
        );
        assert_eq!(shell.mode(), Mode::Memory);
        assert_eq!(shell.bank_filter(), Some(BankId::new(1)));

        // Reopening shows the bank in force rather than the first row.
        tap_star(&mut shell, 30, context(4, 0));
        assert_eq!(shell.source_cursor(), 2);
        assert!(shell.is_active_source(2));
        assert_eq!(
            shell.press(Key::Menu, 40, context(4, 0)),
            Intent::Redraw,
            "choosing the source already in force retunes nothing"
        );

        // Returning to the VFO is one selection, not a mode key.
        tap_star(&mut shell, 50, context(4, 0));
        shell.press(Key::Up, 60, context(4, 0));
        shell.press(Key::Up, 61, context(4, 0));
        assert_eq!(
            shell.press(Key::Menu, 70, context(4, 0)),
            Intent::SetSource(Source::Vfo)
        );
        assert_eq!(shell.mode(), Mode::Vfo);
        assert_eq!(
            shell.bank_filter(),
            Some(BankId::new(1)),
            "the bank filter is kept for the next return to memory"
        );
    }

    /// Up moves towards the top of whatever is on screen.
    ///
    /// Every list this shell draws runs downwards in index order, so an up key
    /// which incremented the cursor moved the highlight the wrong way. The VFO
    /// is deliberately excluded: a frequency is not a list.
    #[test]
    fn up_moves_towards_the_first_row_on_every_list() {
        let mut shell = Shell::new();
        shell.set_banks(bank_table(&[1, 3]));

        // Source list: four rows, opening on the VFO.
        tap_star(&mut shell, 0, context(8, 0));
        assert_eq!(shell.source_cursor(), 0);
        shell.press(Key::Up, 10, context(8, 0));
        assert_eq!(shell.source_cursor(), 3, "up wraps to the last row");
        shell.press(Key::Down, 20, context(8, 0));
        assert_eq!(shell.source_cursor(), 0, "down wraps back to the first row");
        shell.press(Key::Down, 30, context(8, 0));
        assert_eq!(shell.source_cursor(), 1);
        shell.press(Key::Up, 40, context(8, 0));
        assert_eq!(shell.source_cursor(), 0);
        shell.press(Key::Exit, 50, context(8, 0));

        // Step list: opens on the step in force.
        open_step_list(&mut shell, 60);
        assert_eq!(shell.step_cursor(), 1);
        shell.press(Key::Up, 70, context(0, 0));
        assert_eq!(shell.step_cursor(), 0);
        shell.press(Key::Down, 80, context(0, 0));
        shell.press(Key::Down, 90, context(0, 0));
        assert_eq!(shell.step_cursor(), 2);
        shell.press(Key::Exit, 100, context(0, 0));
        shell.press(Key::Exit, 105, context(0, 0));

        // The VFO is the exception: up is a larger frequency, not a lower row.
        let before = shell.vfo_hz();
        assert_eq!(shell.press(Key::Up, 110, context(0, 0)), Intent::TuneVfo);
        assert!(shell.vfo_hz() > before, "up tunes upwards");
    }

    /// The operator can set squelch on the radio, not only from a host.
    #[test]
    fn the_settings_menu_changes_the_squelch_level_from_the_handset() {
        let mut shell = Shell::new();
        assert_eq!(shell.squelch(), SquelchLevel::CONSERVATIVE);

        assert_eq!(shell.press(Key::Menu, 0, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.settings_cursor(), 0);

        assert_eq!(shell.press(Key::Menu, 10, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SquelchList);
        assert_eq!(
            shell.squelch_cursor(),
            SquelchLevel::CONSERVATIVE.get(),
            "the list opens on the level in force"
        );

        shell.press(Key::Down, 20, context(0, 0));
        shell.press(Key::Down, 21, context(0, 0));
        assert_eq!(shell.squelch_cursor(), 5);
        assert_eq!(
            shell.press(Key::Menu, 30, context(0, 0)),
            Intent::SetSquelch(SquelchLevel::new(5).expect("level"))
        );
        assert_eq!(shell.squelch(), SquelchLevel::CONSERVATIVE);
        assert!(shell.accept_squelch(SquelchLevel::new(5).expect("level")));
        assert_eq!(shell.squelch(), SquelchLevel::new(5).expect("level"));
        assert_eq!(
            shell.screen(),
            Screen::Operating,
            "the operator is returned to where they can hear the effect"
        );

        // Choosing the level already in force changes nothing.
        shell.press(Key::Menu, 40, context(0, 0));
        shell.press(Key::Menu, 41, context(0, 0));
        assert_eq!(shell.press(Key::Menu, 42, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.squelch(), SquelchLevel::new(5).expect("level"));

        // A digit is the whole choice on this screen, not a channel number.
        shell.press(Key::Menu, 50, context(0, 0));
        shell.press(Key::Menu, 51, context(0, 0));
        assert_eq!(
            shell.press(Key::Digit(0), 52, context(0, 0)),
            Intent::SetSquelch(SquelchLevel::OPEN)
        );
        assert_eq!(shell.squelch(), SquelchLevel::new(5).expect("level"));
        assert!(shell.accept_squelch(SquelchLevel::OPEN));
        assert_eq!(shell.squelch(), SquelchLevel::OPEN);
        assert_eq!(shell.entry(), None, "no channel number was ever started");
    }

    #[test]
    fn the_settings_menu_unwinds_one_screen_at_a_time() {
        let mut shell = Shell::new();
        shell.press(Key::Menu, 0, context(0, 0));
        shell.press(Key::Menu, 10, context(0, 0));
        assert_eq!(shell.screen(), Screen::SquelchList);

        shell.press(Key::Down, 20, context(0, 0));
        assert_eq!(shell.press(Key::Exit, 30, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings, "exit returns to the menu");
        assert_eq!(
            shell.squelch(),
            SquelchLevel::CONSERVATIVE,
            "a cancelled choice applies nothing"
        );
        assert_eq!(shell.press(Key::Exit, 40, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);

        // Menu opens the menu, as on the stock and Armel firmware; the step
        // list is one of its rows, and Exit unwinds back to it.
        assert_eq!(shell.press(Key::Menu, 50, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.press(Key::Exit, 60, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        open_step_list(&mut shell, 70);
        assert_eq!(shell.press(Key::Exit, 100, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.press(Key::Exit, 110, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
    }

    #[test]
    fn tx_power_is_a_setting_only_where_the_target_can_transmit() {
        let mut shell = Shell::new();
        shell.set_capabilities(Capabilities {
            tx_power: false,
            ..Capabilities::K1
        });
        assert_eq!(shell.settings(), &SETTINGS[..], "no TX, no TX power row");

        shell.set_capabilities(Capabilities::K5);
        shell.select_memory();
        assert_eq!(shell.settings(), &SETTINGS_WITH_TX[..]);
        assert_eq!(
            shell.tx_override(),
            None,
            "the channel's power is the default"
        );
        assert_eq!(shell.tx_level(4), 4);
        shell.press(Key::Menu, 0, context(16, 0));
        shell.press(Key::Down, 1, context(16, 0));
        assert_eq!(shell.press(Key::Menu, 2, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::TxPowerList);
        assert_eq!(shell.tx_power_cursor(), 0, "opens on CHANNEL");
        // Rows are CHANNEL then the levels, so two down is the second level.
        shell.press(Key::Down, 3, context(16, 0));
        shell.press(Key::Down, 4, context(16, 0));
        assert_eq!(shell.press(Key::Menu, 5, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.tx_override(), Some(1));
        assert_eq!(shell.tx_level(4), 1, "the override replaces the channel's");
        assert_eq!(shell.screen(), Screen::Operating);

        // Digits pick a level numbered from one, and 0 picks CHANNEL.
        shell.press(Key::Menu, 6, context(16, 0));
        shell.press(Key::Down, 7, context(16, 0));
        shell.press(Key::Menu, 8, context(16, 0));
        assert_eq!(shell.tx_power_cursor(), 2, "opens on the override");
        assert_eq!(shell.press(Key::Digit(9), 9, context(16, 0)), Intent::Idle);
        shell.press(Key::Digit(4), 10, context(16, 0));
        assert_eq!(shell.tx_override(), Some(3));

        // Exit from the power list returns to the menu and applies nothing.
        shell.press(Key::Menu, 12, context(16, 0));
        shell.press(Key::Down, 13, context(16, 0));
        shell.press(Key::Menu, 14, context(16, 0));
        shell.press(Key::Up, 15, context(16, 0));
        shell.press(Key::Exit, 16, context(16, 0));
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.tx_override(), Some(3));

        // CHANNEL drops the override; a programmed default does not touch it.
        shell.set_tx(5, 0);
        assert_eq!(shell.tx_override(), Some(3));
        shell.press(Key::Menu, 17, context(16, 0));
        shell.press(Key::Digit(0), 18, context(16, 0));
        assert_eq!(shell.tx_override(), None);
        assert_eq!(shell.tx_level(4), 4);
    }

    /// The studio's TX power and timeout are what the menu then shows.
    #[test]
    fn a_programmed_tx_power_and_timeout_replace_what_the_menu_shows() {
        let mut shell = Shell::new();
        shell.set_capabilities(Capabilities::K5);
        shell.set_tx(2, 3);
        assert_eq!(shell.tx_power(), 2);
        assert_eq!((shell.tx_timeout(), shell.tx_timeout_cursor()), (3, 3));
        assert_eq!(shell.tx_timeout_ms(), 240_000);

        shell.set_tx(super::TX_POWER_LEVELS, super::TX_TIMEOUT_CHOICES);
        assert_eq!(shell.tx_power(), 2, "an out-of-range level is ignored");
        assert_eq!(shell.tx_timeout(), 3, "an out-of-range timeout is ignored");
    }

    /// A host write is the authority on the level the menu shows.
    #[test]
    fn a_programmed_squelch_level_replaces_what_the_menu_shows() {
        let mut shell = Shell::new();
        shell.set_squelch(SquelchLevel::new(7).expect("level"));
        assert_eq!(shell.squelch(), SquelchLevel::new(7).expect("level"));
        shell.press(Key::Menu, 0, context(0, 0));
        shell.press(Key::Menu, 10, context(0, 0));
        assert_eq!(shell.squelch_cursor(), 7);
    }

    #[test]
    fn the_source_list_closes_without_changing_the_source() {
        let mut shell = memory_shell(&[2, 5]);
        tap_star(&mut shell, 0, context(8, 0));
        shell.press(Key::Up, 10, context(8, 0));
        assert_eq!(shell.press(Key::Exit, 20, context(8, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.bank_filter(), None, "exit applies nothing");
        assert_eq!(shell.mode(), Mode::Memory);

        tap_star(&mut shell, 30, context(8, 0));
        assert_eq!(
            tap_star(&mut shell, 40, context(8, 0)),
            Intent::Redraw,
            "star closes the list it opened"
        );
        assert_eq!(shell.screen(), Screen::Operating);
    }

    #[test]
    fn an_unprogrammed_radio_offers_the_vfo_and_an_empty_memory() {
        let mut shell = Shell::new();
        tap_star(&mut shell, 0, context(0, 0));
        assert_eq!(shell.screen(), Screen::SourceList);
        assert_eq!(shell.source_rows(), 2, "the VFO and every channel");
        assert!(shell.banks().is_empty());
        shell.press(Key::Down, 10, context(0, 0));
        assert_eq!(
            shell.press(Key::Menu, 20, context(0, 0)),
            Intent::SetSource(Source::AllChannels),
            "an empty memory is selectable and simply shows nothing"
        );
        assert_eq!(shell.mode(), Mode::Memory);
    }

    #[test]
    fn the_operating_screen_steps_channels_and_side_1_switches_tone_squelch() {
        let mut shell = memory_shell(&[0]);
        // Positions are drawn downwards, so the down key walks towards the last
        // channel and the up key walks back towards the first.
        assert_eq!(
            shell.press(Key::Down, 10, context(4, 1)),
            Intent::SelectNext
        );
        assert_eq!(
            shell.press(Key::Up, 20, context(4, 1)),
            Intent::SelectPrevious
        );

        // Side key 1 no longer opens a list: it switches tone squelch, and
        // only on the operating screen.
        assert_eq!(
            shell.press(Key::Side1, 30, context(4, 1)),
            Intent::ToggleRxTone
        );
        assert_eq!(shell.screen(), Screen::Operating);
        shell.press(Key::Menu, 40, context(4, 1));
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.press(Key::Side1, 50, context(4, 1)), Intent::Idle);
        assert_eq!(shell.screen(), Screen::Settings);
    }

    #[test]
    fn the_vfo_step_list_opens_from_settings_and_exit_returns_there() {
        let mut shell = Shell::new();
        let row = SETTINGS_WITH_TX
            .iter()
            .position(|setting| *setting == Setting::VfoStep)
            .expect("VFO step row");
        shell.press(Key::Menu, 0, context(1, 0));
        for at in 0..row {
            shell.press(Key::Down, 1 + u32::try_from(at).unwrap(), context(1, 0));
        }
        shell.press(Key::Menu, 20, context(1, 0));
        assert_eq!(shell.screen(), Screen::StepList);
        assert_eq!(shell.step_cursor(), shell.step_index());
        assert_eq!(shell.press(Key::Exit, 30, context(1, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        shell.press(Key::Menu, 40, context(1, 0));
        shell.press(Key::Down, 50, context(1, 0));
        let chosen = shell.step_cursor();
        assert_eq!(shell.press(Key::Menu, 60, context(1, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.step_index(), chosen);
    }

    #[test]
    fn exit_leaves_a_screen_without_changing_the_selection() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Menu, 10, context(4, 2));
        shell.press(Key::Up, 20, context(4, 2));
        assert_eq!(shell.press(Key::Exit, 30, context(4, 2)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(
            shell.press(Key::Exit, 40, context(4, 2)),
            Intent::Idle,
            "exit on the operating screen has nothing to cancel"
        );
    }

    #[test]
    fn a_two_digit_number_selects_the_position_it_names() {
        let mut shell = memory_shell(&[0]);
        assert_eq!(
            shell.press(Key::Digit(1), 10, context(16, 0)),
            Intent::Redraw
        );
        assert_eq!(shell.entry(), Some(1));
        assert_eq!(
            shell.press(Key::Digit(2), 100, context(16, 0)),
            Intent::SelectIndex(11)
        );
        assert_eq!(shell.entry(), None);
    }

    #[test]
    fn one_digit_commits_after_the_entry_timeout() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Digit(3), 1_000, context(16, 0));
        assert_eq!(shell.tick(1_500, context(16, 0)), Intent::Idle);
        assert_eq!(
            shell.tick(1_000 + ENTRY_TIMEOUT_MILLISECONDS, context(16, 0)),
            Intent::SelectIndex(2)
        );
        assert_eq!(shell.entry(), None);
    }

    #[test]
    fn an_out_of_range_or_zero_number_selects_nothing() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Digit(9), 10, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit(9), 20, context(4, 0)),
            Intent::Redraw,
            "99 is not a channel in a four-channel view"
        );
        shell.press(Key::Digit(0), 30, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit(0), 40, context(4, 0)),
            Intent::Redraw,
            "channel zero does not exist"
        );
    }

    #[test]
    fn exit_clears_a_partial_number_before_it_can_select() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Digit(1), 10, context(16, 0));
        assert_eq!(shell.press(Key::Exit, 20, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.entry(), None);
        assert_eq!(
            shell.tick(10_000, context(16, 0)),
            Intent::Idle,
            "a cancelled entry cannot commit later"
        );
    }

    #[test]
    fn a_filter_the_new_configuration_does_not_populate_is_cleared() {
        let mut shell = Shell::new();
        shell.set_banks(bank_table(&[1, 3]));
        tap_star(&mut shell, 0, context(8, 0));
        shell.press(Key::Down, 5, context(8, 0));
        shell.press(Key::Down, 6, context(8, 0));
        shell.press(Key::Menu, 8, context(8, 0));
        assert_eq!(shell.bank_filter(), Some(BankId::new(1)));

        assert!(
            shell.set_banks(bank_table(&[5])),
            "reprogramming reports that the filter had to be cleared"
        );
        assert_eq!(shell.bank_filter(), None);
    }

    #[test]
    fn no_key_can_produce_a_transmit_intent() {
        for mut shell in [Shell::new(), memory_shell(&[1])] {
            for key in Key::ALL {
                let intent = shell.press(key, 100, context(4, 0));
                assert!(matches!(
                    intent,
                    Intent::Idle
                        | Intent::Redraw
                        | Intent::SelectNext
                        | Intent::SelectPrevious
                        | Intent::SelectIndex(_)
                        | Intent::SetSource(_)
                        | Intent::TuneVfo
                        | Intent::ToggleMonitor
                        | Intent::ToggleRxTone
                        | Intent::SetSquelch(_)
                        | Intent::StartScan
                        | Intent::StopScan
                ));
            }
        }
    }

    #[test]
    fn the_info_screen_toggles_and_leaves_the_selection_alone() {
        let mut shell = Shell::new();
        assert_eq!(shell.press(Key::Function, 0, context(4, 0)), Intent::Redraw);
        assert!(shell.function_pending());
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.press(Key::Function, 5, context(4, 0)), Intent::Redraw);
        assert!(!shell.function_pending());
        assert_eq!(shell.screen(), Screen::Info);
        assert_eq!(
            shell.press(Key::Function, 10, context(4, 0)),
            Intent::Redraw
        );
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.vfo_hz(), VFO_DEFAULT_HZ);
    }

    #[test]
    fn f_then_2_selects_the_other_slot_and_each_keeps_its_own_source() {
        let mut shell = Shell::new();
        shell.set_banks(bank_table(&[3, 7]));
        shell.select_memory();
        // A on bank 3.
        tap_star(&mut shell, 0, context(4, 0));
        shell.press(Key::Down, 1, context(4, 0));
        assert_eq!(
            shell.press(Key::Menu, 3, context(4, 0)),
            Intent::SetSource(Source::Bank(BankId::new(3)))
        );
        assert_eq!(shell.active_slot(), SlotId::A);

        assert_eq!(shell.press(Key::Function, 4, context(4, 0)), Intent::Redraw);
        assert_eq!(
            shell.press(Key::Digit(2), 5, context(4, 0)),
            Intent::SwapSlot
        );
        assert!(!shell.function_pending());
        assert_eq!(shell.active_slot(), SlotId::B);
        // B was given the channels when the radio was programmed, unfiltered.
        assert_eq!(shell.mode(), Mode::Memory);
        assert_eq!(shell.bank_filter(), None);
        assert_eq!(shell.other_slot().bank_filter, Some(BankId::new(3)));

        // B on bank 7; A keeps bank 3.
        tap_star(&mut shell, 6, context(4, 0));
        shell.press(Key::Down, 7, context(4, 0));
        shell.press(Key::Down, 8, context(4, 0));
        assert_eq!(
            shell.press(Key::Menu, 10, context(4, 0)),
            Intent::SetSource(Source::Bank(BankId::new(7)))
        );
        shell.press(Key::Function, 11, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit(2), 12, context(4, 0)),
            Intent::SwapSlot
        );
        assert_eq!(shell.active_slot(), SlotId::A);
        assert_eq!(shell.bank_filter(), Some(BankId::new(3)));
        assert_eq!(shell.other_slot().bank_filter, Some(BankId::new(7)));
    }

    #[test]
    fn f_then_9_asks_for_the_calling_channel_on_channels_only() {
        let mut shell = memory_shell(&[]);
        shell.press(Key::Function, 0, context(4, 2));
        assert_eq!(
            shell.press(Key::Digit(9), 1, context(4, 2)),
            Intent::SelectCalling
        );
        assert!(!shell.function_pending());
        assert_eq!(shell.entry(), None, "9 is not typed as a channel digit");

        // On the VFO there is no bank to have a calling channel.
        shell.press(Key::Function, 2, context(4, 2));
        shell.press(Key::Digit(3), 3, context(4, 2));
        shell.press(Key::Function, 4, context(0, 0));
        assert_eq!(shell.press(Key::Digit(9), 5, context(0, 0)), Intent::Redraw);
    }

    #[test]
    fn f_then_3_switches_the_selected_slot_between_channels_and_vfo() {
        let mut shell = memory_shell(&[]);
        assert_eq!(shell.mode(), Mode::Memory);
        let b = shell.other_slot();
        shell.press(Key::Function, 0, context(4, 2));
        assert_eq!(
            shell.press(Key::Digit(3), 1, context(4, 2)),
            Intent::ToggleVfo
        );
        assert_eq!(shell.mode(), Mode::Vfo);
        assert_eq!(shell.other_slot(), b, "B is untouched");
        shell.press(Key::Function, 2, context(0, 0));
        assert_eq!(
            shell.press(Key::Digit(3), 3, context(0, 0)),
            Intent::ToggleVfo
        );
        assert_eq!(shell.mode(), Mode::Memory);

        // Without a VFO there is nothing to switch to, and F is only cleared.
        let mut k5 = memory_shell(&[]);
        k5.set_capabilities(Capabilities {
            vfo: false,
            ..Capabilities::K5
        });
        k5.press(Key::Function, 0, context(4, 0));
        assert_eq!(k5.press(Key::Digit(3), 1, context(4, 0)), Intent::Redraw);
        assert_eq!(k5.mode(), Mode::Memory);
    }

    #[test]
    fn a_key_with_no_f_meaning_only_clears_f() {
        let mut shell = memory_shell(&[]);
        shell.press(Key::Function, 0, context(4, 1));
        assert_eq!(shell.press(Key::Up, 1, context(4, 1)), Intent::Redraw);
        assert!(!shell.function_pending());
        assert_eq!(
            shell.press(Key::Up, 2, context(4, 1)),
            Intent::SelectPrevious
        );
        // F only arms on the operating screen.
        shell.press(Key::Menu, 3, context(4, 1));
        assert_eq!(shell.press(Key::Function, 4, context(4, 1)), Intent::Idle);
        assert!(!shell.function_pending());
    }

    #[test]
    fn f_then_4_opens_the_catch_and_menu_puts_it_in_the_vfo() {
        use radio_domain::Tone;

        let mut shell = memory_shell(&[2, 5]);
        shell.press(Key::Function, 10, context(2, 0));
        assert_eq!(
            shell.press(Key::Digit(4), 11, context(2, 0)),
            Intent::StartCatch
        );
        assert_eq!(shell.screen(), Screen::Catch);
        // Nothing else acts on the catch.
        for (key, time) in [(Key::Up, 12), (Key::Down, 13), (Key::Digit(7), 14)] {
            assert_eq!(shell.press(key, time, context(2, 0)), Intent::Idle);
        }
        assert_eq!(shell.entry(), None);
        // Menu asks the target, which knows what was caught.
        assert_eq!(shell.press(Key::Menu, 15, context(2, 0)), Intent::UseCatch);
        assert_eq!(shell.screen(), Screen::Catch);
        assert!(!shell.catch_to_vfo(10_000, None), "below the VFO range");
        assert_eq!(shell.screen(), Screen::Catch);
        assert!(shell.catch_to_vfo(438_437_500, Some(Tone::Ctcss(1567))));
        assert_eq!(shell.mode(), Mode::Vfo);
        assert_eq!(shell.vfo_hz(), 438_437_500);
        assert_eq!(shell.vfo_tx_tone(), Tone::Ctcss(1567));
        assert_eq!(shell.screen(), Screen::Operating);
        // Only from the catch.
        assert!(!shell.catch_to_vfo(145_500_000, None));
        assert_eq!(shell.vfo_hz(), 438_437_500);

        // Each slot keeps its own VFO tone across A/B.
        shell.press(Key::Function, 20, context(2, 0));
        assert_eq!(
            shell.press(Key::Digit(2), 21, context(2, 0)),
            Intent::SwapSlot
        );
        assert_eq!(shell.vfo_tx_tone(), Tone::None);
        assert_eq!(shell.other_slot().vfo_tx_tone, Tone::Ctcss(1567));
        // An untoned catch clears it.
        shell.press(Key::Function, 22, context(2, 0));
        shell.press(Key::Digit(2), 23, context(2, 0));
        shell.press(Key::Function, 24, context(2, 0));
        shell.press(Key::Digit(4), 25, context(2, 0));
        assert!(shell.catch_to_vfo(145_500_000, None));
        assert_eq!(shell.vfo_tx_tone(), Tone::None);
    }

    #[test]
    fn exit_leaves_the_catch() {
        let mut shell = memory_shell(&[2, 5]);
        shell.press(Key::Function, 10, context(2, 0));
        shell.press(Key::Digit(4), 11, context(2, 0));
        assert_eq!(shell.press(Key::Exit, 12, context(2, 0)), Intent::StopCatch);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.mode(), Mode::Memory);
    }

    #[test]
    fn the_settings_menu_switches_tone_heard_off_at_start_up() {
        for settings in [&SETTINGS[..], &SETTINGS_WITH_TX[..]] {
            assert_eq!(settings.last(), Some(&Setting::ToneHeard));
        }
        let mut shell = memory_shell(&[]);
        assert!(!shell.tone_heard());
        let row = SETTINGS_WITH_TX
            .iter()
            .position(|setting| *setting == Setting::ToneHeard)
            .expect("tone heard row");
        for (time, expected) in [(0, true), (100, false)] {
            shell.press(Key::Menu, time, context(4, 0));
            for at in 0..row {
                shell.press(
                    Key::Down,
                    time + 1 + u32::try_from(at).unwrap(),
                    context(4, 0),
                );
            }
            assert_eq!(
                shell.press(Key::Menu, time + 20, context(4, 0)),
                Intent::Redraw
            );
            assert_eq!(shell.tone_heard(), expected);
            assert_eq!(shell.screen(), Screen::Operating);
        }
    }

    #[test]
    fn the_settings_menu_switches_dual_watch() {
        let mut shell = memory_shell(&[]);
        assert!(!shell.dual_watch());
        let row = SETTINGS_WITH_TX
            .iter()
            .position(|setting| *setting == Setting::DualWatch)
            .expect("dual watch row");
        shell.press(Key::Menu, 0, context(4, 0));
        for at in 0..row {
            shell.press(Key::Down, 1 + u32::try_from(at).unwrap(), context(4, 0));
        }
        assert_eq!(shell.press(Key::Menu, 20, context(4, 0)), Intent::Redraw);
        assert!(shell.dual_watch());
        assert_eq!(shell.screen(), Screen::Operating);
    }

    #[test]
    fn a_star_tap_opens_the_sources_and_a_star_hold_scans_them() {
        let mut shell = Shell::new();
        shell.select_memory();

        // The press itself commits to neither, so nothing has happened yet.
        assert_eq!(shell.press(Key::Star, 0, context(4, 0)), Intent::Idle);
        assert_eq!(shell.screen(), Screen::Operating);

        // Released early, it is a tap and opens the source list.
        assert_eq!(shell.release(Key::Star), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SourceList);
        // A second press closes the list again, and there is nothing pending to
        // turn into a scan of a screen the operator has left.
        assert_eq!(shell.press(Key::Star, 100, context(4, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.hold(Key::Star, context(4, 0)), Intent::Idle);

        // Held instead, the same press scans, and the release that follows it
        // does not then open the list underneath the running scan.
        assert_eq!(shell.press(Key::Star, 200, context(4, 0)), Intent::Idle);
        assert_eq!(shell.hold(Key::Star, context(4, 0)), Intent::StartScan);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.release(Key::Star), Intent::Idle);
        // Holding it twice starts one scan, not two.
        assert_eq!(shell.hold(Key::Star, context(4, 0)), Intent::Idle);
    }

    #[test]
    fn scanning_is_only_offered_where_there_are_channels_to_walk() {
        // The VFO is one frequency, so a hold there is not a scan.
        let mut shell = Shell::new();
        assert_eq!(shell.press(Key::Star, 0, context(1, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SourceList);
        assert_eq!(shell.hold(Key::Star, context(1, 0)), Intent::Idle);

        // Neither is a hold on a list the operator is part way through.
        let mut shell = Shell::new();
        shell.select_memory();
        assert_eq!(shell.press(Key::Menu, 0, context(4, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        shell.press(Key::Star, 10, context(4, 0));
        assert_eq!(shell.hold(Key::Star, context(4, 0)), Intent::Idle);
    }

    #[test]
    fn every_key_stops_a_running_scan_and_does_nothing_else() {
        for key in Key::ALL {
            let mut shell = Shell::new();
            shell.select_memory();
            assert_eq!(
                shell.press(key, 0, scanning(4, 2)),
                Intent::StopScan,
                "{key:?} did not stop the scan"
            );
            // The key that stopped the scan did not also change the screen out
            // from under the channel the scan settled on.
            assert_eq!(shell.screen(), Screen::Operating);
            assert_eq!(shell.hold(key, scanning(4, 2)), Intent::Idle);
        }
    }

    #[test]
    fn k5_capabilities_keep_the_shared_channel_and_info_navigation() {
        let mut shell = Shell::new();
        shell.set_capabilities(Capabilities::K5);
        shell.select_memory();
        let context = context(16, 0);

        assert_eq!(shell.mode(), Mode::Memory);
        assert_eq!(shell.source_rows(), 2);
        assert_eq!(shell.source_at(0), Some(Source::Vfo));
        assert_eq!(shell.source_at(1), Some(Source::AllChannels));
        assert_eq!(shell.press(Key::Star, 0, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SourceList);
        assert_eq!(shell.press(Key::Exit, 1, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.press(Key::Function, 2, context), Intent::Redraw);
        assert_eq!(shell.press(Key::Function, 2, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Info);
        assert_eq!(shell.press(Key::Exit, 3, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.press(Key::Menu, 4, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Settings);
        assert_eq!(shell.press(Key::Exit, 5, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.press(Key::Star, 6, context), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::SourceList);
        assert_eq!(shell.press(Key::Menu, 8, context), Intent::Redraw);
        assert_eq!(shell.press(Key::Up, 9, context), Intent::SelectPrevious);
    }

    #[test]
    fn a_half_typed_number_does_not_survive_a_scan_starting_or_stopping() {
        let mut shell = Shell::new();
        shell.select_memory();
        assert_eq!(
            shell.press(Key::Digit(1), 0, context(20, 0)),
            Intent::Redraw
        );
        assert_eq!(shell.entry(), Some(1));
        assert_eq!(shell.press(Key::Star, 10, context(20, 0)), Intent::Idle);
        assert_eq!(shell.entry(), None);

        assert_eq!(shell.hold(Key::Star, context(20, 0)), Intent::StartScan);
        assert_eq!(
            shell.press(Key::Digit(9), 20, scanning(20, 0)),
            Intent::StopScan
        );
        assert_eq!(shell.entry(), None);
    }

    #[test]
    fn a_rejected_squelch_intent_leaves_the_displayed_setting_unchanged() {
        let mut shell = Shell::new();
        shell.press(Key::Menu, 0, context(0, 0));
        shell.press(Key::Menu, 1, context(0, 0));
        shell.press(Key::Down, 2, context(0, 0));
        let level = SquelchLevel::new(4).expect("level");
        assert_eq!(shell.squelch_cursor(), level.get());
        assert_eq!(
            shell.press(Key::Menu, 3, context(0, 0)),
            Intent::SetSquelch(level)
        );
        assert_eq!(shell.squelch(), SquelchLevel::CONSERVATIVE);
        shell.reject_squelch();
        assert_eq!(shell.squelch_cursor(), SquelchLevel::CONSERVATIVE.get());
        assert_eq!(shell.squelch(), SquelchLevel::CONSERVATIVE);
    }
}
