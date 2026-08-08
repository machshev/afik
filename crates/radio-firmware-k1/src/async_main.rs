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

use core::cell::RefCell;
use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use py32_hal::gpio::{Flex, Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::peripherals::{PA0, PA1, PA2, PA3, PA8, SPI1, SPI2};
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_bk4819::{
    AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, SquelchThresholds, BK4829_PROFILE,
};
use radio_channel_control::{
    BankedReceiveController, ChannelReceiveSetup, ChannelSource, ReceiveObservation,
};
use radio_channel_plan::{
    BankMask, BankName, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
};
use radio_device::{DeviceEvent, DeviceService};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel,
    Tone, TxClass,
};
use radio_firmware_k1::battery::{Battery, Calibration};
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::configuration::{
    device_service, store_squelch, Programmed, CONFIGURATION_STORE_BYTES, RETAINED_IMAGE_BYTES,
};
use radio_firmware_k1::display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    BankIndicator, ListRow, MemoryState, OperatingView, SelectorRow, SerialCounters, COLUMN_OFFSET,
    FRAME_BYTES, LIST_ROWS, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, KeypadScan, Sample};
use radio_firmware_k1::py32f071_battery::BatterySense;
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_eeprom::{RetainError, RetainedConfiguration};
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;
use radio_firmware_k1::shell::{
    Context, Intent, Mode, Screen, Setting, Shell, Source, SETTINGS, SQUELCH_LEVELS, VFO_STEPS_HZ,
};
use radio_protocol::MAX_ENCODED_FRAME;
use radio_storage::ObjectArena;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// Identity this image reports on the information screen.
const IMAGE_IDENTITY: &[u8] = b"AFIK-K1-5.0";

/// Interval between receive samples while audio is routed.
///
/// This is the squelch's reaction time as well as the meter's refresh rate: the
/// speaker opens and shuts on these samples, so a half-second interval would
/// clip the start of every transmission and leave noise after the end of it.
/// One sample is a handful of bit-banged register reads, and the serial link
/// still holds the bus off while a host is mid-exchange.
const RF_SAMPLE_MILLISECONDS: u64 = 60;

/// Shortest interval between redraws caused by a changed meter reading.
const METER_REDRAW_MILLISECONDS: u64 = 300;

/// Seconds between battery conversions.
///
/// A pack discharges over hours, so this is about how quickly the operator
/// should see a fresh pack after a battery change rather than about tracking
/// the discharge itself.
const BATTERY_SAMPLE_SECONDS: u64 = 2;

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

/// The battery calibration the serial task read from the vendor block.
///
/// The external memory is on the serial task's bus, so it reads the calibration
/// once and hands it over. Until it does, the interface reports that it does not
/// know the charge rather than a number derived from nothing.
static BATTERY_CALIBRATION: Signal<CriticalSectionRawMutex, Option<Calibration>> = Signal::new();

/// Squelch level the operator chose on the handset.
///
/// The serial task owns the store and the external memory, so the interface
/// asks rather than writes. A choice the operator makes while a host is
/// mid-exchange waits its turn instead of racing it.
static SQUELCH_CHOICE: Signal<CriticalSectionRawMutex, SquelchLevel> = Signal::new();

/// One activated configuration handed to the user interface task.
#[derive(Clone, Copy)]
struct Publication {
    programmed: Programmed,
    generation: u32,
    retained: bool,
    memory: MemoryState,
}

fn now_ms() -> u32 {
    u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX)
}

/// Bytes the serial task has received, for the information screen.
static SERIAL_RECEIVED: AtomicU32 = AtomicU32::new(0);
/// Frames the serial task has answered, for the information screen.
static SERIAL_ANSWERED: AtomicU32 = AtomicU32::new(0);
/// Peripheral clock this image inherited, in kilohertz.
///
/// The image does not configure the clock; `init_inherited` adopts whatever the
/// bootloader left in the RCC, and every baud rate is derived from it. It is
/// published here so the information screen can show what the radio believes,
/// because a wrong inheritance is otherwise visible only as a serial link which
/// hears bytes and rejects every packet.
static PERIPHERAL_CLOCK_KHZ: AtomicU32 = AtomicU32::new(0);

