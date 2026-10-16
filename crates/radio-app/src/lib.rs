//! The AFIK radio application, shared by every target.
//!
//! Two tasks own disjoint hardware. [`serial::run`] owns the programming link
//! and the memory and answers the stock memory protocol (`ADR-100`).
//! [`interface::run`] owns the display, the keypad, the radio chip, and the
//! speaker, and runs the operator shell, receive, scanning, and TX. A target
//! supplies [`drivers`] and an executor that starts both.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod configuration;
pub mod drivers;
pub mod interface;
pub mod keys;
pub mod operator_state;
pub mod serial;
mod state;

pub use state::TaskCell;
