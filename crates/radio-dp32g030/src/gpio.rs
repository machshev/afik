//! General-purpose IO, per `EVID-DP32-008`.
//!
//! Two registers per port: one direction, one data. Nothing here knows what a
//! pin is wired to; a board binding belongs in the image, beside its evidence.

use crate::mmio::Register;
use crate::{GPIOA_BASE, GPIOB_BASE, GPIOC_BASE};

/// One general-purpose IO port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Port {
    /// Port A.
    A,
    /// Port B.
    B,
    /// Port C.
    C,
}

impl Port {
    /// Returns the port's base address.
    pub const fn base(self) -> u32 {
        match self {
            Self::A => GPIOA_BASE,
            Self::B => GPIOB_BASE,
            Self::C => GPIOC_BASE,
        }
    }

    /// Returns the port's `GPIODATA` register.
    const fn data(self) -> Register {
        Register::new(self.base(), 0x00)
    }

    /// Returns the port's `GPIODIR` register.
    const fn direction(self) -> Register {
        Register::new(self.base(), 0x04)
    }
}

/// Drives one pin as an output.
pub fn set_output(port: Port, pin: u8) {
    port.direction()
        .modify(|value| value | (1 << u32::from(pin)));
}

/// Leaves one pin as an input.
pub fn set_input(port: Port, pin: u8) {
    port.direction()
        .modify(|value| value & !(1 << u32::from(pin)));
}

/// Sets or clears one output pin.
pub fn write_pin(port: Port, pin: u8, high: bool) {
    port.data().modify(|value| {
        if high {
            value | (1 << u32::from(pin))
        } else {
            value & !(1 << u32::from(pin))
        }
    });
}

/// Reads one pin.
pub fn read_pin(port: Port, pin: u8) -> bool {
    port.data().read() & (1 << u32::from(pin)) != 0
}

#[cfg(test)]
mod tests {
    use super::Port;

    #[test]
    fn port_registers_match_the_recorded_addresses() {
        assert_eq!(Port::A.data().address(), 0x4006_0000);
        assert_eq!(Port::A.direction().address(), 0x4006_0004);
        assert_eq!(Port::B.data().address(), 0x4006_0800);
        assert_eq!(Port::C.data().address(), 0x4006_1000);
    }
}
