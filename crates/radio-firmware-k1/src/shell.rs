//! K1's operator shell is the hardware-independent shared application.
//!
//! The target keeps this module as a named adapter so existing K1 call sites
//! remain readable while the state machine, key semantics, and tests live in
//! `radio-platform` and are also used by K5.

pub use radio_platform::operator_shell::*;
