//! Experimental PMR446 low-power transmit for the UV-K5 V1 (`TXK5-058`).
//!
//! The PTT lifecycle is the shared `radio_platform::tx_control` controller;
//! this module supplies the K5 request, the calibration-derived bias, and the
//! effect adapter over the BK4819 sequence reimplemented from the Armel K5 V1
//! source (`EVID-TX056-003`).
//!
//! Power is the Armel F4HWN "Low1" level: the unit's own low calibration bias
//! scaled by 4/25 (`radio.c`, the `dividers` table), which Armel documents as
//! under about 20 mW. The milliwatt figure here is that nominal claim, not a
//! measurement.

use radio_bk4819::{Bk4819, ReceiveSetup, RegisterBus};
use radio_domain::{ActiveChannel, Bandwidth, Modulation, TxClass};
use radio_platform::receive_app::View;
use radio_platform::tx_control::{Acknowledgement, Action, Controller, Effect, Event};
use radio_tx_policy::regional::{Attestation, Attestations, Region, RuleRange};
use radio_tx_policy::{
    ConfigurationIdentity, OperatingConditions, PermissionSet, StoredPermissions, TxEnvelope,
    TxPolicy, TxRequest, TxState,
};

use crate::application::SpeakerOutput;
use crate::receive::Pmr446Channel;

/// EEPROM address of the three low-power bias bytes for the 400–470 MHz band:
/// `0x1ED0 + band 5 * 16 + low * 3` (`radio.c:555`).
pub const CALIBRATION_ADDRESS: u16 = 0x1F20;
/// TX timeout from the accepted press, in milliseconds.
pub const TIMEOUT_MS: u32 = 5_000;
/// Armel's documented Low1 output, "<~20 mW". Nominal; not measured by AFIK.
pub const NOMINAL_LOW1_MW: u32 = 20;
/// Most controller effects driven in one loop pass; a full entry or exit is
/// four or three, so one pass completes either.
const MAX_EFFECTS_PER_PASS: usize = 8;

/// Opaque identity shared by this build's hardware and calibration envelopes.
const K5_TARGET_ID: u64 = 0x4B35_5631;
const PMR446_FIRST_HZ: u32 = 446_006_250;
const PMR446_LAST_HZ: u32 = 446_193_750;
const PMR446_RASTER_HZ: u32 = 12_500;

/// `frequencyBandTable[BAND6_400MHz]`, in the source's 10 Hz units.
const BAND_LOWER: i32 = 40_000_000;
const BAND_UPPER: i32 = 47_000_000;

/// Settling waits from `RADIO_SetTxParameters`, in milliseconds.
const AFTER_CHIP_TX_MS: u32 = 10;
const AFTER_PA_ENABLE_MS: u32 = 5;
const AFTER_BIAS_MS: u32 = 10;

/// Millisecond busy-wait used for the source's settling delays.
pub trait Delay {
    /// Waits at least `milliseconds`.
    fn delay_ms(&mut self, milliseconds: u32);
}

impl Delay for radio_dp32g030::systick::PeriodicTick {
    fn delay_ms(&mut self, milliseconds: u32) {
        Self::delay_ms(self, milliseconds);
    }
}

