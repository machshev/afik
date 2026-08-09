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
//! No intent can transmit. The set deliberately contains selection, bank
//! filtering, VFO tuning, and monitoring only.

use radio_domain::{BankId, SquelchLevel, MAX_SQUELCH_LEVEL};

use crate::keypad::Key;
use radio_channel_plan::MAX_BANKS;

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

/// One row of the source list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// The directly tuned VFO.
    Vfo,
    /// Every programmed channel, with no bank filter.
    AllChannels,
    /// Only the channels in one bank.
    Bank(BankId),
}

/// Which screen the operator is looking at.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screen {
    /// The active channel or VFO frequency, and the receive state.
    #[default]
    Operating,
    /// Scrollable list of the channels in the active view.
    ChannelList,
    /// The VFO and every populated bank.
    SourceList,
    /// The selectable VFO tuning steps.
    StepList,
    /// The radio-wide settings an operator can change from the handset.
    Settings,
    /// The selectable squelch levels.
    SquelchList,
    /// Image identity and storage state.
    Info,
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
}

/// Every settings row, in the order the menu lists them.
pub const SETTINGS: [Setting; 1] = [Setting::Squelch];

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
pub struct Shell {
    screen: Screen,
    mode: Mode,
    cursor: u16,
    entry: Option<Entry>,
    bank_filter: Option<BankId>,
    banks: [Option<BankId>; MAX_BANKS as usize],
    bank_count: usize,
    source_cursor: usize,
    vfo_hz: u32,
    step_index: usize,
    step_cursor: usize,
    settings_cursor: usize,
    squelch: SquelchLevel,
    squelch_cursor: u8,
    /// Whether a star press is still waiting to become a hold or a release.
    star_pending: bool,
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
            cursor: 0,
            entry: None,
            bank_filter: None,
            banks: [None; MAX_BANKS as usize],
            bank_count: 0,
            source_cursor: 0,
            vfo_hz: VFO_DEFAULT_HZ,
            step_index: 1,
            step_cursor: 1,
            settings_cursor: 0,
            squelch: SquelchLevel::CONSERVATIVE,
            squelch_cursor: SquelchLevel::CONSERVATIVE.get(),
            star_pending: false,
        }
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

    /// Returns the VFO frequency in hertz.
    #[must_use]
    pub const fn vfo_hz(&self) -> u32 {
        self.vfo_hz
    }

    /// Returns the VFO tuning step in hertz.
    #[must_use]
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

    /// Returns the channel-list cursor position.
    #[must_use]
    pub const fn cursor(&self) -> u16 {
        self.cursor
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

    /// Returns the populated banks in identifier order.
    #[must_use]
    pub fn banks(&self) -> &[Option<BankId>] {
        &self.banks[..self.bank_count]
    }

    /// Returns the source-list cursor row.
    #[must_use]
    pub const fn source_cursor(&self) -> usize {
        self.source_cursor
    }

    /// Returns the number of source-list rows: the VFO, every channel, and banks.
    #[must_use]
    pub const fn source_rows(&self) -> usize {
        self.bank_count + 2
    }

    /// Returns the source one row selects.
    #[must_use]
    pub fn source_at(&self, row: usize) -> Option<Source> {
        match row {
            0 => Some(Source::Vfo),
            1 => Some(Source::AllChannels),
            _ => self.banks.get(row - 2).copied().flatten().map(Source::Bank),
        }
    }

    /// Reports whether one source row is the source in use.
    #[must_use]
    pub fn is_active_source(&self, row: usize) -> bool {
        match (self.source_at(row), self.mode) {
            (Some(Source::Vfo), Mode::Vfo) => true,
            (Some(Source::AllChannels), Mode::Memory) => self.bank_filter.is_none(),
            (Some(Source::Bank(bank)), Mode::Memory) => self.bank_filter == Some(bank),
            _ => false,
        }
    }

    /// Selects the programmed memory as the source.
    ///
    /// A host write is the operator asking for their channels, so the radio
    /// leaves the VFO for them rather than waiting to be told twice.
    pub fn select_memory(&mut self) {
        self.mode = Mode::Memory;
        self.entry = None;
        self.cursor = 0;
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
        self.cursor = 0;
        self.mode = if memory_mode { Mode::Memory } else { Mode::Vfo };
        self.bank_filter = bank.filter(|bank| self.banks[..self.bank_count].contains(&Some(*bank)));
        self.source_cursor = self.source_row();
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
    pub fn set_banks(&mut self, banks: [Option<BankId>; MAX_BANKS as usize], count: usize) -> bool {
        self.banks = banks;
        self.bank_count = count.min(MAX_BANKS as usize);
        self.entry = None;
        self.cursor = 0;
        if self
            .bank_filter
            .is_some_and(|filter| !self.banks[..self.bank_count].contains(&Some(filter)))
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
        if matches!(self.mode, Mode::Vfo) {
            return 0;
        }
        self.bank_filter
            .and_then(|filter| {
                self.banks[..self.bank_count]
                    .iter()
                    .position(|bank| *bank == Some(filter))
            })
            .map_or(1, |position| position + 2)
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
            return Intent::StopScan;
        }
        match key {
            // Audio is not a mode the operator has to find, so side key one
            // opens the settings menu instead of routing it.
            Key::Side1 => self.open_settings(),
            Key::Side2 => Intent::ToggleMonitor,
            // Receive-only: the image constructs no transmit path, so the
            // push-to-talk input cannot reach the radio.
            Key::Ptt => Intent::Idle,
            Key::Function => {
                self.entry = None;
                self.screen = if self.screen == Screen::Info {
                    Screen::Operating
                } else {
                    Screen::Info
                };
                Intent::Redraw
            }
            // The star key does two things, so it commits on release rather
            // than on the way down: a tap opens the source list and a hold
            // scans it. Deciding on the press would either open a list the
            // hold then has to close again, or lose the tap altogether.
            Key::Star => {
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
            Key::Up => self.step(Direction::Up, context),
            Key::Down => self.step(Direction::Down, context),
            Key::Digit0 => self.digit(0, now_ms, context),
            Key::Digit1 => self.digit(1, now_ms, context),
            Key::Digit2 => self.digit(2, now_ms, context),
            Key::Digit3 => self.digit(3, now_ms, context),
            Key::Digit4 => self.digit(4, now_ms, context),
            Key::Digit5 => self.digit(5, now_ms, context),
            Key::Digit6 => self.digit(6, now_ms, context),
            Key::Digit7 => self.digit(7, now_ms, context),
            Key::Digit8 => self.digit(8, now_ms, context),
            Key::Digit9 => self.digit(9, now_ms, context),
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
        matches!(self.screen, Screen::Operating) && matches!(self.mode, Mode::Memory)
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

    fn step(&mut self, direction: Direction, context: Context) -> Intent {
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
            Screen::ChannelList => {
                if context.visible_channels == 0 {
                    self.cursor = 0;
                    return Intent::Redraw;
                }
                let last = context.visible_channels - 1;
                self.cursor = match direction {
                    Direction::Up => {
                        if self.cursor == 0 {
                            last
                        } else {
                            self.cursor - 1
                        }
                    }
                    Direction::Down => {
                        if self.cursor >= last {
                            0
                        } else {
                            self.cursor + 1
                        }
                    }
                };
                Intent::Redraw
            }
            Screen::SourceList => self.step_row(direction, self.source_rows(), Cursor::Source),
            Screen::StepList => self.step_row(direction, VFO_STEPS_HZ.len(), Cursor::Step),
            Screen::Settings => self.step_row(direction, SETTINGS.len(), Cursor::Settings),
            Screen::SquelchList => {
                self.step_row(direction, usize::from(SQUELCH_LEVELS), Cursor::Squelch)
            }
            Screen::Info => Intent::Redraw,
        }
    }

    /// Moves the cursor of one bounded list, wrapping at both ends.
    ///
    /// Rows are drawn top to bottom in index order, so Up decrements: the
    /// highlight moves the way the key points.
    fn step_row(&mut self, direction: Direction, rows: usize, which: Cursor) -> Intent {
        let last = rows.saturating_sub(1);
        let mut squelch_cursor = usize::from(self.squelch_cursor);
        let cursor = match which {
            Cursor::Source => &mut self.source_cursor,
            Cursor::Step => &mut self.step_cursor,
            Cursor::Settings => &mut self.settings_cursor,
            Cursor::Squelch => &mut squelch_cursor,
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
                match SETTINGS.get(self.settings_cursor) {
                    Some(Setting::Squelch) => {
                        self.screen = Screen::SquelchList;
                        self.squelch_cursor = self.squelch.get();
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
                self.squelch = level;
                Intent::SetSquelch(level)
            }
            Screen::Operating => {
                // Each mode's list is the one the operator can act on: memory
                // channels in memory mode, tuning steps in the VFO.
                match self.mode {
                    Mode::Vfo => {
                        self.screen = Screen::StepList;
                        self.step_cursor = self.step_index;
                    }
                    Mode::Memory => {
                        self.screen = Screen::ChannelList;
                        self.cursor = context.active_index;
                    }
                }
                Intent::Redraw
            }
            Screen::ChannelList => {
                self.screen = Screen::Operating;
                if self.cursor < context.visible_channels {
                    Intent::SelectIndex(self.cursor)
                } else {
                    Intent::Redraw
                }
            }
            Screen::Info => {
                self.screen = Screen::Operating;
                Intent::Redraw
            }
        }
    }

    fn cancel(&mut self) -> Intent {
        if self.entry.take().is_some() {
            return Intent::Redraw;
        }
        if self.screen == Screen::Operating {
            return Intent::Idle;
        }
        // Exit unwinds one step, so a value list returns to the menu that opened
        // it rather than throwing the operator all the way out.
        self.screen = if self.screen == Screen::SquelchList {
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
            Screen::Settings => return Intent::Idle,
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
        self.entry = None;
        if matches!(self.screen, Screen::Settings | Screen::SquelchList) {
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
        }
        self.cursor = 0;
        Intent::SetSource(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Context, Intent, Mode, Screen, Shell, Source, ENTRY_TIMEOUT_MILLISECONDS, VFO_DEFAULT_HZ,
        VFO_MAXIMUM_HZ, VFO_STEPS_HZ,
    };
    use crate::keypad::Key;
    use radio_channel_plan::MAX_BANKS;
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

    fn bank_table(ids: &[u16]) -> ([Option<BankId>; MAX_BANKS as usize], usize) {
        let mut banks = [None; MAX_BANKS as usize];
        for (slot, id) in banks.iter_mut().zip(ids) {
            *slot = Some(BankId::new(*id));
        }
        (banks, ids.len())
    }

    /// Returns a shell listening to every programmed channel, as an operator
    /// with a programmed radio would leave it.
    fn memory_shell(ids: &[u16]) -> Shell {
        let mut shell = Shell::new();
        let (banks, count) = bank_table(ids);
        shell.set_banks(banks, count);
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
        shell.press(Key::Digit9, 20, context(0, 0));
        shell.press(Key::Digit9, 21, context(0, 0));
        shell.press(Key::Digit9, 22, context(0, 0));
        shell.press(Key::Digit9, 23, context(0, 0));
        shell.press(Key::Digit9, 24, context(0, 0));
        assert_eq!(shell.press(Key::Digit9, 25, context(0, 0)), Intent::TuneVfo);
        assert_eq!(shell.vfo_hz(), VFO_MAXIMUM_HZ);
        assert_eq!(shell.press(Key::Up, 30, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.vfo_hz(), VFO_MAXIMUM_HZ);
    }

    #[test]
    fn a_typed_vfo_frequency_is_kilohertz_and_commits_on_the_sixth_digit() {
        let mut shell = Shell::new();
        for (index, digit) in [
            Key::Digit4,
            Key::Digit3,
            Key::Digit3,
            Key::Digit5,
            Key::Digit0,
        ]
        .into_iter()
        .enumerate()
        {
            let elapsed = u32::try_from(index).unwrap_or(0) * 10;
            assert_eq!(shell.press(digit, elapsed, context(0, 0)), Intent::Redraw);
        }
        assert_eq!(shell.entry(), Some(43_350));
        assert_eq!(
            shell.press(Key::Digit0, 100, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 433_500_000);
        assert_eq!(shell.entry(), None);

        // Digits fill from the megahertz side, so a partial entry needs no
        // padding: "145" and a pause is 145.000 MHz.
        shell.press(Key::Digit1, 200, context(0, 0));
        shell.press(Key::Digit4, 210, context(0, 0));
        shell.press(Key::Digit5, 220, context(0, 0));
        assert_eq!(
            shell.tick(220 + ENTRY_TIMEOUT_MILLISECONDS, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 145_000_000);

        // Four digits are the same rule: 433.5 MHz.
        shell.press(Key::Digit4, 300, context(0, 0));
        shell.press(Key::Digit3, 310, context(0, 0));
        shell.press(Key::Digit3, 320, context(0, 0));
        shell.press(Key::Digit5, 330, context(0, 0));
        assert_eq!(
            shell.tick(330 + ENTRY_TIMEOUT_MILLISECONDS, context(0, 0)),
            Intent::TuneVfo
        );
        assert_eq!(shell.vfo_hz(), 433_500_000);
    }

    #[test]
    fn an_out_of_range_vfo_frequency_is_discarded() {
        let mut shell = Shell::new();
        shell.press(Key::Digit0, 0, context(0, 0));
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
        assert_eq!(shell.press(Key::Menu, 0, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::StepList);
        assert_eq!(shell.step_cursor(), shell.step_index());

        shell.press(Key::Up, 10, context(0, 0));
        assert_eq!(shell.step_cursor(), 0, "up moves towards the first row");
        assert_eq!(shell.press(Key::Menu, 20, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.vfo_step_hz(), VFO_STEPS_HZ[0]);

        // The finest step reaches the PMR446 raster from a whole megahertz.
        shell.press(Key::Digit4, 30, context(0, 0));
        shell.press(Key::Digit4, 31, context(0, 0));
        shell.press(Key::Digit6, 32, context(0, 0));
        shell.press(Key::Digit0, 33, context(0, 0));
        shell.press(Key::Digit0, 34, context(0, 0));
        shell.press(Key::Digit0, 35, context(0, 0));
        assert_eq!(shell.vfo_hz(), 446_000_000);
        shell.press(Key::Up, 40, context(0, 0));
        assert_eq!(shell.vfo_hz(), 446_006_250);
    }

    #[test]
    fn the_source_list_offers_the_vfo_every_channel_and_each_bank() {
        let mut shell = Shell::new();
        let (banks, count) = bank_table(&[1, 3]);
        assert!(!shell.set_banks(banks, count));

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
        let (banks, count) = bank_table(&[1, 3]);
        shell.set_banks(banks, count);

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
        shell.press(Key::Menu, 60, context(0, 0));
        assert_eq!(shell.screen(), Screen::StepList);
        assert_eq!(shell.step_cursor(), 1);
        shell.press(Key::Up, 70, context(0, 0));
        assert_eq!(shell.step_cursor(), 0);
        shell.press(Key::Down, 80, context(0, 0));
        shell.press(Key::Down, 90, context(0, 0));
        assert_eq!(shell.step_cursor(), 2);
        shell.press(Key::Exit, 100, context(0, 0));

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

        assert_eq!(shell.press(Key::Side1, 0, context(0, 0)), Intent::Redraw);
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
        assert_eq!(shell.squelch(), SquelchLevel::new(5).expect("level"));
        assert_eq!(
            shell.screen(),
            Screen::Operating,
            "the operator is returned to where they can hear the effect"
        );

        // Choosing the level already in force changes nothing.
        shell.press(Key::Side1, 40, context(0, 0));
        shell.press(Key::Menu, 41, context(0, 0));
        assert_eq!(shell.press(Key::Menu, 42, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.squelch(), SquelchLevel::new(5).expect("level"));

        // A digit is the whole choice on this screen, not a channel number.
        shell.press(Key::Side1, 50, context(0, 0));
        shell.press(Key::Menu, 51, context(0, 0));
        assert_eq!(
            shell.press(Key::Digit0, 52, context(0, 0)),
            Intent::SetSquelch(SquelchLevel::OPEN)
        );
        assert_eq!(shell.squelch(), SquelchLevel::OPEN);
        assert_eq!(shell.entry(), None, "no channel number was ever started");
    }

    #[test]
    fn the_settings_menu_unwinds_one_screen_at_a_time() {
        let mut shell = Shell::new();
        shell.press(Key::Side1, 0, context(0, 0));
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

        // The side key closes what it opened, from either screen.
        shell.press(Key::Side1, 50, context(0, 0));
        assert_eq!(shell.press(Key::Side1, 60, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        shell.press(Key::Side1, 70, context(0, 0));
        shell.press(Key::Menu, 71, context(0, 0));
        assert_eq!(shell.press(Key::Side1, 80, context(0, 0)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
    }

    /// A host write is the authority on the level the menu shows.
    #[test]
    fn a_programmed_squelch_level_replaces_what_the_menu_shows() {
        let mut shell = Shell::new();
        shell.set_squelch(SquelchLevel::new(7).expect("level"));
        assert_eq!(shell.squelch(), SquelchLevel::new(7).expect("level"));
        shell.press(Key::Side1, 0, context(0, 0));
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
    fn the_operating_screen_steps_channels_and_the_list_moves_a_cursor() {
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

        assert_eq!(shell.press(Key::Menu, 30, context(4, 1)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::ChannelList);
        assert_eq!(shell.cursor(), 1, "the list opens on the active channel");
        shell.press(Key::Down, 40, context(4, 1));
        shell.press(Key::Down, 50, context(4, 1));
        assert_eq!(shell.cursor(), 3);
        shell.press(Key::Down, 60, context(4, 1));
        assert_eq!(shell.cursor(), 0, "the cursor wraps at the end of the view");
        shell.press(Key::Up, 65, context(4, 1));
        assert_eq!(shell.cursor(), 3, "up wraps back to the last row");
        shell.press(Key::Up, 66, context(4, 1));
        shell.press(Key::Up, 67, context(4, 1));
        shell.press(Key::Up, 68, context(4, 1));
        assert_eq!(shell.cursor(), 0, "up walks towards the first row");
        assert_eq!(
            shell.press(Key::Menu, 70, context(4, 1)),
            Intent::SelectIndex(0)
        );
        assert_eq!(shell.screen(), Screen::Operating);
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
        assert_eq!(shell.press(Key::Digit1, 10, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.entry(), Some(1));
        assert_eq!(
            shell.press(Key::Digit2, 100, context(16, 0)),
            Intent::SelectIndex(11)
        );
        assert_eq!(shell.entry(), None);
    }

    #[test]
    fn one_digit_commits_after_the_entry_timeout() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Digit3, 1_000, context(16, 0));
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
        shell.press(Key::Digit9, 10, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit9, 20, context(4, 0)),
            Intent::Redraw,
            "99 is not a channel in a four-channel view"
        );
        shell.press(Key::Digit0, 30, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit0, 40, context(4, 0)),
            Intent::Redraw,
            "channel zero does not exist"
        );
    }

    #[test]
    fn exit_clears_a_partial_number_before_it_can_select() {
        let mut shell = memory_shell(&[0]);
        shell.press(Key::Digit1, 10, context(16, 0));
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
        let (banks, count) = bank_table(&[1, 3]);
        shell.set_banks(banks, count);
        tap_star(&mut shell, 0, context(8, 0));
        shell.press(Key::Down, 5, context(8, 0));
        shell.press(Key::Down, 6, context(8, 0));
        shell.press(Key::Menu, 8, context(8, 0));
        assert_eq!(shell.bank_filter(), Some(BankId::new(1)));

        let (banks, count) = bank_table(&[5]);
        assert!(
            shell.set_banks(banks, count),
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
        assert_eq!(shell.screen(), Screen::Info);
        assert_eq!(
            shell.press(Key::Function, 10, context(4, 0)),
            Intent::Redraw
        );
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(shell.vfo_hz(), VFO_DEFAULT_HZ);
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
        assert_eq!(shell.screen(), Screen::ChannelList);
        assert_eq!(shell.press(Key::Star, 10, context(4, 0)), Intent::Redraw);
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
    fn a_half_typed_number_does_not_survive_a_scan_starting_or_stopping() {
        let mut shell = Shell::new();
        shell.select_memory();
        assert_eq!(shell.press(Key::Digit1, 0, context(20, 0)), Intent::Redraw);
        assert_eq!(shell.entry(), Some(1));
        assert_eq!(shell.press(Key::Star, 10, context(20, 0)), Intent::Idle);
        assert_eq!(shell.entry(), None);

        assert_eq!(shell.hold(Key::Star, context(20, 0)), Intent::StartScan);
        assert_eq!(
            shell.press(Key::Digit9, 20, scanning(20, 0)),
            Intent::StopScan
        );
        assert_eq!(shell.entry(), None);
    }
}
