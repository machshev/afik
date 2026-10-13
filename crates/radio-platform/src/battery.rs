//! Battery voltage and remaining charge from a calibrated sense input.
//!
//! Both UV-K5-family radios divide the pack down onto one analogue input, and
//! their own firmware stores the count that input reads at 7.60 V in a
//! six-half-word vendor block. A raw conversion becomes volts only in company
//! with that calibration, and volts become percent along a discharge curve the
//! target supplies. This module owns all three steps and touches no hardware.
//!
//! `EVID-K1-063` and `EVID-K5-064` record where each constant comes from.

/// Raw conversions averaged into one reading.
///
/// The reference firmware keeps a rolling four and averages them, which is
/// enough to stop a single noisy conversion moving the indicator.
pub const SAMPLES: usize = 4;

/// Hundredths of a volt the calibration count corresponds to.
///
/// The stored count is what the input reads at 7.60 V, so a reading is
/// `raw * 760 / calibration` in the same hundredths-of-a-volt unit.
pub const CALIBRATION_CENTIVOLTS: u32 = 760;

/// Bytes the calibration block occupies: six little-endian half-words.
pub const CALIBRATION_BYTES: usize = 12;

/// Index of the half-word holding the 7.60 V count.
const CALIBRATION_INDEX: usize = 3;

/// Smallest calibration count treated as usable.
///
/// A twelve-bit converter cannot read above 4095, and a plausible count for
/// 7.60 V through this divider is a four-figure number. An erased memory reads
/// `0xFFFF`, so a value outside this range means "no calibration" rather than a
/// battery to be reported wrongly.
const CALIBRATION_MINIMUM: u16 = 1_000;
/// Largest calibration count treated as usable, the twelve-bit maximum.
const CALIBRATION_MAXIMUM: u16 = 4_095;

/// A discharge curve: hundredths of a volt and percent, highest voltage first.
pub type Curve = &'static [(u16, u8)];

/// The count this unit's input reads at 7.60 V.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Calibration(u16);

impl Calibration {
    /// Reads the calibration out of the vendor block, if it holds a usable one.
    ///
    /// A memory which did not answer, was erased, or holds something outside
    /// the converter's range yields `None`, and the radio then reports that it
    /// does not know its battery rather than a number derived from nothing.
    #[must_use]
    pub fn from_vendor_block(bytes: &[u8; CALIBRATION_BYTES]) -> Option<Self> {
        let offset = CALIBRATION_INDEX * 2;
        let count = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if !(CALIBRATION_MINIMUM..=CALIBRATION_MAXIMUM).contains(&count) {
            return None;
        }
        Some(Self(count))
    }

    /// Returns the stored count.
    #[must_use]
    pub const fn count(self) -> u16 {
        self.0
    }

    /// Converts one averaged raw reading into hundredths of a volt.
    #[must_use]
    pub fn centivolts(self, raw: u16) -> u16 {
        let scaled = u32::from(raw) * CALIBRATION_CENTIVOLTS / u32::from(self.0);
        u16::try_from(scaled).unwrap_or(u16::MAX)
    }
}

/// What the radio knows about its battery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Battery {
    curve: Curve,
    samples: [u16; SAMPLES],
    filled: usize,
    next: usize,
    calibration: Option<Calibration>,
}

impl Battery {
    /// Constructs a battery which has not been calibrated or sampled.
    #[must_use]
    pub const fn new(curve: Curve) -> Self {
        Self {
            curve,
            samples: [0; SAMPLES],
            filled: 0,
            next: 0,
            calibration: None,
        }
    }

    /// Adopts the calibration read from the vendor block.
    pub fn calibrate(&mut self, calibration: Option<Calibration>) {
        self.calibration = calibration;
    }

    /// Records one raw conversion.
    pub fn sample(&mut self, raw: u16) {
        self.samples[self.next] = raw;
        self.next = (self.next + 1) % SAMPLES;
        self.filled = (self.filled + 1).min(SAMPLES);
    }

    /// Returns the averaged pack voltage in hundredths of a volt.
    ///
    /// This is `None` until the radio has both a calibration and a full set of
    /// conversions, so a part-filled average can never be shown as a reading.
    #[must_use]
    pub fn centivolts(&self) -> Option<u16> {
        if self.filled < SAMPLES {
            return None;
        }
        let calibration = self.calibration?;
        let total: u32 = self.samples.iter().map(|sample| u32::from(*sample)).sum();
        let count = u32::try_from(SAMPLES).unwrap_or(1);
        let average = u16::try_from(total / count).unwrap_or(u16::MAX);
        Some(calibration.centivolts(average))
    }

    /// Returns the remaining charge as a percentage.
    #[must_use]
    pub fn percent(&self) -> Option<u8> {
        self.centivolts()
            .map(|centivolts| percent_from_centivolts(self.curve, centivolts))
    }

