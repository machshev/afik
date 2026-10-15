//! Transmit shared by the K5 and K1 adapters.
//!
//! The PTT lifecycle is [`crate::tx_control`]; this module supplies what both
//! targets build around it: the complete request, the calibration-derived
//! PA bias, and the effect executor over the BK4819/BK4829 sequence
//! reimplemented from the pinned Armel sources (`EVID-TX056-003`). A target
//! supplies only its identity and its power-level divider table.
//!
//! Power is one of the Armel F4HWN "Low1" to "Low4" levels, the unit's own low
//! calibration bias for the band in use multiplied by 4 and divided by the
//! target's divider, or Mid or High, the unit's mid and high calibration rows as
//! the reference uses them. The bands are Armel's seven with the wide receive
//! range both sources build with, 18 MHz to 1.3 GHz inclusive less the chip's
//! 630–840 MHz gap: what `TX_freq_check` allows with its frequency lock off.
//!
//! A transmission uses the selected channel's transmit frequency, bandwidth
//! and TX tone ([`TxSetup`]), as `RADIO_SetTxParameters` uses `pTX`. The milliwatt figures in
//! [`crate::operator_shell::TX_POWER_NOMINAL_MW`] are Armel's documented
//! nominal outputs, not measurements.

use radio_bk4819::{Bk4819, ReceiveSetup, RegisterBus};
use radio_domain::{ActiveChannel, Bandwidth, Frequency, Modulation, Tone};
use radio_tx_policy::{ConfigurationIdentity, RuleRange, TxEnvelope, TxPolicy, TxRequest, TxState};

use crate::operator_shell::{LOW_POWER_LEVELS, TX_POWER_LEVELS, TX_POWER_NOMINAL_MW};
use crate::tx_control::{Acknowledgement, Action, Controller, Effect, Event};

/// Highest nominal power of the levels offered (High), which bounds each
/// target's hardware envelope; each programmed range's ceiling still applies.
pub const HARDWARE_MAX_MW: u32 = 5_000;
/// Most controller effects driven in one pass; a full entry or exit is four or
/// three, so one pass completes either.
const MAX_EFFECTS_PER_PASS: usize = 8;

/// `frequencyBandTable` in both Armel sources with `ENABLE_WIDE_RX`, in 10 Hz
/// units, lower inclusive and upper exclusive except the last, whose upper
/// edge `TX_freq_check` allows. Band `n`'s calibration rows
/// start `n * 16` bytes into [`TxCalibration`].
const BANDS: [(i32, i32); 7] = [
    (1_800_000, 10_800_000),
    (10_800_000, 13_700_000),
    (13_700_000, 17_400_000),
    (17_400_000, 35_000_000),
    (35_000_000, 40_000_000),
    (40_000_000, 47_000_000),
    (47_000_000, 130_000_000),
];
/// `BX4819_band1.upper` and `BX4819_band2.lower`: the chip does not tune
/// between them, and `TX_freq_check` refuses it.
const CHIP_GAP: (i32, i32) = (63_000_000, 84_000_000);
/// Bytes of calibration each band has, its three rows and padding.
const BAND_CALIBRATION_BYTES: usize = 16;
/// The BK4819/BK4829 frequency word counts 10 Hz.
const FREQUENCY_RESOLUTION_HZ: u32 = 10;

/// Settling waits from `RADIO_SetTxParameters`, in milliseconds.
const AFTER_CHIP_TX_MS: u32 = 10;
const AFTER_PA_ENABLE_MS: u32 = 5;
const AFTER_BIAS_MS: u32 = 10;

/// Converts a band edge in 10 Hz units to hertz.
const fn band_hz(edge: i32) -> u32 {
    edge.unsigned_abs() * FREQUENCY_RESOLUTION_HZ
}

/// Millisecond wait used for the source's settling delays.
pub trait Delay {
    /// Waits at least `milliseconds`.
    fn delay_ms(&mut self, milliseconds: u32);
}

/// What the selected slot transmits with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxSetup {
    /// Receive and transmit frequencies, a repeater's output and input.
    pub channel: ActiveChannel,
    /// Occupied bandwidth.
    pub bandwidth: Bandwidth,
    /// CTCSS or DCS sent with the transmission.
    pub tone: Tone,
    /// The channel's modulation, which the policy permits only as FM: a
    /// broadcast, AM or USB channel never keys the radio.
    pub modulation: Modulation,
    /// The channel only receives, which the policy refuses to key.
    pub receive_only: bool,
}

