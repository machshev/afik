//! Hardware-independent pieces of the K1 target image.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod backlight;
pub mod display;
#[cfg(feature = "embassy-runtime")]
pub mod embassy_runtime;
pub mod keypad;
pub mod protocol;
#[cfg(feature = "py32f071-hal-inventory")]
pub mod py32f071_hal_inventory;
#[cfg(feature = "py32f071-usart1")]
pub mod py32f071_usart1;
