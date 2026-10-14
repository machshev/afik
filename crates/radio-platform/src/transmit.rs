//! Transmit shared by the K5 and K1 adapters.
//!
//! The PTT lifecycle is [`crate::tx_control`]; this module supplies what both
//! targets build around it: the complete request, the calibration-derived
//! PA bias, and the effect executor over the BK4819/BK4829 sequence
//! reimplemented from the pinned Armel sources (`EVID-TX056-003`). A target
//! supplies only its identity and its power-level divider table.
//!
//! Power is one of the Armel F4HWN "Low1" to "Low4" levels, the unit's own low
//! calibration bias for the 400–470 MHz band multiplied by 4 and divided by the
//! target's divider, or Mid or High, the unit's mid and high calibration rows as
//! the reference uses them. The milliwatt figures in
//! [`crate::operator_shell::TX_POWER_NOMINAL_MW`] are Armel's documented
//! nominal outputs, not measurements.

use radio_bk4819::{Bk4819, ReceiveSetup, RegisterBus};
use radio_domain::{ActiveChannel, Bandwidth, Frequency, Modulation};
use radio_tx_policy::{ConfigurationIdentity, RuleRange, TxEnvelope, TxPolicy, TxRequest, TxState};

use crate::operator_shell::{LOW_POWER_LEVELS, TX_POWER_LEVELS, TX_POWER_NOMINAL_MW};
use crate::tx_control::{Acknowledgement, Action, Controller, Effect, Event};

/// Highest nominal power of the levels offered (High), which bounds each
/// target's hardware envelope; each programmed range's ceiling still applies.
pub const HARDWARE_MAX_MW: u32 = 5_000;
/// Most controller effects driven in one pass; a full entry or exit is four or
/// three, so one pass completes either.
const MAX_EFFECTS_PER_PASS: usize = 8;

/// `frequencyBandTable[BAND6_400MHz]` in both Armel sources, in 10 Hz units.
const BAND_LOWER: i32 = 40_000_000;
const BAND_UPPER: i32 = 47_000_000;
/// The same band in hertz, inclusive; the upper bound is exclusive above.
const BAND_LOWER_HZ: u32 = 400_000_000;
const BAND_LAST_HZ: u32 = 469_999_990;
/// The BK4819/BK4829 frequency word counts 10 Hz.
const FREQUENCY_RESOLUTION_HZ: u32 = 10;

/// Settling waits from `RADIO_SetTxParameters`, in milliseconds.
const AFTER_CHIP_TX_MS: u32 = 10;
const AFTER_PA_ENABLE_MS: u32 = 5;
const AFTER_BIAS_MS: u32 = 10;

/// Millisecond wait used for the source's settling delays.
pub trait Delay {
    /// Waits at least `milliseconds`.
    fn delay_ms(&mut self, milliseconds: u32);
}

/// One target's fixed transmit facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxTarget {
    /// Opaque identity shared by this target's hardware and calibration envelopes.
    pub target_id: u64,
    /// Low1–Low4 bias divisors from the target's Armel `dividers` table.
    pub dividers: [i32; LOW_POWER_LEVELS as usize],
}

