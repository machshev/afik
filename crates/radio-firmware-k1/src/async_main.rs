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
use cortex_m::peripheral::SCB;
use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_futures::select::{select, select3, Either, Either3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use py32_hal::gpio::{Flex, Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::pac::RCC;
use py32_hal::peripherals::{PA0, PA1, PA2, PA3, PA8, SPI1, SPI2};
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_bk4819::{
    AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, SquelchThresholds, BK4829_PROFILE,
};
use radio_channel_control::{
    BankedReceiveController, ChannelReceiveSetup, ChannelSource, ReceiveObservation, ReceiveState,
    ReceiveUpdate, TimerDirective, TimerToken,
};
use radio_channel_plan::{
    BankMask, BankName, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
};
use radio_device::{ControlAnswer, DeviceEvent, DeviceService, Push};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel,
    Tone, TxClass,
};
use radio_firmware_k1::battery::{Battery, Calibration};
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::configuration::{
    device_service, store_setting, Programmed, SettingChange, CONFIGURATION_STORE_BYTES,
    RETAINED_IMAGE_BYTES,
};
use radio_firmware_k1::display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    BankIndicator, ListRow, MemoryState, OperatingView, PanicReport, ResetCause, SelectorRow,
    SerialCounters, COLUMN_OFFSET, FRAME_BYTES, LIST_ROWS, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::host_control;
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, KeypadScan, Sample};
use radio_firmware_k1::operator_state::OperatorState;
use radio_firmware_k1::py32f071_battery::BatterySense;
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_eeprom::{RetainError, RetainedConfiguration};
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;
use radio_firmware_k1::shell::{
    Context, Intent, Mode, Screen, Setting, Shell, Source, HOLD_MILLISECONDS, SETTINGS,
    SQUELCH_LEVELS, VFO_STEPS_HZ,
};
use radio_protocol::{ControlRequest, DeviceErrorCode, ReceiveMetricsReport, MAX_ENCODED_FRAME};
use radio_storage::ObjectArena;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// Identity this image reports on the information screen.
///
/// A diagnostic build says so on the screen. An image which drops every byte
/// it receives must not be mistaken for one which answers.
#[cfg(feature = "serial-drop-bytes")]
const IMAGE_IDENTITY: &[u8] = b"AFIK-K1-6.2D";
/// Identity this image reports on the information screen.
#[cfg(not(feature = "serial-drop-bytes"))]
const IMAGE_IDENTITY: &[u8] = b"AFIK-K1-6.2";

/// Interval between receive samples while audio is routed.
///
/// This is the squelch's reaction time as well as the meter's refresh rate: the
/// speaker opens and shuts on these samples, so a half-second interval would
/// clip the start of every transmission and leave noise after the end of it.
/// One sample is a handful of bit-banged register reads, and the serial link
/// still holds the bus off while a host is mid-exchange.
const RF_SAMPLE_MILLISECONDS: u64 = 60;

/// Interval between receive samples while a scan is walking channels.
///
/// This is the scan's resolution. A dwell is judged in whole samples, so a
/// dwell shorter than two of these cannot be judged at all, and the operating
/// interval above would quantise every short dwell to itself.
///
/// It is deliberately far below any plausible receiver settling time. How long
/// this board needs after a retune before a squelch reading means anything is
/// unmeasured — `RISK-008` — and the dwell is the operator's control over that.
/// This must not quietly become a second one.
const SCAN_SAMPLE_MILLISECONDS: u64 = 5;

/// Interval between user interface passes.
const LOOP_MILLISECONDS: u64 = 5;

/// Interval between user interface passes while a scan is running.
///
/// Every deadline the loop has is quantised to its own period, so a scan
/// stepping every twenty milliseconds cannot be paced by a loop which wakes
/// every five. A scan is short-lived and the operator is holding the radio
/// while it runs, so the cost of a faster pass is one they are paying for.
const SCAN_LOOP_MILLISECONDS: u64 = 1;

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

