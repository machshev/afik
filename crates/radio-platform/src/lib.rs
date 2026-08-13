//! Hardware-independent contracts shared by AFIK target applications.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod display;
pub mod receive_app;
pub mod serial;
