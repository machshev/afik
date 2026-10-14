//! K1 target: PY32F071 drivers for the shared AFIK application.
//!
//! Everything the operator and the host see is `radio-app`. This image brings
//! up the clock, the vector table, and the peripherals, wraps them in the
//! application's driver traits, and starts its two tasks: the serial task owns
//! USART1 and the external memory, and the interface task owns the display,
//! the keypad, and the bit-banged radio bus.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::future::Future;
use core::panic::PanicInfo;

use cortex_m::peripheral::SCB;
use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_time::Timer;
use py32_hal::gpio::{Flex, Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::pac::RCC;
use py32_hal::peripherals::{PA0, PA1, PA2, PA3, PA8, SPI1, SPI2};
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_app::drivers::{Display, Keypad, LinkError, SerialLink, Speaker, Target};
use radio_app::keys::Sample;
use radio_app::operator_state::OperatorState;
use radio_app::TaskCell;
use radio_bk4819::{Bk4819, BK4829_PROFILE};
use radio_firmware_k1::battery::Calibration;
use radio_firmware_k1::bk4819_bus::ThreeWireBus;
use radio_firmware_k1::display::{
    MemoryState, ResetCause, COLUMN_OFFSET, FRAME_BYTES, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, KeypadScan};
use radio_firmware_k1::memory_view::{fill_place, locate, ViewSpan};
use radio_firmware_k1::py32f071_battery::BatterySense;
use radio_firmware_k1::py32f071_bk4819::Bk4819Pins;
use radio_firmware_k1::py32f071_eeprom::{RetainError, RetainedConfiguration};
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;
use radio_platform::memory_link::WRITE_BYTES;
use radio_platform::transmit::TxCalibration;
use radio_platform::transmit::TxTarget;

const _: [(); 8] = [(); PAGES];
const K1_VECTOR_TABLE_ORIGIN: u32 = 0x0800_2800;

/// The external memory, shared by the serial task and the interface task's
/// channel lookups.
static MEMORY: TaskCell<Option<Memory>> = TaskCell::new(None);

/// Identity this image reports on the information screen.
///
/// A diagnostic build says so on the screen. An image which drops every byte
/// it receives must not be mistaken for one which answers.
#[cfg(feature = "serial-drop-bytes")]
const IMAGE_IDENTITY: &[u8] = concat!("AFIK-K1-", env!("CARGO_PKG_VERSION"), "D").as_bytes();
/// Identity this image reports on the information screen.
#[cfg(not(feature = "serial-drop-bytes"))]
const IMAGE_IDENTITY: &[u8] = radio_firmware_k1::protocol::APPLICATION_VERSION;

/// The K1 as the application sees it. Armel K1 UHF `dividers_band5` Low1–Low4
/// are 25, 19, 13 and 9.
const K1: Target = Target {
    identity: IMAGE_IDENTITY,
    tx: TxTarget {
        target_id: 0x4B31_4B31,
        dividers: [25, 19, 13, 9],
    },
};

#[entry]
fn main() -> ! {
    relocate_vectors();
    let Ok(runtime_init) = init() else {
        fail_closed();
    };
    radio_app::set_peripheral_clock_khz(runtime_init.clocks.pclk1_hz() / 1_000);

    // Why the radio last reset, taken before anything else can provoke one and
    // cleared immediately so the next boot reports its own cause rather than
    // inheriting this one. `RISK-036`: the unit restarts when a host speaks to
    // it, and this is the part saying whether that was a brown-out, a watchdog,
    // or software asking.
    let csr = RCC.csr().read();
    radio_app::set_reset_cause(ResetCause(u8::try_from((csr.0 >> 24) & 0xff).unwrap_or(0)));
    RCC.csr().modify(|register| register.set_rmvf(true));

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
    let serial = Link(runtime.usart);
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
        // Active-low PTT. It reaches the radio only through the shared TX
        // controller and policy (`TXK1-060`).
        ptt: Input::new(p.PB10, Pull::Up),
    };

    // BK4819 three-wire pins from the pinned K1 pinout. Transmit entry on
    // this bus requires a fresh policy token from the shared TX controller.
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
            K1Speaker {
                pin: Some(p.PA8),
                output: None,
            },
            BatterySense::new(p.ADC1, p.PB0),
        ) else {
            fail_closed();
        };
        spawner.spawn(serial);
        spawner.spawn(ui);
    });
}

// Both tasks are the shared futures themselves rather than async wrappers
// around them. A wrapper builds the inner future in its own poll frame before
// moving it, which cost a 2.8 KB frame on the interface task (`RISK-033`).
#[embassy_executor::task]
fn serial_task(link: Link, memory: EepromPins) -> impl Future<Output = ()> {
    radio_app::serial::run(link, move || open_memory(memory), IMAGE_IDENTITY, &MEMORY)
}

#[embassy_executor::task]
fn ui_task(
    display: DisplayPins,
    keypad: KeypadPins,
    radio_pins: Bk4819Pins,
    speaker: K1Speaker,
    battery_sense: BatterySense,
) -> impl Future<Output = ()> {
    // The pinned K1 build compiles the BK4829 driver, so this board needs that
    // variant's register values.
    let radio = Bk4819::with_profile(ThreeWireBus::new(radio_pins), BK4829_PROFILE);
    radio_app::interface::run(
        display,
        keypad,
        radio,
        speaker,
        battery_sense,
        radio_firmware_k1::battery::new(),
        K1,
        &MEMORY,
    )
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
}

