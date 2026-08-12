//! DP32G030 register drivers for the UV-K5 V1 target.
//!
//! Every address and field here is copied from the DP32G030 reference manual
//! and recorded in `docs/hardware-evidence.md` as `EVID-DP32-004` to
//! `EVID-DP32-009`. Board wiring, which the manual cannot supply, is recorded
//! separately as `EVID-K5-019`.
//!
//! The crate is deliberately small: it drives what an image needs and models
//! nothing it has not been given evidence for. Arithmetic and field encodings
//! are pure functions so they can be tested on the host, and the only
//! memory-mapped access lives in [`mmio`].

#![no_std]

#[cfg(test)]
extern crate std;

pub mod clock;
pub mod gpio;
pub mod mmio;
pub mod portcon;
pub mod spi;
pub mod syscon;
pub mod uart;

/// SYSCON base address, per `EVID-DP32-004`.
pub const SYSCON_BASE: u32 = 0x4000_0000;
/// PMU base address, per `EVID-DP32-004`.
pub const PMU_BASE: u32 = 0x4000_0800;
/// GPIOA base address, per `EVID-DP32-004`.
pub const GPIOA_BASE: u32 = 0x4006_0000;
/// GPIOB base address, per `EVID-DP32-004`.
pub const GPIOB_BASE: u32 = 0x4006_0800;
/// GPIOC base address, per `EVID-DP32-004`.
pub const GPIOC_BASE: u32 = 0x4006_1000;
/// UART0 base address, per `EVID-DP32-004`.
pub const UART0_BASE: u32 = 0x4006_B000;
/// UART1 base address, per `EVID-DP32-004`.
pub const UART1_BASE: u32 = 0x4006_B800;
/// UART2 base address, per `EVID-DP32-004`.
pub const UART2_BASE: u32 = 0x4006_C000;
/// PORTCON base address, per `EVID-DP32-004`.
pub const PORTCON_BASE: u32 = 0x400B_0000;
/// SPI0 base address, per `EVID-DP32-010`.
pub const SPI0_BASE: u32 = 0x400B_8000;