    /// Reports whether the pack has reached the bottom of the curve.
    #[must_use]
    pub fn is_critical(&self) -> bool {
        let floor = self.curve.last().map_or(0, |point| point.0);
        self.centivolts()
            .is_some_and(|centivolts| centivolts <= floor)
    }
}

/// Converts a pack voltage into percent along a discharge curve.
///
/// The curve is a small number of measured points and the space between them is
/// linear, which is what the reference firmware does. A pack above the top
/// point reads full rather than over full, and one below the bottom point reads
/// empty: a battery is not a place to extrapolate.
#[must_use]
pub fn percent_from_centivolts(curve: Curve, centivolts: u16) -> u8 {
    for window in curve.windows(2) {
        let (upper_volts, upper_percent) = window[0];
        let (lower_volts, lower_percent) = window[1];
        if centivolts <= lower_volts {
            continue;
        }
        if centivolts >= upper_volts {
            return upper_percent;
        }
        let span = u32::from(upper_volts - lower_volts);
        let rise = u32::from(upper_percent - lower_percent);
        let above = u32::from(centivolts - lower_volts);
        let percent = u32::from(lower_percent) + above * rise / span;
        return u8::try_from(percent.min(100)).unwrap_or(100);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{percent_from_centivolts, Battery, Calibration, Curve, CALIBRATION_BYTES, SAMPLES};

    const CURVE: Curve = &[(828, 100), (813, 97), (758, 25), (726, 6), (630, 0)];

    fn block(count: u16) -> [u8; CALIBRATION_BYTES] {
        let mut bytes = [0_u8; CALIBRATION_BYTES];
        bytes[6..8].copy_from_slice(&count.to_le_bytes());
        bytes
    }

    #[test]
    fn the_calibration_is_the_fourth_half_word_and_is_range_checked() {
        assert_eq!(
            Calibration::from_vendor_block(&block(2_200))
                .expect("usable")
                .count(),
            2_200
        );
        assert_eq!(
            Calibration::from_vendor_block(&[0xFF; CALIBRATION_BYTES]),
            None,
            "an erased memory is not a calibration"
        );
        assert_eq!(
            Calibration::from_vendor_block(&block(0)),
            None,
            "a memory which answered with zeroes is not a calibration"
        );
        assert_eq!(
            Calibration::from_vendor_block(&block(4_096)),
            None,
            "a twelve-bit converter cannot have read this"
        );
    }

    /// The reading is only as good as its calibration, so it waits for one.
    #[test]
    fn a_reading_needs_both_a_calibration_and_a_full_set_of_samples() {
        let mut battery = Battery::new(CURVE);
        for _ in 0..SAMPLES {
            battery.sample(2_200);
        }
        assert_eq!(battery.centivolts(), None, "no calibration, no volts");
        assert_eq!(battery.percent(), None);
        assert!(!battery.is_critical(), "an unknown battery is not critical");

        let mut battery = Battery::new(CURVE);
        battery.calibrate(Calibration::from_vendor_block(&block(2_200)));
        battery.sample(2_200);
        assert_eq!(
            battery.centivolts(),
            None,
            "a part-filled average is not a reading"
        );
        for _ in 1..SAMPLES {
            battery.sample(2_200);
        }
        assert_eq!(
            battery.centivolts(),
            Some(760),
            "the calibration count is 7.60 V by definition"
        );
    }

    #[test]
    fn the_average_follows_the_last_samples_taken() {
        let mut battery = Battery::new(CURVE);
        battery.calibrate(Calibration::from_vendor_block(&block(2_000)));
        for _ in 0..SAMPLES {
            battery.sample(2_000);
        }
        assert_eq!(battery.centivolts(), Some(760));

        // A fully discharged pack replaces the ring rather than being averaged
        // with a charged one forever.
        for _ in 0..SAMPLES {
            battery.sample(1_650);
        }
        assert_eq!(battery.centivolts(), Some(627));
        assert!(battery.is_critical());
        assert_eq!(battery.percent(), Some(0));
    }

    #[test]
    fn the_curve_reads_full_at_the_top_empty_at_the_bottom_and_between_them() {
        assert_eq!(percent_from_centivolts(CURVE, 900), 100, "above is full");
        assert_eq!(percent_from_centivolts(CURVE, 828), 100);
        assert_eq!(percent_from_centivolts(CURVE, 813), 97);
        assert_eq!(percent_from_centivolts(CURVE, 758), 25);
        assert_eq!(percent_from_centivolts(CURVE, 726), 6);
        assert_eq!(percent_from_centivolts(CURVE, 630), 0);
        assert_eq!(percent_from_centivolts(CURVE, 500), 0, "below is empty");

        let mut previous = 0;
        for centivolts in 600..=900 {
            let percent = percent_from_centivolts(CURVE, centivolts);
            assert!(percent >= previous, "charge cannot fall as voltage rises");
            previous = percent;
        }
    }
}
