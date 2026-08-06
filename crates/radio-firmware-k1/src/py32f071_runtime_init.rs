//! Guarded initialization around the K1 bootloader-provided clock tree.

use crate::clock_handoff::{ClockHandoffError, InheritedClocks};

/// HAL tokens and the validated frequencies adopted at startup.
pub struct K1InheritedRuntime {
    /// HAL-owned singleton peripheral tokens.
    pub peripherals: py32_hal::Peripherals,
    /// Exact frequencies validated before any driver was initialized.
    pub clocks: InheritedClocks,
}

/// Validates and publishes inherited clocks, then initializes GPIO, DMA, and
/// the reserved TIM15 Embassy time driver without reconfiguring the clock tree.
pub fn init() -> Result<K1InheritedRuntime, ClockHandoffError> {
    let clocks = crate::py32f071_clock_publication::publish()?;

    // SAFETY: publication immediately above succeeded from the live,
    // fail-closed RCC snapshot. This wrapper owns the required ordering and
    // returns the singleton tokens, preventing a second successful init.
    #[allow(unsafe_code)]
    let peripherals = unsafe { py32_hal::init_inherited(py32_hal::Config::default()) };

    Ok(K1InheritedRuntime {
        peripherals,
        clocks,
    })
}
