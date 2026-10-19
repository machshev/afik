//! K5 battery constants for the shared [`radio_platform::battery`] arithmetic.
//!
//! `EVID-K5-034` records where each constant comes from; the pack takes the
//! shared [`LITHIUM_ION_2S`] curve.

use radio_platform::battery::{Battery, LITHIUM_ION_2S};

/// Constructs the K5 battery, uncalibrated and unsampled.
#[must_use]
pub const fn new() -> Battery {
    Battery::new(LITHIUM_ION_2S)
}

#[cfg(test)]
mod tests {
    use radio_platform::battery::{Calibration, CALIBRATION_BYTES, SAMPLES};

    #[test]
    fn a_calibrated_k5_reports_percent_along_the_lithium_ion_curve() {
        let mut block = [0_u8; CALIBRATION_BYTES];
        block[6..8].copy_from_slice(&2_000_u16.to_le_bytes());
        let mut battery = super::new();
        battery.calibrate(Calibration::from_vendor_block(&block));
        // 2000 counts is 7.60 V by definition, which this curve puts at 40 %.
        for _ in 0..SAMPLES {
            battery.sample(2_000);
        }
        assert_eq!(battery.centivolts(), Some(760));
        assert_eq!(battery.percent(), Some(40));
        // 8.28 V and above is full.
        for _ in 0..SAMPLES {
            battery.sample(2_200);
        }
        assert_eq!(battery.percent(), Some(100));
    }
}
