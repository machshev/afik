//! Polled Cortex-M0 `SysTick` cadence for the K5 application loop.

use crate::mmio::Register;

const SYSTICK_BASE: u32 = 0xE000_E010;
const CONTROL: Register = Register::new(SYSTICK_BASE, 0x00);
const RELOAD: Register = Register::new(SYSTICK_BASE, 0x04);
const CURRENT: Register = Register::new(SYSTICK_BASE, 0x08);
const CONTROL_ENABLE: u32 = 1;
const CONTROL_CLOCK_SOURCE_PROCESSOR: u32 = 1 << 2;
const CONTROL_COUNT_FLAG: u32 = 1 << 16;
const MAX_RELOAD: u32 = 0x00FF_FFFF;

/// A `SysTick` period could not be represented by its 24-bit reload field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeriodError;

/// Polled periodic tick using the processor clock and no interrupt.
pub struct PeriodicTick {
    period: u32,
    ticks_per_ms: u32,
}

impl PeriodicTick {
    /// Configures a whole-millisecond period against the supplied processor clock.
    pub fn start(clock_hz: u32, milliseconds: u32) -> Result<Self, PeriodError> {
        let reload = reload_value(clock_hz, milliseconds)?;
        RELOAD.write(reload);
        CURRENT.write(0);
        CONTROL.write(CONTROL_ENABLE | CONTROL_CLOCK_SOURCE_PROCESSOR);
        Ok(Self {
            period: reload + 1,
            ticks_per_ms: clock_hz / 1_000,
        })
    }

    /// Busy-waits at least `milliseconds` by following the running counter.
    ///
    /// The period keeps running, so a following [`Self::wait`] still ends at
    /// the next period boundary.
    pub fn delay_ms(&mut self, milliseconds: u32) {
        let target = u64::from(self.ticks_per_ms) * u64::from(milliseconds);
        let mut elapsed = 0_u64;
        let mut previous = CURRENT.read();
        while elapsed < target {
            let current = CURRENT.read();
            elapsed += u64::from(ticks_between(previous, current, self.period));
            previous = current;
        }
    }

    /// Waits until the counter has crossed zero once.
    pub fn wait(&mut self) {
        while CONTROL.read() & CONTROL_COUNT_FLAG == 0 {
            core::hint::spin_loop();
        }
    }
}

/// Counter ticks from `previous` to `current` on a down-counter of `period`.
const fn ticks_between(previous: u32, current: u32, period: u32) -> u32 {
    if current <= previous {
        previous - current
    } else {
        previous + (period - current)
    }
}

/// Converts a clock and whole milliseconds to the value written to `SYST_RVR`.
pub const fn reload_value(clock_hz: u32, milliseconds: u32) -> Result<u32, PeriodError> {
    let Some(ticks) = clock_hz.checked_mul(milliseconds) else {
        return Err(PeriodError);
    };
    let ticks = ticks / 1_000;
    if ticks == 0 || ticks - 1 > MAX_RELOAD {
        Err(PeriodError)
    } else {
        Ok(ticks - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{reload_value, ticks_between, PeriodError};

    #[test]
    fn elapsed_ticks_follow_the_down_counter_across_reload() {
        assert_eq!(ticks_between(1_000, 400, 2_400_000), 600);
        assert_eq!(ticks_between(100, 100, 2_400_000), 0);
        assert_eq!(ticks_between(100, 2_399_900, 2_400_000), 200);
    }

    #[test]
    fn fifty_milliseconds_at_the_k5_clock_fits_exactly() {
        assert_eq!(reload_value(48_000_000, 50), Ok(2_399_999));
    }

    #[test]
    fn zero_overflow_and_more_than_twenty_four_bits_are_refused() {
        assert_eq!(reload_value(48_000_000, 0), Err(PeriodError));
        assert_eq!(reload_value(u32::MAX, 2), Err(PeriodError));
        assert_eq!(reload_value(48_000_000, 1_000), Err(PeriodError));
    }
}