impl Display for DisplayPins {
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

    fn set_backlight(&mut self, on: bool) {
        self.backlight
            .set_level(if on { Level::High } else { Level::Low });
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

    async fn scan_matrix(&mut self) -> KeypadScan {
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

impl Keypad for KeypadPins {
    async fn scan(&mut self) -> Sample {
        match decode(self.scan_matrix().await) {
            Ok(Some(key)) => Sample::Key(key.shared()),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        }
    }

    fn ptt_pressed(&mut self) -> bool {
        self.ptt.is_low()
    }
}

/// Active-high receive audio amplifier enable on `PA8`.
///
/// The pin stays untouched until the application claims it, because the K1
/// programming cable shares the speaker and microphone jack.
struct K1Speaker {
    pin: Option<PA8>,
    output: Option<Output<'static>>,
}

impl Speaker for K1Speaker {
    fn claim(&mut self) {
        if let Some(pin) = self.pin.take() {
            self.output = Some(Output::new(pin, Level::Low, Speed::Low));
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        if let Some(output) = self.output.as_mut() {
            if enabled {
                output.set_high();
            } else {
                output.set_low();
            }
        }
    }
}

/// USART1, collected by DMA and delimited by the idle line.
struct Link(Uart<'static, Async>);

impl SerialLink for Link {
    async fn read_until_idle(&mut self, buffer: &mut [u8]) -> Result<usize, LinkError> {
        self.0.read_until_idle(buffer).await.map_err(|_| LinkError)
    }

    async fn write(&mut self, bytes: &[u8]) {
        let _ = self.0.write(bytes).await;
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

/// Opens the external memory read-only and identifies it.
///
/// Its device and wiring come from the pinned reference firmware, so the
/// identification is read, reported on the information screen, and only a
/// memory which answers is used. A radio which cannot reach its memory is
/// still a working receiver with an empty store.
fn open_memory(memory: EepromPins) -> (Option<Memory>, MemoryState) {
    let mut state = MemoryState::Failed;
    let retained = RetainedConfiguration::new(
        memory.spi,
        memory.sck,
        memory.mosi,
        memory.miso,
        memory.chip_select,
    )
    .ok()
    .and_then(|mut configuration| match configuration.identify() {
        Ok(id) => {
            state = MemoryState::Present([id.manufacturer, id.memory_type, id.capacity_code]);
            Some(Memory(configuration))
        }
        Err(RetainError::Absent(_)) => {
            state = MemoryState::Absent;
            None
        }
        Err(_) => None,
    });
    (retained, state)
}

/// The K1's external SPI memory behind the shared memory view.
///
/// The view is laid out as the K5's (`memory_view`): the image part is the AFIK
/// configuration region, so a K1 programmed by an earlier image keeps its
/// channels; the place part is the latest record from its own sector and slot
/// rotation; the calibration part reads the vendor block, so a host backup
/// holds this unit's calibration. Host writes are refused until the K1 has an
/// erase plan for block writes.
struct Memory(RetainedConfiguration);

impl radio_app::drivers::Memory for Memory {
    fn read(&mut self, address: u16, output: &mut [u8]) -> bool {
        match locate(address, output.len()) {
            Some(ViewSpan::Image(offset)) => self.0.read_configuration(offset, output),
            Some(ViewSpan::Unstored) => {
                output.fill(0xFF);
                true
            }
            Some(ViewSpan::Place(offset)) => {
                let record = self.0.read_operator_state().map(|state| state.encode());
                fill_place(offset, record, output);
                true
            }
            Some(ViewSpan::Vendor(address)) => self.0.read_vendor(address, output),
            None => false,
        }
    }

    fn write(&mut self, _address: u16, _data: &[u8; WRITE_BYTES]) -> bool {
        false
    }

    fn read_battery_calibration(&mut self) -> Option<Calibration> {
        self.0.read_battery_calibration()
    }

    fn read_tx_calibration(&mut self) -> Option<TxCalibration> {
        self.0.read_tx_calibration()
    }

    fn read_operator_state(&mut self) -> Option<OperatorState> {
        self.0.read_operator_state()
    }

    fn write_operator_state(&mut self, state: OperatorState) -> bool {
        self.0.write_operator_state(state).is_ok()
    }
}

fn fail_closed() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Restarts on panic rather than stopping silently.
///
/// A spin loop here is what an unexplainable radio looks like: the display
/// holds its last frame, the keypad is dead, and the link is silent. Resetting
/// restores receive-only operation, and the next boot reports the software
/// reset flag from `RCC_CSR`.
///
/// This is a deliberate change from stopping forever. A restart also ends a
/// transmission: the interface task switches the PA and chip TX off before it
/// uses the radio for anything else. The exact unit proved its bootloader destroys `.uninit`,
/// so no file-and-line report is attempted here.
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    SCB::sys_reset()
}
