//! UV-K5 V1 target: DP32G030 drivers for the shared AFIK application.
//!
//! Everything the operator and the host see is `radio-app`. This image sets
//! the clock, starts the SysTick time driver, binds UART1 to the programming
//! connector, and starts the application's two tasks: the serial task owns
//! UART1 and the EEPROM, and the interface task owns the display, the keypad,
//! the speaker and the BK4819.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::future::Future;
use core::mem::MaybeUninit;
use core::panic::PanicInfo;
// Linked for its single-core critical section, which nothing names directly.
use cortex_m as _;
use embassy_executor::Executor;
use embassy_time::{Duration, Timer};
use radio_app::drivers::{Broadcast, LinkError, SerialLink, Target};
use radio_app::TaskCell;
use radio_bk1080::{Bk1080, I2cRegisters};
use radio_bk4819::Bk4819;
use radio_dp32g030::clock;
use radio_dp32g030::dma::CircularReceiver;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::saradc;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::time_driver;
use radio_dp32g030::uart::{k5_programming_divider, Uart};
use radio_dp32g030::UART1_BASE;
use radio_firmware_k5::audio::SpeakerGate;
use radio_firmware_k5::battery;
use radio_firmware_k5::bk4819_bus::{K5Pins, K5RegisterBus};
use radio_firmware_k5::broadcast::K5Broadcast;
use radio_firmware_k5::display::K5Display;
use radio_firmware_k5::drivers::{K5Battery, K5Keypad};
use radio_firmware_k5::eeprom::{self, K5Bus};
use radio_firmware_k5::memory::K5Memory;
use radio_platform::operator_display::MemoryState;
use radio_platform::transmit::{Delay, TxTarget};

/// Initial stack pointer, per `EVID-K5-019`: the top of the evidenced RAM less
/// the sixteen bytes the firmware running on these units leaves alone.
const INITIAL_STACK_POINTER: u32 = 0x2000_3FF0;

/// The UART bound to the programming connector, per `EVID-K5-019`.
const UART1: Uart = Uart::new(UART1_BASE);
const DMA_BYTES: usize = 256;

static mut DMA_BUFFER: [u8; DMA_BYTES] = [0; DMA_BYTES];
static mut EXECUTOR: MaybeUninit<Executor> = MaybeUninit::uninit();

/// The EEPROM, shared by the serial task and the interface task's lookups.
static MEMORY: TaskCell<Option<K5Memory<K5Bus>>> = TaskCell::new(None);

/// The K5 as the application sees it. Armel K5 `dividers` Low1–Low4 are 25,
/// 19, 13 and 10.
const K5: Target = Target {
    identity: concat!("AFIK-K5-", env!("CARGO_PKG_VERSION")).as_bytes(),
    tx: TxTarget {
        target_id: 0x4B35_5631,
        dividers: [25, 19, 13, 10],
    },
};

/// The sixteen Cortex-M0 system vectors, ending with `SysTick`, which drives
/// the Embassy time driver. Reserved slots are zero.
#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
    non_maskable_interrupt: extern "C" fn() -> !,
    hard_fault: extern "C" fn() -> !,
    reserved_4_10: [u32; 7],
    supervisor_call: extern "C" fn() -> !,
    reserved_12_13: [u32; 2],
    pend_supervisor: extern "C" fn() -> !,
    system_tick: extern "C" fn(),
}

// SAFETY: the linker script owns this section, retains this one table, asserts
// its exact size, and places it at the evidenced Cortex-M0 vector address.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".vector_table")]
static VECTOR_TABLE: VectorTable = VectorTable {
    initial_stack_pointer: INITIAL_STACK_POINTER,
    reset,
    non_maskable_interrupt: fault,
    hard_fault: fault,
    reserved_4_10: [0; 7],
    supervisor_call: fault,
    reserved_12_13: [0; 2],
    pend_supervisor: fault,
    system_tick,
};

// SAFETY: this is the single linker entry symbol named by link.x. The name is
// scoped to this standalone image, and the function has the Cortex-M ABI.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    startup::initialise_ram();
    main()
}

// SAFETY: as `reset`. Only SysTick is enabled, so the other vectors can only
// be reached by a fault, which this image cannot recover from and must not
// pretend to.
#[allow(unsafe_code)]
#[unsafe(export_name = "Fault")]
extern "C" fn fault() -> ! {
    best_effort_transmit_off();
    loop {
        core::hint::spin_loop();
    }
}

// SAFETY: as `reset`; the exception body only touches the time driver's
// critical-section state.
#[allow(unsafe_code)]
#[unsafe(export_name = "SysTick")]
extern "C" fn system_tick() {
    time_driver::on_systick();
}

/// Busy-waits on the time driver, for the display's start-up waits.
struct BlockingDelay;