impl TxSetup {
    /// Simplex narrow FM without a tone, as the VFO transmits.
    #[must_use]
    pub const fn simplex(frequency: Frequency) -> Self {
        Self {
            channel: ActiveChannel {
                receive: frequency,
                transmit: frequency,
            },
            bandwidth: Bandwidth::Narrow,
            tone: Tone::None,
            modulation: Modulation::Fm,
            receive_only: false,
        }
    }
}

/// One target's fixed transmit facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxTarget {
    /// Opaque identity shared by this target's hardware and calibration envelopes.
    pub target_id: u64,
    /// Low1–Low4 bias divisors from the target's Armel `dividers` table.
    pub dividers: [i32; LOW_POWER_LEVELS as usize],
}

/// Bytes of the TX calibration table, sixteen for each of the seven bands.
pub const TX_CALIBRATION_BYTES: usize = BANDS.len() * BAND_CALIBRATION_BYTES;

/// Every band's calibration as the reference reads it from
/// `base + band * 16 + power * 3`: per band the low, mid and high power rows,
/// each three bytes for the band's low, middle and high frequency points.
pub type TxCalibration = [u8; TX_CALIBRATION_BYTES];

/// Returns the band holding `frequency` (10 Hz units), if TX may use one.
fn band(frequency: i32) -> Option<usize> {
    if (CHIP_GAP.0..CHIP_GAP.1).contains(&frequency) {
        return None;
    }
    let last = BANDS.len() - 1;
    BANDS
        .iter()
        .enumerate()
        .position(|(index, (lower, upper))| {
            (*lower..*upper).contains(&frequency) || (index == last && frequency == *upper)
        })
}

/// The power level to transmit at: `selected`, lowered to the highest level
/// whose nominal output a range ceiling of `max_mw` allows.
///
/// With no ceiling (no range covers the frequency) or one below the lowest
/// level, `selected` is kept and the policy refuses the request, so a
/// transmission is never made quieter than Low1 to slip past a ceiling.
#[must_use]
pub fn level_within(selected: u8, max_mw: Option<u16>) -> u8 {
    let Some(max_mw) = max_mw else {
        return selected;
    };
    (0..=selected.min(TX_POWER_LEVELS - 1))
        .rev()
        .find(|level| {
            TX_POWER_NOMINAL_MW
                .get(usize::from(*level))
                .is_some_and(|mw| *mw <= u32::from(max_mw))
        })
        .unwrap_or(selected)
}

impl TxTarget {
    /// Derives the PA bias for one power level (0 = Low1) as
    /// `RADIO_ConfigureSquelchAndOutputPower` and
    /// `FREQUENCY_CalculateOutputPower` do: Low1–Low4 from the low row scaled
    /// by this target's divider, Mid and High from the mid and high rows as
    /// they are (`radio.c:456-482`).
    ///
    /// Returns `None` for an unprogrammed (`0x00`/`0xFF`) byte in the row it
    /// uses, an unknown level, a frequency in no band, or a zero result, so
    /// TX is then denied by missing calibration.
    #[must_use]
    pub fn bias(&self, calibration: TxCalibration, frequency_hz: u32, level: u8) -> Option<u8> {
        if level >= TX_POWER_LEVELS {
            return None;
        }
        let (row, scale): (usize, Option<i32>) = match level.checked_sub(LOW_POWER_LEVELS) {
            None => (0, Some(*self.dividers.get(usize::from(level))?)),
            Some(above) => (usize::from(above) + 1, None),
        };
        let frequency = i32::try_from(frequency_hz / 10).ok()?;
        let band = band(frequency)?;
        let (lower, upper) = BANDS[band];
        let start = band * BAND_CALIBRATION_BYTES + row * 3;
        let points: [u8; 3] = calibration.get(start..start + 3)?.try_into().ok()?;
        if points.iter().any(|byte| matches!(byte, 0x00 | 0xFF)) {
            return None;
        }
        let [low, mid, high] = points.map(|byte| match scale {
            Some(divider) => i32::from(byte) * 4 / divider,
            None => i32::from(byte),
        });
        let middle = (lower + upper) / 2;
        // The source's two interpolation branches, including its use of the
        // low-to-mid slope added to mid below the middle point.
        let bias = if frequency <= lower {
            low
        } else if frequency <= middle {
            mid + (mid - low) * (frequency - lower) / (middle - lower)
        } else {
            mid + (high - mid) * (frequency - middle) / (upper - middle)
        };
        u8::try_from(bias).ok().filter(|bias| *bias != 0)
    }