/// The 400–470 MHz band's calibration: the low, mid and high power rows, each
/// three bytes for the band's low, middle and high frequency points, as the
/// reference reads them from `base + band * 16 + power * 3`.
pub type TxCalibration = [u8; 9];

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
    /// uses, an unknown level, a frequency outside 400–470 MHz, or a zero
    /// result, so TX is then denied by missing calibration.
    #[must_use]
    pub fn bias(&self, calibration: TxCalibration, frequency_hz: u32, level: u8) -> Option<u8> {
        if level >= TX_POWER_LEVELS {
            return None;
        }
        let (row, scale): (usize, Option<i32>) = match level.checked_sub(LOW_POWER_LEVELS) {
            None => (0, Some(*self.dividers.get(usize::from(level))?)),
            Some(above) => (usize::from(above) + 1, None),
        };
        let points: [u8; 3] = calibration.get(row * 3..row * 3 + 3)?.try_into().ok()?;
        if points.iter().any(|byte| matches!(byte, 0x00 | 0xFF)) {
            return None;
        }
        let frequency = i32::try_from(frequency_hz / 10).ok()?;
        if !(BAND_LOWER..BAND_UPPER).contains(&frequency) {
            return None;
        }
        let [low, mid, high] = points.map(|byte| match scale {
            Some(divider) => i32::from(byte) * 4 / divider,
            None => i32::from(byte),
        });
        let middle = (BAND_LOWER + BAND_UPPER) / 2;
        // The source's two interpolation branches, including its use of the
        // low-to-mid slope added to mid below the middle point.
        let bias = if frequency <= BAND_LOWER {
            low
        } else if frequency <= middle {
            mid + (mid - low) * (frequency - BAND_LOWER) / (middle - BAND_LOWER)
        } else {
            mid + (high - mid) * (frequency - middle) / (BAND_UPPER - middle)
        };
        u8::try_from(bias).ok().filter(|bias| *bias != 0)
    }

    fn envelope(&self, evidence_id: u32) -> TxEnvelope {
        TxEnvelope {
            target_id: self.target_id,
            evidence_id,
            ranges: [
                Some(RuleRange {
                    // The calibrated UHF band the PA bias is derived for;
                    // the programmed ranges decide where in it TX may go.
                    first_hz: BAND_LOWER_HZ,
                    last_hz: BAND_LAST_HZ,
                    max_conducted_mw: HARDWARE_MAX_MW,
                    wide_fm: true,
                }),
                None,
                None,
                None,
            ],
            resolution_hz: FREQUENCY_RESOLUTION_HZ,
        }
    }

    /// Builds the complete simplex narrow-FM request for one frequency.
    ///
    /// The calibration envelope is present
    /// only when a bias for `level` was derived from this unit's calibration.
    /// A frequency outside the programmed ranges, or a timeout over the
    /// policy ceiling, is denied by policy, not here.
    #[must_use]
    pub fn request(
        &self,
        frequency: Frequency,
        state: TxState,
        revision: u32,
        bias: Option<u8>,
        level: u8,
        timeout_ms: u32,
    ) -> TxRequest {
        TxRequest {
            target_id: self.target_id,
            channel: ActiveChannel {
                receive: frequency,
                transmit: frequency,
            },
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
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
/// before preparation. Returns whether the whole operation completed.
pub fn execute<B: RegisterBus, D: Delay>(
    effect: &Effect<'_>,
    bias: Option<u8>,
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
            let biased = radio.set_pa_bias(request, token, bias).is_ok();
            delay.delay_ms(AFTER_BIAS_MS);
            biased
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
    use super::TxTarget;
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

    /// A flat low row of 0x5F and mid and high rows of the kind a K5 backup
    /// holds (`EVID-K5-020` shows `32 32 32 64 64 64 8c 8c 8c` for one band).
    const CALIBRATION: [u8; 9] = [0x5F, 0x5F, 0x5F, 0x64, 0x64, 0x64, 0x8C, 0x8C, 0x8C];

    #[test]
    fn each_target_scales_by_its_own_divider_table() {
        let row = CALIBRATION;
        assert_eq!(K5.bias(row, 446_006_250, 3), Some(38), "95 * 4 / 10");
        assert_eq!(K1.bias(row, 446_006_250, 3), Some(42), "95 * 4 / 9");
        assert_eq!(K1.bias(row, 446_006_250, 0), Some(15));
    }

    #[test]
    fn mid_and_high_use_their_own_rows_unscaled() {
        for target in [K5, K1] {
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 4), Some(0x64), "Mid");
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 5), Some(0x8C), "High");
            assert_eq!(target.bias(CALIBRATION, 446_006_250, 6), None, "no level 7");
        }
        // An unprogrammed high row refuses High alone.
        let mut no_high = CALIBRATION;
        no_high[7] = 0xFF;
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
            pmr(446_006_250),
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
                pmr(446_006_250),
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
    fn only_programmed_frequencies_inside_the_calibrated_band_are_authorised() {
        let policy = policy();
        let selected = |hz| K1.request(pmr(hz), TxState::Selected, 1, Some(15), 0, 30_000);
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
        // 2 m amateur is programmed, but this hardware path is UHF only.
        assert_eq!(
            policy.authorise(&selected(145_500_000)).unwrap_err(),
            TxDenied::Hardware
        );
        let scanning = K1.request(pmr(446_006_250), TxState::Scanning, 1, Some(15), 0, 30_000);
        assert_eq!(policy.authorise(&scanning).unwrap_err(), TxDenied::State);
        let uncalibrated = K1.request(pmr(446_006_250), TxState::Selected, 1, None, 0, 30_000);
        assert_eq!(
            policy.authorise(&uncalibrated).unwrap_err(),
            TxDenied::Calibration
        );
    }
}
