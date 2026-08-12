//! A diagnostic image which reports what the receiver actually sees.
//!
//! History, because this image has already answered one question and been
//! rewritten for the next:
//!
//! 1. `AFIK-K5-1.0` was written to the exact UV-K6 with all 240 pages
//!    acknowledged and was completely silent. The first version of this image
//!    removed the clock write, gated the GPIO ports on, set the two pin
//!    directions explicitly, and sent one line at each of three candidate
//!    dividers. `AFIK-K5-BISECT div=1250` came back legible, which proved the
//!    application runs, the part is at 48 MHz, and UART1 transmits on PA7.
//! 2. `AFIK-K5-1.1` folded the port gating and pin directions into the real
//!    application. Its banner arrived — `clk=47796863 div=1245` — so transmit,
//!    the clock and the RC correction are all confirmed on the unit. It never
//!    answers a hello, so the fault is now on the receive side alone.
//!
//! Transmit is known good, so this image uses it to report on the receiver: a
//! byte counter, the last bytes seen, and the raw `UART_IF` status. A counter
//! that stays at zero while the host sends means nothing reaches the receive
//! FIFO, and the question is the pin or the peripheral. A counter that climbs
//! means bytes arrive and the fault is above the driver — which would be
//! surprising, since the frame reader is tested against the host's own encoder.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use radio_dp32g030::clock;
use radio_dp32g030::dma::CircularReceiver;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::uart::{k5_programming_divider, Uart};
use radio_dp32g030::UART1_BASE;
use radio_firmware_k5::boot_display::{BootDisplay, BootStage, ReceiveDiagnostic};

mod k5_display;
use k5_display::K5BootDisplay;

/// Initial stack pointer, per `EVID-K5-019`.
const INITIAL_STACK_POINTER: u32 = 0x2000_3FF0;

/// The UART bound to the programming connector, per `EVID-K5-019`.
const UART1: Uart = Uart::new(UART1_BASE);

/// Roughly fifteen seconds of polling between reports at 48 MHz.
///
/// The interval is long because the image's own transmission is a suspect: a
/// host frame is fourteen back-to-back bytes and this radio has been observed
/// receiving them corrupted at the tail while the same bytes sent with gaps
/// arrive perfectly. A listener which stays quiet almost all the time cannot
/// be the thing corrupting what it hears.
const REPORT_INTERVAL_POLLS: u32 = 30_000_000;

/// How many received bytes are kept verbatim for the report.
///
/// A frame the host sends is fourteen bytes, so this holds two complete frames
/// and can be compared against what the host is known to have sent.
const FIRST_BYTES: usize = 28;
const DMA_BYTES: usize = 256;

static mut DMA_BUFFER: [u8; DMA_BYTES] = [0; DMA_BYTES];

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

// SAFETY: this is the single linker entry symbol named by link.x, with the
// Cortex-M ABI. This image has no initialised or zeroed statics, so it needs no
// startup copy: the linker script asserts both regions are empty.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    main()
}

// SAFETY: as `reset`. No interrupt is ever enabled, so this vector can only be
// reached by a fault.
#[allow(unsafe_code)]
#[unsafe(export_name = "Fault")]
extern "C" fn fault() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn main() -> ! {
    let mut display = K5BootDisplay::initialise();
    let _ = display.show(BootStage::Reset);

    syscon::enable(&[
        Peripheral::GpioA,
        Peripheral::GpioB,
        Peripheral::GpioC,
        Peripheral::Uart1,
    ]);
    gpio::set_output(Port::A, 7);
    gpio::set_input(Port::A, 8);
    portcon::enable_input(Port::A, 8);
    // The receive line read low while idle, which a UART reads as a permanent
    // start bit. Whether that is a floating pad or a measurement artefact of
    // reading a pin the peripheral owns, an idle-high pull is what a receive
    // line needs and it costs nothing to state.
    portcon::enable_pull_up(Port::A, 8);

    let clock = clock::configure();
    portcon::select_pa7_uart1_tx();
    portcon::select_pa8_uart1_rx();
    UART1.configure_with_divider(k5_programming_divider(clock.hertz()));
    // SAFETY: this image has one core and gives the static buffer exclusively
    // to this receiver for the rest of its lifetime.
    #[allow(unsafe_code)]
    let mut receiver = unsafe {
        CircularReceiver::<DMA_BYTES>::new(&raw mut DMA_BUFFER as *mut u8, UART1.receive_address())
    };
    UART1.enable_receive_dma();

    let _ = display.show(BootStage::SerialReady);

    UART1.write(b"AFIK-K5-RXDIAG\r\n");
    UART1.flush();

    let mut received: u32 = 0;
    let mut first = [0_u8; FIRST_BYTES];
    let mut first_len = 0_usize;
    let mut sticky_status: u32 = 0;
    let mut polls: u32 = 0;
    loop {
        sticky_status |= UART1.status_bits();
        if let Some(byte) = receiver.read_byte() {
            received = received.wrapping_add(1);
            if first_len < first.len() {
                first[first_len] = byte;
                first_len += 1;
            }
        }

        polls = polls.wrapping_add(1);
        if polls >= REPORT_INTERVAL_POLLS {
            polls = 0;
            let _ = display.show_receive(ReceiveDiagnostic {
                bytes: received,
                status: sticky_status,
            });
            UART1.write(b"rx=");
            write_decimal(received);
            UART1.write(b" sticky=");
            write_hexadecimal(sticky_status);
            UART1.write(b" pa8=");
            UART1.write(if gpio::read_pin(Port::A, 8) {
                b"1"
            } else {
                b"0"
            });
            UART1.write(b" first=");
            for byte in &first[..first_len] {
                write_byte_hexadecimal(*byte);
            }
            UART1.write(b"\r\n");
            UART1.flush();
        }
    }
}

/// Sends one byte as two hexadecimal digits.
fn write_byte_hexadecimal(byte: u8) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    UART1.write(&[
        DIGITS[usize::from(byte >> 4)],
        DIGITS[usize::from(byte & 0xF)],
    ]);
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

/// Sends one unsigned number as eight hexadecimal digits.
fn write_hexadecimal(value: u32) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = [b'0'; 8];
    for (index, character) in text.iter_mut().enumerate() {
        let shift = 28 - 4 * index;
        let nibble = (value >> shift) & 0xF;
        *character = DIGITS[nibble as usize];
    }
    UART1.write(&text);
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
