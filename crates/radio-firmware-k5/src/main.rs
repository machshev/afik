//! The first UV-K5 V1 application AFIK can observe running.
//!
//! It configures the clock, binds UART1 to the programming connector, says once
//! that it booted, and then answers the read-only hello for as long as it is
//! powered. It drives no display, no keypad, no radio, and no memory, because
//! `K5DRV-048` has evidence for none of those on this board yet.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use radio_dp32g030::clock;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::uart::{divider, Uart};
use radio_dp32g030::UART1_BASE;
use radio_firmware_k5::protocol::{
    encode_hello_response, Request, RequestReader, BOOT_BANNER, RESPONSE_FRAME_BYTES,
};

/// Initial stack pointer, per `EVID-K5-019`: the top of the evidenced RAM less
/// the sixteen bytes the firmware running on these units leaves alone.
const INITIAL_STACK_POINTER: u32 = 0x2000_3FF0;

/// The programming connector's speed, which the stock bootloader also uses.
const PROGRAMMING_BAUD: u32 = 38_400;

/// The UART bound to the programming connector, per `EVID-K5-019`.
const UART1: Uart = Uart::new(UART1_BASE);

#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
    non_maskable_interrupt: extern "C" fn() -> !,
    hard_fault: extern "C" fn() -> !,
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
};

// SAFETY: this is the single linker entry symbol named by link.x. The name is
// scoped to this standalone image, and the function has the Cortex-M ABI.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    startup::initialise_ram();
    main()
}

// SAFETY: as `reset`. No interrupt is ever enabled, so these two vectors can
// only be reached by a fault, which this image cannot recover from and must not
// pretend to.
#[allow(unsafe_code)]
#[unsafe(export_name = "Fault")]
extern "C" fn fault() -> ! {
    loop {
        core::hint::spin_loop();
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
    ]);
    gpio::set_output(Port::A, 7);
    gpio::set_input(Port::A, 8);
    portcon::enable_input(Port::A, 8);

    let clock = clock::configure();
    portcon::select_pa7_uart1_tx();
    portcon::select_pa8_uart1_rx();
    UART1.configure(clock, PROGRAMMING_BAUD);

    // The banner carries the two numbers that decide whether anything else
    // works: the frequency the image believes it is running at, corrected by
    // the part's own measurement, and the divider that follows from it.
    UART1.write(BOOT_BANNER);
    UART1.write(b" clk=");
    write_decimal(clock.hertz());
    UART1.write(b" div=");
    write_decimal(u32::from(divider(clock.hertz(), PROGRAMMING_BAUD)));
    UART1.write(b"\r\n");
    UART1.flush();

    let mut reader = RequestReader::new();
    loop {
        if reader.push(receive_byte()) == Some(Request::Hello) {
            let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
            encode_hello_response(&mut frame);
            UART1.write(&frame);
            UART1.flush();
        }
    }
}

/// Sends one unsigned number as decimal digits, most significant first.
fn write_decimal(mut value: u32) {
    let mut digits = [b'0'; 10];
    let mut index = digits.len();
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
        if value == 0 || index == 0 {
            break;
        }
    }
    UART1.write(&digits[index..]);
}

/// Waits for one received byte.
fn receive_byte() -> u8 {
    loop {
        if let Some(byte) = UART1.read_byte() {
            return byte;
        }
        core::hint::spin_loop();
    }
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

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
