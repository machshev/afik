//! Minimal UV-K1/PY32F071 Cortex-M0+ serial witness application.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use radio_firmware_k1::backlight::constant_on_plan;
use radio_firmware_k1::display::{
    initialise as display_initialise, render_key_witness, render_witness, write_frame, DisplayBus,
    TransferKind, FRAME_BYTES,
};
use radio_firmware_k1::keypad::{
    active_rows_from_gpio_idr, decode, gpio_plan, scan, Debouncer, Edge, MatrixBus, Sample,
};
use radio_firmware_k1::protocol::{
    decode_request, encode_hello_response, encode_keypad_response, Request, REQUEST_BODY_BYTES,
};

const INITIAL_STACK_POINTER: u32 = 0x2000_4000;
const BOOT_SENTINEL_VALUE: u32 = 0x4B31_B007;

const RCC_BASE: usize = 0x4002_1000;
const GPIOA_BASE: usize = 0x5000_0000;
const GPIOB_BASE: usize = 0x5000_0400;
const GPIOF_BASE: usize = 0x5000_1400;
const SPI1_BASE: usize = 0x4001_3000;
const USART1_BASE: usize = 0x4001_3800;

const RCC_IOPENR: usize = RCC_BASE + 0x34;
const RCC_APBRSTR2: usize = RCC_BASE + 0x30;
const RCC_APBENR2: usize = RCC_BASE + 0x40;
const GPIOA_MODER: usize = GPIOA_BASE;
const GPIOA_OTYPER: usize = GPIOA_BASE + 0x04;
const GPIOA_OSPEEDR: usize = GPIOA_BASE + 0x08;
const GPIOA_PUPDR: usize = GPIOA_BASE + 0x0C;
const GPIOA_BSRR: usize = GPIOA_BASE + 0x18;
const GPIOA_AFRL: usize = GPIOA_BASE + 0x20;
const GPIOA_AFRH: usize = GPIOA_BASE + 0x24;
const GPIOA_BRR: usize = GPIOA_BASE + 0x28;
const GPIOB_MODER: usize = GPIOB_BASE;
const GPIOB_OTYPER: usize = GPIOB_BASE + 0x04;
const GPIOB_OSPEEDR: usize = GPIOB_BASE + 0x08;
const GPIOB_PUPDR: usize = GPIOB_BASE + 0x0C;
const GPIOB_BSRR: usize = GPIOB_BASE + 0x18;
const GPIOB_BRR: usize = GPIOB_BASE + 0x28;
const GPIOB_IDR: usize = GPIOB_BASE + 0x10;
const GPIOF_MODER: usize = GPIOF_BASE;
const GPIOF_OTYPER: usize = GPIOF_BASE + 0x04;
const GPIOF_OSPEEDR: usize = GPIOF_BASE + 0x08;
const GPIOF_PUPDR: usize = GPIOF_BASE + 0x0C;
const GPIOF_BSRR: usize = GPIOF_BASE + 0x18;
const SPI1_CR1: usize = SPI1_BASE;
const SPI1_CR2: usize = SPI1_BASE + 0x04;
const SPI1_SR: usize = SPI1_BASE + 0x08;
const SPI1_DR: usize = SPI1_BASE + 0x0C;
const USART1_SR: usize = USART1_BASE;
const USART1_DR: usize = USART1_BASE + 0x04;
const USART1_BRR: usize = USART1_BASE + 0x08;
const USART1_CR1: usize = USART1_BASE + 0x0C;
const USART1_CR2: usize = USART1_BASE + 0x10;
const USART1_CR3: usize = USART1_BASE + 0x14;

const RCC_GPIOA_ENABLE: u32 = 1 << 0;
const RCC_GPIOB_ENABLE: u32 = 1 << 1;
const RCC_SPI1_RESET: u32 = 1 << 12;
const RCC_SPI1_ENABLE: u32 = 1 << 12;
const RCC_USART1_RESET: u32 = 1 << 14;
const RCC_USART1_ENABLE: u32 = 1 << 14;
const USART_STATUS_RXNE: u32 = 1 << 5;
const USART_STATUS_TXE: u32 = 1 << 7;
const USART_CONTROL_ENABLE: u32 = 1 << 13;
const USART_RECEIVER_ENABLE: u32 = 1 << 2;
const USART_TRANSMITTER_ENABLE: u32 = 1 << 3;
const UART_BAUD_DIVISOR_48MHZ_38400: u32 = 1_250;
const SPI_STATUS_RXNE: u32 = 1 << 0;
const SPI_STATUS_TXE: u32 = 1 << 1;
const SPI_STATUS_BUSY: u32 = 1 << 7;
const SPI_CONTROL_MODE_3: u32 = (1 << 0) | (1 << 1);
const SPI_CONTROL_MASTER: u32 = 1 << 2;
const SPI_CONTROL_DIVIDE_64: u32 = 0b101 << 3;
const SPI_CONTROL_ENABLE: u32 = 1 << 6;
const SPI_CONTROL_INTERNAL_SELECT: u32 = 1 << 8;
const SPI_CONTROL_SOFTWARE_SELECT: u32 = 1 << 9;
const DISPLAY_A0_PIN: u32 = 1 << 6;
const DISPLAY_CS_PIN: u32 = 1 << 2;
const DISPLAY_POLL_LIMIT: usize = 96_000;

