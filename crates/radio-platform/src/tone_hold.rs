//! Tone squelch hold, after the reference's `gFoundCTCSSCountdown_10ms`.
//!
//! When the chip reports the receive tone gone, `HandleReceive` keeps the
//! reception for one second and ends it only if the tone has not come back
//! by then (`app.c:263-305`), so a tone dropping out on a weak or fluttering
//! signal does not chop the audio. The carrier squelch still closes at once.

/// How long a matched tone is held after the chip reports it gone: the
/// reference's 100 ticks of 10 ms.
pub const HOLD_MILLISECONDS: u64 = 1_000;

/// Holds a matched tone for [`HOLD_MILLISECONDS`] after it goes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ToneHold {
    /// When the tone was last matched, while the hold may still apply.
    matched_at: Option<u64>,
}

impl ToneHold {
    /// A hold with no tone matched.
    #[must_use]
    pub const fn new() -> Self {
        Self { matched_at: None }
    }

    /// Forgets the tone, as a new tune or a closed carrier squelch does.
    pub fn reset(&mut self) {
        self.matched_at = None;
    }

    /// Applies one tone reading at `now_ms`: `None` for a channel without a
    /// receive tone, otherwise whether the chip has it matched. Returns the
    /// reading with the hold applied.
    pub fn observe(&mut self, matched: Option<bool>, now_ms: u64) -> Option<bool> {
        match matched {
            None => {
                self.matched_at = None;
                None
            }
            Some(true) => {
                self.matched_at = Some(now_ms);
                Some(true)
            }
            Some(false) => {
                let held = self
                    .matched_at
                    .is_some_and(|at| now_ms.saturating_sub(at) < HOLD_MILLISECONDS);
                if !held {
                    self.matched_at = None;
                }
                Some(held)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ToneHold, HOLD_MILLISECONDS};

    #[test]
    fn a_tone_that_goes_is_held_for_a_second() {
        let mut hold = ToneHold::new();
        assert_eq!(hold.observe(Some(false), 0), Some(false), "never matched");
        assert_eq!(hold.observe(Some(true), 100), Some(true));
        assert_eq!(hold.observe(Some(false), 200), Some(true));
        assert_eq!(
            hold.observe(Some(false), 100 + HOLD_MILLISECONDS - 1),
            Some(true)
        );
        assert_eq!(
            hold.observe(Some(false), 100 + HOLD_MILLISECONDS),
            Some(false)
        );
        // Once let go it stays shut until the tone matches again.
        assert_eq!(hold.observe(Some(false), 100), Some(false));
    }

    #[test]
    fn a_tone_coming_back_restarts_the_hold() {
        let mut hold = ToneHold::new();
        hold.observe(Some(true), 0);
        assert_eq!(hold.observe(Some(false), 900), Some(true));
        assert_eq!(hold.observe(Some(true), 950), Some(true));
        assert_eq!(hold.observe(Some(false), 1_900), Some(true));
        assert_eq!(hold.observe(Some(false), 1_950), Some(false));
    }

    #[test]
    fn reset_and_untoned_channels_drop_the_hold() {
        let mut hold = ToneHold::new();
        hold.observe(Some(true), 0);
        hold.reset();
        assert_eq!(hold.observe(Some(false), 10), Some(false));
        hold.observe(Some(true), 20);
        assert_eq!(hold.observe(None, 30), None);
        assert_eq!(hold.observe(Some(false), 40), Some(false));
    }
}
