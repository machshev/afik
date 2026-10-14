//! Dual watch: which of the two slots the one receiver listens to.
//!
//! The radio has one receiver and two slots. With dual watch on it listens to
//! the selected slot and the other in turn, a dwell each, until one of them is
//! busy; it then stays there while the squelch is open and for a hang time
//! after, so a reply on the same channel is not missed. Any operator action
//! and every transmission return it to the selected slot, because that is
//! the one keys act on and PTT transmits on, as on the stock and Armel
//! firmware.
//!
//! The state machine is pure: the caller passes milliseconds and the squelch
//! it read, and retunes when told to.

/// Milliseconds the receiver stays on a slot after its squelch shuts, and on
/// the selected slot after an operator action or a transmission.
///
/// Chosen, not taken from a reference: long enough for the other station to
/// key up in reply.
pub const HANG_MILLISECONDS: u32 = 3_000;

/// Which slot the receiver is tuned to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Listening {
    /// The slot keys act on and PTT transmits on.
    #[default]
    Selected,
    /// The slot not selected.
    Other,
}

impl Listening {
    const fn toggled(self) -> Self {
        match self {
            Self::Selected => Self::Other,
            Self::Other => Self::Selected,
        }
    }
}

/// Dual-watch timing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DualWatch {
    listening: Listening,
    /// When the receiver next moves to the other slot.
    deadline_ms: u32,
}

impl DualWatch {
    /// Starts on the selected slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            listening: Listening::Selected,
            deadline_ms: 0,
        }
    }

    /// Returns which slot the receiver is tuned to.
    #[must_use]
    pub const fn listening(&self) -> Listening {
        self.listening
    }

    /// Returns to the selected slot and stays for the hang time.
    ///
    /// Returns whether the receiver was on the other slot, and so has to be
    /// tuned back.
    pub fn reset(&mut self, now_ms: u32) -> bool {
        let was_other = self.listening == Listening::Other;
        self.listening = Listening::Selected;
        self.deadline_ms = now_ms.wrapping_add(HANG_MILLISECONDS);
        was_other
    }

    /// Advances one pass.
    ///
    /// `busy` is whether the slot tuned now has its squelch open on a settled
    /// reading. Returns the slot to tune when the receiver should move.
    pub fn poll(&mut self, now_ms: u32, dwell_ms: u32, busy: bool) -> Option<Listening> {
        if busy {
            self.deadline_ms = now_ms.wrapping_add(HANG_MILLISECONDS);
            return None;
        }
        if !reached(now_ms, self.deadline_ms) {
            return None;
        }
        self.listening = self.listening.toggled();
        self.deadline_ms = now_ms.wrapping_add(dwell_ms);
        Some(self.listening)
    }
}

/// Reports whether `now` is at or past `deadline` on a wrapping clock.
const fn reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < 1 << 31
}

#[cfg(test)]
mod tests {
    use super::{DualWatch, Listening, HANG_MILLISECONDS};

    const DWELL: u32 = 90;

    #[test]
    fn it_alternates_by_the_dwell_while_both_slots_are_quiet() {
        let mut watch = DualWatch::new();
        assert_eq!(watch.poll(0, DWELL, false), Some(Listening::Other));
        assert_eq!(watch.poll(DWELL - 1, DWELL, false), None);
        assert_eq!(watch.poll(DWELL, DWELL, false), Some(Listening::Selected));
        assert_eq!(watch.poll(2 * DWELL, DWELL, false), Some(Listening::Other));
    }

    #[test]
    fn a_busy_slot_holds_the_receiver_until_the_hang_ends() {
        let mut watch = DualWatch::new();
        watch.poll(0, DWELL, false);
        assert_eq!(watch.listening(), Listening::Other);
        // A signal on the other slot holds it for as long as it lasts.
        for now in (10..5_000).step_by(10) {
            assert_eq!(watch.poll(now, DWELL, true), None);
        }
        // Then for the hang time after it ends.
        assert_eq!(watch.poll(5_000, DWELL, false), None);
        assert_eq!(
            watch.poll(4_990 + HANG_MILLISECONDS - 1, DWELL, false),
            None
        );
        assert_eq!(
            watch.poll(4_990 + HANG_MILLISECONDS, DWELL, false),
            Some(Listening::Selected)
        );
    }

    #[test]
    fn reset_returns_to_the_selected_slot_and_stays_for_the_hang() {
        let mut watch = DualWatch::new();
        watch.poll(0, DWELL, false);
        assert!(watch.reset(10), "it was on the other slot");
        assert_eq!(watch.listening(), Listening::Selected);
        assert_eq!(watch.poll(10 + HANG_MILLISECONDS - 1, DWELL, false), None);
        assert_eq!(
            watch.poll(10 + HANG_MILLISECONDS, DWELL, false),
            Some(Listening::Other)
        );
        assert!(watch.reset(20_000));
        assert!(!watch.reset(20_010), "already on the selected slot");
    }

    #[test]
    fn the_clock_may_wrap() {
        let mut watch = DualWatch::new();
        assert!(!watch.reset(u32::MAX - 100));
        assert_eq!(watch.poll(u32::MAX, DWELL, false), None);
        assert_eq!(
            watch.poll(
                (u32::MAX - 100).wrapping_add(HANG_MILLISECONDS),
                DWELL,
                false
            ),
            Some(Listening::Other)
        );
    }
}
