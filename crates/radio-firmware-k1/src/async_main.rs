//! Receive-only K1 Embassy keypad/display/UART witness.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_time::{Duration, Instant, Timer};
use py32_hal::gpio::{Flex, Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::peripherals::{PA8, SPI1};
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_bk4819::{
    AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, SquelchThresholds, BK4829_PROFILE,
};
use radio_channel_control::{
    BankedReceiveController, ChannelMemory, ChannelReceiveSetup, ReceiveObservation,
};
use radio_domain::{Modulation, RadioConfig, Tone};
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::channels::{built_in, BUILT_IN_CHANNELS};
use radio_firmware_k1::display::{
    render_channel_screen, render_witness, COLUMN_OFFSET, FRAME_BYTES, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, Key, KeypadScan, Sample};
use radio_firmware_k1::protocol::{
    decode_request, encode_hello_response, encode_rf_response, Request, RfObservation,
    REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES, RF_RESPONSE_FRAME_BYTES, RF_STAGE_FAULTED,
    RF_STAGE_INITIALISED, RF_STAGE_RECEIVING, RF_STAGE_STANDBY, RF_STAGE_UNSTARTED,
};
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// Interval between receive samples while audio is routed.
const RF_SAMPLE_MILLISECONDS: u64 = 500;

/// Latest receive snapshot published by the task which owns the radio.
///
/// Only that task touches the bus. The serial responder reads these words, so
/// answering a request never bit-bangs a transfer beside an inbound frame.
static RF_SNAPSHOT: AtomicU32 = AtomicU32::new(0);
static RF_IDENTITY: AtomicU32 = AtomicU32::new(0);
static RF_FREQUENCY: AtomicU32 = AtomicU32::new(0);

fn publish(observation: RfObservation) {
    RF_IDENTITY.store(
        u32::from(observation.identity_address) << 16 | u32::from(observation.identity_register),
        Ordering::Relaxed,
    );
    let rssi = u32::from(u16::from_le_bytes(observation.rssi_dbm_x2.to_le_bytes()));
    let flags = u32::from(observation.stage & 0x0F) << 28
        | u32::from(u8::from(observation.squelch_open)) << 27
        | u32::from(u8::from(observation.audio_routed)) << 26;
    RF_SNAPSHOT.store(
        flags | (rssi & 0xFFFF) << 8 | u32::from(observation.glitch),
        Ordering::Relaxed,
    );
    RF_NOISE_SAMPLES.store(
        u32::from(observation.noise) << 16 | u32::from(observation.samples),
        Ordering::Relaxed,
    );
    RF_FREQUENCY.store(observation.frequency_hz, Ordering::Relaxed);
}

static RF_NOISE_SAMPLES: AtomicU32 = AtomicU32::new(0);

fn published() -> RfObservation {
    let identity = RF_IDENTITY.load(Ordering::Relaxed);
    let snapshot = RF_SNAPSHOT.load(Ordering::Relaxed);
    let noise_samples = RF_NOISE_SAMPLES.load(Ordering::Relaxed);
    let stage = u8::try_from(snapshot >> 28 & 0x0F).unwrap_or(RF_STAGE_UNSTARTED);
    RfObservation {
        identity_register: u16::try_from(identity & 0xFFFF).unwrap_or(0),
        identity_address: u8::try_from(identity >> 16 & 0xFF).unwrap_or(0),
        stage,
        frequency_hz: RF_FREQUENCY.load(Ordering::Relaxed),
        rssi_dbm_x2: i16::from_le_bytes(
            u16::try_from(snapshot >> 8 & 0xFFFF)
                .unwrap_or(0)
                .to_le_bytes(),
        ),
        glitch: u8::try_from(snapshot & 0xFF).unwrap_or(0),
        noise: u8::try_from(noise_samples >> 16 & 0xFF).unwrap_or(0),
        squelch_open: snapshot >> 27 & 1 == 1,
        audio_routed: snapshot >> 26 & 1 == 1,
        samples: u16::try_from(noise_samples & 0xFFFF).unwrap_or(0),
    }
}

#[entry]
fn main() -> ! {
    relocate_vectors();
    let Ok(runtime_init) = init() else {
        fail_closed();
    };
    let p = runtime_init.peripherals;
    let Ok(runtime) = compose(K1RuntimePeripherals {
        usart: p.USART1,
        usart_rx: p.PA10,
        usart_tx: p.PA9,
        usart_tx_dma: p.DMA1_CH1,
        usart_rx_dma: p.DMA1_CH2,
        display_spi: p.SPI1,
        display_sck: p.PA5,
        display_mosi: p.PA7,
    }) else {
        fail_closed();
    };

    let mut executor = runtime.executor;
    let serial = runtime.usart;
    let display = DisplayPins {
        spi: runtime.display_spi,
        a0: Output::new(p.PA6, Level::High, Speed::High),
        cs: Output::new(p.PB2, Level::High, Speed::High),
        backlight: Output::new(p.PF8, Level::High, Speed::High),
    };
    let keypad = KeypadPins {
        columns: [
            Output::new(p.PB6, Level::High, Speed::High),
            Output::new(p.PB5, Level::High, Speed::High),
            Output::new(p.PB4, Level::High, Speed::High),
            Output::new(p.PB3, Level::High, Speed::High),
        ],
        rows: [
            Input::new(p.PB15, Pull::Up),
            Input::new(p.PB14, Pull::Up),
            Input::new(p.PB13, Pull::Up),
            Input::new(p.PB12, Pull::Up),
        ],
        // Active-low PTT, read as an input only. AFIK implements no transmit
        // path, so this pin cannot key the radio.
        ptt: Input::new(p.PB10, Pull::Up),
    };

    // Receive-only BK4819 three-wire pins from the pinned K1 pinout. AFIK
    // constructs no transmit authority, so this bus cannot key the radio.
    let radio_pins = Bk4819Pins::new(
        Output::new(p.PF9, Level::High, Speed::High),
        Output::new(p.PB8, Level::High, Speed::High),
        Flex::new(p.PB9),
    );

    // SAFETY: `main` never returns, so this stack allocation lives for the
    // executor's complete lifetime and no other mutable reference is retained.
    #[allow(unsafe_code)]
    let executor: &'static mut Executor = unsafe { core::mem::transmute(&mut executor) };
    executor.run(|spawner| {
        let Ok(serial) = serial_task(serial) else {
            fail_closed();
        };
        let Ok(ui) = ui_task(display, keypad, radio_pins, p.PA8) else {
            fail_closed();
        };
        spawner.spawn(serial);
        spawner.spawn(ui);
    });
}

#[allow(unsafe_code)]
#[unsafe(export_name = "k1_relocate_vectors")]
#[inline(never)]
fn relocate_vectors() {
    let Some(core) = cortex_m::Peripherals::take() else {
        fail_closed();
    };
    // SAFETY: the PY32F071 device header declares VTOR present, the pinned K1
    // startup writes FLASH_BASE + 0x2800, and the static image gate validates a
    // complete vector table at that exact aligned application origin.
    unsafe {
        core.SCB.vtor.write(K1_VECTOR_TABLE_ORIGIN);
    }
}

struct DisplayPins {
    spi: SpiTx<'static, SPI1>,
    a0: Output<'static>,
    cs: Output<'static>,
    backlight: Output<'static>,
}

impl DisplayPins {
    async fn transfer(&mut self, data: bool, bytes: &[u8]) -> bool {
        self.a0
            .set_level(if data { Level::High } else { Level::Low });
        self.cs.set_low();
        let result = self.spi.write(bytes).await.is_ok();
        self.cs.set_high();
        result
    }

    async fn initialise(&mut self) -> bool {
        self.backlight.set_high();
        if !self.transfer(false, &[0xE2]).await {
            return false;
        }
        Timer::after_millis(120).await;
        if !self.transfer(false, &SETUP_COMMANDS).await || !self.transfer(false, &[0x2B]).await {
            return false;
        }
        Timer::after_millis(1).await;
        if !self.transfer(false, &[0x2E]).await {
            return false;
        }
        Timer::after_millis(1).await;
        if !self.transfer(false, &[0x2F]).await {
            return false;
        }
        Timer::after_millis(40).await;
        self.transfer(false, &[0x40, 0xAF]).await
    }

    async fn frame(&mut self, frame: &[u8; FRAME_BYTES]) -> bool {
        if !self.transfer(false, &[0x40]).await {
            return false;
        }
        for page in 0_u8..8 {
            let address = [
                0xB0 | page,
                0x10 | (COLUMN_OFFSET >> 4),
                COLUMN_OFFSET & 0x0F,
            ];
            if !self.transfer(false, &address).await {
                return false;
            }
            let start = usize::from(page) * WIDTH;
            if !self.transfer(true, &frame[start..start + WIDTH]).await {
                return false;
            }
        }
        true
    }
}

struct KeypadPins {
    columns: [Output<'static>; 4],
    rows: [Input<'static>; 4],
    ptt: Input<'static>,
}

impl KeypadPins {
    fn read_rows(&self) -> u8 {
        let mut mask = 0_u8;
        for (row, input) in self.rows.iter().enumerate() {
            if input.is_low() {
                mask |= 1 << row;
            }
        }
        mask
    }

    async fn scan(&mut self) -> KeypadScan {
        for column in &mut self.columns {
            column.set_high();
        }

        // Unselected pass first, matching the pinned source's scan order. With
        // every column high no matrix button can pull a row low, so a low row
        // is a side key wired directly to it.
        Timer::after_micros(10).await;
        let unselected = self.read_rows();

        let mut columns = [0_u8; 4];
        for (index, column) in self.columns.iter_mut().enumerate() {
            column.set_low();
            Timer::after_micros(10).await;
            for (row, input) in self.rows.iter().enumerate() {
                if input.is_low() {
                    columns[index] |= 1 << row;
                }
            }
            column.set_high();
        }

        KeypadScan {
            unselected,
            columns,
            ptt_pressed: self.ptt.is_low(),
        }
    }
}

/// Receiver bring-up state owned by the serial task.
///
/// Every BK4819 transfer runs inside a request, never concurrently with one.
/// The bus is bit-banged and blocks the executor for a few milliseconds, so
/// doing it while the host is mid-request would drop received bytes.
struct Receiver {
    radio: Bk4819<ThreeWireBus<Bk4819Pins>>,
    /// Active-high receive audio amplifier enable on `PA8`.
    ///
    /// The K1 programming cable shares the speaker and microphone jack, so the
    /// pin is left untouched until the operator explicitly asks for audio.
    speaker: Option<Output<'static>>,
    speaker_pin: Option<PA8>,
    stage: u8,
    identity_address: u8,
    identity_register: u16,
    samples: u16,
    audio_routed: bool,
    frequency_hz: u32,
}

impl Receiver {
    fn new(pins: Bk4819Pins, speaker_pin: PA8) -> Self {
        Self {
            // The pinned K1 build compiles the BK4829 driver, so this board
            // needs that variant's register values.
            radio: Bk4819::with_profile(ThreeWireBus::new(pins), BK4829_PROFILE),
            speaker: None,
            speaker_pin: Some(speaker_pin),
            stage: RF_STAGE_UNSTARTED,
            identity_address: 0,
            identity_register: 0,
            samples: 0,
            audio_routed: false,
            frequency_hz: 0,
        }
    }

    /// Routes or mutes demodulated receive audio.
    ///
    /// This drives the receive audio chain only: the chip's audio output and
    /// the speaker amplifier. It cannot key the radio.
    fn set_audio(&mut self, routed: bool) {
        if self.stage != RF_STAGE_RECEIVING {
            return;
        }
        let output = if routed {
            AfOutput::Demodulated
        } else {
            AfOutput::Mute
        };
        if self.radio.set_af_output(Modulation::Fm, output).is_err() {
            self.stage = RF_STAGE_FAULTED;
            return;
        }
        if self.speaker.is_none() {
            if let Some(pin) = self.speaker_pin.take() {
                self.speaker = Some(Output::new(pin, Level::Low, Speed::Low));
            }
        }
        if let Some(speaker) = self.speaker.as_mut() {
            if routed {
                speaker.set_high();
            } else {
                speaker.set_low();
            }
        }
        self.audio_routed = routed;
    }

    /// Applies one controller-selected channel to the receiver.
    fn tune(&mut self, setup: ChannelReceiveSetup) {
        if self.stage < RF_STAGE_INITIALISED {
            self.bring_up();
        }
        if self.stage == RF_STAGE_FAULTED {
            return;
        }
        let request = ReceiveSetup {
            frequency: setup.frequency,
            modulation: setup.modulation,
            bandwidth: setup.bandwidth,
            tone: setup.tone,
            // The K1 keeps its squelch calibration in external flash which
            // AFIK does not yet read, so this witness reports raw metrics
            // with the pinned source's squelch-off set.
            squelch: SquelchThresholds::squelch_off(),
            af: if self.audio_routed {
                AfOutput::Demodulated
            } else {
                AfOutput::Mute
            },
        };
        if self.radio.configure_receive(&request).is_err() {
            self.stage = RF_STAGE_FAULTED;
            return;
        }
        self.frequency_hz = setup.frequency.as_hz();
        self.stage = RF_STAGE_RECEIVING;
        if let Ok(value) = self.radio.read_back(ReadbackRegister::FilterBandwidth) {
            self.identity_address = ReadbackRegister::FilterBandwidth.address();
            self.identity_register = value;
        }
    }

    /// Samples metrics from the tuned receiver.
    fn observe(&mut self) -> RfObservation {
        if self.stage == RF_STAGE_RECEIVING {
            match self.radio.receive_metrics(Tone::None) {
                Ok(metrics) => {
                    self.samples = self.samples.saturating_add(1);
                    return RfObservation {
                        identity_register: self.identity_register,
                        identity_address: self.identity_address,
                        stage: self.stage,
                        frequency_hz: self.frequency_hz,
                        rssi_dbm_x2: metrics.rssi_dbm_x2,
                        glitch: metrics.glitch,
                        noise: metrics.noise,
                        squelch_open: metrics.squelch_open,
                        samples: self.samples,
                        audio_routed: self.audio_routed,
                    };
                }
                Err(_) => self.stage = RF_STAGE_FAULTED,
            }
        }
        RfObservation {
            identity_register: self.identity_register,
            identity_address: self.identity_address,
            stage: self.stage,
            frequency_hz: 0,
            samples: self.samples,
            audio_routed: self.audio_routed,
            ..RfObservation::default()
        }
    }

    fn bring_up(&mut self) {
        if self.radio.recover_to_standby().is_err() {
            self.stage = RF_STAGE_FAULTED;
            return;
        }
        self.stage = RF_STAGE_STANDBY;

        // The pinned power-on register table must run before any receive
        // configuration can produce meaningful metrics.
        if self.radio.initialise().is_err() {
            self.stage = RF_STAGE_FAULTED;
            return;
        }
        self.stage = RF_STAGE_INITIALISED;
    }
}

#[embassy_executor::task]
async fn serial_task(mut uart: Uart<'static, Async>) {
    let mut window = [0_u8; 16];
    let mut used = 0_usize;
    loop {
        if uart.read(&mut window[used..=used]).await.is_err() {
            // Yield before retrying so a persistent receiver error can never
            // starve the display task.
            Timer::after_millis(1).await;
            continue;
        }
        used += 1;
        if used >= 2 && window[used - 2..used] == [0xAB, 0xCD] {
            window.copy_within(used - 2..used, 0);
            used = 2;
        }
        if used != window.len() {
            continue;
        }
        if window[2..4] == 8_u16.to_le_bytes() && window[14..16] == [0xDC, 0xBA] {
            let mut body = [0_u8; REQUEST_BODY_BYTES];
            body.copy_from_slice(&window[4..14]);
            match decode_request(&mut body) {
                Some(Request::Hello) => {
                    let mut response = [0_u8; RESPONSE_FRAME_BYTES];
                    encode_hello_response(&mut response);
                    let _ = uart.write(&response).await;
                }
                // Serial only reads the published snapshot. The programming
                // cable shares the speaker jack, so audio is operator
                // controlled from the keypad, never from this link.
                Some(Request::RfProbe | Request::RfAudio(_)) => {
                    let mut response = [0_u8; RF_RESPONSE_FRAME_BYTES];
                    encode_rf_response(&mut response, published());
                    let _ = uart.write(&response).await;
                }
                _ => {}
            }
        }
        used = 0;
    }
}

#[embassy_executor::task]
async fn ui_task(
    mut display: DisplayPins,
    mut keypad: KeypadPins,
    radio_pins: Bk4819Pins,
    speaker_pin: PA8,
) {
    let mut frame = [0_u8; FRAME_BYTES];
    render_witness(&mut frame);
    if !display.initialise().await || !display.frame(&frame).await {
        fail_closed();
    }

    let mut memory = ChannelMemory::<BUILT_IN_CHANNELS>::new();
    for index in 0..BUILT_IN_CHANNELS {
        let Ok(channel) = built_in(index) else {
            fail_closed();
        };
        if memory.insert(channel).is_err() {
            fail_closed();
        }
    }
    let Ok((mut controller, update)) =
        BankedReceiveController::activate(memory, RadioConfig::conservative(), None)
    else {
        fail_closed();
    };

    // This task owns the radio because the audio toggle belongs on the keypad:
    // the programming cable shares the speaker jack, so audio cannot be heard
    // or safely switched over the serial link.
    let mut receiver = Receiver::new(radio_pins, speaker_pin);
    if let Some(activation) = update.activation {
        receiver.tune(activation.setup);
    }
    let mut observation = receiver.observe();
    publish(observation);

    let mut debounce = Debouncer::new();
    let mut next_sample = Instant::now();
    let mut redraw = true;
    loop {
        let sample = match decode(keypad.scan().await) {
            Ok(Some(key)) => Sample::Key(key),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        };
        let now = u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX);

        if let Edge::Pressed(key) = debounce.update(now, sample) {
            let selection = match key {
                // Side key one toggles receive audio. AFIK implements no
                // transmit path, so this can only route received audio.
                Key::Side1 => {
                    receiver.set_audio(!observation.audio_routed);
                    None
                }
                Key::Up => controller.select_next().ok(),
                Key::Down => controller.select_previous().ok(),
                _ => None,
            };
            if let Some(activation) = selection.and_then(|update| update.activation) {
                receiver.tune(activation.setup);
            }
            observation = receiver.observe();
            publish(observation);
            redraw = true;
        }

        // Metering runs only while audio is routed, which is also when the
        // cable is expected to be unplugged. With audio muted the bus stays
        // idle so the serial link is never disturbed.
        if observation.audio_routed && Instant::now() >= next_sample {
            observation = receiver.observe();
            controller
                .observe(ReceiveObservation {
                    squelch_open: observation.squelch_open,
                    tone_matched: None,
                })
                .ok();
            publish(observation);
            next_sample = Instant::now() + Duration::from_millis(RF_SAMPLE_MILLISECONDS);
            redraw = true;
        }

        if redraw {
            render_channel_screen(
                &mut frame,
                controller.channel().name().as_str().as_bytes(),
                observation.frequency_hz,
                rssi_raw(observation),
                observation.squelch_open,
                observation.audio_routed,
            );
            if !display.frame(&frame).await {
                fail_closed();
            }
            redraw = false;
        }

        Timer::after(Duration::from_millis(5)).await;
    }
}

/// Converts the reported half-dBm value back to the chip's raw RSSI count.
fn rssi_raw(observation: RfObservation) -> u16 {
    u16::try_from(observation.rssi_dbm_x2 + 320).unwrap_or(0)
}

fn fail_closed() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    fail_closed()
}
