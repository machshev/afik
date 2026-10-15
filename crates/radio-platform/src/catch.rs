//! Frequency and tone catch (F then 4, `FC`), after the reference scanner.
//!
//! `SCANNER_Start(false)` and `SCANNER_TimeSlice10ms` in the reference first
//! run the chip's frequency scan until three readings in a row agree within
//! 1 kHz, then run its tone scan on that frequency until a DCS code is read
//! once or the same CTCSS tone three times. This module holds those
//! decisions, with no hardware: the caller reads the chip every
//! [`POLL_MILLISECONDS`] and does what [`Next`] says.

use radio_domain::Tone;

/// How often the caller reads the chip: the reference's `scan_delay_10ms`,
/// 210 ms.
pub const POLL_MILLISECONDS: u32 = 210;

/// Readings in a row which must agree before the frequency is caught.
const FREQUENCY_HITS: u8 = 3;
/// How far apart two agreeing frequency readings may be, in hertz: the
/// reference's `delta < 100` in 10 Hz units.
const FREQUENCY_AGREEMENT_HZ: u32 = 1_000;
/// Further agreeing CTCSS readings after the first before the tone is caught.
const TONE_HITS: u8 = 2;

/// What the catch is doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Looking for the strongest carrier.
    Frequency,
    /// Looking for the tone on the caught frequency.
    Tone,
    /// Both caught, or the frequency and a DCS code; the chip is idle.
    Done,
}

/// What the caller does to the chip after a reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Next {
    /// Nothing yet: read again after the next poll.
    Wait,
    /// Start the frequency scan again (`BK4819_DisableFrequencyScan` then
    /// `BK4819_EnableFrequencyScan`).
    RestartFrequency,
    /// The frequency is caught: stop the frequency scan, tune it and start
    /// the tone scan (`BK4819_SetScanFrequency`).
    StartTone(u32),
    /// Start the tone scan again after a reading that did not settle it.
    RestartTone,
    /// Everything is caught: stop the chip (`BK4819_Disable`).
    Stop,
}

/// One frequency and tone catch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Catch {
    phase: Phase,
    frequency_hz: Option<u32>,
    last_hz: Option<u32>,
    tone: Option<Tone>,
    hits: u8,
}

impl Default for Catch {
    fn default() -> Self {
        Self::new()
    }
}

impl Catch {
    /// A catch looking for a carrier.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: Phase::Frequency,
            frequency_hz: None,
            last_hz: None,
            tone: None,
            hits: 0,
        }
    }

    /// What the catch is doing.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The caught frequency in hertz, on the raster the reference rounds to.
    #[must_use]
    pub const fn frequency_hz(&self) -> Option<u32> {
        self.frequency_hz
    }

    /// The tone or code read so far; final once [`Phase::Done`].
    #[must_use]
    pub const fn tone(&self) -> Option<Tone> {
        self.tone
    }

    /// Applies one frequency scan result.
    pub fn frequency_reading(&mut self, hertz: u32) -> Next {
        if self.phase != Phase::Frequency {
            return Next::Wait;
        }
        let agrees = self
            .last_hz
            .is_some_and(|last| last.abs_diff(hertz) < FREQUENCY_AGREEMENT_HZ);
        self.hits = if agrees { self.hits + 1 } else { 0 };
        self.last_hz = Some(hertz);
        if self.hits + 1 < FREQUENCY_HITS {
            return Next::RestartFrequency;
        }
        let caught = round_to_raster(hertz);
        self.frequency_hz = Some(caught);
        self.phase = Phase::Tone;
        self.hits = 0;
        Next::StartTone(caught)
    }

    /// Applies one tone scan reading: `None` when the chip has found nothing
    /// standard yet.
    pub fn tone_reading(&mut self, tone: Option<Tone>) -> Next {
        if self.phase != Phase::Tone {
            return Next::Wait;
        }
        let Some(tone) = tone else {
            return Next::Wait;
        };
        match tone {
            Tone::Dcs { .. } => self.phase = Phase::Done,
            Tone::Ctcss(_) if self.tone == Some(tone) => {
                self.hits += 1;
                if self.hits >= TONE_HITS {
                    self.phase = Phase::Done;
                }
            }
            _ => self.hits = 0,
        }
        self.tone = Some(tone);
        if self.phase == Phase::Done {
            Next::Stop
        } else {
            Next::RestartTone
        }
    }
}