/// A radio-wide setting the operator changed on the handset.
///
/// The serial task owns the store and the external memory, so the interface
/// asks rather than writes. A choice the operator makes while a host is
/// mid-exchange waits its turn instead of racing it.
static SETTING_CHOICE: Signal<CriticalSectionRawMutex, SettingChange> = Signal::new();

/// One host runtime-control request, for the task which owns the controller.
///
/// The serial task owns USART1 and the interface task owns the receive
/// controller, so a host request crosses here and its answer comes back on
/// [`CONTROL_ANSWER`]. This is the same shape as the place going the other way:
/// the task which can do the work does it, and neither reaches into the other.
static CONTROL_REQUEST: Signal<CriticalSectionRawMutex, ControlRequest> = Signal::new();

/// The answer to the request on [`CONTROL_REQUEST`].
static CONTROL_ANSWER: Signal<CriticalSectionRawMutex, ControlAnswer> = Signal::new();

/// Milliseconds a host waits for the interface task to answer one request.
///
/// The interface task answers from the controller alone and needs no bus, so
/// this is generous rather than tight. It exists so a host cannot be left
/// waiting forever by an interface task which has stopped: the link keeps
/// working and says the radio did not answer, rather than going silent.
const CONTROL_ANSWER_MILLISECONDS: u64 = 200;

/// Where the operator has left the radio, for the task which owns the memory.
///
/// The interface knows the place and the serial task owns the bus, so the
/// interface says where it is and the serial task writes it down. A place
/// arriving while a host is mid-exchange waits its turn rather than racing it.
static OPERATOR_PLACE: Signal<CriticalSectionRawMutex, OperatorState> = Signal::new();

/// Milliseconds a new place must hold still before it is worth writing down.
///
/// An operator walking to a channel passes through every channel in between,
/// and none of those is where they left the radio. Waiting for the selection to
/// settle turns a walk across a bank into one record instead of thirty.
const PLACE_SETTLE_MILLISECONDS: u64 = 3_000;

