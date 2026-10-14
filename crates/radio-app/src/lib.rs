//! The AFIK radio application, shared by every target.
//!
//! Two tasks own disjoint hardware. [`serial::run`] owns the programming link
//! and the retained memory and runs the shared configuration protocol.
//! [`interface::run`] owns the display, the keypad, the radio chip, and the
//! speaker, and runs the operator shell, receive, scanning, and TX. A target
//! supplies [`drivers`] and an executor that starts both.

#![no_std]

#[cfg(test)]
extern crate std;

mod common;
pub mod configuration;
pub mod drivers;
pub mod host_control;
pub mod interface;
pub mod keys;
pub mod operator_state;
pub mod serial;
mod state;
pub mod unified_receive;

pub use common::apply_common_request;
pub use state::{set_peripheral_clock_khz, set_reset_cause};
