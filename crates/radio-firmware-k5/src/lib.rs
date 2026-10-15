//! UV-K5 V1 (DP32G030) drivers for the shared AFIK application.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod audio;
pub mod battery;
pub mod bk4819_bus;
pub mod broadcast;
pub mod display;
pub mod drivers;
pub mod eeprom;
pub mod keypad;
pub mod memory;
