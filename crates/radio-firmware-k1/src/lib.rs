//! Hardware-independent pieces of the K1 target image.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod aux_inputs;
pub mod backlight;
pub mod bk4819_bus;
pub mod channels;
pub mod clock_handoff;
pub mod configuration;
#[cfg(test)]
mod cooperative_progress;
pub mod display;
#[cfg(feature = "embassy-runtime")]
pub mod embassy_runtime;
pub mod keypad;
pub mod protocol;
#[cfg(feature = "py32f071-bk4819")]
pub mod py32f071_bk4819;
#[cfg(feature = "py32f071-clock-handoff")]
pub mod py32f071_clock_handoff;
#[cfg(feature = "py32f071-clock-publication")]
pub mod py32f071_clock_publication;
#[cfg(feature = "py32f071-hal-inventory")]
pub mod py32f071_hal_inventory;
#[cfg(feature = "py32f071-retained")]
pub mod py32f071_retained;
#[cfg(feature = "py32f071-runtime-composition")]
pub mod py32f071_runtime;
#[cfg(feature = "py32f071-runtime-init")]
pub mod py32f071_runtime_init;
#[cfg(feature = "py32f071-spi1")]
pub mod py32f071_spi1;
#[cfg(feature = "py32f071-usart1")]
pub mod py32f071_usart1;
pub mod shell;
