//! Hardware-independent K1 BK4819 three-wire register bus sequencing.
//!
//! The pin assignment and the exact transfer order come from the pinned K1
//! reference firmware recorded in `docs/hardware-evidence.md`: `CSN` is `PF9`,
//! `SCL` is `PB8`, and the bidirectional `SDA` is `PB9`. Bits are shifted most
//! significant first with one microsecond between every edge, and a read
//! address carries bit seven.
//!
//! This module performs the sequencing only. A board adapter supplies the pin
//! and delay primitives, so the complete transfer order is host testable.

use radio_bk4819::{RegisterAddress, RegisterBus};

/// Port F pin driving the active-low chip select.
pub const CSN_PIN: u8 = 9;
/// Port B pin driving the clock.
pub const SCL_PIN: u8 = 8;
/// Port B pin carrying the bidirectional data line.
pub const SDA_PIN: u8 = 9;
/// Settling delay applied between every bus edge.
pub const EDGE_DELAY_MICROSECONDS: u32 = 1;
/// Bit set in the address byte to request a read.
pub const READ_ADDRESS_FLAG: u8 = 0x80;

/// Direction of the shared data line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataDirection {
    /// The host drives the data line.
    Output,
    /// The chip drives the data line.
    Input,
}

/// Board primitives required to sequence one three-wire transfer.
pub trait ThreeWirePins {
    /// Adapter-specific failure. Any failure abandons the transfer.
    type Error;

    /// Drives the active-low chip select.
    fn set_chip_select(&mut self, asserted: bool) -> Result<(), Self::Error>;

    /// Drives the clock line.
    fn set_clock(&mut self, high: bool) -> Result<(), Self::Error>;

    /// Drives the data line while it is an output.
    fn set_data(&mut self, high: bool) -> Result<(), Self::Error>;

    /// Switches the shared data line direction.
    fn set_data_direction(&mut self, direction: DataDirection) -> Result<(), Self::Error>;

    /// Samples the data line while it is an input.
    fn read_data(&mut self) -> Result<bool, Self::Error>;

    /// Waits for at least the requested number of microseconds.
    fn delay_microseconds(&mut self, microseconds: u32) -> Result<(), Self::Error>;
}

/// A BK4819 register bus sequenced over the pinned K1 three-wire pins.
pub struct ThreeWireBus<P: ThreeWirePins> {
    pins: P,
}

impl<P: ThreeWirePins> ThreeWireBus<P> {
    /// Wraps board pins without performing any transfer.
    pub const fn new(pins: P) -> Self {
        Self { pins }
    }

    /// Returns an immutable reference to the pins for observation.
    pub const fn pins(&self) -> &P {
        &self.pins
    }

    fn settle(&mut self) -> Result<(), P::Error> {
        self.pins.delay_microseconds(EDGE_DELAY_MICROSECONDS)
    }

    fn begin(&mut self) -> Result<(), P::Error> {
        self.pins.set_chip_select(false)?;
        self.pins.set_clock(false)?;
        self.settle()?;
        self.pins.set_chip_select(true)
    }

    fn end(&mut self) -> Result<(), P::Error> {
        self.pins.set_chip_select(false)?;
        self.settle()?;
        self.pins.set_clock(true)?;
        self.pins.set_data(true)
    }

    fn write_bits(&mut self, value: u32, bits: u8) -> Result<(), P::Error> {
        self.pins.set_clock(false)?;
        for index in (0..bits).rev() {
            self.pins.set_data(value >> index & 1 == 1)?;
            self.settle()?;
            self.pins.set_clock(true)?;
            self.settle()?;
            self.pins.set_clock(false)?;
            self.settle()?;
        }
        Ok(())
    }

    fn read_word(&mut self) -> Result<u16, P::Error> {
        self.pins.set_data_direction(DataDirection::Input)?;
        self.settle()?;
        let mut value = 0_u16;
        for _ in 0..16 {
            value <<= 1;
            if self.pins.read_data()? {
                value |= 1;
            }
            self.pins.set_clock(true)?;
            self.settle()?;
            self.pins.set_clock(false)?;
            self.settle()?;
        }
        self.pins.set_data_direction(DataDirection::Output)?;
        Ok(value)
    }
}