/// Derives the Low1 PA bias for a 400–470 MHz frequency from the three low
/// calibration bytes, as `RADIO_ConfigureSquelchAndOutputPower` and
/// `FREQUENCY_CalculateOutputPower` do.
///
/// Returns `None` for an unprogrammed (`0x00`/`0xFF`) byte, a frequency outside
/// the band, or a zero result, so TX is then denied by missing calibration.
pub fn low1_bias(calibration: [u8; 3], frequency_hz: u32) -> Option<u8> {
    if calibration.iter().any(|byte| matches!(byte, 0x00 | 0xFF)) {
        return None;
    }
    let frequency = i32::try_from(frequency_hz / 10).ok()?;
    if !(BAND_LOWER..BAND_UPPER).contains(&frequency) {
        return None;
    }
    let [low, mid, high] = calibration.map(|byte| i32::from(byte) * 4 / 25);
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

/// The fixed experimental policy: licence-free PMR446 only.
pub fn policy() -> TxPolicy {
    let permissions = PermissionSet::none().with(TxClass::LicenceFreePlan, true);
    TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0
}

fn envelope(evidence_id: u32) -> TxEnvelope {
    TxEnvelope {
        target_id: K5_TARGET_ID,
        evidence_id,
        ranges: [
            Some(RuleRange {
                first_hz: PMR446_FIRST_HZ,
                last_hz: PMR446_LAST_HZ,
                max_conducted_mw: NOMINAL_LOW1_MW,
                wide_fm: false,
            }),
            None,
            None,
            None,
        ],
        resolution_hz: PMR446_RASTER_HZ,
    }
}

/// Builds the complete request from the current receive view.
///
/// The attestations are the operator's, given for this experimental bench
/// build on 2026-10-13 (`TXK5-058`). The calibration envelope is present only
/// when a Low1 bias was derived from this unit's EEPROM.
pub fn request(view: View, revision: u32, bias: Option<u8>) -> TxRequest {
    let channel = Pmr446Channel::new(view.channel);
    let frequency = channel.unwrap_or(Pmr446Channel::FIRST).frequency();
    TxRequest {
        target_id: K5_TARGET_ID,
        channel: ActiveChannel {
            receive: frequency,
            transmit: frequency,
            tx_class: TxClass::LicenceFreePlan,
        },
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        conducted_mw: NOMINAL_LOW1_MW,
        timeout_ms: TIMEOUT_MS,
        state: if view.receiver_ok && channel.is_some() {
            TxState::Selected
        } else {
            TxState::Faulted
        },
        configuration: Some(ConfigurationIdentity {
            instance: 1,
            generation: 0,
            revision,
        }),
        region: Region::UnitedKingdom,
        attestations: Attestations {
            location: Attestation::Attested,
            licence_valid: Attestation::Attested,
            operator_authorised: Attestation::Attested,
            equipment_eligible: Attestation::Attested,
        },
        grant: None,
        hardware: Some(envelope(1)),
        calibration: bias.map(|_| envelope(2)),
        operating: OperatingConditions {
            mobile: Attestation::Attested,
            non_airborne: Attestation::Attested,
            receiver_capable: Attestation::Attested,
        },
    }
}

/// Executes one controller effect against the K5 radio, speaker and delay.
///
/// Returns whether the whole operation completed.
pub fn execute<B: RegisterBus, S: SpeakerOutput, D: Delay>(
    effect: &Effect<'_>,
    bias: Option<u8>,
    receive: Option<ReceiveSetup>,
    radio: &mut Bk4819<B>,
    speaker: &mut S,
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
            speaker.set_enabled(false);
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
    use super::{execute, low1_bias, policy, request, run_effects, Delay, TIMEOUT_MS};
    use crate::application::SpeakerOutput;
    use crate::receive::Pmr446Channel;
    use radio_bk4819::{Bk4819, RegisterAddress, RegisterBus, GPIO_PA_ENABLE, MODE_TRANSMIT_K5};
    use radio_platform::receive_app::View;
    use radio_platform::tx_control::{Controller, Event, State};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::vec::Vec;

    #[derive(Default)]
    struct Log {
        writes: Vec<(u8, u16)>,
        fail: bool,
    }

    #[derive(Clone, Default)]
    struct Bus(Rc<RefCell<Log>>);

    impl RegisterBus for Bus {
        type Error = ();

        fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
            let mut log = self.0.borrow_mut();
            if log.fail {
                return Err(());
            }
            log.writes.push((address.get(), value));
            Ok(())
        }

        fn read(&mut self, _: RegisterAddress) -> Result<u16, Self::Error> {
            if self.0.borrow().fail {
                Err(())
            } else {
                Ok(0)
            }
        }
    }

    #[derive(Default)]
    struct Speaker(Option<bool>);

    impl SpeakerOutput for Speaker {
        fn set_enabled(&mut self, enabled: bool) {
            self.0 = Some(enabled);
        }
    }

    #[derive(Default)]
    struct Waits(Vec<u32>);

    impl Delay for Waits {
        fn delay_ms(&mut self, milliseconds: u32) {
            self.0.push(milliseconds);
        }
    }

    struct Bench {
        log: Rc<RefCell<Log>>,
        radio: Bk4819<Bus>,
        speaker: Speaker,
        waits: Waits,
        controller: Controller,
        bias: Option<u8>,
    }

    const VIEW: View = View {
        channel: 3,
        audio: true,
        monitoring: false,
        squelch_open: false,
        receiver_ok: true,
    };

    impl Bench {
        fn new(bias: Option<u8>) -> Self {
            let bus = Bus::default();
            let log = Rc::clone(&bus.0);
            let mut radio = Bk4819::new(bus);
            radio.initialise().unwrap();
            let mut bench = Self {
                log,
                radio,
                speaker: Speaker::default(),
                waits: Waits::default(),
                controller: Controller::new(1, false, 0).unwrap(),
                bias,
            };
            bench.step(0, Event::Tick);
            assert_eq!(bench.controller.state(), State::Ready);
            bench.writes();
            bench
        }

        fn step(&mut self, now: u64, event: Event) {
            let policy = policy();
            let request = request(VIEW, 1, self.bias);
            let _ = self.controller.handle(now, event, &policy, &request);
            let receive = Pmr446Channel::new(VIEW.channel).map(|channel| channel.setup(true));
            let Self {
                log: _,
                radio,
                speaker,
                waits,
                controller,
                bias,
            } = self;
            run_effects(controller, &policy, &request, now, |effect| {
                execute(effect, *bias, receive, radio, speaker, waits)
            });
        }

        fn writes(&mut self) -> Vec<(u8, u16)> {
            core::mem::take(&mut self.log.borrow_mut().writes)
        }
    }

    fn pa_enabled(writes: &[(u8, u16)]) -> Option<bool> {
        writes
            .iter()
            .rev()
            .find(|(address, _)| *address == 0x33)
            .map(|(_, value)| value & GPIO_PA_ENABLE != 0)
    }

    #[test]
    fn low1_bias_scales_and_interpolates_like_the_source() {
        // Armel's sample 400 MHz row is 5f 5f 5f: 95 * 4 / 25 = 15.
        assert_eq!(low1_bias([0x5F, 0x5F, 0x5F], 446_006_250), Some(15));
        // 446.00625 MHz is above the 435 MHz middle: mid + (high - mid) * f.
        // low 50 -> 8, mid 100 -> 16, high 200 -> 32; 16 + 16 * 1_100_625 / 3_500_000 = 21.
        assert_eq!(low1_bias([50, 100, 200], 446_006_250), Some(21));
        assert_eq!(low1_bias([0xFF, 0x5F, 0x5F], 446_006_250), None);
        assert_eq!(low1_bias([0x5F, 0x00, 0x5F], 446_006_250), None);
        assert_eq!(low1_bias([0x5F, 0x5F, 0x5F], 145_500_000), None);
        assert_eq!(low1_bias([1, 1, 1], 446_006_250), None);
    }

    #[test]
    fn press_transmits_and_release_clears_pa_before_idle_then_restores_receive() {
        let mut bench = Bench::new(Some(15));
        bench.step(100, Event::Pressed);
        assert_eq!(bench.controller.state(), State::Transmitting);
        let entry = bench.writes();
        assert!(entry.contains(&(0x30, MODE_TRANSMIT_K5)));
        assert_eq!(pa_enabled(&entry), Some(true));
        assert!(entry.contains(&(0x36, (15 << 8) | 0xA2)));
        assert_eq!(bench.speaker.0, Some(false));
        assert_eq!(bench.waits.0, [10, 5, 10]);

        bench.step(1_000, Event::Released);
        assert_eq!(bench.controller.state(), State::Ready);
        let exit = bench.writes();
        assert_eq!(exit[0], (0x36, 0x0088));
        assert_eq!(exit[1].0, 0x33);
        assert_eq!(exit[1].1 & GPIO_PA_ENABLE, 0);
        assert_eq!(exit[2], (0x30, 0x0000));
        assert_eq!(exit.last(), Some(&(0x47, (6 << 12) | (1 << 6) | (1 << 8))));
        assert!(!exit.contains(&(0x30, MODE_TRANSMIT_K5)));
    }

    #[test]
    fn timeout_shuts_down_while_still_held_and_requires_release() {
        let mut bench = Bench::new(Some(15));
        bench.step(100, Event::Pressed);
        bench.writes();
        bench.step(100 + u64::from(TIMEOUT_MS), Event::Tick);
        assert_eq!(bench.controller.state(), State::ReleaseRequired);
        assert_eq!(pa_enabled(&bench.writes()), Some(false));
        bench.step(6_000, Event::Pressed);
        assert!(bench.writes().is_empty());
    }

    #[test]
    fn missing_calibration_or_receiver_fault_denies_before_any_write() {
        let mut bench = Bench::new(None);
        bench.step(100, Event::Pressed);
        assert_eq!(bench.controller.state(), State::ReleaseRequired);
        assert!(bench.writes().is_empty());

        let faulted = View {
            receiver_ok: false,
            ..VIEW
        };
        assert!(policy().authorise(&request(faulted, 1, Some(15))).is_err());
        assert!(policy().authorise(&request(VIEW, 1, Some(15))).is_ok());
    }

    #[test]
    fn a_bus_failure_during_entry_latches_fault_after_shutdown() {
        let mut bench = Bench::new(Some(15));
        bench.log.borrow_mut().fail = true;
        bench.step(100, Event::Pressed);
        assert_eq!(bench.controller.state(), State::Faulted);
    }
}
