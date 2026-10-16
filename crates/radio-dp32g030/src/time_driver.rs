//! Embassy time driver on the Cortex-M0 `SysTick`, per `EVID-ARM-003`.
//!
//! `SysTick` reloads every millisecond and its exception counts the periods.
//! Embassy ticks at 1 kHz on this driver (`tick-hz-1_000`), so `now()` is the
//! period count: a timer wakes within a millisecond of its deadline either
//! way, and the K5's 60 KiB image has no room for the 64-bit division a
//! microsecond reading and every millisecond conversion of it cost.
//! Expired timers are woken from the exception.
//!
//! The image must route the `SysTick` vector to [`on_systick`] and call
//! [`start`] once, after the clock is configured and before anything awaits.

#[cfg(feature = "time-driver")]
pub use driver::{cycles_taken, on_systick, start};

#[cfg(feature = "time-driver")]
mod driver {
    use core::cell::{Cell, RefCell};
    use core::task::Waker;

    use critical_section::Mutex;
    use embassy_time_driver::Driver;
    use embassy_time_queue_utils::Queue;

    use super::{elapsed_cycles, period, ticks, ClockError};
    use crate::mmio::Register;

    const SYSTICK_BASE: u32 = 0xE000_E010;
    const CONTROL: Register = Register::new(SYSTICK_BASE, 0x00);
    const RELOAD: Register = Register::new(SYSTICK_BASE, 0x04);
    const CURRENT: Register = Register::new(SYSTICK_BASE, 0x08);
    const CONTROL_ENABLE: u32 = 1;
    const CONTROL_TICK_INTERRUPT: u32 = 1 << 1;
    const CONTROL_CLOCK_SOURCE_PROCESSOR: u32 = 1 << 2;
    /// `ICSR`, whose `PENDSTSET` bit says a reload happened that the exception
    /// has not yet counted.
    const INTERRUPT_CONTROL: Register = Register::new(0xE000_ED00, 0x04);
    const PENDSTSET: u32 = 1 << 26;

    struct SysTickDriver {
        periods: Mutex<Cell<u64>>,
        queue: Mutex<RefCell<Queue>>,
    }

    impl SysTickDriver {
        fn now_in(&self, cs: critical_section::CriticalSection<'_>) -> u64 {
            let pending = INTERRUPT_CONTROL.read() & PENDSTSET != 0;
            ticks(self.periods.borrow(cs).get(), pending)
        }
    }

    impl Driver for SysTickDriver {
        fn now(&self) -> u64 {
            critical_section::with(|cs| self.now_in(cs))
        }

        fn schedule_wake(&self, at: u64, waker: &Waker) {
            critical_section::with(|cs| {
                let now = self.now_in(cs);
                let mut queue = self.queue.borrow_ref_mut(cs);
                queue.schedule_wake(at, waker);
                // Wakes anything already due; the rest wait for the exception.
                queue.next_expiration(now);
            });
        }
    }

    #[allow(unsafe_code)]
    mod registration {
        // SAFETY (macro): registers the single global driver this image has.
        embassy_time_driver::time_driver_impl!(static DRIVER: super::SysTickDriver = super::SysTickDriver {
            periods: critical_section::Mutex::new(core::cell::Cell::new(0)),
            queue: critical_section::Mutex::new(core::cell::RefCell::new(
                embassy_time_queue_utils::Queue::new()
            )),
        });

