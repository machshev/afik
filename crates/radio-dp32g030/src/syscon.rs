//! Peripheral clock gating, per `EVID-DP32-006`.
//!
//! `DEV_CLK_GATE` resets to zero, so every peripheral an image uses has to be
//! named. Only the peripherals AFIK has evidence for are listed: a gate bit
//! this crate cannot name is a peripheral no AFIK image can accidentally start.

use crate::mmio::Register;
use crate::SYSCON_BASE;

/// `SYSCON_DEV_CLK_GATE`, the peripheral clock-gate register.
const DEV_CLK_GATE: Register = Register::new(SYSCON_BASE, 0x08);

/// One peripheral whose clock can be gated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Peripheral {
    /// General-purpose IO port A.
    GpioA,
    /// General-purpose IO port B.
    GpioB,
    /// General-purpose IO port C.
    GpioC,
    /// Serial port 0.
    Uart0,
    /// Serial port 1, which is the V1 programming port per `EVID-K5-019`.
    Uart1,
    /// Serial port 2.
    Uart2,
    /// SPI controller 0, which drives the V1 display per `EVID-K5-022`.
    Spi0,
}

impl Peripheral {
    /// Returns this peripheral's `DEV_CLK_GATE` bit.
    pub const fn gate_bit(self) -> u32 {
        match self {
            Self::GpioA => 1 << 0,
            Self::GpioB => 1 << 1,
            Self::GpioC => 1 << 2,
            Self::Uart0 => 1 << 6,
            Self::Uart1 => 1 << 7,
            Self::Uart2 => 1 << 8,
            Self::Spi0 => 1 << 10,
        }
    }
}

/// Enables the clock of every named peripheral, leaving the others as found.
pub fn enable(peripherals: &[Peripheral]) {
    let mut bits = 0;
    for peripheral in peripherals {
        bits |= peripheral.gate_bit();
    }
    DEV_CLK_GATE.modify(|value| value | bits);
}

#[cfg(test)]
mod tests {
    use super::Peripheral;

    #[test]
    fn gate_bits_match_the_recorded_register_layout() {
        assert_eq!(Peripheral::GpioA.gate_bit(), 0x0000_0001);
        assert_eq!(Peripheral::GpioB.gate_bit(), 0x0000_0002);
        assert_eq!(Peripheral::GpioC.gate_bit(), 0x0000_0004);
        assert_eq!(Peripheral::Uart0.gate_bit(), 0x0000_0040);
        assert_eq!(Peripheral::Uart1.gate_bit(), 0x0000_0080);
        assert_eq!(Peripheral::Uart2.gate_bit(), 0x0000_0100);
        assert_eq!(Peripheral::Spi0.gate_bit(), 0x0000_0400);
    }

    #[test]
    fn every_named_peripheral_has_a_distinct_bit() {
        let peripherals = [
            Peripheral::GpioA,
            Peripheral::GpioB,
            Peripheral::GpioC,
            Peripheral::Uart0,
            Peripheral::Uart1,
            Peripheral::Uart2,
            Peripheral::Spi0,
        ];
        let mut seen = 0_u32;
        for peripheral in peripherals {
            assert_eq!(seen & peripheral.gate_bit(), 0);
            seen |= peripheral.gate_bit();
        }
    }
}
