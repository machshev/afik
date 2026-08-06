//! Read-only PY32F071 RCC snapshot for the inherited K1 clock contract.

use py32_hal::pac::RCC;

use crate::clock_handoff::{ClockSnapshot, ClockSourceState};

/// Reads the RCC fields required by the fail-closed clock handoff.
///
/// This performs no register write and does not publish clocks to the HAL.
#[must_use]
pub fn snapshot() -> ClockSnapshot {
    let cr = RCC.cr().read();
    let icscr = RCC.icscr().read();
    let cfgr = RCC.cfgr().read();
    let pllcfgr = RCC.pllcfgr().read();

    ClockSnapshot {
        hsi: ClockSourceState::from_flags(cr.hsion(), cr.hsirdy()),
        hsi_frequency: icscr.hsi_fs().to_bits(),
        pll: ClockSourceState::from_flags(cr.pllon(), cr.pllrdy()),
        pll_source: pllcfgr.pllsrc().to_bits(),
        system_source: cfgr.sw().to_bits(),
        active_system_source: cfgr.sws().to_bits(),
        ahb_prescaler: cfgr.hpre().to_bits(),
        apb_prescaler: cfgr.ppre().to_bits(),
    }
}
