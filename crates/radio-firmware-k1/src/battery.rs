//! K1 battery constants for the shared [`radio_platform::battery`] arithmetic.
//!
//! `EVID-K1-063` records where each constant comes from. The calibration
//! location is read out of the pinned firmware; the 1400 mAh pack takes the
//! shared [`LITHIUM_ION_2S`] curve.

use radio_platform::battery::LITHIUM_ION_2S;
pub use radio_platform::battery::{Battery, Calibration, CALIBRATION_BYTES, SAMPLES};

/// Address of the six calibration half-words in the external memory.
///
/// This is the radio's own data, below anything AFIK claims, and is read
/// without a region because it is never written.
pub const CALIBRATION_ADDRESS: u32 = 0x0001_0140;

/// Constructs the K1 battery, uncalibrated and unsampled.
#[must_use]
pub const fn new() -> Battery {
    Battery::new(LITHIUM_ION_2S)
}
