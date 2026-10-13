//! Hardware-independent contracts shared by AFIK target applications.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod configuration;
pub mod configuration_change;
pub mod display;
pub mod operator_display;
pub mod operator_shell;
pub mod receive_app;
pub mod serial;
pub mod serial_control;
pub mod tx_control;
