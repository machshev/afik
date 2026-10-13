//! K1 battery constants for the shared [`radio_platform::battery`] arithmetic.
//!
//! `EVID-K1-063` records where each constant comes from. Nothing here is
//! invented: the calibration location and the curve are both read out of the
//! pinned firmware.

use radio_platform::battery::Curve;
pub use radio_platform::battery::{Battery, Calibration, CALIBRATION_BYTES, SAMPLES};

/// Address of the six calibration half-words in the external memory.
///
/// This is the radio's own data, below anything AFIK claims, and is read
/// without a region because it is never written.
pub const CALIBRATION_ADDRESS: u32 = 0x0001_0140;

/// Discharge curve for the stock K1 pack, in hundredths of a volt and percent.
///
/// The pinned firmware carries one of these per battery type. AFIK does not
/// know which pack this radio has, so it uses the 1500 mAh curve the pinned
/// source records for the K1 and does not offer a choice it cannot verify. The
/// two K1 curves agree within a few percent over most of the range.
pub const CURVE: Curve = &[(828, 100), (813, 97), (758, 25), (726, 6), (630, 0)];

/// Constructs the K1 battery, uncalibrated and unsampled.
#[must_use]
pub const fn new() -> Battery {
    Battery::new(CURVE)
}

#[cfg(test)]
mod tests {
    use radio_platform::battery::percent_from_centivolts;

    #[test]
    fn the_k1_curve_matches_the_pinned_1500_mah_entry() {
        assert_eq!(percent_from_centivolts(super::CURVE, 828), 100);
        assert_eq!(percent_from_centivolts(super::CURVE, 813), 97);
        assert_eq!(percent_from_centivolts(super::CURVE, 758), 25);
        assert_eq!(percent_from_centivolts(super::CURVE, 726), 6);
        assert_eq!(percent_from_centivolts(super::CURVE, 630), 0);
    }
}