        pub(super) fn driver() -> &'static super::SysTickDriver {
            &DRIVER
        }
    }

    /// Starts the millisecond period and its exception against `clock_hz`.
    pub fn start(clock_hz: u32) -> Result<(), ClockError> {
        let reload = period(clock_hz)?;
        let driver = registration::driver();
        critical_section::with(|cs| driver.periods.borrow(cs).set(0));
        RELOAD.write(reload);
        CURRENT.write(0);
        CONTROL.write(CONTROL_ENABLE | CONTROL_TICK_INTERRUPT | CONTROL_CLOCK_SOURCE_PROCESSOR);
        Ok(())
    }

    /// Processor cycles `work` takes, counted by `SysTick` with its exception
    /// held off, so `work` must take less than a millisecond.
    pub fn cycles_taken(work: impl FnOnce()) -> u32 {
        critical_section::with(|_| {
            let before = CURRENT.read();
            work();
            let after = CURRENT.read();
            elapsed_cycles(before, after, RELOAD.read())
        })
    }

    /// The `SysTick` exception body: counts the period and wakes expired timers.
    pub fn on_systick() {
        let driver = registration::driver();
        critical_section::with(|cs| {
            let periods = driver.periods.borrow(cs);
            periods.set(periods.get() + 1);
            let now = driver.now_in(cs);
            driver.queue.borrow_ref_mut(cs).next_expiration(now);
        });
    }
}

/// `SysTick` cannot count one millisecond of this clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockError;

/// Returns the reload value for a 1 ms period at `clock_hz`.
///
/// Any clock is accepted: the K5 reports its RCHF corrected by the part's
/// measured deviation, so it is never a whole number of megahertz. The period
/// is `clock_hz / 1000` cycles, which is a millisecond to within one cycle.
pub const fn period(clock_hz: u32) -> Result<u32, ClockError> {
    let cycles = clock_hz / 1_000;
    if cycles < 2 || cycles - 1 > 0x00FF_FFFF {
        return Err(ClockError);
    }
    Ok(cycles - 1)
}

/// Cycles between two readings of the down-counting `SysTick` current value,
/// across at most one reload from `reload` to zero.
pub const fn elapsed_cycles(before: u32, after: u32, reload: u32) -> u32 {
    if after <= before {
        before - after
    } else {
        before + reload + 1 - after
    }
}

/// Milliseconds since start: the counted periods, plus a reload the
/// exception has not counted yet (`pending`).
pub const fn ticks(periods: u64, pending: bool) -> u64 {
    if pending {
        periods + 1
    } else {
        periods
    }
}

#[cfg(test)]
mod tests {
    use super::{period, ticks, ClockError};

    #[test]
    fn the_nominal_k5_clock_gives_a_whole_millisecond_period() {
        assert_eq!(period(48_000_000), Ok(47_999));
    }

    #[test]
    fn a_deviation_corrected_clock_is_accepted() {
        // The K5 reports RCHF corrected by its measured deviation, which is
        // why a whole-megahertz requirement stopped `7.0.10` at boot.
        assert_eq!(period(47_987_456), Ok(47_986));
        assert_eq!(period(48_012_345), Ok(48_011));
    }

    #[test]
    fn clocks_too_slow_for_a_millisecond_period_are_refused() {
        assert_eq!(period(0), Err(ClockError));
        assert_eq!(period(1_999), Err(ClockError));
        // Every u32 clock fits the 24-bit reload, so none is too fast.
        assert_eq!(period(u32::MAX), Ok(4_294_966));
    }

    #[test]
    fn cycles_count_down_and_across_one_reload() {
        assert_eq!(super::elapsed_cycles(40_000, 39_000, 47_999), 1_000);
        // 499 cycles down to zero, one to reload, and 500 more down from the
        // top.
        assert_eq!(super::elapsed_cycles(499, 47_499, 47_999), 1_000);
        assert_eq!(super::elapsed_cycles(7, 7, 47_999), 0);
    }

    #[test]
    fn time_is_the_period_count_plus_an_uncounted_reload() {
        assert_eq!(ticks(0, false), 0);
        assert_eq!(ticks(3, false), 3);
        assert_eq!(ticks(3, true), 4);
    }

    #[test]
    fn time_never_runs_backwards_across_a_reload() {
        let before = ticks(7, false);
        let pending = ticks(7, true);
        let counted = ticks(8, false);
        assert!(before <= pending && pending <= counted);
    }
}