#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
}

// SAFETY: the K1 linker script owns this section, retains this one table, and
// places it at the evidenced application origin.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".vector_table")]
static VECTOR_TABLE: VectorTable = VectorTable {
    initial_stack_pointer: INITIAL_STACK_POINTER,
    reset,
};

// SAFETY: the linker script owns this section, asserts its one-word size, and
// places it at the development-only RAM witness address.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".boot_sentinel")]
static BOOT_SENTINEL: AtomicU32 = AtomicU32::new(0);

// SAFETY: this is the single linker entry symbol named by link.x. The function
// uses the Cortex-M ABI and has no peripheral or external-memory side effect.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    BOOT_SENTINEL.store(BOOT_SENTINEL_VALUE, Ordering::Release);
    uart_init();
    backlight_init();
    display_init();
    keypad_init();
    let mut debounce = Debouncer::new();
    let mut elapsed_ms = 0_u32;
    let mut latched_rows = [0_u8; 4];
    loop {
        if uart_has_byte() {
            match receive_request() {
                Some(Request::Hello) => {
                    let mut response = [0_u8; 48];
                    encode_hello_response(&mut response);
                    uart_send(&response);
                }
                Some(Request::KeypadMatrix) => {
                    let mut matrix = K1MatrixBus;
                    let (rows, valid) = match scan(&mut matrix) {
                        Ok(rows) => (rows, true),
                        Err(_) => ([0_u8; 4], false),
                    };
                    if rows.iter().any(|row| *row != 0) {
                        latched_rows = rows;
                    }
                    let captured = latched_rows.iter().any(|row| *row != 0);
                    let reported_rows = if captured { latched_rows } else { rows };
                    let mut response = [0_u8; 20];
                    encode_keypad_response(&mut response, reported_rows, valid, captured);
                    uart_send(&response);
                    latched_rows = [0_u8; 4];
                }
                None => {}
            }
        }

        let mut matrix = K1MatrixBus;
        let scanned_rows = scan(&mut matrix).ok();
        if let Some(rows) = scanned_rows {
            if rows.iter().any(|row| *row != 0) {
                latched_rows = rows;
            }
        }
        let sample = match scanned_rows.and_then(|rows| decode(rows).ok()) {
            Some(Some(key)) => Sample::Key(key),
            Some(None) => Sample::Released,
            None => Sample::Invalid,
        };
        if let Edge::Pressed(key) = debounce.update(elapsed_ms, sample) {
            let mut frame = [0_u8; FRAME_BYTES];
            render_key_witness(&mut frame, key);
            // Physical MENU observation showed that entering the synchronous
            // SPI transfer prevents the retained serial diagnostic from
            // answering. Keep decode/render execution for the bounded Renode
            // proof, but suppress the physical transfer while raw matrix
            // sampling localises the GPIO/display boundary.
        }
        delay_milliseconds(1);
        elapsed_ms = elapsed_ms.wrapping_add(1);
    }
}

