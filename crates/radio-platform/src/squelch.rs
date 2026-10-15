//! The unit's own squelch calibration, as the reference reads it.
//!
//! `RADIO_ConfigureSquelchAndOutputPower` reads, for squelch levels 1 to 9,
//! six rows of thresholds from the vendor calibration: RSSI, noise and glitch,
//! open and close. One table serves 174 MHz and up, another the bands below,
//! at `0x1E00` and `0x1E60` (the K1's memory view maps the same offsets onto
//! its vendor data). Noise and glitch thresholds are what shut the squelch on
//! a receiver's own noise; carrier strength alone cannot, once RX enable
//! makes the front end live (`7.0.35` on the K1: open on S8 noise).
//!
//! The stock thresholds are used as read. Armel's build halves the open RSSI
//! and doubles the open noise and glitch (`ENABLE_SQUELCH_MORE_SENSITIVE`),
//! which inverts their hysteresis; AFIK does not take that on trust.

use radio_bk4819::SquelchThresholds;
use radio_domain::SquelchLevel;

/// Where the tables start in the radio's memory view.
pub const SQUELCH_CALIBRATION_ADDRESS: u16 = 0x1E00;
/// Both tables: six rows of sixteen bytes each.
pub const SQUELCH_CALIBRATION_BYTES: usize = 0xC0;
/// Offset of the table for below 174 MHz.
const LOW_BAND_TABLE: usize = 0x60;
/// Bytes between one threshold's row and the next.
const ROW: usize = 0x10;
/// The reference's `BAND4_174MHz` lower edge: the high-band table from here.
const HIGH_BAND_FROM_HZ: u32 = 174_000_000;

/// The unit's squelch calibration, as read from its memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SquelchCalibration([u8; SQUELCH_CALIBRATION_BYTES]);

impl SquelchCalibration {
    /// Wraps the bytes read at [`SQUELCH_CALIBRATION_ADDRESS`].
    #[must_use]
    pub const fn new(bytes: [u8; SQUELCH_CALIBRATION_BYTES]) -> Self {
        Self(bytes)
    }

    /// The thresholds for `level` at `frequency_hz`: none for level zero,
    /// which the caller leaves open, or when this unit's rows for the level
    /// are not a usable set.
    #[must_use]
    pub fn thresholds(&self, frequency_hz: u32, level: SquelchLevel) -> Option<SquelchThresholds> {
        if level.is_open() {
            return None;
        }
        let table = if frequency_hz < HIGH_BAND_FROM_HZ {
            LOW_BAND_TABLE
        } else {
            0
        };
        let at = table + usize::from(level.get());
        let mut rows = [0_u8; 6];
        for (row, value) in rows.iter_mut().enumerate() {
            *value = *self.0.get(at + row * ROW)?;
        }
        SquelchThresholds::from_calibration(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::{SquelchCalibration, SQUELCH_CALIBRATION_BYTES};
    use radio_bk4819::SquelchThresholds;
    use radio_domain::SquelchLevel;

    /// The K1's tables from its 2026-10-14 backup: high band rows, then low.
    const K1: [[u8; 10]; 12] = [
        [10, 94, 96, 98, 100, 102, 104, 106, 108, 110],
        [5, 92, 93, 95, 97, 99, 101, 103, 105, 107],
        [90, 45, 41, 38, 35, 32, 29, 26, 23, 20],
        [100, 48, 45, 41, 38, 35, 32, 29, 26, 23],
        [90, 20, 17, 14, 11, 8, 3, 2, 2, 2],
        [100, 17, 14, 11, 8, 5, 5, 4, 4, 4],
        [50, 107, 109, 111, 113, 115, 117, 119, 121, 123],
        [40, 104, 106, 108, 110, 112, 114, 116, 118, 120],
        [65, 50, 45, 40, 36, 33, 30, 26, 23, 22],
        [70, 55, 50, 45, 40, 37, 34, 30, 27, 25],
        [90, 25, 15, 10, 9, 8, 7, 6, 5, 4],
        [100, 30, 20, 15, 13, 12, 11, 10, 9, 8],
    ];

    fn k1() -> SquelchCalibration {
        let mut bytes = [0xFF_u8; SQUELCH_CALIBRATION_BYTES];
        for (row, values) in K1.iter().enumerate() {
            bytes[row * 0x10..row * 0x10 + 10].copy_from_slice(values);
        }
        SquelchCalibration::new(bytes)
    }

    fn level(level: u8) -> SquelchLevel {
        SquelchLevel::new(level).unwrap()
    }

    #[test]
    fn a_level_reads_its_column_of_the_band_table() {
        let calibration = k1();
        // PMR446, SQL 3: RSSI 98/95, noise 38/41, glitch 11/14.
        assert_eq!(
            calibration.thresholds(446_006_250, level(3)),
            Some(SquelchThresholds::new(98, 95, 38, 41, 11, 14).unwrap())
        );
        // 2 m, SQL 9, from the low-band table, whose glitch pair opens
        // above where it closes: taken as the factory wrote it.
        assert_eq!(
            calibration.thresholds(145_500_000, level(9)),
            SquelchThresholds::from_calibration([123, 120, 22, 25, 4, 8])
        );
        assert!(calibration.thresholds(145_500_000, level(9)).is_some());
        // 174 MHz is the first high-band frequency.
        assert_eq!(
            calibration.thresholds(174_000_000, level(1)),
            Some(SquelchThresholds::new(94, 92, 45, 48, 17, 20).unwrap())
        );
    }

    #[test]
    fn level_zero_and_erased_rows_give_no_thresholds() {
        assert_eq!(k1().thresholds(446_006_250, SquelchLevel::OPEN), None);
        let erased = SquelchCalibration::new([0xFF; SQUELCH_CALIBRATION_BYTES]);
        assert_eq!(erased.thresholds(446_006_250, level(3)), None);
    }
}
