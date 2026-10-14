//! Experimental PMR446 transmit shared by the K5 and K1 adapters.
//!
//! The PTT lifecycle is [`crate::tx_control`]; this module supplies what both
//! targets build around it: the complete PMR446 request, the calibration-derived
//! PA bias, and the effect executor over the BK4819/BK4829 sequence
//! reimplemented from the pinned Armel sources (`EVID-TX056-003`). A target
//! supplies only its identity and its power-level divider table.
//!
//! Power is one of the Armel F4HWN "Low1" to "Low4" levels: the unit's own low
//! calibration bias for the 400–470 MHz band multiplied by 4 and divided by the
//! target's divider. The milliwatt figures in
//! [`crate::operator_shell::TX_POWER_NOMINAL_MW`] are Armel's documented
//! nominal outputs, not measurements.

use radio_bk4819::{Bk4819, ReceiveSetup, RegisterBus};
use radio_domain::{ActiveChannel, Bandwidth, Frequency, Modulation, TxClass};
use radio_tx_policy::{
    ConfigurationIdentity, RuleRange, TxEnvelope, TxPolicy, TxRange, TxRequest, TxState,
};

use crate::operator_shell::{TX_POWER_LEVELS, TX_POWER_NOMINAL_MW};
use crate::tx_control::{Acknowledgement, Action, Controller, Effect, Event};

/// PMR446 ceiling, which also bounds each target's hardware envelope.
pub const PMR446_MAX_MW: u32 = 500;
/// Most controller effects driven in one pass; a full entry or exit is four or
/// three, so one pass completes either.
const MAX_EFFECTS_PER_PASS: usize = 8;

const PMR446_FIRST_HZ: u32 = 446_006_250;
const PMR446_LAST_HZ: u32 = 446_193_750;
const PMR446_RASTER_HZ: u32 = 12_500;

/// `frequencyBandTable[BAND6_400MHz]` in both Armel sources, in 10 Hz units.
const BAND_LOWER: i32 = 40_000_000;
const BAND_UPPER: i32 = 47_000_000;

/// Settling waits from `RADIO_SetTxParameters`, in milliseconds.
const AFTER_CHIP_TX_MS: u32 = 10;
const AFTER_PA_ENABLE_MS: u32 = 5;
const AFTER_BIAS_MS: u32 = 10;

/// Millisecond wait used for the source's settling delays.
pub trait Delay {
    /// Waits at least `milliseconds`.
    fn delay_ms(&mut self, milliseconds: u32);
}

/// One target's fixed PMR446 transmit facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pmr446Target {
    /// Opaque identity shared by this target's hardware and calibration envelopes.
    pub target_id: u64,
    /// Low1–Low4 bias divisors from the target's Armel `dividers` table.
    pub dividers: [i32; TX_POWER_LEVELS as usize],
}

impl Pmr446Target {
    /// Derives the PA bias for one power level (0 = Low1) from the three low
    /// calibration bytes, as `RADIO_ConfigureSquelchAndOutputPower` and
    /// `FREQUENCY_CalculateOutputPower` do.
    ///
    /// Returns `None` for an unprogrammed (`0x00`/`0xFF`) byte, an unknown
    /// level, a frequency outside 400–470 MHz, or a zero result, so TX is then
    /// denied by missing calibration.
    #[must_use]
    pub fn bias(&self, calibration: [u8; 3], frequency_hz: u32, level: u8) -> Option<u8> {
        if calibration.iter().any(|byte| matches!(byte, 0x00 | 0xFF)) {
            return None;
        }
        let divider = *self.dividers.get(usize::from(level))?;
        let frequency = i32::try_from(frequency_hz / 10).ok()?;
        if !(BAND_LOWER..BAND_UPPER).contains(&frequency) {
            return None;
        }
        let [low, mid, high] = calibration.map(|byte| i32::from(byte) * 4 / divider);
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
                    first_hz: PMR446_FIRST_HZ,
                    last_hz: PMR446_LAST_HZ,
                    max_conducted_mw: PMR446_MAX_MW,
                    wide_fm: false,
                }),
                None,
                None,
                None,
            ],
            resolution_hz: PMR446_RASTER_HZ,
        }
    }

    /// Builds the complete simplex narrow-FM request for one frequency.
    ///
    /// The calibration envelope is present
    /// only when a bias for `level` was derived from this unit's calibration.
    /// A frequency off the PMR446 raster, or a timeout over the policy
    /// ceiling, is denied by policy, not here.
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

/// PMR446 as one programmed range: the 16 channels, narrow FM, 500 mW.
pub const PMR446_RANGE: TxRange = TxRange {
    class: TxClass::LicenceFreePlan,
    first_hz: PMR446_FIRST_HZ,
    last_hz: PMR446_LAST_HZ,
    max_mw: 500,
    wide_fm: false,
    raster_hz: 12_500,
};

/// The fixed experimental policy: licence-free PMR446 only.
#[must_use]
pub fn policy() -> TxPolicy {
    TxPolicy::new(&[PMR446_RANGE]).unwrap_or_default()
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
    use super::{policy, Pmr446Target};
    use crate::operator_shell::TX_TIMEOUT_CHOICES;
    use radio_domain::Frequency;
    use radio_tx_policy::{TxDenied, TxState};

    const K5: Pmr446Target = Pmr446Target {
        target_id: 5,
        dividers: [25, 19, 13, 10],
    };
    const K1: Pmr446Target = Pmr446Target {
        target_id: 1,
        dividers: [25, 19, 13, 9],
    };

    fn pmr(hz: u32) -> Frequency {
        Frequency::from_hz(hz).unwrap()
    }

    #[test]
    fn each_target_scales_by_its_own_divider_table() {
        let row = [0x5F, 0x5F, 0x5F];
        assert_eq!(K5.bias(row, 446_006_250, 3), Some(38), "95 * 4 / 10");
        assert_eq!(K1.bias(row, 446_006_250, 3), Some(42), "95 * 4 / 9");
        assert_eq!(K1.bias(row, 446_006_250, 0), Some(15));
        assert_eq!(K1.bias(row, 446_006_250, 4), None, "no Low5");
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
    fn only_selected_pmr446_raster_frequencies_are_authorised() {
        let policy = policy();
        let selected = |hz| K1.request(pmr(hz), TxState::Selected, 1, Some(15), 0, 30_000);
        assert!(policy.authorise(&selected(446_006_250)).is_ok());
        assert!(policy.authorise(&selected(446_193_750)).is_ok());
        assert_eq!(
            policy.authorise(&selected(446_012_500)).unwrap_err(),
            TxDenied::Range,
            "between raster channels"
        );
        assert!(
            policy.authorise(&selected(145_500_000)).is_err(),
            "amateur VHF is not PMR446"
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