/// The nearer of the 2.5 kHz and 6.25 kHz rasters, as the reference rounds a
/// caught frequency before saving it; a tie goes to 2.5 kHz.
fn round_to_raster(hertz: u32) -> u32 {
    let nearest = |step: u32| (hertz + step / 2) / step * step;
    let (fine, coarse) = (nearest(2_500), nearest(6_250));
    if fine.abs_diff(hertz) > coarse.abs_diff(hertz) {
        coarse
    } else {
        fine
    }
}

#[cfg(test)]
mod tests {
    use super::{Catch, Next, Phase};
    use radio_domain::Tone;

    #[test]
    fn three_agreeing_readings_catch_the_frequency_on_its_raster() {
        let mut catch = Catch::new();
        assert_eq!(catch.frequency_reading(430_837_520), Next::RestartFrequency);
        // 2 kHz away: the count starts again.
        assert_eq!(catch.frequency_reading(430_839_600), Next::RestartFrequency);
        assert_eq!(catch.frequency_reading(430_838_900), Next::RestartFrequency);
        assert_eq!(catch.phase(), Phase::Frequency);
        // 430.8382 rounds to 430.8375, which is on both rasters.
        assert_eq!(
            catch.frequency_reading(430_838_200),
            Next::StartTone(430_837_500)
        );
        assert_eq!(catch.phase(), Phase::Tone);
        assert_eq!(catch.frequency_hz(), Some(430_837_500));
        // Further frequency readings change nothing.
        assert_eq!(catch.frequency_reading(145_500_000), Next::Wait);
    }

    #[test]
    fn a_frequency_on_the_fine_raster_keeps_it() {
        let mut catch = Catch::new();
        for _ in 0..2 {
            catch.frequency_reading(145_502_510);
        }
        assert_eq!(
            catch.frequency_reading(145_502_490),
            Next::StartTone(145_502_500)
        );
    }

    fn caught() -> Catch {
        let mut catch = Catch::new();
        for _ in 0..3 {
            catch.frequency_reading(145_500_000);
        }
        catch
    }

    #[test]
    fn a_ctcss_tone_must_read_the_same_three_times() {
        let mut catch = caught();
        assert_eq!(catch.tone_reading(None), Next::Wait);
        assert_eq!(
            catch.tone_reading(Some(Tone::Ctcss(1567))),
            Next::RestartTone
        );
        assert_eq!(
            catch.tone_reading(Some(Tone::Ctcss(1567))),
            Next::RestartTone
        );
        // A different tone starts the count again.
        assert_eq!(
            catch.tone_reading(Some(Tone::Ctcss(1622))),
            Next::RestartTone
        );
        assert_eq!(
            catch.tone_reading(Some(Tone::Ctcss(1622))),
            Next::RestartTone
        );
        assert_eq!(catch.tone(), Some(Tone::Ctcss(1622)));
        assert_eq!(catch.tone_reading(Some(Tone::Ctcss(1622))), Next::Stop);
        assert_eq!(catch.phase(), Phase::Done);
        assert_eq!(catch.tone_reading(Some(Tone::Ctcss(670))), Next::Wait);
        assert_eq!(catch.tone(), Some(Tone::Ctcss(1622)));
    }

    #[test]
    fn a_dcs_code_is_caught_at_once() {
        let mut catch = caught();
        let code = Tone::Dcs {
            code: 23,
            inverted: false,
        };
        assert_eq!(catch.tone_reading(Some(code)), Next::Stop);
        assert_eq!((catch.phase(), catch.tone()), (Phase::Done, Some(code)));
        assert_eq!(catch.frequency_hz(), Some(145_500_000));
    }

    #[test]
    fn no_tone_is_read_before_the_frequency() {
        let mut catch = Catch::new();
        assert_eq!(catch.tone_reading(Some(Tone::Ctcss(885))), Next::Wait);
        assert_eq!(catch.tone(), None);
    }
}
