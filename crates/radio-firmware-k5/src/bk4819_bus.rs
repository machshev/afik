//! K5 V1 three-wire adapter for the hardware-independent BK4819 driver.

use core::convert::Infallible;
use radio_bk4819::{RegisterAddress, RegisterBus};
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

/// Pin-level operations used by one complete register transaction.
pub trait ThreeWire {
    /// Selects or releases the chip; selected is active low on the board.
    fn select(&mut self, selected: bool);
    /// Drives the serial clock.
    fn clock(&mut self, high: bool);
    /// Drives one data bit.
    fn write_data(&mut self, high: bool);
    /// Changes data direction for register read-back.
    fn data_input(&mut self, input: bool);
    /// Samples one data bit.
    fn read_data(&mut self) -> bool;
    /// Provides the sourced one-microsecond edge spacing.
    fn delay(&mut self);
}

/// Register adapter over an owned three-wire pin implementation.
pub struct K5RegisterBus<P> {
    pins: P,
}

impl<P> K5RegisterBus<P> {
    /// Wraps initialized pins without touching the BK4819.
    pub const fn new(pins: P) -> Self {
        Self { pins }
    }
    /// Returns the pin implementation.
    pub fn into_inner(self) -> P {
        self.pins
    }
}

impl<P: ThreeWire> RegisterBus for K5RegisterBus<P> {
    type Error = Infallible;
    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
        begin(&mut self.pins);
        shift_out(&mut self.pins, u32::from(address.get()), 8);
        shift_out(&mut self.pins, u32::from(value), 16);
        end(&mut self.pins);
        Ok(())
    }
    fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error> {
        begin(&mut self.pins);
        shift_out(&mut self.pins, u32::from(address.get() | 0x80), 8);
        self.pins.data_input(true);
        self.pins.delay();
        let mut value = 0_u16;
        for _ in 0..16 {
            value = (value << 1) | u16::from(self.pins.read_data());
            self.pins.clock(true);
            self.pins.delay();
            self.pins.clock(false);
            self.pins.delay();
        }
        self.pins.data_input(false);
        end(&mut self.pins);
        Ok(value)
    }
}

fn begin<P: ThreeWire>(pins: &mut P) {
    pins.select(false);
    pins.clock(false);
    pins.delay();
    pins.select(true);
}
fn end<P: ThreeWire>(pins: &mut P) {
    pins.select(false);
    pins.delay();
    pins.clock(true);
    pins.write_data(true);
}
fn shift_out<P: ThreeWire>(pins: &mut P, value: u32, bits: u8) {
    pins.clock(false);
    for bit in (0..bits).rev() {
        pins.write_data(value & (1 << bit) != 0);
        pins.delay();
        pins.clock(true);
        pins.delay();
        pins.clock(false);
        pins.delay();
    }
}

/// Direct PC0/PC1/PC2 implementation for the V1 board.
pub struct K5Pins;
impl K5Pins {
    /// Configures select, clock, and bidirectional data as idle GPIO outputs.
    pub fn initialise() -> Self {
        for pin in 0..=2 {
            portcon::select_gpio(Port::C, pin);
            gpio::set_output(Port::C, pin);
        }
        gpio::write_pin(Port::C, 0, true);
        gpio::write_pin(Port::C, 1, true);
        gpio::write_pin(Port::C, 2, true);
        Self
    }
}
impl ThreeWire for K5Pins {
    fn select(&mut self, selected: bool) {
        gpio::write_pin(Port::C, 0, !selected);
    }
    fn clock(&mut self, high: bool) {
        gpio::write_pin(Port::C, 1, high);
    }
    fn write_data(&mut self, high: bool) {
        gpio::write_pin(Port::C, 2, high);
    }
    fn data_input(&mut self, input: bool) {
        if input {
            gpio::set_input(Port::C, 2);
            portcon::enable_input(Port::C, 2);
        } else {
            gpio::set_output(Port::C, 2);
        }
    }
    fn read_data(&mut self) -> bool {
        gpio::read_pin(Port::C, 2)
    }
    fn delay(&mut self) {
        for _ in 0..48 {
            core::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{K5RegisterBus, ThreeWire};
    use radio_bk4819::{RegisterAddress, RegisterBus};
    #[derive(Default)]
    struct Pins {
        selected: bool,
        bits: std::vec::Vec<bool>,
        input: bool,
        reads: u16,
    }
    impl ThreeWire for Pins {
        fn select(&mut self, selected: bool) {
            self.selected = selected;
        }
        fn clock(&mut self, _: bool) {}
        fn write_data(&mut self, high: bool) {
            if self.selected && !self.input {
                self.bits.push(high);
            }
        }
        fn data_input(&mut self, input: bool) {
            self.input = input;
        }
        fn read_data(&mut self) -> bool {
            let bit = self.reads & 0x8000 != 0;
            self.reads <<= 1;
            bit
        }
        fn delay(&mut self) {}
    }
    #[test]
    fn writes_address_then_value_msb_first() {
        let mut bus = K5RegisterBus::new(Pins::default());
        bus.write(RegisterAddress::new(0x33).unwrap(), 0x9000)
            .unwrap();
        let pins = bus.into_inner();
        let expected: std::vec::Vec<bool> = (0..24)
            .rev()
            .map(|bit| 0x33_9000 & (1 << bit) != 0)
            .collect();
        assert_eq!(pins.bits, expected);
    }
    #[test]
    fn read_sets_command_bit_and_returns_sixteen_bits() {
        let pins = Pins {
            reads: 0xa55a,
            ..Pins::default()
        };
        let mut bus = K5RegisterBus::new(pins);
        assert_eq!(bus.read(RegisterAddress::new(0x67).unwrap()), Ok(0xa55a));
        let pins = bus.into_inner();
        assert_eq!(pins.bits.len(), 8);
        assert!(pins.bits[0]);
    }
}
