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
pub struct PeriodicTick;

impl PeriodicTick {
    /// Configures a whole-millisecond period against the supplied processor clock.
    pub fn start(clock_hz: u32, milliseconds: u32) -> Result<Self, PeriodError> {
        let reload = reload_value(clock_hz, milliseconds)?;
        RELOAD.write(reload);
        CURRENT.write(0);
        CONTROL.write(CONTROL_ENABLE | CONTROL_CLOCK_SOURCE_PROCESSOR);
        Ok(Self)
    }

    /// Waits until the counter has crossed zero once.
    pub fn wait(&mut self) {
        while CONTROL.read() & CONTROL_COUNT_FLAG == 0 {
            core::hint::spin_loop();
        }
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
    use super::{reload_value, PeriodError};

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
