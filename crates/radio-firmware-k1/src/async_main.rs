//! Receive-only K1 application: programmable channels, operator shell, audio.
//!
//! Two tasks own disjoint hardware. The serial task owns USART1 and the
//! internal flash and runs the shared AFIK configuration protocol, so the host
//! tooling programs this radio exactly as it programs the simulator. The user
//! interface task owns the display, the keypad, and the bit-banged radio bus.
//!
//! There is no transmit path. The push-to-talk input is read but reaches
//! nothing, every built-in channel denies transmission, and no code here can
//! construct transmit authority.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use py32_hal::gpio::{Flex, Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::peripherals::{FLASH, PA8, SPI1};
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_bk4819::{
    AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, SquelchThresholds, BK4829_PROFILE,
};
use radio_channel_control::{
    BankedReceiveController, ChannelMemory, ChannelReceiveSetup, ReceiveObservation,
};
use radio_device::DeviceService;
use radio_domain::{Modulation, Tone};
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::channels::{built_in, BUILT_IN_CHANNELS};
use radio_firmware_k1::configuration::{
    device_service, Programmed, MAX_CHANNELS, RETAINED_IMAGE_BYTES,
};
use radio_firmware_k1::display::{
    render_channel_list, render_info_screen, render_operating_screen, ListRow, OperatingView,
    COLUMN_OFFSET, FRAME_BYTES, LIST_ROWS, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, KeypadScan, Sample};
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_retained::RetainedConfiguration;
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;
use radio_firmware_k1::shell::{Context, Intent, Screen, Shell};
use radio_protocol::MAX_ENCODED_FRAME;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// Identity this image reports on the information screen.
const IMAGE_IDENTITY: &[u8] = b"AFIK-K1-2.0";

/// Interval between receive samples while audio is routed.
const RF_SAMPLE_MILLISECONDS: u64 = 500;

/// Milliseconds the radio bus stays idle after the last serial byte.
///
/// The three-wire bus is bit-banged and blocks the executor for milliseconds,
/// which would drop inbound bytes. Keeping the bus quiet while the host is
/// mid-exchange makes programming and tuning safe to use in either order.
const LINK_QUIET_MILLISECONDS: u32 = 250;

/// Deadline, in milliseconds since boot, before which the bus must stay idle.
static LINK_QUIET_UNTIL: AtomicU32 = AtomicU32::new(0);

/// Latest configuration the serial task activated.
static PROGRAMMED: Signal<CriticalSectionRawMutex, Publication> = Signal::new();

/// One activated configuration handed to the user interface task.
#[derive(Clone, Copy)]
struct Publication {
    programmed: Programmed,
    generation: u32,
    retained: bool,
}

fn now_ms() -> u32 {
    u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX)
}

fn hold_bus_idle() {
    LINK_QUIET_UNTIL.store(
        now_ms().saturating_add(LINK_QUIET_MILLISECONDS),
        Ordering::Relaxed,
    );
}