#[allow(unsafe_code)]
fn read_register(address: usize) -> u32 {
    // SAFETY: addresses and access widths are taken from the pinned PY32F071
    // device header; this function is used only for volatile MMIO access.
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

#[allow(unsafe_code)]
fn write_register(address: usize, value: u32) {
    // SAFETY: addresses and access widths are taken from the pinned PY32F071
    // device header; this function is used only for volatile MMIO access.
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}

#[allow(unsafe_code)]
fn write_register_u8(address: usize, value: u8) {
    // SAFETY: the pinned device header specifies an eight-bit access to the
    // SPI data register for eight-bit transfers.
    unsafe { core::ptr::write_volatile(address as *mut u8, value) }
}

fn update_register(address: usize, clear: u32, set: u32) {
    let current = read_register(address);
    write_register(address, (current & !clear) | set);
}

fn uart_init() {
    update_register(RCC_IOPENR, 0, RCC_GPIOA_ENABLE);
    update_register(RCC_APBENR2, 0, RCC_USART1_ENABLE);
    update_register(RCC_APBRSTR2, 0, RCC_USART1_RESET);
    update_register(RCC_APBRSTR2, RCC_USART1_RESET, 0);

    let pin_mask = (0b11 << 18) | (0b11 << 20);
    let alternate_mode = (0b10 << 18) | (0b10 << 20);
    update_register(GPIOA_MODER, pin_mask, alternate_mode);
    update_register(GPIOA_OTYPER, (1 << 9) | (1 << 10), 0);
    update_register(GPIOA_OSPEEDR, pin_mask, (0b11 << 18) | (0b11 << 20));
    update_register(GPIOA_PUPDR, pin_mask, (0b01 << 18) | (0b01 << 20));
    update_register(GPIOA_AFRH, (0xF << 4) | (0xF << 8), (1 << 4) | (1 << 8));

    write_register(USART1_CR1, 0);
    write_register(USART1_CR2, 0);
    write_register(USART1_CR3, 0);
    write_register(USART1_BRR, UART_BAUD_DIVISOR_48MHZ_38400);
    write_register(
        USART1_CR1,
        USART_CONTROL_ENABLE | USART_RECEIVER_ENABLE | USART_TRANSMITTER_ENABLE,
    );
}

fn backlight_init() {
    let plan = constant_on_plan();
    update_register(RCC_IOPENR, 0, plan.clock_enable);
    write_register(GPIOF_BSRR, plan.output_high);
    update_register(GPIOF_OTYPER, plan.output_type_clear, 0);
    update_register(GPIOF_OSPEEDR, plan.speed_clear, plan.speed_set);
    update_register(GPIOF_PUPDR, plan.pull_clear, plan.pull_set);
    update_register(GPIOF_MODER, plan.mode_clear, plan.mode_set);
}

fn display_init() {
    update_register(RCC_IOPENR, 0, RCC_GPIOA_ENABLE | RCC_GPIOB_ENABLE);
    update_register(RCC_APBENR2, 0, RCC_SPI1_ENABLE);
    update_register(RCC_APBRSTR2, 0, RCC_SPI1_RESET);
    update_register(RCC_APBRSTR2, RCC_SPI1_RESET, 0);

    write_register(GPIOA_BSRR, DISPLAY_A0_PIN);
    write_register(GPIOB_BSRR, DISPLAY_CS_PIN);

    let gpioa_display_mask = (0b11 << 10) | (0b11 << 12) | (0b11 << 14);
    let gpioa_display_modes = (0b10 << 10) | (0b01 << 12) | (0b10 << 14);
    update_register(GPIOA_MODER, gpioa_display_mask, gpioa_display_modes);
    update_register(GPIOA_OTYPER, (1 << 5) | (1 << 6) | (1 << 7), 0);
    update_register(GPIOA_OSPEEDR, gpioa_display_mask, gpioa_display_mask);
    update_register(GPIOA_PUPDR, gpioa_display_mask, 0b01 << 10);
    update_register(GPIOA_AFRL, (0xF << 20) | (0xF << 28), 0);

    update_register(GPIOB_MODER, 0b11 << 4, 0b01 << 4);
    update_register(GPIOB_OTYPER, DISPLAY_CS_PIN, 0);
    update_register(GPIOB_OSPEEDR, 0b11 << 4, 0b11 << 4);
    update_register(GPIOB_PUPDR, 0b11 << 4, 0b01 << 4);

    write_register(SPI1_CR1, 0);
    write_register(SPI1_CR2, 0);
    write_register(
        SPI1_CR1,
        SPI_CONTROL_MODE_3
            | SPI_CONTROL_MASTER
            | SPI_CONTROL_DIVIDE_64
            | SPI_CONTROL_INTERNAL_SELECT
            | SPI_CONTROL_SOFTWARE_SELECT
            | SPI_CONTROL_ENABLE,
    );

    let mut display = K1DisplayBus;
    let mut frame = [0_u8; FRAME_BYTES];
    render_witness(&mut frame);
    let _ = display_initialise(&mut display).and_then(|()| write_frame(&mut display, &frame));
}

fn keypad_init() {
    let plan = gpio_plan();
    update_register(RCC_IOPENR, 0, plan.clock_enable);
    write_register(GPIOB_BSRR, plan.columns_high);
    update_register(GPIOB_OTYPER, plan.column_type_clear, 0);
    update_register(
        GPIOB_OSPEEDR,
        plan.column_speed_clear,
        plan.column_speed_set,
    );
    update_register(GPIOB_PUPDR, plan.row_pull_clear, plan.row_pull_set);
    update_register(GPIOB_PUPDR, plan.column_pull_clear, plan.column_pull_set);
    update_register(GPIOB_MODER, plan.row_mode_clear, 0);
    update_register(GPIOB_MODER, plan.column_mode_clear, plan.column_mode_set);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayError {
    Timeout,
}

struct K1DisplayBus;

impl DisplayBus for K1DisplayBus {
    type Error = DisplayError;

    fn write(&mut self, kind: TransferKind, bytes: &[u8]) -> Result<(), Self::Error> {
        match kind {
            TransferKind::Command => write_register(GPIOA_BRR, DISPLAY_A0_PIN),
            TransferKind::Data => write_register(GPIOA_BSRR, DISPLAY_A0_PIN),
        }
        write_register(GPIOB_BRR, DISPLAY_CS_PIN);

        for byte in bytes {
            if !poll_register(SPI1_SR, SPI_STATUS_TXE, true) {
                write_register(GPIOB_BSRR, DISPLAY_CS_PIN);
                return Err(DisplayError::Timeout);
            }
            write_register_u8(SPI1_DR, *byte);
            if !poll_register(SPI1_SR, SPI_STATUS_RXNE, true) {
                write_register(GPIOB_BSRR, DISPLAY_CS_PIN);
                return Err(DisplayError::Timeout);
            }
            let _ = read_register(SPI1_DR);
        }
        if !poll_register(SPI1_SR, SPI_STATUS_BUSY, false) {
            write_register(GPIOB_BSRR, DISPLAY_CS_PIN);
            return Err(DisplayError::Timeout);
        }
        write_register(GPIOB_BSRR, DISPLAY_CS_PIN);
        Ok(())
    }

    fn delay_ms(&mut self, milliseconds: u8) {
        delay_milliseconds(u32::from(milliseconds));
    }
}

struct K1MatrixBus;

impl MatrixBus for K1MatrixBus {
    type Error = ();

    fn drive_all_columns_high(&mut self) -> Result<(), Self::Error> {
        write_register(GPIOB_BSRR, gpio_plan().columns_high);
        Ok(())
    }

    fn drive_column_low(&mut self, column: usize) -> Result<(), Self::Error> {
        write_register(GPIOB_BRR, gpio_plan().selected_low[column]);
        Ok(())
    }

    fn read_active_rows(&mut self) -> Result<u8, Self::Error> {
        // The pinned board source uses a 10 us settling interval. At the
        // evidenced 48 MHz bootloader handoff this bounded spin is conservative
        // and does not establish a production scan cadence.
        for _ in 0..120 {
            core::hint::spin_loop();
        }
        Ok(active_rows_from_gpio_idr(read_register(GPIOB_IDR)))
    }
}

fn delay_milliseconds(milliseconds: u32) {
    for _ in 0..milliseconds * 12_000 {
        core::hint::spin_loop();
    }
}

fn poll_register(address: usize, mask: u32, set: bool) -> bool {
    for _ in 0..DISPLAY_POLL_LIMIT {
        if (read_register(address) & mask != 0) == set {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn uart_receive_byte() -> u8 {
    while read_register(USART1_SR) & USART_STATUS_RXNE == 0 {
        core::hint::spin_loop();
    }
    read_register(USART1_DR).to_le_bytes()[0]
}

fn uart_has_byte() -> bool {
    read_register(USART1_SR) & USART_STATUS_RXNE != 0
}

fn uart_send_byte(byte: u8) {
    while read_register(USART1_SR) & USART_STATUS_TXE == 0 {
        core::hint::spin_loop();
    }
    write_register(USART1_DR, u32::from(byte));
}

fn uart_send(bytes: &[u8]) {
    for byte in bytes {
        uart_send_byte(*byte);
    }
}

fn receive_request() -> Option<Request> {
    let mut previous_was_header = false;
    loop {
        let byte = uart_receive_byte();
        if previous_was_header && byte == 0xCD {
            break;
        }
        previous_was_header = byte == 0xAB;
    }

    let length = u16::from_le_bytes([uart_receive_byte(), uart_receive_byte()]);
    if length != 8 {
        let discard = usize::from(length).saturating_add(4).min(276);
        for _ in 0..discard {
            let _ = uart_receive_byte();
        }
        return None;
    }

    let mut body = [0_u8; REQUEST_BODY_BYTES];
    for byte in &mut body {
        *byte = uart_receive_byte();
    }
    let footer = [uart_receive_byte(), uart_receive_byte()];
    if footer == [0xDC, 0xBA] {
        decode_request(&mut body)
    } else {
        None
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
