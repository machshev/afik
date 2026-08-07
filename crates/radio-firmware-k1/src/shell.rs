//! Receive-only operator shell: screens, channel selection, and bank filter.
//!
//! The shell is pure. It consumes debounced key presses and explicit
//! milliseconds and returns the intent the caller should apply to the shared
//! receive controller, so every screen transition, numeric entry, and bank
//! filter step is host-testable without a display, a keypad, or a radio.
//!
//! No intent can transmit. The set deliberately contains selection, bank
//! filtering, monitoring, and receive-audio routing only.

use radio_domain::BankId;

use crate::configuration::MAX_BANKS;
use crate::keypad::Key;

/// Digits a channel-number entry accepts.
pub const ENTRY_DIGITS: usize = 2;

/// Milliseconds an incomplete channel-number entry waits before it commits.
pub const ENTRY_TIMEOUT_MILLISECONDS: u32 = 1_200;

/// Which screen the operator is looking at.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screen {
    /// Channel name, frequency, and receive state.
    #[default]
    Operating,
    /// Scrollable list of the channels in the active view.
    ChannelList,
    /// Image identity and storage state.
    Info,
}

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
    /// Apply a bank filter, or clear it with `None`.
    SetBank(Option<BankId>),
    /// Route or mute demodulated receive audio.
    ToggleAudio,
    /// Open or close the squelch override.
    ToggleMonitor,
}

/// The receive state the shell needs to bound its own decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Context {
    /// Channels selectable in the active bank view.
    pub visible_channels: u16,
    /// Zero-based index of the active channel in that view.
    pub active_index: u16,
}

/// A pending channel-number entry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Entry {
    digits: u8,
    value: u16,
    started_ms: u32,
}