    fn envelope(&self, evidence_id: u32) -> TxEnvelope {
        TxEnvelope {
            target_id: self.target_id,
            evidence_id,
            // Every calibrated band either side of the chip's gap; the
            // programmed ranges decide where in them TX may go.
            ranges: [
                Some(RuleRange {
                    first_hz: band_hz(BANDS[0].0),
                    last_hz: band_hz(CHIP_GAP.0) - FREQUENCY_RESOLUTION_HZ,
                    max_conducted_mw: HARDWARE_MAX_MW,
                    wide_fm: true,
                }),
                Some(RuleRange {
                    first_hz: band_hz(CHIP_GAP.1),
                    last_hz: band_hz(BANDS[BANDS.len() - 1].1),
                    max_conducted_mw: HARDWARE_MAX_MW,
                    wide_fm: true,
                }),
                None,
                None,
            ],
            resolution_hz: FREQUENCY_RESOLUTION_HZ,
        }
    }

    /// Builds the complete FM request for `setup`; the policy checks its
    /// transmit frequency and bandwidth.
    ///
    /// The calibration envelope is present
    /// only when a bias for `level` was derived from this unit's calibration.
    /// A frequency outside the programmed ranges, or a timeout over the
    /// policy ceiling, is denied by policy, not here.
    #[must_use]
    pub fn request(
        &self,
        setup: TxSetup,
        state: TxState,
        revision: u32,
        bias: Option<u8>,
        level: u8,
        timeout_ms: u32,
    ) -> TxRequest {
        TxRequest {
            target_id: self.target_id,
            channel: setup.channel,
            modulation: setup.modulation,
            receive_only: setup.receive_only,
            bandwidth: setup.bandwidth,
            conducted_mw: TX_POWER_NOMINAL_MW
                .get(usize::from(level))
                .copied()
                .unwrap_or(0),
            timeout_ms,
            state,
            configuration: Some(ConfigurationIdentity {
                instance: 1,
                generation: 0,
                revision,
            }),
            hardware: Some(self.envelope(1)),
            calibration: bias.map(|_| self.envelope(2)),
        }
    }
}

/// Executes one controller effect against the radio, muting receive audio
/// before preparation and sending `tone` once the bias has settled. Returns
/// whether the whole operation completed.
pub fn execute<B: RegisterBus, D: Delay>(
    effect: &Effect<'_>,
    bias: Option<u8>,
    tone: Tone,
    receive: Option<ReceiveSetup>,
    radio: &mut Bk4819<B>,
    mute_audio: impl FnOnce(),
    delay: &mut D,
) -> bool {
    let request = effect.request();
    match (effect.action(), effect.authorisation()) {
        (Action::DisablePa, _) => radio.disable_pa().is_ok(),
        (Action::DisableTransmit, _) => radio.disable_transmit().is_ok(),
        (Action::RestoreReceive, _) => {
            receive.is_some_and(|setup| radio.configure_receive(&setup).is_ok())
        }
        (Action::PrepareTransmit, Some(token)) => {
            mute_audio();
            radio.prepare_transmit(request, token).is_ok()
        }
        (Action::EnableTransmit, Some(token)) => {
            let entered = radio.enable_transmit(request, token).is_ok();
            delay.delay_ms(AFTER_CHIP_TX_MS);
            entered
        }
        (Action::EnablePa, Some(token)) => {
            let Some(bias) = bias else {
                return false;
            };
            if radio.enable_pa(request, token).is_err() {
                return false;
            }
            delay.delay_ms(AFTER_PA_ENABLE_MS);
            if radio.set_pa_bias(request, token, bias).is_err() {
                return false;
            }
            delay.delay_ms(AFTER_BIAS_MS);
            radio.set_transmit_tone(request, token, tone).is_ok()
        }
        (Action::PrepareTransmit | Action::EnableTransmit | Action::EnablePa, None) => false,
    }
}

