//! Deterministic proof that banked receive control drives the receive path.
//!
//! The controller decides what to receive; the driver decides which registers
//! that requires. This test walks one complete loop over a fake bus: activate,
//! observe a busy tone-coded channel, scan past a skipped channel, and confirm
//! that no transmit mode word is ever written.

use radio_bk4819::{
    AfOutput, Bk4819, DriverState, ReceiveSetup, RegisterAddress, RegisterBus, SquelchThresholds,
    ToneStatus,
};
use radio_channel_control::{
    BankedReceiveController, ChannelMemory, ChannelReceiveSetup, ChannelSource, ReceiveObservation,
    TimerDirective,
};
use radio_channel_plan::{BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord};
use std::{cell::RefCell, rc::Rc};

use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    SquelchLevel, Tone, TxClass,
};

const TRANSMIT_MODE: u16 = 0x80FE;
const REG_MODE_CONTROL: u8 = 0x30;
const REG_FREQUENCY_LOW: u8 = 0x38;
const REG_SUB_AUDIO_FREQUENCY: u8 = 0x07;
const REG_INTERRUPT_FLAGS: u8 = 0x02;
const REG_RSSI: u8 = 0x67;
const REG_SQUELCH_STATUS: u8 = 0x0C;
const REG_AF_OUTPUT: u8 = 0x47;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BusError;

impl core::fmt::Display for BusError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("test bus failure")
    }
}

#[derive(Debug)]
struct BusState {
    registers: [u16; 128],
    writes: Vec<(u8, u16)>,
}

impl Default for BusState {
    fn default() -> Self {
        Self {
            registers: [0; 128],
            writes: Vec::new(),
        }
    }
}

impl BusState {
    fn written(&self, address: u8) -> Option<u16> {
        self.writes
            .iter()
            .rev()
            .find(|(written, _)| *written == address)
            .map(|(_, value)| *value)
    }
}

/// The chip side of the bus. The test drives it through a shared handle rather
/// than through the driver, so the driver never needs to expose raw writes.
#[derive(Clone)]
struct FakeBus {
    state: Rc<RefCell<BusState>>,
}

impl FakeBus {
    fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(BusState::default())),
        }
    }

    fn set(&self, address: u8, value: u16) {
        self.state.borrow_mut().registers[usize::from(address)] = value;
    }

    fn written(&self, address: u8) -> Option<u16> {
        self.state.borrow().written(address)
    }

    fn wrote_transmit_mode(&self) -> bool {
        self.state
            .borrow()
            .writes
            .iter()
            .any(|(address, value)| *address == REG_MODE_CONTROL && *value == TRANSMIT_MODE)
    }
}

impl RegisterBus for FakeBus {
    type Error = BusError;

    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
        let mut state = self.state.borrow_mut();
        state.registers[usize::from(address.get())] = value;
        state.writes.push((address.get(), value));
        Ok(())
    }

    fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error> {
        Ok(self.state.borrow().registers[usize::from(address.get())])
    }
}

fn thresholds() -> SquelchThresholds {
    SquelchThresholds::new(72, 70, 46, 47, 8, 10).expect("calibration fixture")
}

/// Maps one controller decision onto the driver's receive request.
fn apply(radio: &mut Bk4819<FakeBus>, setup: ChannelReceiveSetup, audio_open: bool) {
    radio
        .configure_receive(&ReceiveSetup {
            frequency: setup.frequency,
            modulation: setup.modulation,
            bandwidth: setup.bandwidth,
            tone: setup.tone,
            squelch: thresholds(),
            af: if audio_open {
                AfOutput::Demodulated
            } else {
                AfOutput::Mute
            },
        })
        .expect("receive configuration");
}

fn channel(id: u16, hertz: u32, tone: Tone, flags: u8) -> ChannelRecord {
    ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(id),
        name: ChannelName::new("CH").expect("channel name"),
        receive: Frequency::from_hz(hertz).expect("receive frequency"),
        transmit: Frequency::from_hz(hertz).expect("transmit frequency"),
        rx_tone: tone,
        tx_tone: tone,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low,
        step: FrequencyStep::from_hz(12_500).expect("step"),
        squelch: SquelchLevel::new(4).expect("squelch"),
        flags: ChannelFlags::from_bits(flags).expect("flags"),
        banks: BankMask::from_bits(0b0000_0001),
        tx_class: TxClass::Amateur,
    })
    .expect("channel record")
}