impl Delay for BlockingDelay {
    fn delay_ms(&mut self, milliseconds: u32) {
        embassy_time::block_for(Duration::from_millis(u64::from(milliseconds)));
    }
}

impl radio_bk1080::Delay for BlockingDelay {
    fn delay_ms(&mut self, milliseconds: u32) {
        Delay::delay_ms(self, milliseconds);
    }
}

fn main() -> ! {
    // The ports are gated on beside the UART, and the two pins are given an
    // explicit direction, because `AFIK-K5-1.0` did neither and was silent:
    // this GPIO block takes its pad direction from `GPIODIR`, so a pin whose
    // port has no clock is not driven however PORTCON selects it.
    syscon::enable(&[
        Peripheral::GpioA,
        Peripheral::GpioB,
        Peripheral::GpioC,
        Peripheral::Uart1,
        Peripheral::Saradc,
    ]);
    gpio::set_output(Port::A, 7);
    gpio::set_input(Port::A, 8);
    portcon::enable_input(Port::A, 8);
    portcon::enable_pull_up(Port::A, 8);

    let clock = clock::configure();
    time_driver::start(clock.hertz()).expect("SysTick counts a millisecond of the K5 clock");
    // The bus's edge wait is a microsecond as this part actually runs.
    eeprom::calibrate(
        clock.hertz(),
        time_driver::cycles_taken(|| eeprom::spin(eeprom::CALIBRATION_SPINS)),
    );

    portcon::select_pa7_uart1_tx();
    portcon::select_pa8_uart1_rx();
    portcon::select_k5_battery_saradc();
    saradc::initialise();
    UART1.prepare_receive_dma_with_divider(k5_programming_divider(clock.hertz()));
    // SAFETY: this single-core image gives the static buffer exclusively to
    // this receiver for the rest of its lifetime.
    #[allow(unsafe_code)]
    let receiver = unsafe {
        CircularReceiver::<DMA_BYTES>::new(&raw mut DMA_BUFFER as *mut u8, UART1.receive_address())
    };
    UART1.start_receive_dma();

    // PC4 starts low; the application opens it only once squelch is open.
    let speaker = SpeakerGate::initialise();
    let keypad = K5Keypad::initialise();
    let radio = Bk4819::new(K5RegisterBus::new(K5Pins::initialise()));
    // PB15 high leaves the BK1080 off until a broadcast channel is chosen.
    let broadcast = Bk1080::new(I2cRegisters(K5Broadcast::initialise()));

    // SAFETY: `main` never returns and is the only code to touch this static,
    // so the executor lives for the rest of the image and is borrowed once.
    #[allow(unsafe_code)]
    let executor = unsafe { (*(&raw mut EXECUTOR)).write(Executor::new()) };
    executor.run(|spawner| {
        spawner.spawn(
            serial_task(Link {
                receiver,
                held: [0; LINK_BYTES],
                held_len: 0,
            })
            .expect("one serial task"),
        );
        spawner.spawn(
            interface_task(
                K5Display::new(BlockingDelay),
                keypad,
                radio,
                broadcast,
                speaker,
            )
            .expect("one interface task"),
        );
    });
}

// Both tasks are the shared futures themselves rather than async wrappers
// around them, so no wrapper frame holds the inner future (`RISK-033`).
#[embassy_executor::task]
fn serial_task(link: Link) -> impl Future<Output = ()> {
    radio_app::serial::run(link, open_memory, K5.identity, &MEMORY)
}

#[embassy_executor::task]
fn interface_task(
    display: K5Display<BlockingDelay>,
    keypad: K5Keypad,
    radio: Bk4819<K5RegisterBus<K5Pins>>,
    broadcast: Bk1080<I2cRegisters<K5Broadcast>>,
    speaker: SpeakerGate,
) -> impl Future<Output = ()> {
    radio_app::interface::run(
        display,
        keypad,
        radio,
        Broadcast {
            chip: broadcast,
            delay: BlockingDelay,
        },
        speaker,
        K5Battery,
        battery::new(),
        K5,
        &MEMORY,
    )
}

/// Takes the EEPROM and checks it answers. It has no identification bytes, so
/// a present one reports zeros.
fn open_memory() -> (Option<K5Memory<K5Bus>>, MemoryState) {
    let mut probe = [0_u8; 1];
    let mut bus = K5Bus::initialise();
    if eeprom::read(&mut bus, 0, &mut probe).is_ok() {
        (Some(K5Memory::new(bus)), MemoryState::Present([0; 3]))
    } else {
        (None, MemoryState::Failed)
    }
}

/// Bytes the link holds between polls.
const LINK_BYTES: usize = 64;

