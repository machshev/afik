//! The quick carrier check a scan makes before it tunes a channel fully.
//!
//! The reference's fast scan (`ENABLE_FEAT_F4HWN_SCAN_FASTER`, Armel
//! `App/app/chFrScanner.c`, `ADR-110`) retunes only the synthesiser, reads
//! the RSSI once, and moves on at once unless the reading stands out from
//! both the noise floor it has learned on this pass and the squelch's open
//! threshold. Only a channel that does gets the full setup and the dwell.
//! Readings and thresholds are the chip's RSSI counts, half a decibel each.

/// Counts above the learned floor a reading must reach, 8 dB.
const FLOOR_MARGIN: u16 = 16;
/// Counts below the open threshold a reading may still pass at, 4 dB.
const SQUELCH_MARGIN: u16 = 8;
/// Counts a weak reading is credited with before it is judged, 4 dB.
const WEAK_MARGIN: u16 = 8;

/// The noise floor one scan pass has learned, and the test against it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QuietGate {
    floor: Option<u16>,
}

impl QuietGate {
    /// A gate with no floor learned yet.
    pub const fn new() -> Self {
        Self { floor: None }
    }

    /// Forgets the floor: the reference does so when a scan starts, after a
    /// reception, and at each wrap, so a stale floor is never carried over.
    pub fn reset(&mut self) {
        self.floor = None;
    }

    /// The floor learned so far, if any.
    pub const fn floor(&self) -> Option<u16> {
        self.floor
    }

    /// Whether `rssi` is close enough to a signal to be worth reading again
    /// before it is judged. `open` is the squelch's open threshold; zero is
    /// a squelch that never shuts, which the reference does not precheck.
    pub fn is_near(&self, rssi: u16, open: u16) -> bool {
        if open == 0 {
            return true;
        }
        let credited = rssi.saturating_add(WEAK_MARGIN);
        match self.floor {
            None => credited >= open,
            Some(floor) => {
                credited >= floor.saturating_add(FLOOR_MARGIN)
                    && credited >= open.saturating_sub(SQUELCH_MARGIN)
            }
        }
    }

    /// Whether the channel read at `rssi` may carry a signal and so gets
    /// the full setup and the dwell. A quiet reading moves the floor: down
    /// to it at once, up towards it by an eighth.
    pub fn judge(&mut self, rssi: u16, open: u16) -> bool {
        if open == 0 {
            return true;
        }
        let Some(floor) = self.floor else {
            self.floor = Some(rssi);
            return rssi >= open;
        };
        let noise = floor.saturating_add(FLOOR_MARGIN);
        let squelch = open.saturating_sub(SQUELCH_MARGIN);
        let credited = rssi.saturating_add(WEAK_MARGIN);
        if (rssi >= noise && rssi >= squelch) || (credited >= noise && credited >= squelch) {
            return true;
        }
        self.floor = Some(if rssi < floor {
            rssi
        } else {
            let average = (7 * u32::from(floor) + u32::from(rssi) + 4) >> 3;
            u16::try_from(average).unwrap_or(u16::MAX)
        });
        false
    }
}

#[cfg(test)]
mod tests {
    use super::QuietGate;

    // About -130 dBm opens; the floor sits around -140 dBm.
    const OPEN: u16 = 60;
    const NOISE: u16 = 40;

    #[test]
    fn the_first_reading_is_judged_against_the_squelch_alone() {
        let mut gate = QuietGate::new();
        assert!(!gate.judge(OPEN - 1, OPEN));
        assert_eq!(gate.floor(), Some(OPEN - 1));
        gate.reset();
        assert!(gate.judge(OPEN, OPEN));
        assert_eq!(gate.floor(), Some(OPEN));
    }

    #[test]
    fn a_squelch_that_never_shuts_is_never_prechecked() {
        let mut gate = QuietGate::new();
        assert!(gate.is_near(0, 0));
        assert!(gate.judge(0, 0));
        assert_eq!(gate.floor(), None);
    }

    #[test]
    fn quiet_readings_learn_the_floor_and_a_carrier_stands_out() {
        let mut gate = QuietGate::new();
        assert!(!gate.judge(NOISE, OPEN));
        // Lower readings take the floor down at once, higher ones by an eighth.
        assert!(!gate.judge(NOISE - 4, OPEN));
        assert_eq!(gate.floor(), Some(NOISE - 4));
        assert!(!gate.judge(NOISE + 3, OPEN));
        assert_eq!(gate.floor(), Some(NOISE - 3));
        // Above the squelch but within 8 dB of the floor is still noise; that
        // is how a raised floor keeps a noisy band from stopping every step.
        let mut noisy = QuietGate::new();
        noisy.judge(OPEN + 10, OPEN);
        assert!(!noisy.judge(OPEN + 10 + 7, OPEN));
        // A carrier clear of both is a candidate and leaves the floor alone.
        let floor = gate.floor();
        assert!(gate.judge(OPEN + 20, OPEN));
        assert_eq!(gate.floor(), floor);
    }

    #[test]
    fn a_weak_signal_is_credited_four_decibels() {
        let mut gate = QuietGate::new();
        gate.judge(NOISE, OPEN);
        // Floor 40: the noise trigger is 56 and the squelch trigger 52, so
        // 48 is credited to 56 and passes, 47 to 55 and does not.
        assert!(gate.is_near(48, OPEN));
        assert!(!gate.is_near(47, OPEN));
        assert!(gate.judge(48, OPEN));
        assert!(!gate.judge(47, OPEN));
    }

    #[test]
    fn readings_at_the_ends_of_the_range_saturate() {
        let mut gate = QuietGate::new();
        gate.judge(u16::MAX, OPEN);
        assert!(gate.judge(u16::MAX, OPEN));
        assert!(!gate.judge(0, u16::MAX));
    }
}