/// Complete packets the serial task rejected as malformed.
///
/// The device service reports these to an observer, and this image discarded
/// them silently, which left a hole exactly where a fault hides: a frame heard
/// and rejected looked the same as a frame never received. Counting them is
/// what tells those apart without a host.
static SERIAL_DISCARDED: AtomicU32 = AtomicU32::new(0);

/// Returns the current serial counters.
fn serial_counters() -> SerialCounters {
    SerialCounters {
        received: u16::try_from(SERIAL_RECEIVED.load(Ordering::Relaxed) % 10_000).unwrap_or(0),
        answered: u16::try_from(SERIAL_ANSWERED.load(Ordering::Relaxed) % 10_000).unwrap_or(0),
        discarded: u16::try_from(SERIAL_DISCARDED.load(Ordering::Relaxed) % 10_000).unwrap_or(0),
    }
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
    PERIPHERAL_CLOCK_KHZ.store(runtime_init.clocks.pclk1_hz() / 1_000, Ordering::Relaxed);
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
        let Ok(serial) = serial_task(
            serial,
            EepromPins {
                spi: p.SPI2,
                sck: p.PA0,
                mosi: p.PA1,
                miso: p.PA2,
                chip_select: p.PA3,
            },
        ) else {
            fail_closed();
        };
        let Ok(ui) = ui_task(
            display,
            keypad,
            radio_pins,
            p.PA8,
            BatterySense::new(p.ADC1, p.PB0),
        ) else {
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
    /// The pin is claimed the first time a channel is tuned rather than at boot,
    /// because the K1 programming cable shares the speaker and microphone jack
    /// and an unprogrammed radio has nothing to play through it.
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

    /// Enables the receive audio chain.
    ///
    /// A receiver nobody can hear is not receiving, so this is not an operator
    /// mode: audio follows the tuned channel. This drives the receive audio
    /// chain only, the chip's audio output and the speaker amplifier, and it
    /// cannot key the radio.
    fn route_audio(&mut self) {
        if self.faulted || !self.started {
            return;
        }
        if self
            .radio
            .set_af_output(Modulation::Fm, AfOutput::Demodulated)
            .is_err()
        {
            self.faulted = true;
            return;
        }
        if self.speaker.is_none() {
            if let Some(pin) = self.speaker_pin.take() {
                self.speaker = Some(Output::new(pin, Level::Low, Speed::Low));
            }
        }
        self.audio_routed = true;
        // The amplifier is claimed, not opened: the squelch decides what the
        // operator actually hears, and it has not been sampled on this channel
        // yet. A retune therefore lands in silence rather than in noise.
        self.gate_audio();
    }

    /// Drives the speaker amplifier from the squelch link.
    ///
    /// The chip's carrier squelch is the decision and this is the consequence,
    /// so the operator hears exactly what the level they chose lets through. At
    /// level zero, and while monitoring, the link reads permanently open and
    /// the amplifier simply stays on.
    fn gate_audio(&mut self) {
        let open = self.squelch_open;
        if let Some(speaker) = self.speaker.as_mut() {
            if open {
                speaker.set_high();
            } else {
                speaker.set_low();
            }
        }
    }

    /// Applies one controller-selected channel to the receiver.
    ///
    /// `radio_wide` is the level the operator set for the whole radio. It
    /// replaces whatever level the channel was programmed with, because a
    /// global control a stored channel could silently veto would not be one.
    /// A level the controller has already forced open is left alone: that is
    /// monitoring, and monitoring outranks both.
    fn tune(&mut self, setup: ChannelReceiveSetup, radio_wide: SquelchLevel) {
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
            squelch: SquelchThresholds::for_level(if setup.squelch.is_open() {
                setup.squelch
            } else {
                radio_wide
            }),
            af: AfOutput::Demodulated,
        };
        if self.radio.configure_receive(&request).is_err() {
            self.faulted = true;
            return;
        }
        self.frequency_hz = setup.frequency.as_hz();
        // The previous channel's link result says nothing about this one.
        self.squelch_open = false;
        let _ = self.radio.read_back(ReadbackRegister::FilterBandwidth);
        self.route_audio();
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
                self.gate_audio();
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

/// The external configuration memory's peripheral and pins.
///
/// `EVID-K1-060` records this wiring: the memory is on `SPI2` with an
/// active-low chip select the peripheral does not drive.
struct EepromPins {
    spi: SPI2,
    sck: PA0,
    mosi: PA1,
    miso: PA2,
    chip_select: PA3,
}

#[embassy_executor::task]
async fn serial_task(mut uart: Uart<'static, Async>, memory: EepromPins) {
    // A radio which cannot reach its external memory is still a working
    // receiver, so an absent or unresponsive memory leaves the store empty
    // rather than stopping the radio.
    let mut image = [0_u8; RETAINED_IMAGE_BYTES];
    // The device stores channels, named banks, plans, and the global
    // configuration, in whatever mixture fits the bytes it declares.
    let mut service = device_service();

    // Publish before touching the external memory. The interface task waits for
    // the first publication before it reads a key, so anything slow or broken
    // in the memory path would otherwise leave the operator with a frozen
    // screen and no way out. An empty configuration is the honest state until
    // a restore succeeds.
    publish(&service, false, MemoryState::Unknown);

    // The external memory is opened read-only first. Its device and wiring come
    // from the pinned reference firmware and have not been observed on this
    // unit, so the identification is read, reported on the information screen,
    // and only a memory which answers is used. The operator interface does not
    // wait for any of this, so a memory which never answers costs a bounded
    // delay here and nothing else.
    let mut memory_state = MemoryState::Failed;
    let mut retained = RetainedConfiguration::new(
        memory.spi,
        memory.sck,
        memory.mosi,
        memory.miso,
        memory.chip_select,
    )
    .ok()
    .and_then(|mut configuration| match configuration.identify() {
        Ok(id) => {
            memory_state =
                MemoryState::Present([id.manufacturer, id.memory_type, id.capacity_code]);
            Some(configuration)
        }
        Err(RetainError::Absent(_)) => {
            memory_state = MemoryState::Absent;
            None
        }
        Err(_) => {
            memory_state = MemoryState::Failed;
            None
        }
    });

    // The calibration is the radio's own data and never changes, so it is read
    // once, here, while the bus is already in hand.
    BATTERY_CALIBRATION.signal(
        retained
            .as_mut()
            .and_then(RetainedConfiguration::read_battery_calibration),
    );

    // Restore the retained configuration before the host or the operator can
    // see this radio. A missing, erased, or corrupt region simply leaves the
    // store empty and the built-in channels in charge.
    let restored = retained.as_mut().is_some_and(|configuration| {
        configuration
            .read(&mut image)
            .is_some_and(|length| service.load_image(&image[..length]).is_ok())
    });
    publish(&service, restored, memory_state);

    let mut response = [0_u8; MAX_ENCODED_FRAME];
    // A whole frame at a time, collected by DMA and delimited by the idle line.
    //
    // Reading one byte per await lost bytes: this core runs one task at a time,
    // and the interface task holds it for the length of a bit-banged BK4819
    // transfer, so a byte arriving in that window had nowhere to go and the
    // frame never completed. `EVID-K1-061` is that failure, seen as a radio
    // which counted received bytes and answered nothing. DMA collects the burst
    // whether or not this task is running.
    let mut received = [0_u8; MAX_ENCODED_FRAME];
    loop {
        // The host and the operator are two sources of change for one store, so
        // this task waits on both and serves whichever arrives. Nothing here
        // interrupts a frame in progress: `read_until_idle` has already returned
        // by the time a handset choice can be taken.
        let count = match select(uart.read_until_idle(&mut received), SQUELCH_CHOICE.wait()).await {
            Either::First(Ok(count)) => count,
            Either::First(Err(_)) => {
                // Yield before retrying so a persistent receiver error can never
                // starve the interface task.
                Timer::after_millis(1).await;
                continue;
            }
            Either::Second(level) => {
                // A handset setting which did not survive a battery change would
                // not be worth the menu, so it is stored exactly like a host
                // write and republished from the store rather than assumed.
                if store_squelch(&mut service, level).is_ok() {
                    let mut retained_now = false;
                    if let Ok(length) = service.encode_active_image(&mut image) {
                        if let Some(configuration) = retained.as_mut() {
                            retained_now = configuration.write(&image, length).await.is_ok();
                        }
                    }
                    publish(&service, retained_now, memory_state);
                }
                continue;
            }
        };
        for index in 0..count {
            let byte = received[index];
            // Only this task writes these, so a load and store is sufficient;
            // this core has no atomic read-modify-write.
            SERIAL_RECEIVED.store(
                SERIAL_RECEIVED.load(Ordering::Relaxed).wrapping_add(1),
                Ordering::Relaxed,
            );
            hold_bus_idle();
            let before = service.generation();
            // Only this task writes this, so a load and store is sufficient.
            let mut observe = |event| {
                if matches!(event, DeviceEvent::PacketDiscarded(_)) {
                    SERIAL_DISCARDED.store(
                        SERIAL_DISCARDED.load(Ordering::Relaxed).wrapping_add(1),
                        Ordering::Relaxed,
                    );
                }
            };
            let Some(length) = service.push(byte, &mut response, &mut observe) else {
                continue;
            };
            if service.generation() != before {
                // Retain the new configuration before answering, so a host told
                // that a transaction committed can rely on it being stored.
                let mut retained_now = false;
                if let Ok(length) = service.encode_active_image(&mut image) {
                    if let Some(configuration) = retained.as_mut() {
                        retained_now = configuration.write(&image, length).await.is_ok();
                    }
                }
                publish(&service, retained_now, memory_state);
            }
            SERIAL_ANSWERED.store(
                SERIAL_ANSWERED.load(Ordering::Relaxed).wrapping_add(1),
                Ordering::Relaxed,
            );
            let _ = uart.write(&response[..length]).await;
            hold_bus_idle();
        }
    }
}

/// Publishes the active configuration for the user interface task.
///
/// A snapshot the interface cannot use is published as an empty configuration
/// rather than dropped, so the display always reports what the radio really
/// holds and falls back to the built-in channels.
fn publish<const BYTES: usize>(
    service: &DeviceService<BYTES>,
    retained: bool,
    memory: MemoryState,
) {
    store_objects(service.active_payload());
    let programmed = Programmed::index(service.active_objects()).unwrap_or_default();
    PROGRAMMED.signal(Publication {
        programmed,
        generation: service.generation(),
        retained: retained && !programmed.is_empty(),
        memory,
    });
}

/// Builds the receive source the shell says the operator is listening to.
///
/// The image carries no channel set of its own: the VFO is the source which
/// always has something to tune, so an unprogrammed radio is a VFO radio rather
/// than an inert one. Both sources drive the same banked controller, so tuning,
/// monitoring, and metering behave identically either way.

/// The one RAM copy of the configuration, as the external memory stores it.
///
/// The serial task owns the device service and republishes here; the interface
/// task reads. Nothing decoded is held anywhere: a channel is built from these
/// objects on the lookup that needs it and dropped again, so what a radio can
/// hold is bounded by the storage it advertises rather than by its SRAM.
static ACTIVE: Mutex<CriticalSectionRawMutex, RefCell<ObjectArena<CONFIGURATION_STORE_BYTES>>> =
    Mutex::new(RefCell::new(ObjectArena::new()));

/// Replaces the shared object snapshot from one packed payload.
///
/// The store keeps its objects packed and ordered, so this is a byte copy of
/// what the device is running rather than an object table rebuilt beside it.
fn store_objects(payload: &[u8]) {
    ACTIVE.lock(|cell| {
        let mut snapshot = cell.borrow_mut();
        *snapshot = ObjectArena::from_payload(payload).unwrap_or_default();
    });
}

/// What the operator is listening to: the VFO, or the programmed channels.
///
/// The programmed variant holds only the index. Counting, bank filtering and
/// scan navigation — the walks which touch every channel — are answered from it
/// without a lock or a decode. Only materialising a record, once per channel
/// actually shown or tuned, reaches the shared snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Listening {
    Vfo(ChannelRecord),
    Memory(Programmed),
}

impl ChannelSource for Listening {
    fn len(&self) -> u16 {
        match self {
            Self::Vfo(_) => 1,
            Self::Memory(programmed) => programmed.len(),
        }
    }

    fn get(&self, index: u16) -> Option<ChannelRecord> {
        match self {
            Self::Vfo(record) => (index == 0).then_some(*record),
            Self::Memory(programmed) => {
                ACTIVE.lock(|cell| programmed.channel_at(&*cell.borrow(), index))
            }
        }
    }

    fn member_at(&self, index: u16, bank: BankId) -> bool {
        match self {
            Self::Vfo(_) => false,
            Self::Memory(programmed) => {
                ACTIVE.lock(|cell| programmed.member_at(&*cell.borrow(), index, bank))
            }
        }
    }
}

fn activate(
    programmed: &Programmed,
    shell: &Shell,
) -> Option<(
    BankedReceiveController<Listening>,
    Option<ChannelReceiveSetup>,
)> {
    let (memory, bank) = match shell.mode() {
        Mode::Vfo => (vfo_source(shell, programmed.config().squelch)?, None),
        Mode::Memory => {
            if programmed.is_empty() {
                return None;
            }
            (Listening::Memory(*programmed), shell.bank_filter())
        }
    };
    let (controller, update) =
        BankedReceiveController::activate(memory, programmed.config(), bank).ok()?;
    Some((
        controller,
        update.activation.map(|activation| activation.setup),
    ))
}

/// Builds the one-channel source holding the VFO frequency.
///
/// The VFO is expressed as an ordinary receive-only channel record so it reuses
/// the whole programmed receive path unchanged. It is `TxClass::Never` like
/// everything else this image constructs.
fn vfo_source(shell: &Shell, squelch: SquelchLevel) -> Option<Listening> {
    let receive = Frequency::from_hz(shell.vfo_hz()).ok()?;
    let step = FrequencyStep::from_hz(shell.vfo_step_hz()).ok()?;
    let record = ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(1),
        name: ChannelName::new("VFO").ok()?,
        receive,
        // Receive-only: the transmit frequency mirrors receive and the class
        // denies transmission outright.
        transmit: receive,
        rx_tone: Tone::None,
        tx_tone: Tone::None,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low,
        step,
        squelch,
        flags: ChannelFlags::default(),
        banks: BankMask::default(),
        tx_class: TxClass::Never,
    })
    .ok()?;
    Some(Listening::Vfo(record))
}

