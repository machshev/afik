//! Guarded publication of the validated K1 bootloader-provided clock tree.

use py32_hal::rcc::Clocks;
use py32_hal::time::Hertz;

use crate::clock_handoff::{validate, ClockHandoffError, InheritedClocks};
use crate::py32f071_clock_handoff::snapshot;

/// Validates the live RCC state and publishes its frequencies to the HAL.
///
/// This reads RCC and updates only the HAL's software clock table. It does not
/// write RCC, take peripheral tokens, or initialize any driver. Call it once at
/// startup, before constructing drivers that query their peripheral clock.
pub fn publish() -> Result<InheritedClocks, ClockHandoffError> {
    let inherited = validate(snapshot())?;
    let clocks = hal_clocks(inherited);

    // SAFETY: `InheritedClocks` cannot be constructed outside the pure,
    // fail-closed validator. This optional boundary is not called by the
    // current entry point and must run once before any HAL driver construction.
    #[allow(unsafe_code)]
    unsafe {
        py32_hal::rcc::publish_inherited_freqs(clocks);
    }

    Ok(inherited)
}

fn hal_clocks(inherited: InheritedClocks) -> Clocks {
    Clocks {
        hclk1: Some(Hertz(inherited.hclk1_hz())).into(),
        pclk1: Some(Hertz(inherited.pclk1_hz())).into(),
        pclk1_tim: Some(Hertz(inherited.pclk1_tim_hz())).into(),
        sys: Some(Hertz(inherited.sys_hz())).into(),
        hsi: Some(Hertz(16_000_000)).into(),
        lse: None.into(),
        pll: Some(Hertz(inherited.sys_hz())).into(),
    }
}