/// Drives every pending controller effect for this pass, acknowledging each.
///
/// A stale request or withdrawn authority invalidates, which starts shutdown.
pub fn run_effects(
    controller: &mut Controller,
    policy: &TxPolicy,
    request: &TxRequest,
    now_ms: u64,
    mut run: impl FnMut(&Effect<'_>) -> bool,
) {
    for _ in 0..MAX_EFFECTS_PER_PASS {
        let acknowledgement = match controller.effect(policy, request) {
            Ok(Some(effect)) => {
                if run(&effect) {
                    Acknowledgement::applied(effect.id())
                } else {
                    Acknowledgement::failed(effect.id())
                }
            }
            Ok(None) => return,
            Err(_) => {
                let _ = controller.handle(now_ms, Event::Invalidated, policy, request);
                continue;
            }
        };
        let _ = controller.handle(now_ms, Event::Completed(acknowledgement), policy, request);
    }
}

#[cfg(test)]
mod tests {
    use super::{TxSetup, TxTarget};
    use crate::operator_shell::TX_TIMEOUT_CHOICES;
    use radio_domain::{Frequency, TxClass};
    use radio_tx_policy::{TxDenied, TxPolicy, TxRange, TxState};

    const K5: TxTarget = TxTarget {
        target_id: 5,
        dividers: [25, 19, 13, 10],
    };
    const K1: TxTarget = TxTarget {
        target_id: 1,
        dividers: [25, 19, 13, 9],
    };

    fn pmr(hz: u32) -> Frequency {
        Frequency::from_hz(hz).unwrap()
    }

    fn simplex(hz: u32) -> TxSetup {
        TxSetup::simplex(pmr(hz))
    }

    /// A flat low row of 0x5F and mid and high rows of the kind a K5 backup
    /// holds (`EVID-K5-020` shows `32 32 32 64 64 64 8c 8c 8c` for one band).
    const ROWS: [u8; 9] = [0x5F, 0x5F, 0x5F, 0x64, 0x64, 0x64, 0x8C, 0x8C, 0x8C];

    /// The table with `rows` in the 2 m (band 2) and 70 cm (band 5) bands
    /// and every other band unprogrammed.
    const fn table(rows: [u8; 9]) -> super::TxCalibration {
        let mut table = [0xFF; super::TX_CALIBRATION_BYTES];
        let mut index = 0;
        while index < 9 {
            table[2 * 16 + index] = rows[index];
            table[5 * 16 + index] = rows[index];
            index += 1;
        }
        table
    }

    const CALIBRATION: super::TxCalibration = table(ROWS);

    #[test]
    fn each_target_scales_by_its_own_divider_table() {
        let row = CALIBRATION;
        assert_eq!(K5.bias(row, 446_006_250, 3), Some(38), "95 * 4 / 10");
        assert_eq!(K1.bias(row, 446_006_250, 3), Some(42), "95 * 4 / 9");
        assert_eq!(K1.bias(row, 446_006_250, 0), Some(15));
    }

    #[test]
    fn each_band_uses_its_own_rows_and_edges() {
        // Band 2 (137–174 MHz) has its own rows, interpolated over its edges.
        let mut rows = CALIBRATION;
        rows[2 * 16..2 * 16 + 9].copy_from_slice(&[40, 40, 40, 80, 100, 120, 1, 2, 3]);
        assert_eq!(K5.bias(rows, 137_000_000, 4), Some(80), "the lower edge");
        // The reference adds the low-to-mid slope to mid below the middle.
        assert_eq!(K5.bias(rows, 155_500_000, 4), Some(120), "the middle");
        assert_eq!(
            K5.bias(rows, 173_999_990, 4),
            Some(119),
            "just below the top"
        );
        assert_eq!(
            K5.bias(rows, 145_500_000, 5),
            Some(2),
            "High uses its own row"
        );
        assert_eq!(K5.bias(rows, 446_006_250, 4), Some(0x64), "70 cm unchanged");
        // An unprogrammed band, the chip gap and beyond 1.3 GHz have none.
        assert_eq!(K5.bias(rows, 220_000_000, 4), None, "band 3 unprogrammed");
        assert_eq!(K5.bias(rows, 700_000_000, 4), None, "chip gap");
        assert_eq!(K5.bias(rows, 1_300_000_010, 4), None);
        // 1.3 GHz itself is the top band's upper point.
        rows[6 * 16..6 * 16 + 9].copy_from_slice(&[40, 40, 40, 80, 100, 120, 1, 2, 3]);
        assert_eq!(K5.bias(rows, 1_300_000_000, 4), Some(120));
    }

    #[test]
    fn the_hardware_reaches_18_mhz_to_1_3_ghz_inclusive_less_the_chip_gap() {
        let wide = |first_hz, last_hz| TxRange {
            class: TxClass::Experimental,
            first_hz,
            last_hz,
            max_mw: 5_000,
            wide_fm: true,
            raster_hz: 0,
        };
        let policy = TxPolicy::new(&[wide(1_000_000, 1_400_000_000)]).unwrap();
        let at = |hz| {
            policy
                .authorise(&K5.request(simplex(hz), TxState::Selected, 1, Some(15), 0, 30_000))
                .map(|_| ())
        };
        assert!(at(18_000_000).is_ok());
        assert!(at(629_999_990).is_ok());
        assert_eq!(at(630_000_000).unwrap_err(), TxDenied::Hardware);
        assert_eq!(at(839_999_990).unwrap_err(), TxDenied::Hardware);
        assert!(at(840_000_000).is_ok());
        assert!(at(1_300_000_000).is_ok(), "TX_freq_check allows the edge");
        assert_eq!(at(1_300_000_010).unwrap_err(), TxDenied::Hardware);
        assert_eq!(at(17_999_990).unwrap_err(), TxDenied::Hardware);
    }

    #[test]
    fn mid_and_high_use_their_own_rows_unscaled() {
        for target in [K5, K1] {
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 4), Some(0x64), "Mid");
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 5), Some(0x8C), "High");
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 6), None, "no level 7");
        }
        // An unprogrammed high row refuses High alone.
        let mut no_high = ROWS;
        no_high[7] = 0xFF;
        let no_high = table(no_high);
        assert_eq!(K5.bias(no_high, 446_006_250, 5), None);
        assert_eq!(K5.bias(no_high, 446_006_250, 4), Some(0x64));
        assert_eq!(K5.bias(no_high, 446_006_250, 0), Some(15));
    }

    /// PMR446, part of 70 cm and 2 m, as a studio might program them.
    fn policy() -> TxPolicy {
        let range = |class, first_hz, last_hz, max_mw, raster_hz| TxRange {
            class,
            first_hz,
            last_hz,
            max_mw,
            wide_fm: false,
            raster_hz,
        };
        TxPolicy::new(&[
            range(
                TxClass::LicenceFreePlan,
                446_006_250,
                446_193_750,
                500,
                12_500,
            ),
            range(TxClass::Amateur, 433_000_000, 434_000_000, 5_000, 0),
            range(TxClass::Amateur, 144_000_000, 146_000_000, 5_000, 0),
        ])
        .unwrap()
    }

    #[test]
    fn power_is_lowered_to_the_range_ceiling_and_never_raised() {
        use super::level_within;
        assert_eq!(level_within(5, Some(500)), 3, "High on PMR446 is Low4");
        assert_eq!(level_within(4, Some(2_000)), 4, "Mid fits a 2 W ceiling");
        assert_eq!(level_within(5, Some(2_000)), 4, "High drops to Mid");
        assert_eq!(level_within(1, Some(5_000)), 1, "never raised");
        assert_eq!(level_within(5, Some(5_000)), 5);
        assert_eq!(level_within(5, Some(130)), 1, "125 mW fits 130");
        assert_eq!(level_within(5, None), 5, "no range: the policy refuses");
        assert_eq!(
            level_within(2, Some(10)),
            2,
            "below Low1: the policy refuses"
        );
        // The lowered level is then authorised on PMR446.
        let policy = policy();
        let level = level_within(5, policy.max_mw_at(pmr(446_006_250)));
        let request = K5.request(
            simplex(446_006_250),
            TxState::Selected,
            1,
            Some(15),
            level,
            60_000,
        );
        assert!(policy.authorise(&request).is_ok());
    }

    #[test]
    fn every_selectable_timeout_is_authorised() {
        let policy = policy();
        for minutes in 1..=u32::from(TX_TIMEOUT_CHOICES) {
            let timeout_ms = minutes * 60_000;
            let request = K5.request(
                simplex(446_006_250),
                TxState::Selected,
                1,
                Some(15),
                0,
                timeout_ms,
            );
            assert!(policy.authorise(&request).is_ok(), "{minutes} min");
        }
    }

    #[test]
    fn a_repeater_transmits_on_its_input_with_its_own_bandwidth() {
        use radio_domain::{ActiveChannel, Bandwidth, Modulation, Tone};
        let policy = policy();
        // A 70 cm repeater: output 439.4, input 431.8 (outside the 433–434
        // amateur range programmed here), then input 433.4.
        let repeater = |input_hz, bandwidth| TxSetup {
            channel: ActiveChannel {
                receive: pmr(439_400_000),
                transmit: pmr(input_hz),
            },
            bandwidth,
            tone: Tone::Ctcss(885),
            modulation: Modulation::Fm,
            receive_only: false,
        };
        let request = |setup| K5.request(setup, TxState::Selected, 1, Some(15), 0, 30_000);
        let refused = request(repeater(431_800_000, Bandwidth::Narrow));
        assert_eq!(refused.channel.transmit, pmr(431_800_000));
        assert_eq!(policy.authorise(&refused).unwrap_err(), TxDenied::Range);
        let narrow = request(repeater(433_400_000, Bandwidth::Narrow));
        assert!(policy.authorise(&narrow).is_ok());
        // A receive-only channel is refused wherever it would transmit.
        let listener = request(TxSetup {
            receive_only: true,
            ..repeater(433_400_000, Bandwidth::Narrow)
        });
        assert_eq!(
            policy.authorise(&listener).unwrap_err(),
            TxDenied::ReceiveOnly
        );
        // These ranges do not permit wide FM.
        let wide = request(repeater(433_400_000, Bandwidth::Wide));
        assert_eq!(wide.bandwidth, Bandwidth::Wide);
        assert_eq!(policy.authorise(&wide).unwrap_err(), TxDenied::Range);
        // Inside a range, only FM keys the radio.
        for modulation in [Modulation::Am, Modulation::Usb, Modulation::Broadcast] {
            let other = request(TxSetup {
                modulation,
                ..repeater(433_400_000, Bandwidth::Narrow)
            });
            assert_eq!(policy.authorise(&other).unwrap_err(), TxDenied::Modulation);
        }
    }

    #[test]
    fn only_programmed_frequencies_inside_the_calibrated_band_are_authorised() {
        let policy = policy();
        let selected = |hz| K1.request(simplex(hz), TxState::Selected, 1, Some(15), 0, 30_000);
        assert!(policy.authorise(&selected(446_006_250)).is_ok());
        assert!(policy.authorise(&selected(446_193_750)).is_ok());
        assert_eq!(
            policy.authorise(&selected(446_012_500)).unwrap_err(),
            TxDenied::Range,
            "between PMR446 raster channels"
        );
        assert!(
            policy.authorise(&selected(433_412_500)).is_ok(),
            "70 cm amateur, as the VFO could tune it"
        );
        assert_eq!(
            policy.authorise(&selected(435_000_000)).unwrap_err(),
            TxDenied::Range,
            "inside the band but outside every programmed range"
        );
        assert!(
            policy.authorise(&selected(145_500_000)).is_ok(),
            "2 m amateur"
        );
        let scanning = K1.request(
            simplex(446_006_250),
            TxState::Scanning,
            1,
            Some(15),
            0,
            30_000,
        );
        assert_eq!(policy.authorise(&scanning).unwrap_err(), TxDenied::State);
        let uncalibrated = K1.request(simplex(446_006_250), TxState::Selected, 1, None, 0, 30_000);
        assert_eq!(
            policy.authorise(&uncalibrated).unwrap_err(),
            TxDenied::Calibration
        );
    }
}
