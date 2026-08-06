//! Read-only PY32F071 RCC snapshot for the inherited K1 clock contract.

use py32_hal::pac::RCC;

use crate::clock_handoff::{snapshot_from_registers, ClockSnapshot};

/// Reads the RCC fields required by the fail-closed clock handoff.
///
/// This performs no register write and does not publish clocks to the HAL.
#[must_use]
pub fn snapshot() -> ClockSnapshot {
    let cr = RCC.cr().read();
    let icscr = RCC.icscr().read();
    let cfgr = RCC.cfgr().read();
    let pllcfgr = RCC.pllcfgr().read();

    snapshot_from_registers(cr.0, icscr.0, cfgr.0, pllcfgr.0)
}