#[test]
fn a_banked_scan_drives_the_receive_path_without_reaching_transmit() {
    let mut memory = ChannelMemory::<8>::new();
    memory
        .insert(channel(1, 145_100_000, Tone::Ctcss(1_000), 0))
        .expect("first channel");
    memory
        .insert(channel(2, 145_200_000, Tone::None, ChannelFlags::SCAN_SKIP))
        .expect("skipped channel");
    memory
        .insert(channel(3, 145_300_000, Tone::None, 0))
        .expect("third channel");
    assert_eq!(memory.len(), 3);

    let (mut controller, update) = BankedReceiveController::activate(
        memory,
        RadioConfig::conservative(),
        Some(BankId::new(0)),
    )
    .expect("activation");

    let chip = FakeBus::new();
    let mut radio = Bk4819::new(chip.clone());
    radio.recover_to_standby().expect("standby");
    let activation = update.activation.expect("initial activation");
    apply(&mut radio, activation.setup, update.audio_open);

    assert_eq!(
        radio.state(),
        DriverState::Receiving {
            frequency: Frequency::from_hz(145_100_000).expect("frequency")
        }
    );
    // 100.0 Hz CTCSS control word from the pinned source's rounding formula.
    assert_eq!(chip.written(REG_SUB_AUDIO_FREQUENCY), Some(2_065));
    assert_eq!(chip.written(REG_FREQUENCY_LOW), Some(0x67B0));
    // Audio starts muted because nothing has been observed yet.
    assert_eq!(chip.written(REG_AF_OUTPUT), Some(0x6040));

    // A carrier without the coded tone must not open audio.
    chip.set(REG_RSSI, 0x0100);
    chip.set(REG_SQUELCH_STATUS, 0x0002);
    chip.set(REG_INTERRUPT_FLAGS, 1 << 6);
    let metrics = radio
        .receive_metrics(activation.setup.tone)
        .expect("metrics");
    assert_eq!(metrics.tone, Some(ToneStatus::Lost));
    assert!(!metrics.should_unmute());
    let update = controller
        .observe(ReceiveObservation {
            squelch_open: metrics.squelch_open,
            tone_matched: metrics.tone.map(|tone| tone == ToneStatus::Matched),
        })
        .expect("observation");
    assert!(!update.audio_open);

    // The matching tone opens audio through the controller's decision.
    chip.set(REG_INTERRUPT_FLAGS, 1 << 7);
    let metrics = radio
        .receive_metrics(activation.setup.tone)
        .expect("metrics");
    assert!(metrics.should_unmute());
    let update = controller
        .observe(ReceiveObservation {
            squelch_open: metrics.squelch_open,
            tone_matched: metrics.tone.map(|tone| tone == ToneStatus::Matched),
        })
        .expect("observation");
    assert!(update.audio_open);
    radio
        .set_af_output(activation.setup.modulation, AfOutput::Demodulated)
        .expect("audio route");
    assert_eq!(chip.written(REG_AF_OUTPUT), Some(0x6140));

    // Scanning skips the marked channel and retunes the receiver.
    let start = controller.start_scanning().expect("scan start");
    let TimerDirective::Arm { token, .. } = start.timer else {
        panic!("scanning must arm a timer");
    };
    let advanced = controller.timer_elapsed(token).expect("dwell expiry");
    let activation = advanced.activation.expect("scan activation");
    assert_eq!(activation.setup.frequency.as_hz(), 145_300_000);
    apply(&mut radio, activation.setup, advanced.audio_open);
    assert_eq!(chip.written(REG_FREQUENCY_LOW), Some(0xB5D0));
    // An untoned channel disables sub-audio decoding rather than leaving the
    // previous channel's tone armed.
    assert_eq!(chip.written(0x51), Some(0x904A));

    assert!(
        !chip.wrote_transmit_mode(),
        "the receive path must never write the transmit mode word"
    );
}
