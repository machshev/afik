//! K5 battery constants for the shared [`radio_platform::battery`] arithmetic.
//!
//! `EVID-K5-034` records where each constant comes from.

use radio_platform::battery::{Battery, Curve};

/// Discharge curve for the stock 1600 mAh K5 pack, in hundredths of a volt
/// and percent, copied from the reference firmware's default battery type.
pub const CURVE: Curve = &[(828, 100), (814, 97), (760, 25), (729, 6), (630, 0)];

/// Constructs the K5 battery, uncalibrated and unsampled.
#[must_use]
pub const fn new() -> Battery {
    Battery::new(CURVE)
}

#[cfg(test)]
mod tests {
    use radio_platform::battery::{Calibration, CALIBRATION_BYTES, SAMPLES};

    #[test]
    fn a_calibrated_k5_reports_percent_along_the_1600_mah_curve() {
        let mut block = [0_u8; CALIBRATION_BYTES];
        block[6..8].copy_from_slice(&2_000_u16.to_le_bytes());
        let mut battery = super::new();
        battery.calibrate(Calibration::from_vendor_block(&block));
        // 2000 counts is 7.60 V by definition, which this curve puts at 25 %.
        for _ in 0..SAMPLES {
            battery.sample(2_000);
        }
        assert_eq!(battery.centivolts(), Some(760));
        assert_eq!(battery.percent(), Some(25));
        // 8.28 V and above is full.
        for _ in 0..SAMPLES {
            battery.sample(2_200);
        }
        assert_eq!(battery.percent(), Some(100));
    }
}
