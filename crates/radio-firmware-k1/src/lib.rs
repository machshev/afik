//! Hardware-independent pieces of the K1 target image.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod display;
pub mod protocol;
