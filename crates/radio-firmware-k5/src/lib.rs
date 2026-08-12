//! Hardware-independent pieces of the UV-K5 V1 target image.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod boot_display;
pub mod protocol;
