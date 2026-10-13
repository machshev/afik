//! Experimental PMR446 low-power transmit for the UV-K5 V1 (`TXK5-058`).
//!
//! The request, bias derivation and effect executor are shared with the K1 in
//! [`radio_platform::transmit`]; this module supplies the K5's identity, its
//! Armel divider table and calibration address, and its speaker and delay.

use radio_bk4819::{Bk4819, ReceiveSetup, RegisterBus};
use radio_platform::receive_app::View;
use radio_platform::transmit::{self as shared, Pmr446Target};
use radio_platform::tx_control::Effect;
use radio_tx_policy::{TxPolicy, TxRequest, TxState};

use crate::application::SpeakerOutput;
use crate::receive::Pmr446Channel;

pub use radio_platform::transmit::{run_effects, Delay};

/// EEPROM address of the three low-power bias bytes for the 400–470 MHz band:
/// `0x1ED0 + band 5 * 16 + low * 3` (`radio.c:555`).
pub const CALIBRATION_ADDRESS: u16 = 0x1F20;

/// The K5 V1: Armel K5 `dividers` Low1–Low4 are 25, 19, 13 and 10.
const K5: Pmr446Target = Pmr446Target {
    target_id: 0x4B35_5631,
    dividers: [25, 19, 13, 10],
};

/// The K5 `SysTick` as the shared settling delay.
pub struct TickDelay<'a>(pub &'a mut radio_dp32g030::systick::PeriodicTick);

impl Delay for TickDelay<'_> {
    fn delay_ms(&mut self, milliseconds: u32) {
        self.0.delay_ms(milliseconds);
    }
}

/// Derives the PA bias for one power level (0 = Low1); see [`Pmr446Target::bias`].
pub fn level_bias(calibration: [u8; 3], frequency_hz: u32, level: u8) -> Option<u8> {
    K5.bias(calibration, frequency_hz, level)
}

/// The fixed experimental policy: licence-free PMR446 only.
pub fn policy() -> TxPolicy {
    shared::policy()
}

/// Builds the complete request from the current receive view.
pub fn request(
    view: View,
    revision: u32,
    bias: Option<u8>,
    level: u8,
    timeout_ms: u32,
) -> TxRequest {
    let channel = Pmr446Channel::new(view.channel);
    let state = if view.receiver_ok && channel.is_some() {
        TxState::Selected
    } else {
        TxState::Faulted
    };
    K5.request(
        channel.unwrap_or(Pmr446Channel::FIRST).frequency(),
        state,
        revision,
        bias,
        level,
        timeout_ms,
    )
}

/// Executes one controller effect against the K5 radio, speaker and delay.
pub fn execute<B: RegisterBus, S: SpeakerOutput, D: Delay>(
    effect: &Effect<'_>,
    bias: Option<u8>,
    receive: Option<ReceiveSetup>,
    radio: &mut Bk4819<B>,
    speaker: &mut S,
    delay: &mut D,
) -> bool {
    shared::execute(
        effect,
        bias,
        receive,
        radio,
        || speaker.set_enabled(false),
        delay,
    )
}

#[cfg(test)]
mod tests {
    use super::{execute, level_bias, policy, request, run_effects, Delay};
    use crate::application::SpeakerOutput;
    use crate::receive::Pmr446Channel;
    use radio_bk4819::{Bk4819, RegisterAddress, RegisterBus, GPIO_PA_ENABLE, MODE_TRANSMIT_K5};
    use radio_platform::receive_app::View;
    use radio_platform::tx_control::{Controller, Event, State};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::vec::Vec;

    const TIMEOUT_MS: u32 = 15_000;

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
            let request = request(VIEW, 1, self.bias, 0, TIMEOUT_MS);
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
    fn level_bias_scales_and_interpolates_like_the_source() {
        let row = [0x5F, 0x5F, 0x5F];
        // Armel's sample 400 MHz row is 5f 5f 5f: 95 * 4 / 25, /19, /13, /10.
        let levels: std::vec::Vec<_> = (0..4)
            .map(|level| level_bias(row, 446_006_250, level))
            .collect();
        assert_eq!(levels, [Some(15), Some(20), Some(29), Some(38)]);
        assert_eq!(level_bias(row, 446_006_250, 4), None, "no Low5");
        // 446.00625 MHz is above the 435 MHz middle: mid + (high - mid) * f.
        // low 50 -> 8, mid 100 -> 16, high 200 -> 32; 16 + 16 * 1_100_625 / 3_500_000 = 21.
        assert_eq!(level_bias([50, 100, 200], 446_006_250, 0), Some(21));
        assert_eq!(level_bias([0xFF, 0x5F, 0x5F], 446_006_250, 0), None);
        assert_eq!(level_bias([0x5F, 0x00, 0x5F], 446_006_250, 0), None);
        assert_eq!(level_bias(row, 145_500_000, 0), None);
        assert_eq!(level_bias([1, 1, 1], 446_006_250, 0), None);
    }

    #[test]
    fn every_level_is_within_the_pmr446_ceiling_and_authorised() {
        for level in 0..4 {
            let request = request(VIEW, 1, Some(15), level, TIMEOUT_MS);
            assert!(policy().authorise(&request).is_ok(), "level {level}");
        }
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
        bench.step(1_100 + u64::from(TIMEOUT_MS), Event::Pressed);
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
        assert!(policy()
            .authorise(&request(faulted, 1, Some(15), 0, TIMEOUT_MS))
            .is_err());
        assert!(policy()
            .authorise(&request(VIEW, 1, Some(15), 0, TIMEOUT_MS))
            .is_ok());
    }

    #[test]
    fn a_bus_failure_during_entry_latches_fault_after_shutdown() {
        let mut bench = Bench::new(Some(15));
        bench.log.borrow_mut().fail = true;
        bench.step(100, Event::Pressed);
        assert_eq!(bench.controller.state(), State::Faulted);
    }
}
