//! Battery save: the receiver sleeps while nothing is happening.
//!
//! After [`IDLE_MILLISECONDS`] with no key, no PTT, no signal and no scan, the
//! receiver sleeps for the programmed ratio times [`LISTEN_MILLISECONDS`],
//! then wakes to listen for [`LISTEN_MILLISECONDS`]. A signal found while
//! listening, or any activity at all, keeps it awake. This is Armel's
//! `FUNCTION_PowerSave` cycle (`EVID-TX056-003`): `battery_save_count_10ms`,
//! `power_save1_10ms`, and `BATTERY_SAVE * 10` ticks of sleep.
//!
//! The state machine is pure: the caller passes milliseconds and whether
//! anything is happening, and sleeps or wakes the receiver when told to.

/// Milliseconds of quiet before the receiver first sleeps.
pub const IDLE_MILLISECONDS: u32 = 10_000;

/// Milliseconds the receiver listens between sleeps, and one unit of sleep.
pub const LISTEN_MILLISECONDS: u32 = 100;

/// What the receiver should do now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Put the receiver to sleep.
    Sleep,
    /// Tune the receiver again to listen.
    Wake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Receiving; sleeps when the deadline passes quietly.
    Awake,
    /// Asleep until the deadline.
    Asleep,
    /// Listening briefly; sleeps again at the deadline unless something
    /// happens.
    Listening,
}

/// Battery-save timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PowerSave {
    phase: Phase,
    deadline_ms: u32,
}

impl PowerSave {
    /// Starts awake, the idle time from `now_ms`.
    #[must_use]
    pub const fn new(now_ms: u32) -> Self {
        Self {
            phase: Phase::Awake,
            deadline_ms: now_ms.wrapping_add(IDLE_MILLISECONDS),
        }
    }

    /// Reports whether the receiver is asleep.
    #[must_use]
    pub const fn asleep(&self) -> bool {
        matches!(self.phase, Phase::Asleep)
    }

    /// Advances one pass.
    ///
    /// `ratio` is the programmed sleep ratio, zero for off. `active` is
    /// whether anything is happening: a key, PTT, an open squelch, a scan, a
    /// retune. Returns what the receiver should do now.
    pub fn poll(&mut self, now_ms: u32, ratio: u8, active: bool) -> Option<Action> {
        if active || ratio == 0 {
            let was_asleep = self.asleep();
            self.phase = Phase::Awake;
            self.deadline_ms = now_ms.wrapping_add(IDLE_MILLISECONDS);
            return was_asleep.then_some(Action::Wake);
        }
        if !reached(now_ms, self.deadline_ms) {
            return None;
        }
        match self.phase {
            Phase::Awake | Phase::Listening => {
                self.phase = Phase::Asleep;
                self.deadline_ms = now_ms.wrapping_add(u32::from(ratio) * LISTEN_MILLISECONDS);
                Some(Action::Sleep)
            }
            Phase::Asleep => {
                self.phase = Phase::Listening;
                self.deadline_ms = now_ms.wrapping_add(LISTEN_MILLISECONDS);
                Some(Action::Wake)
            }
        }
    }
}

/// Reports whether `now` is at or past `deadline` on a wrapping clock.
const fn reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < 1 << 31
}

#[cfg(test)]
mod tests {
    use super::{Action, PowerSave, IDLE_MILLISECONDS, LISTEN_MILLISECONDS};

    #[test]
    fn it_sleeps_after_the_idle_time_and_listens_between_sleeps() {
        let mut save = PowerSave::new(0);
        assert_eq!(save.poll(IDLE_MILLISECONDS - 1, 2, false), None);
        assert_eq!(save.poll(IDLE_MILLISECONDS, 2, false), Some(Action::Sleep));
        assert!(save.asleep());
        // Ratio 2 sleeps for two listening times.
        let woken = IDLE_MILLISECONDS + 2 * LISTEN_MILLISECONDS;
        assert_eq!(save.poll(woken - 1, 2, false), None);
        assert_eq!(save.poll(woken, 2, false), Some(Action::Wake));
        assert!(!save.asleep());
        assert_eq!(save.poll(woken + LISTEN_MILLISECONDS - 1, 2, false), None);
        assert_eq!(
            save.poll(woken + LISTEN_MILLISECONDS, 2, false),
            Some(Action::Sleep)
        );
    }

    #[test]
    fn a_signal_heard_while_listening_keeps_the_receiver_awake() {
        let mut save = PowerSave::new(0);
        save.poll(IDLE_MILLISECONDS, 1, false);
        let woken = IDLE_MILLISECONDS + LISTEN_MILLISECONDS;
        assert_eq!(save.poll(woken, 1, false), Some(Action::Wake));
        // Already awake, so nothing to do; the idle time starts again.
        assert_eq!(save.poll(woken + 50, 1, true), None);
        assert_eq!(
            save.poll(woken + 50 + IDLE_MILLISECONDS - 1, 1, false),
            None
        );
        assert_eq!(
            save.poll(woken + 50 + IDLE_MILLISECONDS, 1, false),
            Some(Action::Sleep)
        );
    }

    #[test]
    fn activity_wakes_a_sleeping_receiver_at_once() {
        let mut save = PowerSave::new(0);
        save.poll(IDLE_MILLISECONDS, 4, false);
        assert_eq!(
            save.poll(IDLE_MILLISECONDS + 1, 4, true),
            Some(Action::Wake)
        );
        assert!(!save.asleep());
    }

    #[test]
    fn ratio_zero_never_sleeps_and_wakes_a_sleeping_receiver() {
        let mut save = PowerSave::new(0);
        for now in (0..60_000).step_by(100) {
            assert_eq!(save.poll(now, 0, false), None);
        }
        let mut save = PowerSave::new(0);
        save.poll(IDLE_MILLISECONDS, 3, false);
        assert_eq!(
            save.poll(IDLE_MILLISECONDS + 1, 0, false),
            Some(Action::Wake),
            "switching battery save off by reprogramming wakes the receiver"
        );
    }
}
