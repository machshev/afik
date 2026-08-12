//! Hardware-independent pieces of the UV-K5 V1 target image.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod audio;
pub mod bk4819_bus;
pub mod eeprom;
pub mod k5_display;
pub mod keypad;
pub mod protocol;
