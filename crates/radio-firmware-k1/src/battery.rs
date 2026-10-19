//! K1 battery constants for the shared [`radio_platform::battery`] arithmetic.
//!
//! `EVID-K1-063` records where each constant comes from: the calibration
//! location is read out of the pinned firmware, and the curve is a lithium-ion
//! model fitted to that firmware's measured full and empty points.

use radio_platform::battery::Curve;
pub use radio_platform::battery::{Battery, Calibration, CALIBRATION_BYTES, SAMPLES};

/// Address of the six calibration half-words in the external memory.
///
/// This is the radio's own data, below anything AFIK claims, and is read
/// without a region because it is never written.
pub const CALIBRATION_ADDRESS: u32 = 0x0001_0140;

/// Discharge curve for the stock 1400 mAh K1 pack, in hundredths of a volt
/// and percent.
///
/// The pinned firmware's entry for this pack (`{828,100} {813,97} {758,25}
/// {726,6} {630,0}`) is marked estimated and is nearly a straight line, so it
/// read low through the long flat middle of a lithium-ion discharge. This is
/// instead the usual LCO/NMC lithium-ion open-circuit voltage against charge,
/// doubled for the two cells in series. The top point is the 8.28 V the
/// pinned source measured on a charged K1 rather than 8.40 V, so a pack just
/// off charge reads full, and the bottom keeps the reference 6.30 V floor.
/// `EVID-K1-063` records it; it is a model, not a measurement of this pack.
pub const CURVE: Curve = &[
    (828, 100),
    (822, 90),
    (804, 80),
    (790, 70),
    (774, 60),
    (768, 50),
    (760, 40),
    (754, 30),
    (746, 20),
    (738, 10),
    (722, 5),
    (630, 0),
];

/// Constructs the K1 battery, uncalibrated and unsampled.
#[must_use]
pub const fn new() -> Battery {
    Battery::new(CURVE)
}

#[cfg(test)]
mod tests {
    use radio_platform::battery::percent_from_centivolts;

    #[test]
    fn the_k1_curve_follows_a_two_cell_lithium_ion_discharge() {
        let percent = |centivolts| percent_from_centivolts(super::CURVE, centivolts);
        assert_eq!(percent(840), 100, "off charge is full");
        assert_eq!(percent(828), 100);
        // The flat middle: 3.84 V a cell is half charge, not a quarter.
        assert_eq!(percent(768), 50);
        assert_eq!(percent(758), 36);
        assert_eq!(percent(738), 10);
        assert_eq!(percent(630), 0, "the reference floor is empty");
        assert_eq!(percent(600), 0);

        let mut previous = 0;
        for centivolts in 600..=850 {
            let now = percent(centivolts);
            assert!(now >= previous, "charge cannot fall as voltage rises");
            previous = now;
        }
    }
}