impl<P: ThreeWirePins> RegisterBus for ThreeWireBus<P> {
    type Error = P::Error;

    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
        self.begin()?;
        self.write_bits(u32::from(address.get()), 8)?;
        self.settle()?;
        self.write_bits(u32::from(value), 16)?;
        self.settle()?;
        self.end()
    }

    fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error> {
        self.begin()?;
        self.write_bits(u32::from(address.get() | READ_ADDRESS_FLAG), 8)?;
        let value = self.read_word()?;
        self.end()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{DataDirection, ThreeWireBus, ThreeWirePins, READ_ADDRESS_FLAG};
    use radio_bk4819::{RegisterAddress, RegisterBus};
    use std::{vec, vec::Vec};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Event {
        ChipSelect(bool),
        Clock(bool),
        Data(bool),
        Direction(DataDirection),
        Sample(bool),
        Delay(u32),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct PinError;

    struct FakePins {
        events: Vec<Event>,
        input_bits: Vec<bool>,
        fail_after: Option<usize>,
        operations: usize,
    }

    impl FakePins {
        fn new(input: u16, fail_after: Option<usize>) -> Self {
            let mut input_bits = Vec::new();
            for index in (0..16).rev() {
                input_bits.push(input >> index & 1 == 1);
            }
            input_bits.reverse();
            Self {
                events: Vec::new(),
                input_bits,
                fail_after,
                operations: 0,
            }
        }

        fn record(&mut self, event: Event) -> Result<(), PinError> {
            self.operations += 1;
            if self.fail_after.is_some_and(|limit| self.operations > limit) {
                return Err(PinError);
            }
            self.events.push(event);
            Ok(())
        }

        fn clocked_data(&self) -> Vec<bool> {
            // The data value latched immediately before each rising clock edge.
            let mut bits = Vec::new();
            let mut last = false;
            let mut selected = false;
            for event in &self.events {
                match event {
                    Event::ChipSelect(asserted) => selected = *asserted,
                    Event::Data(high) => last = *high,
                    Event::Clock(true) if selected => bits.push(last),
                    _ => {}
                }
            }
            bits
        }
    }

    impl ThreeWirePins for FakePins {
        type Error = PinError;

        fn set_chip_select(&mut self, asserted: bool) -> Result<(), Self::Error> {
            self.record(Event::ChipSelect(asserted))
        }

        fn set_clock(&mut self, high: bool) -> Result<(), Self::Error> {
            self.record(Event::Clock(high))
        }

        fn set_data(&mut self, high: bool) -> Result<(), Self::Error> {
            self.record(Event::Data(high))
        }

        fn set_data_direction(&mut self, direction: DataDirection) -> Result<(), Self::Error> {
            self.record(Event::Direction(direction))
        }

        fn read_data(&mut self) -> Result<bool, Self::Error> {
            let bit = self.input_bits.pop().unwrap_or(false);
            self.record(Event::Sample(bit))?;
            Ok(bit)
        }

        fn delay_microseconds(&mut self, microseconds: u32) -> Result<(), Self::Error> {
            self.record(Event::Delay(microseconds))
        }
    }

    fn bits(value: u32, count: u8) -> Vec<bool> {
        (0..count)
            .rev()
            .map(|index| value >> index & 1 == 1)
            .collect()
    }

    #[test]
    fn a_write_shifts_the_address_then_the_value_most_significant_first() {
        let mut bus = ThreeWireBus::new(FakePins::new(0, None));
        bus.write(RegisterAddress::new(0x30).unwrap(), 0xBEF1)
            .unwrap();

        let mut expected = bits(0x30, 8);
        expected.extend(bits(0xBEF1, 16));
        assert_eq!(bus.pins().clocked_data(), expected);

        let events = &bus.pins().events;
        assert_eq!(events.first(), Some(&Event::ChipSelect(false)));
        assert_eq!(events.get(1), Some(&Event::Clock(false)));
        assert_eq!(events.get(3), Some(&Event::ChipSelect(true)));
        assert_eq!(
            &events[events.len() - 4..],
            &[
                Event::ChipSelect(false),
                Event::Delay(1),
                Event::Clock(true),
                Event::Data(true)
            ]
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::Direction(_))));
    }

    #[test]
    fn a_read_sets_the_address_flag_and_releases_the_shared_line() {
        let mut bus = ThreeWireBus::new(FakePins::new(0x1234, None));
        let value = bus.read(RegisterAddress::new(0x67).unwrap()).unwrap();
        assert_eq!(value, 0x1234);
        assert_eq!(
            bus.pins().clocked_data()[..8],
            bits(u32::from(0x67 | READ_ADDRESS_FLAG), 8)[..]
        );

        let directions = bus
            .pins()
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Direction(direction) => Some(*direction),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            directions,
            vec![DataDirection::Input, DataDirection::Output]
        );
    }

    #[test]
    fn every_edge_is_separated_by_the_sourced_settling_delay() {
        let mut bus = ThreeWireBus::new(FakePins::new(0, None));
        bus.write(RegisterAddress::new(0x00).unwrap(), 0).unwrap();
        assert!(bus
            .pins()
            .events
            .iter()
            .filter(|event| matches!(event, Event::Delay(_)))
            .all(|event| matches!(event, Event::Delay(1))));
        assert!(
            bus.pins()
                .events
                .iter()
                .filter(|event| matches!(event, Event::Delay(_)))
                .count()
                >= 24
        );
    }

    #[test]
    fn any_pin_failure_abandons_the_transfer() {
        for limit in 0..12 {
            let mut bus = ThreeWireBus::new(FakePins::new(0x00FF, Some(limit)));
            assert_eq!(
                bus.write(RegisterAddress::new(0x30).unwrap(), 0x0001),
                Err(PinError)
            );
            let mut bus = ThreeWireBus::new(FakePins::new(0x00FF, Some(limit)));
            assert_eq!(bus.read(RegisterAddress::new(0x30).unwrap()), Err(PinError));
        }
    }
}
