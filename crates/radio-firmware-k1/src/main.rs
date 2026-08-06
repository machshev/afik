//! Minimal UV-K1/PY32F071 Cortex-M0+ reset image.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use radio_firmware_k1::protocol::{
    encode_hello_response, is_valid_hello_request, REQUEST_BODY_BYTES,
};

const INITIAL_STACK_POINTER: u32 = 0x2000_4000;
const BOOT_SENTINEL_VALUE: u32 = 0x4B31_B007;

const RCC_BASE: usize = 0x4002_1000;
const GPIOA_BASE: usize = 0x5000_0000;
const USART1_BASE: usize = 0x4001_3800;

const RCC_IOPENR: usize = RCC_BASE + 0x34;
const RCC_APBRSTR2: usize = RCC_BASE + 0x30;
const RCC_APBENR2: usize = RCC_BASE + 0x40;
const GPIOA_MODER: usize = GPIOA_BASE;
const GPIOA_OTYPER: usize = GPIOA_BASE + 0x04;
const GPIOA_OSPEEDR: usize = GPIOA_BASE + 0x08;
const GPIOA_PUPDR: usize = GPIOA_BASE + 0x0C;
const GPIOA_AFRH: usize = GPIOA_BASE + 0x24;
const USART1_SR: usize = USART1_BASE;
const USART1_DR: usize = USART1_BASE + 0x04;
const USART1_BRR: usize = USART1_BASE + 0x08;
const USART1_CR1: usize = USART1_BASE + 0x0C;
const USART1_CR2: usize = USART1_BASE + 0x10;
const USART1_CR3: usize = USART1_BASE + 0x14;

const RCC_GPIOA_ENABLE: u32 = 1 << 0;
const RCC_USART1_RESET: u32 = 1 << 14;
const RCC_USART1_ENABLE: u32 = 1 << 14;
const USART_STATUS_RXNE: u32 = 1 << 5;
const USART_STATUS_TXE: u32 = 1 << 7;
const USART_CONTROL_ENABLE: u32 = 1 << 13;
const USART_RECEIVER_ENABLE: u32 = 1 << 2;
const USART_TRANSMITTER_ENABLE: u32 = 1 << 3;
const UART_BAUD_DIVISOR_48MHZ_38400: u32 = 1_250;

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
    loop {
        if receive_hello_request() {
            let mut response = [0_u8; 48];
            encode_hello_response(&mut response);
            uart_send(&response);
        }
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

fn uart_receive_byte() -> u8 {
    while read_register(USART1_SR) & USART_STATUS_RXNE == 0 {
        core::hint::spin_loop();
    }
    read_register(USART1_DR) as u8
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

fn receive_hello_request() -> bool {
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
        return false;
    }

    let mut body = [0_u8; REQUEST_BODY_BYTES];
    for byte in &mut body {
        *byte = uart_receive_byte();
    }
    let footer = [uart_receive_byte(), uart_receive_byte()];
    footer == [0xDC, 0xBA] && is_valid_hello_request(&mut body)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
