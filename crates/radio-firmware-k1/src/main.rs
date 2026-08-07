//! Minimal UV-K1/PY32F071 Cortex-M0+ serial-only clock diagnostic.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use radio_firmware_k1::clock_handoff::{snapshot_from_registers, validate};
use radio_firmware_k1::protocol::{
    decode_request, encode_clock_control_response, encode_clock_register_response,
    encode_clock_response, encode_hello_response, Request, REQUEST_BODY_BYTES,
};

const INITIAL_STACK_POINTER: u32 = 0x2000_4000;
const BOOT_SENTINEL_VALUE: u32 = 0x4B31_B007;

const RCC_BASE: usize = 0x4002_1000;
const GPIOA_BASE: usize = 0x5000_0000;
const USART1_BASE: usize = 0x4001_3800;

const RCC_IOPENR: usize = RCC_BASE + 0x34;
const RCC_CR: usize = RCC_BASE;
const RCC_ICSCR: usize = RCC_BASE + 0x04;
const RCC_CFGR: usize = RCC_BASE + 0x08;
const RCC_PLLCFGR: usize = RCC_BASE + 0x0C;
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
        if !uart_has_byte() {
            continue;
        }
        match receive_request() {
            Some(Request::Hello) => {
                let mut response = [0_u8; 48];
                encode_hello_response(&mut response);
                uart_send(&response);
            }
            Some(Request::ClockSnapshot) => {
                let registers = [
                    read_register(RCC_CR),
                    read_register(RCC_ICSCR),
                    read_register(RCC_CFGR),
                    read_register(RCC_PLLCFGR),
                ];
                let snapshot =
                    snapshot_from_registers(registers[0], registers[1], registers[2], registers[3]);
                let mut response = [0_u8; 32];
                encode_clock_response(&mut response, registers, validate(snapshot).is_ok());
                uart_send(&response);
            }
            Some(Request::ClockRegister(register)) => {
                let addresses = [RCC_CR, RCC_ICSCR, RCC_CFGR, RCC_PLLCFGR];
                if let Some(address) = addresses.get(usize::from(register)) {
                    let value = read_register(*address);
                    let mut response = [0_u8; 20];
                    encode_clock_register_response(&mut response, register, value);
                    uart_send(&response);
                }
            }
            Some(Request::ClockControl) => {
                let mut response = [0_u8; 20];
                encode_clock_control_response(&mut response);
                uart_send(&response);
            }
            // The serial-only diagnostic image binds no BK4819 bus and no
            // keypad, so it answers neither request.
            Some(Request::KeypadMatrix | Request::RfProbe) | None => {}
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

#[cfg(any())]
fn backlight_init() {
    let plan = constant_on_plan();
    update_register(RCC_IOPENR, 0, plan.clock_enable);
    write_register(GPIOF_BSRR, plan.output_high);
    update_register(GPIOF_OTYPER, plan.output_type_clear, 0);
    update_register(GPIOF_OSPEEDR, plan.speed_clear, plan.speed_set);
    update_register(GPIOF_PUPDR, plan.pull_clear, plan.pull_set);
    update_register(GPIOF_MODER, plan.mode_clear, plan.mode_set);
}

#[cfg(any())]
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

#[cfg(any())]
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

#[cfg(any())]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayError {
    Timeout,
}

#[cfg(any())]
struct K1DisplayBus;

#[cfg(any())]
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

#[cfg(any())]
struct K1MatrixBus {
    raw_idr: [u16; 4],
    read_index: usize,
}

#[cfg(any())]
impl K1MatrixBus {
    const fn new() -> Self {
        Self {
            raw_idr: [0; 4],
            read_index: 0,
        }
    }
}

#[cfg(any())]
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
        let idr = read_register(GPIOB_IDR);
        let bytes = idr.to_le_bytes();
        self.raw_idr[self.read_index] = u16::from_le_bytes([bytes[0], bytes[1]]);
        self.read_index += 1;
        Ok(active_rows_from_gpio_idr(idr))
    }

    fn read_ptt_pressed(&mut self) -> Result<bool, Self::Error> {
        // PB10 is active low. This is an input observation only; this image
        // exposes no transmit path.
        Ok(read_register(GPIOB_IDR) & (1 << 10) == 0)
    }
}

#[cfg(any())]
fn delay_milliseconds(milliseconds: u32) {
    for _ in 0..milliseconds * 12_000 {
        core::hint::spin_loop();
    }
}

#[cfg(any())]
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
