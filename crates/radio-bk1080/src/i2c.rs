//! The BK1080's I2C register framing over a bit-level two-wire bus.
//!
//! As the reference driver frames it (`EVID-BCFM-066-001`): device byte
//! `0x80`, then the register shifted left with the read bit, then two data
//! bytes high first. A read sends no repeated start.

use crate::RegisterBus;

const DEVICE: u8 = 0x80;
const READ: u8 = 1;

/// Byte-level two-wire operations a target provides.
pub trait TwoWire {
    /// Emits a start condition.
    fn start(&mut self);
    /// Emits a stop condition.
    fn stop(&mut self);
    /// Writes one byte and reports whether the device acknowledged it.
    fn write(&mut self, byte: u8) -> bool;
    /// Reads one byte, acknowledging it unless `final_byte` is true.
    fn read(&mut self, final_byte: bool) -> u8;
    /// Takes the pins back before a transaction, where they are shared.
    fn claim(&mut self) {}
    /// Drives the chip's enable line, where the board has one.
    fn set_enabled(&mut self, _enabled: bool) {}
}

/// The BK1080 did not acknowledge a byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotAcknowledged;

/// [`RegisterBus`] over a [`TwoWire`] bus.
#[derive(Debug)]
pub struct I2cRegisters<W>(pub W);

impl<W: TwoWire> I2cRegisters<W> {
    fn address(&mut self, register: u8, read: bool) -> Result<(), NotAcknowledged> {
        self.0.claim();
        self.0.start();
        let acknowledged =
            self.0.write(DEVICE) && self.0.write((register << 1) | if read { READ } else { 0 });
        if acknowledged {
            Ok(())
        } else {
            self.0.stop();
            Err(NotAcknowledged)
        }
    }
}

impl<W: TwoWire> RegisterBus for I2cRegisters<W> {
    type Error = NotAcknowledged;

    // Kept out of line: inlined at every register access, the bit-banged
    // transaction would cost firmware hundreds of bytes it does not have.
    #[inline(never)]
    fn write(&mut self, register: u8, value: u16) -> Result<(), NotAcknowledged> {
        self.address(register, false)?;
        let [high, low] = value.to_be_bytes();
        let acknowledged = self.0.write(high) && self.0.write(low);
        self.0.stop();
        if acknowledged {
            Ok(())
        } else {
            Err(NotAcknowledged)
        }
    }

    #[inline(never)]
    fn read(&mut self, register: u8) -> Result<u16, NotAcknowledged> {
        self.address(register, true)?;
        let high = self.0.read(false);
        let low = self.0.read(true);
        self.0.stop();
        Ok(u16::from_be_bytes([high, low]))
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.0.set_enabled(enabled);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{I2cRegisters, NotAcknowledged, TwoWire};
    use crate::RegisterBus;
    use std::vec::Vec;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Line {
        Claim,
        Start,
        Stop,
        Write(u8),
        Read(bool),
        Enable(bool),
    }

    #[derive(Default)]
    struct Wire {
        lines: Vec<Line>,
        reads: Vec<u8>,
        refuse: Option<u8>,
    }

    impl TwoWire for Wire {
        fn start(&mut self) {
            self.lines.push(Line::Start);
        }
        fn stop(&mut self) {
            self.lines.push(Line::Stop);
        }
        fn write(&mut self, byte: u8) -> bool {
            self.lines.push(Line::Write(byte));
            self.refuse != Some(byte)
        }
        fn read(&mut self, final_byte: bool) -> u8 {
            self.lines.push(Line::Read(final_byte));
            self.reads.remove(0)
        }
        fn claim(&mut self) {
            self.lines.push(Line::Claim);
        }
        fn set_enabled(&mut self, enabled: bool) {
            self.lines.push(Line::Enable(enabled));
        }
    }

    #[test]
    fn a_write_sends_device_register_and_the_high_byte_first() {
        let mut bus = I2cRegisters(Wire::default());
        bus.write(0x19, 0xA83C).unwrap();
        assert_eq!(
            bus.0.lines,
            [
                Line::Claim,
                Line::Start,
                Line::Write(0x80),
                Line::Write(0x32),
                Line::Write(0xA8),
                Line::Write(0x3C),
                Line::Stop,
            ]
        );
    }

    #[test]
    fn a_read_sets_the_read_bit_and_naks_the_last_byte() {
        let mut bus = I2cRegisters(Wire {
            reads: std::vec![0x12, 0x34],
            ..Wire::default()
        });
        assert_eq!(bus.read(0x0A), Ok(0x1234));
        assert_eq!(
            bus.0.lines,
            [
                Line::Claim,
                Line::Start,
                Line::Write(0x80),
                Line::Write(0x15),
                Line::Read(false),
                Line::Read(true),
                Line::Stop,
            ]
        );
        bus.set_enabled(true);
        assert_eq!(bus.0.lines.last(), Some(&Line::Enable(true)));
    }

    #[test]
    fn a_missing_acknowledge_stops_the_bus_and_fails() {
        let mut bus = I2cRegisters(Wire {
            refuse: Some(0x80),
            ..Wire::default()
        });
        assert_eq!(bus.write(0x02, 0x0201), Err(NotAcknowledged));
        assert_eq!(bus.read(0x02), Err(NotAcknowledged));
        assert_eq!(bus.0.lines.last(), Some(&Line::Stop));
        assert!(!bus.0.lines.iter().any(|line| matches!(line, Line::Read(_))));
    }
}