/// One activated configuration handed to the user interface task.
#[derive(Clone, Copy)]
struct Publication {
    programmed: Programmed,
    generation: u32,
    retained: bool,
    memory: MemoryState,
    /// Where the operator left this radio, on the one publication which knows.
    ///
    /// The place is read once, beside the configuration it refers to, and
    /// travels with it. Carrying it here rather than signalling it separately
    /// is what stops the interface from restoring a place into a configuration
    /// it has not adopted yet.
    place: Option<OperatorState>,
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

/// Reset flags from `RCC_CSR`, bits 24 to 31, read once at boot.
static RESET_CAUSE: AtomicU32 = AtomicU32::new(0);

/// Boots since this radio last lost its memory, as one digit.
static BOOTS: AtomicU32 = AtomicU32::new(0);

/// Returns the boot count digit.
fn boots() -> u8 {
    u8::try_from(BOOTS.load(Ordering::Relaxed) % 10).unwrap_or(0)
}

/// Returns the reset cause this boot began with.
fn reset_cause() -> ResetCause {
    ResetCause(u8::try_from(RESET_CAUSE.load(Ordering::Relaxed) & 0xff).unwrap_or(0))
}

/// Receiver errors: bytes which arrived and could not be framed.
///
/// A byte the UART cannot frame is never delivered, so it cannot be counted as
/// received. Without this, a link running at the wrong baud rate looks exactly
/// like a link with nothing on it: `RISK-036` records a whole evening spent
/// unable to tell those apart.
static SERIAL_ERRORS: AtomicU32 = AtomicU32::new(0);

/// Returns the current serial counters.
fn serial_counters() -> SerialCounters {
    SerialCounters {
        received: u16::try_from(SERIAL_RECEIVED.load(Ordering::Relaxed) % 1_000).unwrap_or(0),
        answered: u16::try_from(SERIAL_ANSWERED.load(Ordering::Relaxed) % 1_000).unwrap_or(0),
        discarded: u16::try_from(SERIAL_DISCARDED.load(Ordering::Relaxed) % 1_000).unwrap_or(0),
        errors: u16::try_from(SERIAL_ERRORS.load(Ordering::Relaxed) % 1_000).unwrap_or(0),
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

    // Why the radio last reset, taken before anything else can provoke one and
    // cleared immediately so the next boot reports its own cause rather than
    // inheriting this one. `RISK-036`: the unit restarts when a host speaks to
    // it, and this is the part saying whether that was a brown-out, a watchdog,
    // or software asking.
    let csr = RCC.csr().read();
    RESET_CAUSE.store((csr.0 >> 24) & 0xff, Ordering::Relaxed);
    RCC.csr().modify(|register| register.set_rmvf(true));

    // Counted here, beside the reset cause, and before any peripheral is
    // touched. A counter which reads one after a reset says this memory did not
    // survive it, and the panic report cannot be carried this way.
    BOOTS.store(u32::from(count_boot()), Ordering::Relaxed);
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
    /// Completed metric samples since boot, saturating.
    ///
    /// A host cannot otherwise tell a fresh reading from the one it already
    /// had. The receiver needs an unmeasured settling time after a retune —
    /// `RISK-008` — so a reading taken too soon measures settling rather than
    /// signal, and a host which wants a settled one waits for this to advance.
    samples: u16,
    /// The most recent complete sample, as a host would read it.
    last_metrics: Option<ReceiveMetricsReport>,
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
            samples: 0,
            last_metrics: None,
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
                // Saturating, so a long-running radio stops counting rather
                // than wrapping past a value a host is waiting to see pass.
                self.samples = self.samples.saturating_add(1);
                self.last_metrics = Some(ReceiveMetricsReport {
                    // The frequency this sample was actually taken at, which is
                    // the one the receiver is tuned to and not necessarily the
                    // one the controller has most recently been asked for.
                    frequency_hz: self.frequency_hz,
                    samples: self.samples,
                    rssi_dbm_x2: metrics.rssi_dbm_x2,
                    glitch: metrics.glitch,
                    noise: metrics.noise,
                    squelch_open: metrics.squelch_open,
                });
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
    publish(&service, false, MemoryState::Unknown, None);

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
    // The place is read beside the configuration it refers to and published
    // with it, so the interface never restores a channel into a channel list it
    // has not adopted. A radio which has never been used simply has none.
    let place = retained
        .as_mut()
        .and_then(RetainedConfiguration::read_operator_state);
    publish(&service, restored, memory_state, place);

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
        let count = match select3(
            uart.read_until_idle(&mut received),
            SETTING_CHOICE.wait(),
            OPERATOR_PLACE.wait(),
        )
        .await
        {
            Either3::First(Ok(count)) => count,
            Either3::First(Err(_)) => {
                // Counted before yielding. This arm firing while nothing is
                // received is what a wrong baud rate looks like, and it was
                // previously discarded without trace.
                //
                // Only this task writes this, so a load and store is sufficient.
                SERIAL_ERRORS.store(
                    SERIAL_ERRORS.load(Ordering::Relaxed).wrapping_add(1),
                    Ordering::Relaxed,
                );
                // Yield before retrying so a persistent receiver error can never
                // starve the interface task.
                Timer::after_millis(1).await;
                continue;
            }
            Either3::Second(change) => {
                // A handset setting which did not survive a battery change would
                // not be worth the menu, so it is stored exactly like a host
                // write and republished from the store rather than assumed.
                if store_setting(&mut service, change).is_ok() {
                    let mut retained_now = false;
                    if let Ok(length) = service.encode_active_image(&mut image) {
                        if let Some(configuration) = retained.as_mut() {
                            retained_now = configuration.write(&image, length).await.is_ok();
                        }
                    }
                    publish(&service, retained_now, memory_state, None);
                }
                continue;
            }
            Either3::Third(place) => {
                // One page program into an erased slot, which is why this can
                // be written every time the operator settles somewhere new
                // rather than only when they turn the radio off — and a radio
                // is rarely turned off deliberately enough to be asked.
                if let Some(configuration) = retained.as_mut() {
                    let _ = configuration.write_operator_state(place).await;
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

            // `RISK-036`: with this feature the byte is counted and dropped,
            // and nothing below this line runs. See the feature's own note.
            if cfg!(feature = "serial-drop-bytes") {
                continue;
            }

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
            let length = match service.push_control(byte, &mut response, &mut observe) {
                Push::Idle => continue,
                Push::Response(length) => length,
                // The receiver belongs to the interface task, so the answer has
                // to come from there. Nothing is held while waiting: this task
                // owns only the link, and the operator's keypad is unaffected.
                Push::Control(control) => {
                    CONTROL_REQUEST.signal(control.request());
                    let answered = select(
                        CONTROL_ANSWER.wait(),
                        Timer::after_millis(CONTROL_ANSWER_MILLISECONDS),
                    )
                    .await;
                    let answer = match answered {
                        Either::First(answer) => answer,
                        // An interface task which did not answer leaves the
                        // link working and the host told, rather than a serial
                        // port which has silently stopped replying.
                        Either::Second(()) => ControlAnswer::Refused(DeviceErrorCode::Internal),
                    };
                    let mut observe = |event| {
                        if matches!(event, DeviceEvent::PacketDiscarded(_)) {
                            SERIAL_DISCARDED.store(
                                SERIAL_DISCARDED.load(Ordering::Relaxed).wrapping_add(1),
                                Ordering::Relaxed,
                            );
                        }
                    };
                    let Some(length) =
                        service.answer_control(control, answer, &mut response, &mut observe)
                    else {
                        continue;
                    };
                    length
                }
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
                publish(&service, retained_now, memory_state, None);
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
    place: Option<OperatorState>,
) {
    store_objects(service.active_payload());
    let programmed = Programmed::index(service.active_objects()).unwrap_or_default();
    PROGRAMMED.signal(Publication {
        programmed,
        generation: service.generation(),
        retained: retained && !programmed.is_empty(),
        memory,
        place,
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

/// The memory channel the operator last selected.
///
/// The index is how selection addresses it and the identifier is how a restore
/// checks the index still names the same channel, so both are kept.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Selected {
    index: u16,
    channel_id: u16,
}

impl Selected {
    /// Returns the memory selection one retained place names, if it named one.
    fn from_place(place: OperatorState) -> Option<Self> {
        place.memory_mode.then_some(Self {
            index: place.index,
            channel_id: place.channel_id,
        })
    }
}

/// Applies one controller update: the tuning it asks for, and the timer it arms.
///
/// The controller owns every deadline a scan has and expresses them as
/// directives rather than as waits, so this is the whole of the scan's clock:
/// arm what it asked for, cancel what it cancelled, and leave the rest alone.
fn apply(
    update: ReceiveUpdate,
    pending: &mut Option<ChannelReceiveSetup>,
    timer: &mut Option<(TimerToken, Instant)>,
) {
    if let Some(activation) = update.activation {
        *pending = Some(activation.setup);
    }
    match update.timer {
        TimerDirective::Unchanged => {}
        TimerDirective::Cancel => *timer = None,
        TimerDirective::Arm { token, after_ms } => {
            *timer = Some((
                token,
                Instant::now() + Duration::from_millis(u64::from(after_ms)),
            ));
        }
    }
}

/// Points the shell at one configuration, and at the place it was left in.
///
/// A radio which has been used before comes back to the source, bank, and
/// frequency the operator chose. One which has not starts on its channels,
/// because a programmed radio which opened in the VFO would look unprogrammed.
fn adopt_settings(shell: &mut Shell, programmed: &Programmed) {
    let (banks, bank_count) = programmed.populated_banks();
    shell.set_banks(banks, bank_count);
    shell.set_squelch(programmed.config().squelch);
}

/// Points the shell at the source the operator was last listening to.
///
/// This runs once, when the radio adopts the configuration it booted with. A
/// later configuration replaces what the radio holds, not what the operator is
/// doing with it: someone changing a squelch level has not asked to be returned
/// to the top of their channel list, or dragged out of the VFO.
fn adopt_place(shell: &mut Shell, programmed: &Programmed, place: Option<OperatorState>) {
    if let Some(place) = place {
        shell.restore_vfo(place.vfo_hz, usize::from(place.step_index));
    }
    if programmed.is_empty() {
        // The VFO is the only source this radio has, whatever it was left on.
        return;
    }
    match place {
        Some(place) => shell.restore_source(place.memory_mode, place.bank),
        None => shell.select_memory(),
    }
}

/// Builds the receive controller for whatever the shell says is in force.
///
/// `keep` is the memory channel the operator is on, which survives the rebuild
/// if the configuration still has it. Every caller has one except the first: a
/// radio which has just been switched on takes it from the retained place, and
/// everything afterwards takes it from where the operator actually is.
fn activate(
    programmed: &Programmed,
    shell: &Shell,
    keep: Option<Selected>,
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
    let keep = keep.map(|kept| (kept.index, ChannelId::new(kept.channel_id)));
    let (controller, update) =
        BankedReceiveController::activate_at(memory, programmed.config(), bank, keep).ok()?;
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
        reset_cause(),
        boots(),
        // A radio which panicked says so on the first frame it draws, before
        // the operator has to know to go looking for it.
        recorded_panic(),
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
    let place = publication.as_ref().and_then(|value| value.place);
    let mut programmed = publication.map_or_else(Programmed::empty, |value| value.programmed);

    let mut shell = Shell::new();
    adopt_settings(&mut shell, &programmed);
    adopt_place(&mut shell, &programmed, place);
    // The retained place is where the operator left this radio, and it is the
    // only thing which knows on the pass that adopts it.
    let mut selected = place.and_then(Selected::from_place);
    let mut activation = activate(&programmed, &shell, selected);
    let mut pending = activation.as_mut().and_then(|(_, setup)| setup.take());

    let mut receiver = Receiver::new(radio_pins, speaker_pin);
    let mut debounce = Debouncer::new();
    // When the key currently held down went down, so a hold can be told from a
    // press. Cleared once the hold has acted, so one press acts once.
    let mut held_since: Option<u32> = None;
    // The scan timer the controller asked for, and when it expires.
    let mut scan_timer: Option<(TimerToken, Instant)> = None;
    let mut next_sample = Instant::now();
    // Whether the receiver has produced a reading on the channel it is now on.
    let mut settled = false;
    let mut next_meter_redraw = Instant::now();
    let mut battery = Battery::new();
    let mut next_battery_sample = Instant::now();
    let mut redraw = true;
    // The place already written down, and the one waiting to settle into it.
    let mut saved_place = place;
    let mut settling: Option<(OperatorState, Instant)> = None;

    loop {
        let context = activation
            .as_ref()
            .map_or(Context::default(), |(controller, _)| Context {
                visible_channels: controller.visible_channels(),
                active_index: controller.visible_position(),
                scanning: matches!(controller.state(), ReceiveState::Scanning(_)),
            });

        // The scan's own clock. The controller decides what a dwell or a hold
        // expiring means; this only tells it that one has.
        if let Some((token, deadline)) = scan_timer {
            if Instant::now() >= deadline {
                scan_timer = None;
                if let Some((controller, _)) = activation.as_mut() {
                    if let Ok(update) = controller.timer_elapsed(token) {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                redraw = true;
            }
        }

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
            let was_empty = programmed.is_empty();
            programmed = publication.programmed;
            adopt_settings(&mut shell, &programmed);
            // A radio which had no channels and now has some is being given
            // them, so it leaves the VFO for them. One which already had them
            // is only being told about a changed setting, and the operator
            // stays exactly where they were.
            if was_empty && !programmed.is_empty() {
                shell.select_memory();
                selected = None;
            }
            activation = activate(&programmed, &shell, selected);
            pending = activation.as_mut().and_then(|(_, setup)| setup.take());
            // A configuration arriving replaces what a scan was walking.
            scan_timer = None;
            redraw = true;
        }

        // A host request reaches the same controller a key press does, and its
        // update is applied by the same call. The operator is not locked out
        // while this happens and nothing here is suspended: both peers drive
        // one controller, and whichever arrives first is served first.
        if let Some(request) = CONTROL_REQUEST.try_take() {
            let answer = match activation.as_mut() {
                Some((controller, _)) => {
                    let performed =
                        host_control::perform(controller, request, receiver.last_metrics);
                    if let Some(update) = performed.update {
                        apply(update, &mut pending, &mut scan_timer);
                        redraw = true;
                    }
                    performed.answer
                }
                // No controller yet means no channels and no VFO: the radio has
                // not finished starting. That is a state the request cannot be
                // performed from, not a fault.
                None => ControlAnswer::Refused(DeviceErrorCode::InvalidState),
            };
            CONTROL_ANSWER.signal(answer);
        }

        let sample = match decode(keypad.scan().await) {
            Ok(Some(key)) => Sample::Key(key),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        };
        let now = now_ms();

        let intent = match debounce.update(now, sample) {
            Edge::Pressed(key) => {
                held_since = Some(now);
                shell.press(key, now, context)
            }
            Edge::Released(key) => {
                held_since = None;
                shell.release(key)
            }
            // A key still down past the hold interval is a second, different
            // input from the press which started it, and it acts once.
            Edge::None => match (debounce.held_key(), held_since) {
                (Some(key), Some(since)) if now.wrapping_sub(since) >= HOLD_MILLISECONDS => {
                    held_since = None;
                    shell.hold(key, context)
                }
                _ => shell.tick(now, context),
            },
        };
        match intent {
            Intent::Idle => {}
            Intent::Redraw => redraw = true,
            // Changing source or retuning the VFO rebuilds the receive source
            // from the shell, so both paths share one activation rule. They come
            // before the guard below because selecting the VFO is exactly how an
            // operator leaves an empty memory.
            Intent::SetSource(_) | Intent::TuneVfo => {
                // Choosing a source is the one rebuild the operator did ask
                // for, so a bank change lands on that bank's first channel
                // rather than dragging the old selection into it.
                selected = None;
                activation = activate(&programmed, &shell, None);
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
                SETTING_CHOICE.signal(SettingChange::Squelch(level));
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
                    apply(update, &mut pending, &mut scan_timer);
                }
                redraw = true;
            }
            Intent::StartScan | Intent::StopScan => {
                if let Some((controller, _)) = activation.as_mut() {
                    let update = if matches!(intent, Intent::StartScan) {
                        controller.start_scanning()
                    } else {
                        controller.stop_scanning()
                    };
                    // A scan of one channel, or of none, is refused by the
                    // controller and leaves the radio exactly where it was.
                    if let Ok(update) = update {
                        apply(update, &mut pending, &mut scan_timer);
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
                    if let Ok(update) = update {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                redraw = true;
            }
        }

        // What the operator is listening to, in the form the radio would have to
        // restore it from. A memory selection is remembered even while the VFO
        // is in use, so leaving the VFO returns to the channel rather than to
        // the top of the list.
        if let Some((controller, _)) = activation.as_ref() {
            if matches!(shell.mode(), Mode::Memory) {
                selected = Some(Selected {
                    index: controller.index(),
                    channel_id: controller.channel().id().get(),
                });
            }
        }
        let kept = selected.unwrap_or_default();
        let place = OperatorState {
            memory_mode: matches!(shell.mode(), Mode::Memory),
            bank: shell.bank_filter(),
            index: kept.index,
            channel_id: kept.channel_id,
            vfo_hz: shell.vfo_hz(),
            step_index: u8::try_from(shell.step_index()).unwrap_or(0),
        };
        if context.scanning {
            // A scan moves the selection several times a second and none of
            // those is a place the operator chose. Where it stops is.
            settling = None;
        } else if saved_place == Some(place) {
            settling = None;
        } else {
            match settling {
                Some((candidate, deadline)) if candidate == place => {
                    if Instant::now() >= deadline {
                        OPERATOR_PLACE.signal(place);
                        saved_place = Some(place);
                        settling = None;
                    }
                }
                _ => {
                    settling = Some((
                        place,
                        Instant::now() + Duration::from_millis(PLACE_SETTLE_MILLISECONDS),
                    ));
                }
            }
        }

        // The bit-banged radio bus blocks the executor, so it only runs while
        // the serial link is quiet. Bus work is deferred, never dropped.
        if let Some(setup) = pending.filter(|_| bus_available()) {
            receiver.tune(setup, shell.squelch());
            pending = None;
            // Sample this channel from now rather than on whatever grid the
            // last one left behind. A free-running interval meant the first
            // reading of a scanned channel landed anywhere inside the dwell,
            // so how many readings a channel got was a matter of phase.
            next_sample = Instant::now();
            settled = false;
            redraw = true;
        } else if receiver.audio_routed && Instant::now() >= next_sample && bus_available() {
            let was_open = receiver.squelch_open;
            let scanning = activation.as_ref().is_some_and(|(controller, _)| {
                matches!(controller.state(), ReceiveState::Scanning(_))
            });
            if let (Some(observation), Some((controller, _))) =
                (receiver.observe(), activation.as_mut())
            {
                // While scanning this is what decides whether the channel is
                // busy, so the hold it asks for has to reach the scan's clock.
                //
                // The first reading after a retune is taken while the
                // synthesiser is still settling on the new frequency, and how
                // long that takes on this board is unmeasured — `RISK-008`. It
                // still updates the meter, which is honest about what was read,
                // but it does not get to tell a scan a channel is busy: a false
                // stop costs the whole hold, which is the one failure an
                // operator would notice and could not explain.
                if settled || !scanning {
                    if let Ok(update) = controller.observe(observation) {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                settled = true;
            }
            next_sample = Instant::now()
                + Duration::from_millis(if scanning {
                    SCAN_SAMPLE_MILLISECONDS
                } else {
                    RF_SAMPLE_MILLISECONDS
                });
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

        Timer::after(Duration::from_millis(if context.scanning {
            SCAN_LOOP_MILLISECONDS
        } else {
            LOOP_MILLISECONDS
        }))
        .await;
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
                scanning: controller.is_some_and(|controller| {
                    matches!(controller.state(), ReceiveState::Scanning(_))
                }),
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
            reset_cause(),
            boots(),
            recorded_panic(),
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

/// Marks the panic report as written by this image rather than left in RAM.
///
/// The report lives in memory startup does not clear, so a cold boot reads
/// whatever the SRAM happened to hold. Only this word says the rest is real.
const PANIC_REPORT_MAGIC: u32 = 0x4B31_DEAD;

/// Bytes of the panicking file's name kept, after the last path separator.
const PANIC_FILE_BYTES: usize = 8;

/// Marks the boot counter as this image's rather than leftover SRAM.
const BOOT_COUNT_MAGIC: u32 = 0x4B31_B007;

/// Set when the boot counter below is this image's.
#[allow(unsafe_code)]
#[unsafe(link_section = ".uninit.afik_panic_report")]
static BOOT_MAGIC: AtomicU32 = AtomicU32::new(0);

/// Boots since the last time this memory was lost.
///
/// This exists to test the panic report rather than to be useful in itself.
/// The report is written by the panic handler and read by the next boot, and
/// `RISK-036` has it arriving empty after a reset the flags say was software —
/// which the handler is the only caller of. Either this memory does not survive
/// a reset at all, in which case this counter reads one every time, or it does
/// and something is destroying the report specifically.
#[allow(unsafe_code)]
#[unsafe(link_section = ".uninit.afik_panic_report")]
static BOOT_COUNT: AtomicU32 = AtomicU32::new(0);

/// Counts this boot and returns the total, saturating at a single digit.
///
/// Called once, before anything else can reset the radio.
fn count_boot() -> u8 {
    let counted = if BOOT_MAGIC.load(Ordering::Relaxed) == BOOT_COUNT_MAGIC {
        BOOT_COUNT.load(Ordering::Relaxed).saturating_add(1)
    } else {
        1
    };
    BOOT_COUNT.store(counted, Ordering::Relaxed);
    BOOT_MAGIC.store(BOOT_COUNT_MAGIC, Ordering::Relaxed);
    u8::try_from(counted % 10).unwrap_or(0)
}

/// Set when the fields below describe a real panic.
///
/// The three statics below are placed by hand in the section `cortex-m-rt`
/// leaves untouched at startup, which is the whole mechanism: anything in
/// `.bss` is zeroed before the next boot could read it. Placement is the only
/// thing being overridden, the type stays an ordinary atomic, and a cold boot
/// is told apart from a real report by the magic word rather than by trusting
/// the contents.
#[allow(unsafe_code)]
#[unsafe(link_section = ".uninit.afik_panic_report")]
static PANIC_MAGIC: AtomicU32 = AtomicU32::new(0);

/// Source line the panic came from.
#[allow(unsafe_code)]
#[unsafe(link_section = ".uninit.afik_panic_report")]
static PANIC_LINE: AtomicU32 = AtomicU32::new(0);

/// First [`PANIC_FILE_BYTES`] of the panicking file's name, packed little-endian.
#[allow(unsafe_code)]
#[unsafe(link_section = ".uninit.afik_panic_report")]
static PANIC_FILE: [AtomicU32; PANIC_FILE_BYTES / 4] = [AtomicU32::new(0), AtomicU32::new(0)];

/// Returns the panic which reset this radio, if the last stop was one.
///
/// The report is deliberately not cleared. It describes the run before this
/// one and stays readable until the next panic replaces it or the battery
/// comes out, because an operator who was not watching the screen at the
/// moment of the reset is exactly who needs to read it.
fn recorded_panic() -> Option<PanicReport> {
    if PANIC_MAGIC.load(Ordering::Relaxed) != PANIC_REPORT_MAGIC {
        return None;
    }
    let mut file = [0_u8; PANIC_FILE_BYTES];
    for (index, word) in PANIC_FILE.iter().enumerate() {
        let bytes = word.load(Ordering::Relaxed).to_le_bytes();
        let start = index * 4;
        file[start..start + 4].copy_from_slice(&bytes);
    }
    Some(PanicReport {
        file,
        line: PANIC_LINE.load(Ordering::Relaxed),
    })
}

/// Records where a panic happened and restarts, rather than stopping silently.
///
/// A spin loop here is what an unexplainable radio looks like: the display
/// holds its last frame, the keypad is dead, and the link is silent, with the
/// one thing that knows what went wrong holding it. `PanicInfo` carries the
/// file and line; this keeps them somewhere startup does not clear and resets,
/// so the next boot can say so on the information screen.
///
/// This is a deliberate change from stopping forever. Nothing here can
/// transmit — the image constructs no transmit path at all — so a restart
/// risks no emission, and a radio which reboots and explains itself is more
/// use than one which stops and does not.
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    let mut file = [0_u8; PANIC_FILE_BYTES];
    let mut line = 0;
    if let Some(location) = info.location() {
        line = location.line();
        let bytes = location.file().as_bytes();
        // The basename identifies the file; the path in front of it is the same
        // for every source in this crate.
        let start = bytes
            .iter()
            .rposition(|byte| *byte == b'/' || *byte == b'\\')
            .map_or(0, |index| index.saturating_add(1));
        // Indexing is avoided throughout: a panic raised inside the panic
        // handler cannot be reported by it.
        for (slot, byte) in file.iter_mut().zip(bytes.get(start..).unwrap_or(&[])) {
            *slot = *byte;
        }
    }
    for (index, word) in PANIC_FILE.iter().enumerate() {
        let start = index * 4;
        let mut packed = [0_u8; 4];
        for (slot, byte) in packed.iter_mut().zip(file.get(start..).unwrap_or(&[])) {
            *slot = *byte;
        }
        word.store(u32::from_le_bytes(packed), Ordering::Relaxed);
    }
    PANIC_LINE.store(line, Ordering::Relaxed);
    // Written last: the fields are only claimed once they are all in place.
    PANIC_MAGIC.store(PANIC_REPORT_MAGIC, Ordering::Relaxed);
    SCB::sys_reset()
}
