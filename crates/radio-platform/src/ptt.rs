//! PTT input filtering ahead of the TX controller.
//!
//! The programming cable shares the radio's accessory jack, so serial traffic
//! and the host opening its port can pull the PTT input low for moments at a
//! time. The reference firmware (`app/app.c`, `app/uart.c`) keys only after
//! PTT has read low for 30 ms, and ignores PTT, ending any transmission, for
//! 6 s after each programming request. This filter does both over explicit
//! millisecond time. A release takes effect at once.

/// Time PTT must read pressed before it counts as a press.
pub const PRESS_DEBOUNCE_MS: u64 = 30;

/// Time after a programming request during which PTT counts as released.
pub const SERIAL_QUIET_MS: u64 = 6_000;

/// Debounced PTT level with a quiet window after serial activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PttFilter {
    pressed: bool,
    low_since: Option<u64>,
    quiet_until: u64,
}

impl PttFilter {
    /// Starts from the raw level at boot, so a PTT already held at startup is
    /// reported pressed and the controller can require its release.
    pub const fn new(raw_pressed: bool, now_ms: u64) -> Self {
        Self {
            pressed: raw_pressed,
            low_since: if raw_pressed { Some(now_ms) } else { None },
            quiet_until: 0,
        }
    }

    /// Records a programming request; PTT reads released until the window ends.
    pub fn serial_activity(&mut self, now_ms: u64) {
        self.quiet_until = now_ms.saturating_add(SERIAL_QUIET_MS);
        self.pressed = false;
        self.low_since = None;
    }

    /// Takes one raw sample and returns the filtered level.
    pub fn sample(&mut self, now_ms: u64, raw_pressed: bool) -> bool {
        if !raw_pressed || now_ms < self.quiet_until {
            self.pressed = false;
            self.low_since = None;
            return false;
        }
        let since = *self.low_since.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= PRESS_DEBOUNCE_MS {
            self.pressed = true;
        }
        self.pressed
    }

    /// Reports the most recent filtered level.
    pub const fn pressed(&self) -> bool {
        self.pressed
    }
}

#[cfg(test)]
mod tests {
    use super::{PttFilter, PRESS_DEBOUNCE_MS, SERIAL_QUIET_MS};

    #[test]
    fn a_single_low_sample_does_not_press() {
        let mut filter = PttFilter::new(false, 0);
        assert!(!filter.sample(50, true));
        assert!(!filter.sample(100, false));
        assert!(!filter.sample(150, true));
        assert!(!filter.pressed());
    }

    #[test]
    fn a_held_press_counts_after_the_debounce() {
        let mut filter = PttFilter::new(false, 0);
        assert!(!filter.sample(100, true));
        assert!(!filter.sample(100 + PRESS_DEBOUNCE_MS - 1, true));
        assert!(filter.sample(100 + PRESS_DEBOUNCE_MS, true));
        assert!(filter.sample(200, true));
    }

    #[test]
    fn at_a_fifty_millisecond_loop_two_samples_press() {
        let mut filter = PttFilter::new(false, 0);
        assert!(!filter.sample(50, true));
        assert!(filter.sample(100, true));
    }

    #[test]
    fn a_release_takes_effect_at_once() {
        let mut filter = PttFilter::new(false, 0);
        filter.sample(0, true);
        assert!(filter.sample(50, true));
        assert!(!filter.sample(100, false));
        assert!(!filter.sample(150, true));
    }

    #[test]
    fn a_ptt_held_at_boot_reads_pressed_immediately() {
        let mut filter = PttFilter::new(true, 0);
        assert!(filter.pressed());
        assert!(filter.sample(50, true));
    }

    #[test]
    fn serial_activity_releases_and_ignores_ptt_for_the_quiet_window() {
        let mut filter = PttFilter::new(false, 0);
        filter.sample(0, true);
        assert!(filter.sample(50, true));
        filter.serial_activity(100);
        assert!(!filter.pressed());
        for now in (100..100 + SERIAL_QUIET_MS).step_by(50) {
            assert!(!filter.sample(now, true), "pressed at {now}");
        }
        let end = 100 + SERIAL_QUIET_MS;
        assert!(!filter.sample(end, true));
        assert!(filter.sample(end + PRESS_DEBOUNCE_MS, true));
    }

    #[test]
    fn further_activity_extends_the_window() {
        let mut filter = PttFilter::new(false, 0);
        filter.serial_activity(0);
        filter.serial_activity(5_000);
        assert!(!filter.sample(SERIAL_QUIET_MS, true));
        assert!(!filter.sample(SERIAL_QUIET_MS + 1_000, true));
        assert!(!filter.sample(5_000 + SERIAL_QUIET_MS, true));
        assert!(filter.sample(5_000 + SERIAL_QUIET_MS + 50, true));
    }
}
