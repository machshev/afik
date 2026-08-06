//! Minimal HAL-independent Embassy executor boundary.

use embassy_executor::Executor;

/// Constructs the heap-free Cortex-M thread executor selected by the crate.
///
/// Running it remains a target entry-point responsibility; this module touches
/// no PY32 peripheral, interrupt, clock, timer, or linker state.
#[must_use]
pub fn executor() -> Executor {
    Executor::new()
}