/// UART1 received by circular DMA and polled for an idle line.
///
/// Bytes taken from the DMA ring are held here rather than in the caller's
/// buffer until the line goes idle, so a read dropped part way through by the
/// serial task's `select` loses nothing.
struct Link {
    receiver: CircularReceiver<DMA_BYTES>,
    held: [u8; LINK_BYTES],
    held_len: usize,
}

impl Link {
    /// Moves what DMA has received into `held` and reports whether any came.
    fn collect(&mut self) -> bool {
        let before = self.held_len;
        while self.held_len < LINK_BYTES {
            let Some(byte) = self.receiver.read_byte() else {
                break;
            };
            self.held[self.held_len] = byte;
            self.held_len += 1;
        }
        self.held_len != before
    }
}

impl SerialLink for Link {
    /// Returns once a burst is followed by a millisecond with no new byte,
    /// which at 38,400 baud is several character times.
    async fn read_until_idle(&mut self, buffer: &mut [u8]) -> Result<usize, LinkError> {
        loop {
            let arrived = self.collect();
            if self.held_len == LINK_BYTES || (self.held_len > 0 && !arrived) {
                let taken = self.held_len.min(buffer.len());
                buffer[..taken].copy_from_slice(&self.held[..taken]);
                self.held.copy_within(taken..self.held_len, 0);
                self.held_len -= taken;
                return Ok(taken);
            }
            Timer::after_millis(1).await;
        }
    }

    async fn write(&mut self, bytes: &[u8]) {
        UART1.write(bytes);
        UART1.flush();
    }
}

/// Byte-at-a-time `memcpy`, `memmove` and `memset`, and the ARM EABI entry
/// points Rust actually calls.
///
/// compiler-builtins supplies word-copying versions tuned for speed, about
/// 1.2 KiB together, as weak symbols. The K5's 60 KiB application region has
/// no room for them, and nothing here copies enough for speed to matter: a
/// whole display frame is 1 KiB, about 0.13 ms at 48 MHz. These are Thumb-1
/// loops, written in assembly so the compiler cannot turn them back into
/// calls to themselves. `memmove` copies forward when the destination is
/// below the source and backward otherwise, so overlap is always safe.
/// `__aeabi_memset` takes its fill and length the other way round from
/// `memset`, and `__aeabi_memclr` fills with zero.
mod mem {
    // SAFETY: each routine reads and writes exactly the bytes its C or EABI
    // contract names, uses only r0-r4 (r4 saved), keeps the stack balanced and
    // returns per the AAPCS, so any caller of the standard symbols is served.
    #![allow(unsafe_code)]

    core::arch::global_asm!(include_str!("mem_routines.s"));
}

/// Startup work that has to touch memory the linker owns rather than Rust.
mod startup {
    // SAFETY: every item in this module operates on the four linker-defined
    // region symbols below. The linker script places `.data` and `.bss` inside
    // the evidenced RAM, word-aligns both, and asserts that the loaded image
    // ends inside the evidenced flash, so the regions copied and cleared here
    // exist and do not overlap.
    #![allow(unsafe_code)]

    unsafe extern "C" {
        /// First word of the initialised data region in RAM.
        static mut __data_start: u32;
        /// One past the last word of the initialised data region in RAM.
        static mut __data_end: u32;
        /// First word of that region's contents, held in flash.
        static __data_load_start: u32;
        /// First word of the zero-initialised region in RAM.
        static mut __bss_start: u32;
        /// One past the last word of the zero-initialised region in RAM.
        static mut __bss_end: u32;
    }

    /// Copies initialised data out of flash and clears the zeroed region.
    ///
    /// A Cortex-M0 comes out of reset with RAM in an undefined state and no
    /// runtime behind it. Until this has run, no static in this image holds the
    /// value its source says it does.
    pub fn initialise_ram() {
        // SAFETY: see the module note. The symbols are addresses, not values,
        // and are only ever used to bound a word-aligned walk.
        unsafe {
            let mut source = &raw const __data_load_start;
            let mut destination = &raw mut __data_start;
            let data_end = &raw mut __data_end;
            while destination < data_end {
                destination.write_volatile(source.read_volatile());
                source = source.add(1);
                destination = destination.add(1);
            }

            let mut zeroed = &raw mut __bss_start;
            let bss_end = &raw mut __bss_end;
            while zeroed < bss_end {
                zeroed.write_volatile(0);
                zeroed = zeroed.add(1);
            }
        }
    }
}

/// Rebinds the BK4819 pins and writes PA off then chip idle, ignoring state.
///
/// Best effort only: it needs the bus and the core to be working.
fn best_effort_transmit_off() {
    let mut radio = Bk4819::new(K5RegisterBus::new(K5Pins::initialise()));
    let _ = radio.disable_pa();
    let _ = radio.disable_transmit();
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    best_effort_transmit_off();
    loop {
        core::hint::spin_loop();
    }
}
