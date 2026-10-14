//! Embassy time driver on the Cortex-M0 `SysTick`, per `EVID-ARM-003`.
//!
//! `SysTick` reloads every millisecond and its exception counts the periods.
//! The time between periods comes from the running counter, so `now()` has
//! microsecond resolution while a timer wakes within a millisecond of its
//! deadline. Expired timers are woken from the exception.
//!
//! The image must route the `SysTick` vector to [`on_systick`] and call
//! [`start`] once, after the clock is configured and before anything awaits.

const MICROS_PER_PERIOD: u64 = 1_000;

#[cfg(feature = "time-driver")]
pub use driver::{on_systick, start};

#[cfg(feature = "time-driver")]
mod driver {
    use core::cell::{Cell, RefCell};
    use core::task::Waker;

    use critical_section::Mutex;
    use embassy_time_driver::Driver;
    use embassy_time_queue_utils::Queue;

    use super::{micros, period, ClockError};
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
        reload: Mutex<Cell<u32>>,
        cycles_per_micro: Mutex<Cell<u32>>,
        queue: Mutex<RefCell<Queue>>,
    }

    impl SysTickDriver {
        fn now_in(&self, cs: critical_section::CriticalSection<'_>) -> u64 {
            let reload = self.reload.borrow(cs).get();
            let mut current = CURRENT.read();
            let pending = INTERRUPT_CONTROL.read() & PENDSTSET != 0;
            if pending {
                // The counter may have been read just before the reload; read it
                // again so it belongs to the period the pending flag adds.
                current = CURRENT.read();
            }
            micros(
                self.periods.borrow(cs).get(),
                pending,
                reload,
                current,
                self.cycles_per_micro.borrow(cs).get(),
            )
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
            reload: critical_section::Mutex::new(core::cell::Cell::new(0)),
            cycles_per_micro: critical_section::Mutex::new(core::cell::Cell::new(1)),
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
        let (reload, cycles_per_micro) = period(clock_hz)?;
        let driver = registration::driver();
        critical_section::with(|cs| {
            driver.reload.borrow(cs).set(reload);
            driver.cycles_per_micro.borrow(cs).set(cycles_per_micro);
            driver.periods.borrow(cs).set(0);
        });
        RELOAD.write(reload);
        CURRENT.write(0);
        CONTROL.write(CONTROL_ENABLE | CONTROL_TICK_INTERRUPT | CONTROL_CLOCK_SOURCE_PROCESSOR);
        Ok(())
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

/// `SysTick` cannot divide this clock into whole-microsecond millisecond periods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockError;

/// Returns the reload value and cycles per microsecond for a 1 ms period.
pub const fn period(clock_hz: u32) -> Result<(u32, u32), ClockError> {
    let cycles = clock_hz / 1_000;
    let cycles_per_micro = clock_hz / 1_000_000;
    if cycles_per_micro == 0 || clock_hz % 1_000_000 != 0 || cycles == 0 || cycles - 1 > 0x00FF_FFFF
    {
        return Err(ClockError);
    }
    Ok((cycles - 1, cycles_per_micro))
}

/// Microseconds since start from the counted periods and the running counter.
///
/// `pending` is a reload the exception has not counted yet.
pub const fn micros(
    periods: u64,
    pending: bool,
    reload: u32,
    current: u32,
    cycles_per_micro: u32,
) -> u64 {
    let periods = if pending { periods + 1 } else { periods };
    let elapsed = reload.saturating_sub(current) / cycles_per_micro;
    periods * MICROS_PER_PERIOD + elapsed as u64
}

#[cfg(test)]
mod tests {
    use super::{micros, period, ClockError};

    #[test]
    fn the_k5_clock_gives_a_whole_millisecond_period() {
        assert_eq!(period(48_000_000), Ok((47_999, 48)));
    }

    #[test]
    fn clocks_without_whole_microseconds_are_refused() {
        assert_eq!(period(0), Err(ClockError));
        assert_eq!(period(999_999), Err(ClockError));
        assert_eq!(period(48_000_500), Err(ClockError));
    }

    #[test]
    fn time_is_periods_plus_the_counter_position() {
        // Counter just reloaded: no time into the period yet.
        assert_eq!(micros(0, false, 47_999, 47_999, 48), 0);
        assert_eq!(micros(3, false, 47_999, 47_999 - 480, 48), 3_010);
        assert_eq!(micros(3, false, 47_999, 0, 48), 3_999);
    }

    #[test]
    fn an_uncounted_reload_adds_its_period() {
        assert_eq!(micros(3, true, 47_999, 47_999 - 48, 48), 4_001);
    }

    #[test]
    fn time_never_runs_backwards_across_a_reload() {
        let before = micros(7, false, 47_999, 0, 48);
        let pending = micros(7, true, 47_999, 47_998, 48);
        let counted = micros(8, false, 47_999, 47_990, 48);
        assert!(before <= pending && pending <= counted);
    }
}