#[embassy_executor::task]
async fn ui_task(
    mut display: DisplayPins,
    mut keypad: KeypadPins,
    radio_pins: Bk4819Pins,
    speaker_pin: PA8,
    mut battery_sense: BatterySense,
) {
    let mut frame = [0_u8; FRAME_BYTES];
    render_info_screen(
        &mut frame,
        IMAGE_IDENTITY,
        0,
        0,
        false,
        MemoryState::Unknown,
        serial_counters(),
        PERIPHERAL_CLOCK_KHZ.load(Ordering::Relaxed),
    );
    if !display.initialise().await || !display.frame(&frame).await {
        fail_closed();
    }

    // The operator interface does not wait for the serial task. Waiting for its
    // first publication tied the whole radio to that task starting: when it
    // died, the display kept showing this boot frame and no key did anything,
    // which is how `AFIK-K1-2.6` to `2.9` reached the operator. An empty
    // configuration is the correct starting state, and the loop below adopts a
    // publication the moment one arrives.
    let publication = PROGRAMMED.try_take();
    let mut generation = publication.as_ref().map_or(0, |value| value.generation);
    let mut retained = publication.as_ref().is_some_and(|value| value.retained);
    let mut memory_state = publication
        .as_ref()
        .map_or(MemoryState::Unknown, |value| value.memory);
    let mut programmed = publication.map_or_else(Programmed::empty, |value| value.programmed);

    let mut shell = Shell::new();
    let (banks, bank_count) = programmed.populated_banks();
    shell.set_banks(banks, bank_count);
    shell.set_squelch(programmed.config().squelch);
    // A programmed radio starts on its channels; only an empty one stays in the
    // VFO, which is the source it can always use.
    if !programmed.is_empty() {
        shell.select_memory();
    }
    let mut activation = activate(&programmed, &shell);

    let mut receiver = Receiver::new(radio_pins, speaker_pin);
    let mut pending = activation.as_mut().and_then(|(_, setup)| setup.take());
    let mut debounce = Debouncer::new();
    let mut next_sample = Instant::now();
    let mut next_meter_redraw = Instant::now();
    let mut battery = Battery::new();
    let mut next_battery_sample = Instant::now();
    let mut redraw = true;

    loop {
        let context = activation
            .as_ref()
            .map_or(Context::default(), |(controller, _)| Context {
                visible_channels: controller.visible_channels(),
                active_index: controller.visible_position(),
            });

        if let Some(calibration) = BATTERY_CALIBRATION.try_take() {
            battery.calibrate(calibration);
        }

        // The converter is on no shared bus, so this needs no quiet link and
        // costs one short blocking conversion. A pack discharges over hours;
        // sampling it every couple of seconds is already generous, and the
        // first full set of samples is what the indicator waits for.
        if Instant::now() >= next_battery_sample {
            let previous = battery.percent();
            battery.sample(battery_sense.read());
            next_battery_sample = Instant::now() + Duration::from_secs(BATTERY_SAMPLE_SECONDS);
            if battery.percent() != previous {
                redraw = true;
            }
        }

        if let Some(publication) = PROGRAMMED.try_take() {
            generation = publication.generation;
            retained = publication.retained;
            memory_state = publication.memory;
            programmed = publication.programmed;
            let (banks, bank_count) = programmed.populated_banks();
            shell.set_banks(banks, bank_count);
            shell.set_squelch(programmed.config().squelch);
            if !programmed.is_empty() {
                shell.select_memory();
            }
            activation = activate(&programmed, &shell);
            pending = activation.as_mut().and_then(|(_, setup)| setup.take());
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
            // Changing source or retuning the VFO rebuilds the receive source
            // from the shell, so both paths share one activation rule. They come
            // before the guard below because selecting the VFO is exactly how an
            // operator leaves an empty memory.
            Intent::SetSource(_) | Intent::TuneVfo => {
                activation = activate(&programmed, &shell);
                if let Some(setup) = activation.as_mut().and_then(|(_, setup)| setup.take()) {
                    pending = Some(setup);
                }
                redraw = true;
            }
            // Every remaining intent acts on a channel. An empty memory has
            // none, so there is nothing to hold open or select, and the
            // interface only redraws.
            // Squelch is a radio-wide setting, so it applies with or without a
            // channel to hear it on and is handled before the guard below.
            Intent::SetSquelch(level) => {
                SQUELCH_CHOICE.signal(level);
                // Retune so the new level reaches the chip now rather than at
                // the next channel change.
                if let Some((controller, _)) = activation.as_ref() {
                    pending = Some(controller.setup());
                }
                redraw = true;
            }
            _ if activation.is_none() => redraw = true,
            Intent::ToggleMonitor => {
                if let Some((controller, _)) = activation.as_mut() {
                    let update = controller.set_monitor(!controller.is_monitoring());
                    if let Some(activation) = update.activation {
                        pending = Some(activation.setup);
                    }
                }
                redraw = true;
            }
            Intent::SelectNext | Intent::SelectPrevious | Intent::SelectIndex(_) => {
                if let Some((controller, _)) = activation.as_mut() {
                    let update = match intent {
                        Intent::SelectNext => controller.select_next(),
                        Intent::SelectPrevious => controller.select_previous(),
                        Intent::SelectIndex(position) => controller.select_visible(position),
                        _ => unreachable!(),
                    };
                    if let Some(activation) = update.ok().and_then(|update| update.activation) {
                        pending = Some(activation.setup);
                    }
                }
                redraw = true;
            }
        }

        // The bit-banged radio bus blocks the executor, so it only runs while
        // the serial link is quiet. Bus work is deferred, never dropped.
        if let Some(setup) = pending.filter(|_| bus_available()) {
            receiver.tune(setup, shell.squelch());
            pending = None;
            redraw = true;
        } else if receiver.audio_routed && Instant::now() >= next_sample && bus_available() {
            let was_open = receiver.squelch_open;
            if let (Some(observation), Some((controller, _))) =
                (receiver.observe(), activation.as_mut())
            {
                controller.observe(observation).ok();
            }
            next_sample = Instant::now() + Duration::from_millis(RF_SAMPLE_MILLISECONDS);
            // The squelch is sampled far faster than a screen needs redrawing.
            // The link opening or shutting is worth showing at once; a moving
            // meter reading is not, and repainting every sample would spend the
            // display bus on nothing an operator can read.
            if was_open != receiver.squelch_open || Instant::now() >= next_meter_redraw {
                next_meter_redraw =
                    Instant::now() + Duration::from_millis(METER_REDRAW_MILLISECONDS);
                redraw = true;
            }
        }

        if redraw {
            render(
                &mut frame,
                &shell,
                activation.as_ref().map(|(controller, _)| controller),
                &receiver,
                battery.percent(),
                &programmed,
                generation,
                retained,
                memory_state,
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
    controller: Option<&BankedReceiveController<Listening>>,
    receiver: &Receiver,
    battery_percent: Option<u8>,
    programmed: &Programmed,
    generation: u32,
    retained: bool,
    memory: MemoryState,
) {
    // Names are fixed-capacity values copied out of the configuration, so they
    // are held here for as long as the view borrows their bytes.
    let filter = shell.bank_filter();
    let filter_name = filter.and_then(|bank| bank_name(programmed, bank));
    // Only an empty memory has no controller: the VFO always has one, so this is
    // the "you have selected channels but programmed none" case.
    let (visible, channel) = match controller {
        Some(controller) => (controller.visible_channels(), Some(controller.channel())),
        None => (0, None),
    };
    let vfo_step_hz = matches!(shell.mode(), Mode::Vfo).then(|| shell.vfo_step_hz());
    // The name is a fixed-capacity value copied out of the record, so it is held
    // here for as long as the view borrows its bytes.
    let channel_name = channel.map(|channel| channel.name());
    match shell.screen() {
        Screen::Operating => render_operating_screen(
            frame,
            &OperatingView {
                position: controller.map_or(0, |controller| {
                    controller.visible_position().saturating_add(1)
                }),
                total: visible,
                name: channel_name
                    .as_ref()
                    .map_or(&[][..], |name| name.as_str().as_bytes()),
                frequency_hz: receiver.frequency_hz,
                rssi_raw: receiver.rssi_raw,
                squelch_open: receiver.squelch_open,
                battery_percent,
                monitoring: controller.is_some_and(BankedReceiveController::is_monitoring),
                bank: filter.map(|bank| BankIndicator {
                    id: bank.get(),
                    name: filter_name
                        .as_ref()
                        .map_or(&[][..], |name| name.as_str().as_bytes()),
                }),
                entry: shell.entry(),
                vfo_step_hz,
            },
        ),
        Screen::SourceList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            // The rows the shell offers are paged directly, so the cursor it
            // reports needs no translation into a second numbering scheme.
            let first = shell.source_cursor() / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let row = first + offset;
                let Some(source) = shell.source_at(row) else {
                    break;
                };
                let active = shell.is_active_source(row);
                rows[offset] = match source {
                    Source::Vfo => SelectorRow::text(b"VFO", active),
                    Source::AllChannels => SelectorRow::text(b"ALL CHANNELS", active),
                    Source::Bank(bank) => {
                        let name = bank_name(programmed, bank);
                        SelectorRow::bank(
                            bank.get(),
                            name.as_ref()
                                .map_or(&[][..], |name| name.as_str().as_bytes()),
                            active,
                        )
                    }
                };
                count += 1;
            }
            render_selector_list(
                frame,
                b"SOURCE",
                &rows[..count],
                shell.source_cursor() - first,
            );
        }
        Screen::Settings => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let mut count = 0;
            for (offset, setting) in SETTINGS.iter().take(LIST_ROWS).enumerate() {
                rows[offset] = match setting {
                    Setting::Squelch => SelectorRow::squelch_setting(shell.squelch().get()),
                };
                count += 1;
            }
            render_selector_list(frame, b"SETTINGS", &rows[..count], shell.settings_cursor());
        }
        Screen::SquelchList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let cursor = usize::from(shell.squelch_cursor());
            let first = cursor / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let Ok(level) = u8::try_from(first + offset) else {
                    break;
                };
                if level >= SQUELCH_LEVELS {
                    break;
                }
                rows[offset] = SelectorRow::squelch_level(level, level == shell.squelch().get());
                count += 1;
            }
            render_selector_list(frame, b"SQUELCH", &rows[..count], cursor - first);
        }
        Screen::StepList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let first = shell.step_cursor() / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let row = first + offset;
                let Some(step) = VFO_STEPS_HZ.get(row) else {
                    break;
                };
                rows[offset] = SelectorRow::step(*step, row == shell.step_index());
                count += 1;
            }
            render_selector_list(frame, b"STEP", &rows[..count], shell.step_cursor() - first);
        }
        Screen::ChannelList => {
            let mut rows = [ListRow::default(); LIST_ROWS];
            let visible_rows = u16::try_from(LIST_ROWS).unwrap_or(u16::MAX);
            // Scroll by whole pages so the cursor is always on screen without
            // the list jittering under a single key press.
            let first = shell.cursor() / visible_rows * visible_rows;
            let mut count = 0;
            for offset in 0..visible_rows {
                let position = first + offset;
                let Some((channel, active)) = controller.and_then(|controller| {
                    controller
                        .visible_channel(position)
                        .map(|channel| (channel, position == controller.visible_position()))
                }) else {
                    break;
                };
                rows[usize::from(offset)] = ListRow::new(
                    position.saturating_add(1),
                    channel.name().as_str().as_bytes(),
                    active,
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
        Screen::Info => render_info_screen(
            frame,
            IMAGE_IDENTITY,
            generation,
            visible,
            retained,
            memory,
            serial_counters(),
            PERIPHERAL_CLOCK_KHZ.load(Ordering::Relaxed),
        ),
    }
}

/// Returns the host-programmed name of one bank, if the host named it.
fn bank_name(programmed: &Programmed, bank: BankId) -> Option<BankName> {
    ACTIVE.lock(|cell| programmed.bank_name(&*cell.borrow(), bank))
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
