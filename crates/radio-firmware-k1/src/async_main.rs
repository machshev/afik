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
use py32_hal::peripherals::SPI1;
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_bk4819::{AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, SquelchThresholds};
use radio_domain::{Bandwidth, Frequency, Modulation, Tone};
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::display::{
    render_key_witness, render_witness, COLUMN_OFFSET, FRAME_BYTES, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, KeypadScan, Sample};
use radio_firmware_k1::protocol::{
    decode_request, encode_hello_response, encode_rf_response, Request, RfObservation,
    REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES, RF_RESPONSE_FRAME_BYTES, RF_STAGE_FAULTED,
    RF_STAGE_READ_BACK, RF_STAGE_RECEIVING, RF_STAGE_STANDBY, RF_STAGE_UNSTARTED,
};
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// Fixed receive frequency for this bounded bring-up witness.
const WITNESS_RECEIVE_HZ: u32 = 145_500_000;
/// Interval between receive metric samples.
const RF_SAMPLE_MILLISECONDS: u64 = 250;

/// Published receive observation, packed so tasks share it without a lock.
///
/// `identity` holds the read-back value and its address, `progress` holds the
/// bring-up stage and sample counter, and `metrics` holds the last sample.
static RF_IDENTITY: AtomicU32 = AtomicU32::new(0);
static RF_PROGRESS: AtomicU32 = AtomicU32::new(0);
static RF_METRICS: AtomicU32 = AtomicU32::new(0);

fn publish_identity(address: u8, value: u16) {
    RF_IDENTITY.store(
        u32::from(address) << 16 | u32::from(value),
        Ordering::Relaxed,
    );
}

fn publish_stage(stage: u8, samples: u16) {
    RF_PROGRESS.store(
        u32::from(stage) << 16 | u32::from(samples),
        Ordering::Relaxed,
    );
}

fn publish_metrics(rssi_dbm_x2: i16, glitch: u8, noise: u8, squelch_open: bool) {
    let rssi = u32::from(u16::from_le_bytes(rssi_dbm_x2.to_le_bytes()));
    let flags = u32::from(u8::from(squelch_open));
    RF_METRICS.store(
        rssi << 16 | u32::from(glitch) << 8 | u32::from(noise) | flags << 24,
        Ordering::Relaxed,
    );
}

fn observation() -> RfObservation {
    let identity = RF_IDENTITY.load(Ordering::Relaxed);
    let progress = RF_PROGRESS.load(Ordering::Relaxed);
    let metrics = RF_METRICS.load(Ordering::Relaxed);
    let stage = u8::try_from(progress >> 16 & 0xFF).unwrap_or(RF_STAGE_UNSTARTED);
    RfObservation {
        identity_register: u16::try_from(identity & 0xFFFF).unwrap_or(0),
        identity_address: u8::try_from(identity >> 16 & 0xFF).unwrap_or(0),
        stage,
        frequency_hz: if stage >= RF_STAGE_RECEIVING && stage != RF_STAGE_FAULTED {
            WITNESS_RECEIVE_HZ
        } else {
            0
        },
        rssi_dbm_x2: i16::from_le_bytes(
            u16::try_from(metrics >> 16 & 0xFFFF)
                .unwrap_or(0)
                .to_le_bytes(),
        ),
        glitch: u8::try_from(metrics >> 8 & 0xFF).unwrap_or(0),
        noise: u8::try_from(metrics & 0xFF).unwrap_or(0),
        squelch_open: metrics >> 24 & 1 == 1,
        samples: u16::try_from(progress & 0xFFFF).unwrap_or(0),
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
        let Ok(ui) = ui_task(display, keypad) else {
            fail_closed();
        };
        let Ok(radio) = rf_task(radio_pins) else {
            fail_closed();
        };
        spawner.spawn(serial);
        spawner.spawn(ui);
        spawner.spawn(radio);
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

#[embassy_executor::task]
async fn serial_task(mut uart: Uart<'static, Async>) {
    let mut window = [0_u8; 16];
    let mut used = 0_usize;
    loop {
        if uart.read(&mut window[used..=used]).await.is_err() {
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
                Some(Request::RfProbe) => {
                    let mut response = [0_u8; RF_RESPONSE_FRAME_BYTES];
                    encode_rf_response(&mut response, observation());
                    let _ = uart.write(&response).await;
                }
                _ => {}
            }
        }
        used = 0;
    }
}

/// Brings up the receiver and publishes bounded read-only observations.
///
/// The task writes only the documented receive path. It never constructs a
/// transmit authorisation, so the transmit mode word is unreachable from here.
#[embassy_executor::task]
async fn rf_task(pins: Bk4819Pins) {
    let mut radio = Bk4819::new(ThreeWireBus::new(pins));
    if radio.recover_to_standby().is_err() {
        publish_stage(RF_STAGE_FAULTED, 0);
        return;
    }
    publish_stage(RF_STAGE_STANDBY, 0);

    let Ok(frequency) = Frequency::from_hz(WITNESS_RECEIVE_HZ) else {
        publish_stage(RF_STAGE_FAULTED, 0);
        return;
    };
    let setup = ReceiveSetup {
        frequency,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        tone: Tone::None,
        // The K1 keeps its squelch calibration in external flash which AFIK
        // does not yet read, so this witness runs with the pinned source's
        // squelch-off set and reports raw metrics instead of gating audio.
        squelch: SquelchThresholds::squelch_off(),
        af: AfOutput::Mute,
    };
    if radio.configure_receive(&setup).is_err() {
        publish_stage(RF_STAGE_FAULTED, 0);
        return;
    }
    publish_stage(RF_STAGE_RECEIVING, 0);

    match radio.read_back(ReadbackRegister::FilterBandwidth) {
        Ok(value) => {
            publish_identity(ReadbackRegister::FilterBandwidth.address(), value);
            publish_stage(RF_STAGE_READ_BACK.max(RF_STAGE_RECEIVING), 0);
        }
        Err(_) => {
            publish_stage(RF_STAGE_FAULTED, 0);
            return;
        }
    }

    let mut samples = 0_u16;
    loop {
        Timer::after_millis(RF_SAMPLE_MILLISECONDS).await;
        let Ok(metrics) = radio.receive_metrics(Tone::None) else {
            publish_stage(RF_STAGE_FAULTED, samples);
            return;
        };
        samples = samples.saturating_add(1);
        publish_metrics(
            metrics.rssi_dbm_x2,
            metrics.glitch,
            metrics.noise,
            metrics.squelch_open,
        );
        publish_stage(RF_STAGE_RECEIVING, samples);
    }
}

#[embassy_executor::task]
async fn ui_task(mut display: DisplayPins, mut keypad: KeypadPins) {
    let mut frame = [0_u8; FRAME_BYTES];
    render_witness(&mut frame);
    if !display.initialise().await || !display.frame(&frame).await {
        fail_closed();
    }
    let mut debounce = Debouncer::new();
    loop {
        let sample = match decode(keypad.scan().await) {
            Ok(Some(key)) => Sample::Key(key),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        };
        let now = u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX);
        if let Edge::Pressed(key) = debounce.update(now, sample) {
            render_key_witness(&mut frame, key);
            if !display.frame(&frame).await {
                fail_closed();
            }
        }
        Timer::after(Duration::from_millis(5)).await;
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
