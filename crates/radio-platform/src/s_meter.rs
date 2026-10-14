//! Calibrated signal meter: the chip's RSSI count corrected for the front
//! end's band, then read on the IARU Region 1 VHF/UHF S scale.
//!
//! The chip's count reads `dBm = count / 2 - 160` (`EVID-BK4819-053`), which
//! is what the BK4819 sees after the radio's own filters and LNA. Each band's
//! front end shifts that reading by its own amount; the correction table is
//! egzumer's `dBmCorrTable`, indexed by egzumer's band edges
//! (`EVID-SMETER-065-001`). Above 30 MHz IARU Region 1 Technical
//! Recommendation R.1 puts S9 at -93 dBm and S1 at -141 dBm, 6 dB per unit,
//! with readings above S9 in decibels over S9.

/// Carrier strength at S9 above 30 MHz, in dBm.
pub const S9_DBM: i16 = -93;
/// Decibels in one S unit.
pub const S_UNIT_DB: i16 = 6;
/// Highest reading over S9 the meter shows, in decibels.
pub const MAX_OVER_S9_DB: u8 = 60;

/// Lower band edges in hertz and the dBm correction each band's front end
/// needs, from egzumer's `frequencyBandTable` and `dBmCorrTable`. A frequency
/// takes the last band whose lower edge it reaches; anything below the
/// second edge takes the first band's correction, as `FREQUENCY_GetBand`
/// does.
const BAND_CORRECTIONS: [(u32, i16); 7] = [
    (0, -15),
    (108_000_000, -25),
    (137_000_000, -20),
    (174_000_000, -4),
    (350_000_000, -7),
    (400_000_000, -6),
    (470_000_000, -1),
];

/// The front end's correction at `frequency_hz`, in decibels.
#[must_use]
pub fn band_correction_db(frequency_hz: u32) -> i16 {
    BAND_CORRECTIONS
        .iter()
        .rev()
        .find(|(lower, _)| frequency_hz >= *lower)
        .map_or(BAND_CORRECTIONS[0].1, |(_, correction)| *correction)
}

/// The carrier at the antenna in whole dBm, from the chip's raw RSSI count
/// received on `frequency_hz`.
#[must_use]
pub fn carrier_dbm(rssi_raw: u16, frequency_hz: u32) -> i16 {
    let chip = i16::try_from(rssi_raw.min(0x01FF)).unwrap_or(0) / 2 - 160;
    chip + band_correction_db(frequency_hz)
}

/// One meter reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SReading {
    /// The carrier in dBm.
    pub dbm: i16,
    /// S units, 0 below S1 to 9.
    pub s_units: u8,
    /// Decibels over S9, whole tens, up to [`MAX_OVER_S9_DB`]; zero at or
    /// below S9.
    pub over_s9_db: u8,
}

impl SReading {
    /// Reads `dbm` on the S scale.
    #[must_use]
    pub fn from_dbm(dbm: i16) -> Self {
        let above_s0 = dbm - (S9_DBM - 9 * S_UNIT_DB);
        let s_units = u8::try_from((above_s0 / S_UNIT_DB).clamp(0, 9)).unwrap_or(0);
        let over = (dbm - S9_DBM).clamp(0, i16::from(MAX_OVER_S9_DB));
        let over_s9_db = u8::try_from(over / 10 * 10).unwrap_or(0);
        Self {
            dbm,
            s_units,
            over_s9_db,
        }
    }

    /// Reads the chip's raw RSSI count received on `frequency_hz`.
    #[must_use]
    pub fn from_rssi(rssi_raw: u16, frequency_hz: u32) -> Self {
        Self::from_dbm(carrier_dbm(rssi_raw, frequency_hz))
    }

    /// Meter segments lit: one per S unit, then one per 10 dB over S9.
    #[must_use]
    pub fn segments(self) -> u8 {
        self.s_units + self.over_s9_db / 10
    }

    /// The reading as `S7`, or `S9+20` over S9, padded to five bytes.
    #[must_use]
    pub fn label(self) -> [u8; 5] {
        let mut label = *b"S0   ";
        label[1] = b'0' + self.s_units;
        if self.over_s9_db > 0 {
            label[2] = b'+';
            label[3] = b'0' + self.over_s9_db / 10;
            label[4] = b'0';
        }
        label
    }
}

/// Segments in a full-scale meter.
pub const SEGMENTS: u8 = 9 + MAX_OVER_S9_DB / 10;

#[cfg(test)]
mod tests {
    use super::{band_correction_db, carrier_dbm, SReading};

    #[test]
    fn every_band_takes_its_own_correction_from_its_lower_edge() {
        assert_eq!(band_correction_db(50_000_000), -15);
        assert_eq!(band_correction_db(107_999_999), -15);
        assert_eq!(band_correction_db(108_000_000), -25);
        assert_eq!(band_correction_db(145_500_000), -20);
        assert_eq!(band_correction_db(200_000_000), -4);
        assert_eq!(band_correction_db(380_000_000), -7);
        assert_eq!(band_correction_db(446_006_250), -6);
        assert_eq!(band_correction_db(470_000_000), -1);
    }

    #[test]
    fn the_carrier_is_the_chip_reading_plus_the_band_correction() {
        // Count 148 is -86 dBm at the chip; PMR446 reads 6 dB lower.
        assert_eq!(carrier_dbm(148, 446_006_250), -92);
        assert_eq!(carrier_dbm(148, 145_500_000), -106);
        assert_eq!(carrier_dbm(0, 446_006_250), -166);
    }

    #[test]
    fn the_scale_puts_s9_at_minus_93_and_six_db_per_unit() {
        let read = |dbm| {
            let reading = SReading::from_dbm(dbm);
            (reading.s_units, reading.over_s9_db)
        };
        assert_eq!(read(-150), (0, 0));
        assert_eq!(read(-142), (0, 0));
        assert_eq!(read(-141), (1, 0));
        assert_eq!(read(-136), (1, 0));
        assert_eq!(read(-135), (2, 0));
        assert_eq!(read(-99), (8, 0));
        assert_eq!(read(-94), (8, 0));
        assert_eq!(read(-93), (9, 0));
        assert_eq!(read(-84), (9, 0));
        assert_eq!(read(-83), (9, 10));
        assert_eq!(read(-53), (9, 40));
        assert_eq!(read(-20), (9, 60));
    }

    #[test]
    fn labels_and_segments_follow_the_reading() {
        let s7 = SReading::from_dbm(-105);
        assert_eq!(&s7.label(), b"S7   ");
        assert_eq!(s7.segments(), 7);
        let over = SReading::from_dbm(-70);
        assert_eq!(&over.label(), b"S9+20");
        assert_eq!(over.segments(), 11);
        assert_eq!(SReading::from_dbm(0).segments(), super::SEGMENTS);
    }
}
