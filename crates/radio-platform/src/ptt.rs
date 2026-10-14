//! PTT input filtering ahead of the TX controller, shared by every target.
//!
//! The programming cable shares the radio's accessory jack, so serial traffic
//! and the host opening its port can pull the PTT input low for moments at a
//! time. The reference firmware (`app/app.c`, `app/uart.c`) keys only after
//! PTT has read low for 30 ms, and ignores PTT, ending any transmission, for
//! 6 s after each programming request. This filter does both over explicit
//! millisecond time and hands the controller its press, release, or tick. A
//! release takes effect at once.

use crate::tx_control::Event;

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

    /// Records a programming request; PTT reads released until the window
    /// ends, and a press in progress is released by the next sample.
    pub fn serial_activity(&mut self, now_ms: u64) {
        self.quiet_until = now_ms.saturating_add(SERIAL_QUIET_MS);
    }

    /// Takes one raw sample and returns the controller event for it.
    pub fn sample(&mut self, now_ms: u64, raw_pressed: bool) -> Event {
        let was = self.pressed;
        self.pressed = self.level(now_ms, raw_pressed);
        match (was, self.pressed) {
            (false, true) => Event::Pressed,
            (true, false) => Event::Released,
            _ => Event::Tick,
        }
    }

    fn level(&mut self, now_ms: u64, raw_pressed: bool) -> bool {
        if !raw_pressed || now_ms < self.quiet_until {
            self.low_since = None;
            return false;
        }
        let since = *self.low_since.get_or_insert(now_ms);
        self.pressed || now_ms.saturating_sub(since) >= PRESS_DEBOUNCE_MS
    }

    /// Reports the most recent filtered level.
    pub const fn pressed(&self) -> bool {
        self.pressed
    }
}

#[cfg(test)]
mod tests {
    use super::{PttFilter, PRESS_DEBOUNCE_MS, SERIAL_QUIET_MS};
    use crate::tx_control::Event;

    #[test]
    fn a_single_low_sample_does_not_press() {
        let mut filter = PttFilter::new(false, 0);
        assert_eq!(filter.sample(50, true), Event::Tick);
        assert_eq!(filter.sample(100, false), Event::Tick);
        assert_eq!(filter.sample(150, true), Event::Tick);
        assert!(!filter.pressed());
    }

    #[test]
    fn a_held_press_counts_once_after_the_debounce() {
        let mut filter = PttFilter::new(false, 0);
        assert_eq!(filter.sample(100, true), Event::Tick);
        assert_eq!(
            filter.sample(100 + PRESS_DEBOUNCE_MS - 1, true),
            Event::Tick
        );
        assert_eq!(filter.sample(100 + PRESS_DEBOUNCE_MS, true), Event::Pressed);
        assert_eq!(filter.sample(200, true), Event::Tick);
        assert!(filter.pressed());
    }

    #[test]
    fn the_debounce_is_time_not_samples() {
        // K5 samples every 50 ms, K1 every 5 ms.
        let mut k5 = PttFilter::new(false, 0);
        assert_eq!(k5.sample(50, true), Event::Tick);
        assert_eq!(k5.sample(100, true), Event::Pressed);
        let mut k1 = PttFilter::new(false, 0);
        for now in (5..35).step_by(5) {
            assert_eq!(k1.sample(now, true), Event::Tick, "at {now}");
        }
        assert_eq!(k1.sample(35, true), Event::Pressed);
    }

    #[test]
    fn a_release_takes_effect_at_once() {
        let mut filter = PttFilter::new(false, 0);
        filter.sample(0, true);
        assert_eq!(filter.sample(50, true), Event::Pressed);
        assert_eq!(filter.sample(100, false), Event::Released);
        assert_eq!(filter.sample(150, true), Event::Tick);
    }

    #[test]
    fn a_ptt_held_at_boot_reads_pressed_without_a_press_event() {
        let mut filter = PttFilter::new(true, 0);
        assert!(filter.pressed());
        assert_eq!(filter.sample(50, true), Event::Tick);
        assert_eq!(filter.sample(100, false), Event::Released);
    }

    #[test]
    fn serial_activity_releases_and_ignores_ptt_for_the_quiet_window() {
        let mut filter = PttFilter::new(false, 0);
        filter.sample(0, true);
        assert_eq!(filter.sample(50, true), Event::Pressed);
        filter.serial_activity(100);
        assert_eq!(filter.sample(100, true), Event::Released);
        for now in (150..100 + SERIAL_QUIET_MS).step_by(50) {
            assert_eq!(filter.sample(now, true), Event::Tick, "at {now}");
        }
        let end = 100 + SERIAL_QUIET_MS;
        assert_eq!(filter.sample(end, true), Event::Tick);
        assert_eq!(filter.sample(end + PRESS_DEBOUNCE_MS, true), Event::Pressed);
    }

    #[test]
    fn further_activity_extends_the_window() {
        let mut filter = PttFilter::new(false, 0);
        filter.serial_activity(0);
        filter.serial_activity(5_000);
        assert_eq!(filter.sample(SERIAL_QUIET_MS + 1_000, true), Event::Tick);
        assert_eq!(filter.sample(5_000 + SERIAL_QUIET_MS, true), Event::Tick);
        assert_eq!(
            filter.sample(5_000 + SERIAL_QUIET_MS + 50, true),
            Event::Pressed
        );
    }
}