/// Operator shell state.
#[derive(Clone, Copy, Debug)]
pub struct Shell {
    screen: Screen,
    cursor: u16,
    entry: Option<Entry>,
    bank_filter: Option<BankId>,
    banks: [Option<BankId>; MAX_BANKS],
    bank_count: usize,
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
            cursor: 0,
            entry: None,
            bank_filter: None,
            banks: [None; MAX_BANKS],
            bank_count: 0,
        }
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

    /// Returns the digits typed so far, if a channel number is being entered.
    #[must_use]
    pub fn entry(&self) -> Option<u16> {
        self.entry.map(|entry| entry.value)
    }

    /// Replaces the selectable banks after a configuration is programmed.
    ///
    /// A filter which the new configuration does not populate is cleared, so
    /// the operator can never be left looking at an empty view.
    pub fn set_banks(&mut self, banks: [Option<BankId>; MAX_BANKS], count: usize) -> bool {
        self.banks = banks;
        self.bank_count = count.min(MAX_BANKS);
        self.entry = None;
        self.cursor = 0;
        if self
            .bank_filter
            .is_some_and(|filter| !self.banks[..self.bank_count].contains(&Some(filter)))
        {
            self.bank_filter = None;
            return true;
        }
        false
    }

    /// Applies one debounced key press.
    pub fn press(&mut self, key: Key, now_ms: u32, context: Context) -> Intent {
        match key {
            Key::Side1 => Intent::ToggleAudio,
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
            Key::Star => self.cycle_bank(),
            Key::Menu => self.confirm(context),
            Key::Exit => self.cancel(),
            Key::Up => self.step(true, context),
            Key::Down => self.step(false, context),
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

    fn step(&mut self, forwards: bool, context: Context) -> Intent {
        self.entry = None;
        match self.screen {
            Screen::Operating => {
                if forwards {
                    Intent::SelectNext
                } else {
                    Intent::SelectPrevious
                }
            }
            Screen::ChannelList => {
                if context.visible_channels == 0 {
                    self.cursor = 0;
                    return Intent::Redraw;
                }
                let last = context.visible_channels - 1;
                self.cursor = if forwards {
                    if self.cursor >= last {
                        0
                    } else {
                        self.cursor + 1
                    }
                } else if self.cursor == 0 {
                    last
                } else {
                    self.cursor - 1
                };
                Intent::Redraw
            }
            Screen::Info => Intent::Redraw,
        }
    }

    fn confirm(&mut self, context: Context) -> Intent {
        if let Some(entry) = self.entry.take() {
            return self.commit_entry(entry, context);
        }
        match self.screen {
            Screen::Operating => {
                self.screen = Screen::ChannelList;
                self.cursor = context.active_index;
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
        self.screen = Screen::Operating;
        Intent::Redraw
    }

    fn digit(&mut self, digit: u16, now_ms: u32, context: Context) -> Intent {
        let mut entry = self.entry.unwrap_or(Entry {
            digits: 0,
            value: 0,
            started_ms: now_ms,
        });
        entry.value = entry.value * 10 + digit;
        entry.digits += 1;
        entry.started_ms = now_ms;
        if usize::from(entry.digits) >= ENTRY_DIGITS {
            self.entry = None;
            return self.commit_entry(entry, context);
        }
        self.entry = Some(entry);
        Intent::Redraw
    }

    /// Resolves a completed entry into a selection.
    ///
    /// The typed number is the one-based position shown on the operating
    /// screen, so what the operator reads is what the operator can type. An
    /// out-of-range number is discarded rather than clamped onto a channel the
    /// operator did not ask for.
    fn commit_entry(&mut self, entry: Entry, context: Context) -> Intent {
        self.entry = None;
        self.screen = Screen::Operating;
        if entry.value == 0 || entry.value > context.visible_channels {
            return Intent::Redraw;
        }
        Intent::SelectIndex(entry.value - 1)
    }

    fn cycle_bank(&mut self) -> Intent {
        self.entry = None;
        self.cursor = 0;
        if self.bank_count == 0 {
            self.bank_filter = None;
            return Intent::Redraw;
        }
        let next = match self.bank_filter {
            None => self.banks[0],
            Some(current) => {
                let position = self.banks[..self.bank_count]
                    .iter()
                    .position(|bank| *bank == Some(current));
                match position {
                    Some(position) if position + 1 < self.bank_count => self.banks[position + 1],
                    // Past the last populated bank the filter clears, so every
                    // programmed channel is reachable again.
                    _ => None,
                }
            }
        };
        self.bank_filter = next;
        Intent::SetBank(next)
    }
}

#[cfg(test)]
mod tests {
    use super::{Context, Intent, Screen, Shell, ENTRY_TIMEOUT_MILLISECONDS};
    use crate::configuration::MAX_BANKS;
    use crate::keypad::Key;
    use radio_domain::BankId;

    fn context(visible: u16, active: u16) -> Context {
        Context {
            visible_channels: visible,
            active_index: active,
        }
    }

    fn bank_table(ids: &[u16]) -> ([Option<BankId>; MAX_BANKS], usize) {
        let mut banks = [None; MAX_BANKS];
        for (slot, id) in banks.iter_mut().zip(ids) {
            *slot = Some(BankId::new(*id));
        }
        (banks, ids.len())
    }

    #[test]
    fn the_operating_screen_steps_channels_and_the_list_moves_a_cursor() {
        let mut shell = Shell::new();
        assert_eq!(shell.press(Key::Up, 0, context(4, 1)), Intent::SelectNext);
        assert_eq!(
            shell.press(Key::Down, 10, context(4, 1)),
            Intent::SelectPrevious
        );

        assert_eq!(shell.press(Key::Menu, 20, context(4, 1)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::ChannelList);
        assert_eq!(shell.cursor(), 1, "the list opens on the active channel");
        shell.press(Key::Up, 30, context(4, 1));
        shell.press(Key::Up, 40, context(4, 1));
        assert_eq!(shell.cursor(), 3);
        shell.press(Key::Up, 50, context(4, 1));
        assert_eq!(shell.cursor(), 0, "the cursor wraps at the end of the view");
        assert_eq!(
            shell.press(Key::Menu, 60, context(4, 1)),
            Intent::SelectIndex(0)
        );
        assert_eq!(shell.screen(), Screen::Operating);
    }

    #[test]
    fn exit_leaves_a_screen_without_changing_the_selection() {
        let mut shell = Shell::new();
        shell.press(Key::Menu, 0, context(4, 2));
        shell.press(Key::Up, 10, context(4, 2));
        assert_eq!(shell.press(Key::Exit, 20, context(4, 2)), Intent::Redraw);
        assert_eq!(shell.screen(), Screen::Operating);
        assert_eq!(
            shell.press(Key::Exit, 30, context(4, 2)),
            Intent::Idle,
            "exit on the operating screen has nothing to cancel"
        );
    }

    #[test]
    fn a_two_digit_number_selects_the_position_it_names() {
        let mut shell = Shell::new();
        assert_eq!(shell.press(Key::Digit1, 0, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.entry(), Some(1));
        assert_eq!(
            shell.press(Key::Digit2, 100, context(16, 0)),
            Intent::SelectIndex(11)
        );
        assert_eq!(shell.entry(), None);
    }

    #[test]
    fn one_digit_commits_after_the_entry_timeout() {
        let mut shell = Shell::new();
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
        let mut shell = Shell::new();
        shell.press(Key::Digit9, 0, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit9, 10, context(4, 0)),
            Intent::Redraw,
            "99 is not a channel in a four-channel view"
        );
        shell.press(Key::Digit0, 20, context(4, 0));
        assert_eq!(
            shell.press(Key::Digit0, 30, context(4, 0)),
            Intent::Redraw,
            "channel zero does not exist"
        );
    }

    #[test]
    fn exit_clears_a_partial_number_before_it_can_select() {
        let mut shell = Shell::new();
        shell.press(Key::Digit1, 0, context(16, 0));
        assert_eq!(shell.press(Key::Exit, 10, context(16, 0)), Intent::Redraw);
        assert_eq!(shell.entry(), None);
        assert_eq!(
            shell.tick(10_000, context(16, 0)),
            Intent::Idle,
            "a cancelled entry cannot commit later"
        );
    }

    #[test]
    fn the_bank_filter_cycles_through_populated_banks_and_clears() {
        let mut shell = Shell::new();
        let (banks, count) = bank_table(&[1, 3]);
        assert!(!shell.set_banks(banks, count));
        assert_eq!(
            shell.press(Key::Star, 0, context(8, 0)),
            Intent::SetBank(Some(BankId::new(1)))
        );
        assert_eq!(
            shell.press(Key::Star, 10, context(4, 0)),
            Intent::SetBank(Some(BankId::new(3)))
        );
        assert_eq!(
            shell.press(Key::Star, 20, context(2, 0)),
            Intent::SetBank(None)
        );
        assert_eq!(shell.bank_filter(), None);
    }

    #[test]
    fn a_filter_the_new_configuration_does_not_populate_is_cleared() {
        let mut shell = Shell::new();
        let (banks, count) = bank_table(&[1, 3]);
        shell.set_banks(banks, count);
        shell.press(Key::Star, 0, context(8, 0));
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
        let mut shell = Shell::new();
        for key in [
            Key::Side1,
            Key::Side2,
            Key::Ptt,
            Key::Menu,
            Key::Up,
            Key::Down,
            Key::Exit,
            Key::Star,
            Key::Function,
            Key::Digit0,
            Key::Digit1,
            Key::Digit9,
        ] {
            let intent = shell.press(key, 0, context(4, 0));
            assert!(matches!(
                intent,
                Intent::Idle
                    | Intent::Redraw
                    | Intent::SelectNext
                    | Intent::SelectPrevious
                    | Intent::SelectIndex(_)
                    | Intent::SetBank(_)
                    | Intent::ToggleAudio
                    | Intent::ToggleMonitor
            ));
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
    }
}
