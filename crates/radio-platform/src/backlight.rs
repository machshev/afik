//! When the display backlight is lit.
//!
//! The programmed configuration's `backlight_seconds` is the whole policy, as
//! in the reference firmware: zero keeps the light off, [`BACKLIGHT_ALWAYS_ON`]
//! keeps it on, and anything else lights it for that many seconds after the
//! operator last pressed a key or the squelch last stood open. Every caller
//! passes the same wrapping millisecond clock, so this owns no time of its own
//! and tests run on virtual time.

use radio_domain::BACKLIGHT_ALWAYS_ON;

/// Backlight timeout state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backlight {
    seconds: u8,
    /// When a timed light was last woken; `None` once it has gone out.
    woken_at: Option<u32>,
    lit: bool,
}

impl Backlight {
    /// A light which is on, as the display starts it, and will go out after
    /// `seconds` from `now_ms`.
    #[must_use]
    pub const fn new(seconds: u8, now_ms: u32) -> Self {
        Self {
            seconds,
            woken_at: Some(now_ms),
            lit: true,
        }
    }

    /// Whether the light should be on now.
    #[must_use]
    pub const fn lit(&self) -> bool {
        self.lit
    }

    /// Adopts a programmed timeout and restarts it from `now_ms`.
    ///
    /// Returns the new state when it differs from the old one.
    pub fn set_seconds(&mut self, seconds: u8, now_ms: u32) -> Option<bool> {
        self.seconds = seconds;
        self.wake(now_ms)
    }

    /// The operator did something: light up and restart the timeout.
    ///
    /// Returns the new state when it differs from the old one.
    pub fn wake(&mut self, now_ms: u32) -> Option<bool> {
        self.woken_at = Some(now_ms);
        self.settle(now_ms)
    }

    /// Lets the timeout run out.
    ///
    /// Returns the new state when it differs from the old one.
    pub fn poll(&mut self, now_ms: u32) -> Option<bool> {
        self.settle(now_ms)
    }

    /// Wakes the light while `active` holds, such as an open squelch, and
    /// otherwise lets it run out: it stays lit through a signal and goes out
    /// the timeout after the signal ends.
    ///
    /// Returns the new state when it differs from the old one.
    pub fn keep_lit_while(&mut self, active: bool, now_ms: u32) -> Option<bool> {
        if active {
            self.wake(now_ms)
        } else {
            self.poll(now_ms)
        }
    }

    fn settle(&mut self, now_ms: u32) -> Option<bool> {
        let lit = match self.seconds {
            0 => false,
            BACKLIGHT_ALWAYS_ON => true,
            seconds => self
                .woken_at
                .is_some_and(|woken| now_ms.wrapping_sub(woken) < u32::from(seconds) * 1_000),
        };
        if !lit {
            self.woken_at = None;
        }
        (lit != self.lit).then(|| {
            self.lit = lit;
            lit
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Backlight;
    use radio_domain::BACKLIGHT_ALWAYS_ON;

    #[test]
    fn a_timed_light_goes_out_and_a_key_brings_it_back() {
        let mut light = Backlight::new(10, 1_000);
        assert!(light.lit());
        assert_eq!(light.poll(10_999), None);
        assert_eq!(light.poll(11_000), Some(false));
        assert_eq!(light.poll(20_000), None);
        assert_eq!(light.wake(30_000), Some(true));
        assert_eq!(light.wake(35_000), None, "a press while lit restarts it");
        assert_eq!(light.poll(44_999), None);
        assert_eq!(light.poll(45_000), Some(false));
    }

    #[test]
    fn zero_keeps_the_light_off_and_always_on_keeps_it_on() {
        let mut light = Backlight::new(10, 0);
        assert_eq!(light.set_seconds(0, 1), Some(false));
        assert_eq!(light.wake(2), None, "zero ignores key presses");
        assert_eq!(light.set_seconds(BACKLIGHT_ALWAYS_ON, 3), Some(true));
        assert_eq!(light.poll(u32::MAX), None);
    }

    #[test]
    fn an_open_squelch_keeps_the_light_on_until_the_timeout_after_it_shuts() {
        let mut light = Backlight::new(10, 0);
        assert_eq!(light.poll(10_000), Some(false));
        assert_eq!(light.keep_lit_while(true, 20_000), Some(true));
        assert_eq!(light.keep_lit_while(true, 45_000), None, "a long over");
        assert_eq!(light.keep_lit_while(false, 54_999), None);
        assert_eq!(light.keep_lit_while(false, 55_000), Some(false));
    }

    #[test]
    fn the_timeout_survives_the_clock_wrapping() {
        let mut light = Backlight::new(5, u32::MAX - 1_000);
        assert_eq!(light.poll(3_998), None);
        assert_eq!(light.poll(3_999), Some(false));
    }

    #[test]
    fn a_light_that_has_gone_out_stays_out_when_the_clock_wraps_round() {
        let mut light = Backlight::new(5, 0);
        assert_eq!(light.poll(5_000), Some(false));
        assert_eq!(light.poll(1), None);
    }
}