fn bus_available() -> bool {
    now_ms() >= LINK_QUIET_UNTIL.load(Ordering::Relaxed)
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
        let Ok(serial) = serial_task(serial, p.FLASH) else {
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

/// Receive state owned by the user interface task.
struct Receiver {
    radio: Bk4819<ThreeWireBus<Bk4819Pins>>,
    /// Active-high receive audio amplifier enable on `PA8`.
    ///
    /// The K1 programming cable shares the speaker and microphone jack, so the
    /// pin is left untouched until the operator explicitly asks for audio.
    speaker: Option<Output<'static>>,
    speaker_pin: Option<PA8>,
    faulted: bool,
    started: bool,
    audio_routed: bool,
    frequency_hz: u32,
    rssi_raw: u16,
    squelch_open: bool,
}

impl Receiver {
    fn new(pins: Bk4819Pins, speaker_pin: PA8) -> Self {
        Self {
            // The pinned K1 build compiles the BK4829 driver, so this board
            // needs that variant's register values.
            radio: Bk4819::with_profile(ThreeWireBus::new(pins), BK4829_PROFILE),
            speaker: None,
            speaker_pin: Some(speaker_pin),
            faulted: false,
            started: false,
            audio_routed: false,
            frequency_hz: 0,
            rssi_raw: 0,
            squelch_open: false,
        }
    }

    /// Routes or mutes demodulated receive audio.
    ///
    /// This drives the receive audio chain only: the chip's audio output and
    /// the speaker amplifier. It cannot key the radio.
    fn set_audio(&mut self, routed: bool) {
        if self.faulted || !self.started {
            return;
        }
        let output = if routed {
            AfOutput::Demodulated
        } else {
            AfOutput::Mute
        };
        if self.radio.set_af_output(Modulation::Fm, output).is_err() {
            self.faulted = true;
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
        if !self.started {
            self.bring_up();
        }
        if self.faulted {
            return;
        }
        let request = ReceiveSetup {
            frequency: setup.frequency,
            modulation: setup.modulation,
            bandwidth: setup.bandwidth,
            tone: setup.tone,
            // The K1 keeps its squelch calibration in external flash which
            // AFIK does not yet read, so this image reports raw metrics with
            // the pinned source's squelch-off set.
            squelch: SquelchThresholds::squelch_off(),
            af: if self.audio_routed {
                AfOutput::Demodulated
            } else {
                AfOutput::Mute
            },
        };
        if self.radio.configure_receive(&request).is_err() {
            self.faulted = true;
            return;
        }
        self.frequency_hz = setup.frequency.as_hz();
        let _ = self.radio.read_back(ReadbackRegister::FilterBandwidth);
    }

    /// Samples metrics from the tuned receiver.
    fn observe(&mut self) -> Option<ReceiveObservation> {
        if self.faulted || !self.started {
            return None;
        }
        match self.radio.receive_metrics(Tone::None) {
            Ok(metrics) => {
                self.rssi_raw = u16::try_from(metrics.rssi_dbm_x2 + 320).unwrap_or(0);
                self.squelch_open = metrics.squelch_open;
                Some(ReceiveObservation {
                    squelch_open: metrics.squelch_open,
                    tone_matched: None,
                })
            }
            Err(_) => {
                self.faulted = true;
                None
            }
        }
    }

    fn bring_up(&mut self) {
        if self.radio.recover_to_standby().is_err() || self.radio.initialise().is_err() {
            self.faulted = true;
            return;
        }
        self.started = true;
    }
}

#[embassy_executor::task]
async fn serial_task(mut uart: Uart<'static, Async>, flash: FLASH) {
    let mut retained = RetainedConfiguration::new(flash);
    let mut image = [0_u8; RETAINED_IMAGE_BYTES];
    // The device stores channels, named banks, and the global configuration,
    // and refuses at validation time to activate more channels than the
    // interface can select.
    let mut service = device_service();

    // Restore the retained configuration before the host or the operator can
    // see this radio. A missing, erased, or corrupt region simply leaves the
    // store empty and the built-in channels in charge.
    let restored = retained
        .read(&mut image)
        .is_some_and(|length| service.load_image(&image[..length]).is_ok());
    publish(&service, restored);

    let mut response = [0_u8; MAX_ENCODED_FRAME];
    let mut received = [0_u8; 1];
    loop {
        if uart.read(&mut received).await.is_err() {
            // Yield before retrying so a persistent receiver error can never
            // starve the interface task.
            Timer::after_millis(1).await;
            continue;
        }
        hold_bus_idle();
        let before = service.generation();
        let Some(length) = service.push(received[0], &mut response, &mut |_| {}) else {
            continue;
        };
        if service.generation() != before {
            // Retain the new configuration before answering. The host is
            // waiting for this response, so masking interrupts for the flash
            // write cannot drop an inbound byte.
            let retained_now = service
                .encode_active_image(&mut image)
                .ok()
                .is_some_and(|length| retained.write(&image, length).is_ok());
            publish(&service, retained_now);
        }
        let _ = uart.write(&response[..length]).await;
        hold_bus_idle();
    }
}

/// Publishes the active configuration for the user interface task.
///
/// A snapshot the interface cannot use is published as an empty configuration
/// rather than dropped, so the display always reports what the radio really
/// holds and falls back to the built-in channels.
fn publish<const OBJECTS: usize>(service: &DeviceService<OBJECTS>, retained: bool) {
    let programmed = Programmed::from_objects(service.active_objects()).unwrap_or_default();
    PROGRAMMED.signal(Publication {
        programmed,
        generation: service.generation(),
        retained: retained && !programmed.is_empty(),
    });
}

/// Builds the receive-only channel set this image ships with.
fn built_in_memory() -> Option<ChannelMemory<MAX_CHANNELS>> {
    let mut memory = ChannelMemory::new();
    for index in 0..BUILT_IN_CHANNELS {
        memory.insert(built_in(index).ok()?).ok()?;
    }
    Some(memory)
}

#[embassy_executor::task]
async fn ui_task(
    mut display: DisplayPins,
    mut keypad: KeypadPins,
    radio_pins: Bk4819Pins,
    speaker_pin: PA8,
) {
    let mut frame = [0_u8; FRAME_BYTES];
    render_info_screen(&mut frame, IMAGE_IDENTITY, 0, 0, false);
    if !display.initialise().await || !display.frame(&frame).await {
        fail_closed();
    }

    // The serial task publishes exactly once at start-up, either the retained
    // configuration or an empty one, so waiting for it avoids showing the
    // built-in set to an operator whose radio is programmed.
    let publication = PROGRAMMED.wait().await;
    let mut generation = publication.generation;
    let mut retained = publication.retained;
    let Some(built_in) = built_in_memory() else {
        fail_closed();
    };
    let mut programmed = publication.programmed;
    let mut memory = if programmed.is_empty() {
        built_in
    } else {
        programmed.memory()
    };
    let Ok((mut controller, update)) =
        BankedReceiveController::activate(memory, programmed.config(), None)
    else {
        fail_closed();
    };

    let mut shell = Shell::new();
    let (banks, bank_count) = programmed.populated_banks();
    shell.set_banks(banks, bank_count);

    let mut receiver = Receiver::new(radio_pins, speaker_pin);
    let mut pending = update.activation.map(|activation| activation.setup);
    let mut debounce = Debouncer::new();
    let mut next_sample = Instant::now();
    let mut redraw = true;

    loop {
        let context = Context {
            visible_channels: controller.visible_channels(),
            active_index: controller.visible_position(),
        };

        if let Some(publication) = PROGRAMMED.try_take() {
            generation = publication.generation;
            retained = publication.retained;
            programmed = publication.programmed;
            memory = if programmed.is_empty() {
                built_in
            } else {
                programmed.memory()
            };
            if let Ok((replacement, update)) =
                BankedReceiveController::activate(memory, programmed.config(), None)
            {
                controller = replacement;
                pending = update.activation.map(|activation| activation.setup);
            }
            let (banks, bank_count) = programmed.populated_banks();
            shell.set_banks(banks, bank_count);
            redraw = true;
        }

        let sample = match decode(keypad.scan().await) {
            Ok(Some(key)) => Sample::Key(key),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        };
        let now = now_ms();

        let intent = match debounce.update(now, sample) {
            Edge::Pressed(key) => shell.press(key, now, context),
            _ => shell.tick(now, context),
        };
        match intent {
            Intent::Idle => {}
            Intent::Redraw => redraw = true,
            Intent::ToggleAudio => {
                if bus_available() {
                    receiver.set_audio(!receiver.audio_routed);
                }
                redraw = true;
            }
            Intent::ToggleMonitor => {
                let update = controller.set_monitor(!controller.is_monitoring());
                if let Some(activation) = update.activation {
                    pending = Some(activation.setup);
                }
                redraw = true;
            }
            Intent::SelectNext
            | Intent::SelectPrevious
            | Intent::SelectIndex(_)
            | Intent::SetBank(_) => {
                let update = match intent {
                    Intent::SelectNext => controller.select_next(),
                    Intent::SelectPrevious => controller.select_previous(),
                    Intent::SelectIndex(position) => controller.select_visible(position),
                    Intent::SetBank(bank) => controller.set_bank(bank),
                    _ => unreachable!(),
                };
                if let Some(activation) = update.ok().and_then(|update| update.activation) {
                    pending = Some(activation.setup);
                }
                redraw = true;
            }
        }

        // The bit-banged radio bus blocks the executor, so it only runs while
        // the serial link is quiet. Retuning is deferred, never dropped.
        if let Some(setup) = pending {
            if bus_available() {
                receiver.tune(setup);
                pending = None;
                redraw = true;
            }
        } else if receiver.audio_routed && Instant::now() >= next_sample && bus_available() {
            if let Some(observation) = receiver.observe() {
                controller.observe(observation).ok();
            }
            next_sample = Instant::now() + Duration::from_millis(RF_SAMPLE_MILLISECONDS);
            redraw = true;
        }

        if redraw {
            render(
                &mut frame,
                &shell,
                &controller,
                &receiver,
                generation,
                retained,
            );
            if !display.frame(&frame).await {
                fail_closed();
            }
            redraw = false;
        }

        Timer::after(Duration::from_millis(5)).await;
    }
}

fn render(
    frame: &mut [u8; FRAME_BYTES],
    shell: &Shell,
    controller: &BankedReceiveController<ChannelMemory<MAX_CHANNELS>>,
    receiver: &Receiver,
    generation: u32,
    retained: bool,
) {
    let visible = controller.visible_channels();
    match shell.screen() {
        Screen::Operating => render_operating_screen(
            frame,
            &OperatingView {
                position: controller.visible_position().saturating_add(1),
                total: visible,
                name: controller.channel().name().as_str().as_bytes(),
                frequency_hz: receiver.frequency_hz,
                rssi_raw: receiver.rssi_raw,
                squelch_open: receiver.squelch_open,
                audio_routed: receiver.audio_routed,
                monitoring: controller.is_monitoring(),
                bank: shell.bank_filter().map(|bank| bank.get()),
                entry: shell.entry(),
            },
        ),
        Screen::ChannelList => {
            let mut rows = [ListRow::default(); LIST_ROWS];
            let visible_rows = u16::try_from(LIST_ROWS).unwrap_or(u16::MAX);
            // Scroll by whole pages so the cursor is always on screen without
            // the list jittering under a single key press.
            let first = shell.cursor() / visible_rows * visible_rows;
            let mut count = 0;
            for offset in 0..visible_rows {
                let position = first + offset;
                let Some(channel) = controller.visible_channel(position) else {
                    break;
                };
                rows[usize::from(offset)] = ListRow::new(
                    position.saturating_add(1),
                    channel.name().as_str().as_bytes(),
                    position == controller.visible_position(),
                );
                count += 1;
            }
            render_channel_list(
                frame,
                &rows[..count],
                usize::from(shell.cursor() - first),
                visible,
            );
        }
        Screen::Info => render_info_screen(frame, IMAGE_IDENTITY, generation, visible, retained),
    }
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
