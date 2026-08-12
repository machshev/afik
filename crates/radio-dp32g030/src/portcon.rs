//! Pin function selection and input enables, per `EVID-DP32-007`.
//!
//! A pin's function is a four-bit field, and which value names which function
//! differs per pin. Rather than expose a number, this module exposes the
//! bindings AFIK has evidence for: the two UART1 pins of `EVID-K5-019`, and the
//! GPIO function, which is field value zero on every pin the manual lists.

use crate::gpio::Port;
use crate::mmio::Register;
use crate::PORTCON_BASE;

/// Width of one pin's function field.
const FUNCTION_BITS: u32 = 4;
/// Mask of one pin's function field before it is shifted into place.
const FUNCTION_MASK: u32 = 0xF;
/// Number of pins described by a `SEL0` register.
const PINS_PER_SELECT_REGISTER: u8 = 8;

/// Function field value selecting the digital GPIO function.
const FUNCTION_GPIO: u32 = 0;
/// Function field value selecting `UART1_TX` on PA7.
const FUNCTION_PA7_UART1_TX: u32 = 1;
/// Function field value selecting `UART1_RX` on PA8.
const FUNCTION_PA8_UART1_RX: u32 = 1;
/// Function field value selecting the K5 display's SPI0 pins on port B.
const FUNCTION_PORT_B_SPI0: u32 = 1;

/// Returns the function-select register holding `pin` of `port`.
const fn select_register(port: Port, pin: u8) -> Register {
    let offset = match (port, pin < PINS_PER_SELECT_REGISTER) {
        (Port::A, true) => 0x00,
        (Port::A, false) => 0x04,
        (Port::B, true) => 0x08,
        (Port::B, false) => 0x0C,
        (Port::C, _) => 0x10,
    };
    Register::new(PORTCON_BASE, offset)
}

/// Returns the input-enable register of `port`.
const fn input_enable_register(port: Port) -> Register {
    let offset = match port {
        Port::A => 0x100,
        Port::B => 0x104,
        Port::C => 0x108,
    };
    Register::new(PORTCON_BASE, offset)
}

/// Replaces one pin's function field, leaving every other field as found.
pub const fn with_function(register_value: u32, pin: u8, function: u32) -> u32 {
    let shift = u32::wrapping_mul((pin % PINS_PER_SELECT_REGISTER) as u32, FUNCTION_BITS);
    (register_value & !(FUNCTION_MASK << shift)) | ((function & FUNCTION_MASK) << shift)
}

/// Selects the digital GPIO function on one pin.
pub fn select_gpio(port: Port, pin: u8) {
    select_register(port, pin).modify(|value| with_function(value, pin, FUNCTION_GPIO));
}

/// Selects `UART1_TX` on PA7, which is the V1 programming port's transmit pin.
pub fn select_pa7_uart1_tx() {
    select_register(Port::A, 7).modify(|value| with_function(value, 7, FUNCTION_PA7_UART1_TX));
}

/// Selects `UART1_RX` on PA8, which is the V1 programming port's receive pin.
pub fn select_pa8_uart1_rx() {
    select_register(Port::A, 8).modify(|value| with_function(value, 8, FUNCTION_PA8_UART1_RX));
}

/// Selects SPI0 SSN on PB7, clock on PB8, and MOSI on PB10.
pub fn select_k5_display_spi0() {
    for pin in [7, 8, 10] {
        select_register(Port::B, pin)
            .modify(|value| with_function(value, pin, FUNCTION_PORT_B_SPI0));
    }
}

/// Selects `PWM_PLUS0` channel zero on the V1 backlight's PB6 pin.
pub fn select_k5_backlight_pwm() {
    select_register(Port::B, 6).modify(|value| with_function(value, 6, FUNCTION_PORT_B_SPI0));
}

/// Enables the input buffer of one pin, which a pin that is read requires.
pub fn enable_input(port: Port, pin: u8) {
    input_enable_register(port).modify(|value| value | (1 << u32::from(pin)));
}

/// Enables the internal pull-up of one pin.
pub fn enable_pull_up(port: Port, pin: u8) {
    pull_up_register(port).modify(|value| value | (1 << u32::from(pin)));
}

/// Returns the pull-up enable register of `port`.
const fn pull_up_register(port: Port) -> Register {
    let offset = match port {
        Port::A => 0x200,
        Port::B => 0x204,
        Port::C => 0x208,
    };
    Register::new(PORTCON_BASE, offset)
}

/// Selects push-pull output behavior for one pin.
pub fn disable_open_drain(port: Port, pin: u8) {
    open_drain_register(port).modify(|value| value & !(1 << u32::from(pin)));
}

/// Returns the open-drain selection register of `port`.
const fn open_drain_register(port: Port) -> Register {
    let offset = match port {
        Port::A => 0x400,
        Port::B => 0x404,
        Port::C => 0x408,
    };
    Register::new(PORTCON_BASE, offset)
}

#[cfg(test)]
mod tests {
    use super::{input_enable_register, open_drain_register, select_register, with_function};
    use crate::gpio::Port;

    #[test]
    fn a_function_field_replaces_only_its_own_pin() {
        assert_eq!(with_function(0xFFFF_FFFF, 7, 0), 0x0FFF_FFFF);
        assert_eq!(with_function(0x0000_0000, 7, 1), 0x1000_0000);
    }

    #[test]
    fn a_pin_above_the_first_eight_is_addressed_within_its_own_register() {
        assert_eq!(with_function(0, 8, 1), 0x0000_0001);
        assert_eq!(with_function(0, 15, 1), 0x1000_0000);
    }

    #[test]
    fn select_registers_match_the_recorded_offsets() {
        assert_eq!(select_register(Port::A, 7).address(), 0x400B_0000);
        assert_eq!(select_register(Port::A, 8).address(), 0x400B_0004);
        assert_eq!(select_register(Port::B, 0).address(), 0x400B_0008);
        assert_eq!(select_register(Port::B, 9).address(), 0x400B_000C);
        assert_eq!(select_register(Port::C, 1).address(), 0x400B_0010);
    }

    #[test]
    fn input_enable_registers_match_the_recorded_offsets() {
        assert_eq!(input_enable_register(Port::A).address(), 0x400B_0100);
        assert_eq!(input_enable_register(Port::B).address(), 0x400B_0104);
        assert_eq!(input_enable_register(Port::C).address(), 0x400B_0108);
    }

    #[test]
    fn open_drain_registers_match_the_recorded_offsets() {
        assert_eq!(open_drain_register(Port::A).address(), 0x400B_0400);
        assert_eq!(open_drain_register(Port::B).address(), 0x400B_0404);
        assert_eq!(open_drain_register(Port::C).address(), 0x400B_0408);
    }
}
