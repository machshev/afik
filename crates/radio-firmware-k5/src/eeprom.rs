//! Read-only access to the K5 V1's 8 KiB configuration EEPROM.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

/// Capacity established by the retained complete K5 backup.
pub const CAPACITY: usize = 8 * 1024;
const WRITE_ADDRESS: u8 = 0xA0;
const READ_ADDRESS: u8 = 0xA1;

/// Minimal two-wire operations used by a random read.
pub trait Bus {
    /// Emits a start or repeated-start condition.
    fn start(&mut self);
    /// Emits a stop condition.
    fn stop(&mut self);
    /// Writes one byte and reports whether the device acknowledged it.
    fn write(&mut self, byte: u8) -> bool;
    /// Reads one byte, acknowledging it unless `final_byte` is true.
    fn read(&mut self, final_byte: bool) -> u8;
}

/// Why a read-only EEPROM operation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The requested range is empty or crosses the evidenced capacity.
    OutOfRange,
    /// The EEPROM did not acknowledge an address-phase byte.
    NotAcknowledged,
}

/// Reads exactly `output.len()` bytes without exposing any write operation.
pub fn read<B: Bus>(bus: &mut B, address: u16, output: &mut [u8]) -> Result<(), Error> {
    let start = usize::from(address);
    if output.is_empty()
        || start
            .checked_add(output.len())
            .is_none_or(|end| end > CAPACITY)
    {
        return Err(Error::OutOfRange);
    }
    bus.start();
    let address_bytes = address.to_be_bytes();
    if !bus.write(WRITE_ADDRESS) || !bus.write(address_bytes[0]) || !bus.write(address_bytes[1]) {
        bus.stop();
        return Err(Error::NotAcknowledged);
    }
    bus.start();
    if !bus.write(READ_ADDRESS) {
        bus.stop();
        return Err(Error::NotAcknowledged);
    }
    let last = output.len() - 1;
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = bus.read(index == last);
    }
    bus.stop();
    Ok(())
}

/// Bit-banged PA10/PA11 adapter for the evidenced V1 board.
pub struct K5Bus;

impl K5Bus {
    /// Leaves both bus lines released high.
    pub fn initialise() -> Self {
        for pin in [10, 11] {
            portcon::select_gpio(Port::A, pin);
            portcon::enable_input(Port::A, pin);
            portcon::enable_pull_up(Port::A, pin);
            release(pin);
        }
        Self
    }
}

impl Bus for K5Bus {
    fn start(&mut self) {
        release(11);
        delay();
        release(10);
        delay();
        low(11);
        delay();
        low(10);
        delay();
    }
    fn stop(&mut self) {
        low(11);
        delay();
        low(10);
        delay();
        release(10);
        delay();
        release(11);
        delay();
    }
    fn write(&mut self, mut byte: u8) -> bool {
        low(10);
        for _ in 0..8 {
            if byte & 0x80 == 0 {
                low(11);
            } else {
                release(11);
            }
            delay();
            release(10);
            delay();
            low(10);
            byte <<= 1;
        }
        release(11);
        delay();
        release(10);
        delay();
        let acknowledged = !gpio::read_pin(Port::A, 11);
        low(10);
        acknowledged
    }
    fn read(&mut self, final_byte: bool) -> u8 {
        release(11);
        let mut value = 0;
        for _ in 0..8 {
            low(10);
            delay();
            release(10);
            delay();
            value = (value << 1) | u8::from(gpio::read_pin(Port::A, 11));
        }
        low(10);
        if final_byte {
            release(11);
        } else {
            low(11);
        }
        delay();
        release(10);
        delay();
        low(10);
        release(11);
        value
    }
}

fn low(pin: u8) {
    gpio::write_pin(Port::A, pin, false);
    gpio::set_output(Port::A, pin);
}
fn release(pin: u8) {
    gpio::set_input(Port::A, pin);
}
fn delay() {
    for _ in 0..48 {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::{read, Bus, Error, CAPACITY};
    #[derive(Default)]
    struct Fake {
        writes: std::vec::Vec<u8>,
        starts: usize,
        stops: usize,
        reads: std::vec::Vec<bool>,
    }
    impl Bus for Fake {
        fn start(&mut self) {
            self.starts += 1;
        }
        fn stop(&mut self) {
            self.stops += 1;
        }
        fn write(&mut self, byte: u8) -> bool {
            self.writes.push(byte);
            true
        }
        fn read(&mut self, final_byte: bool) -> u8 {
            self.reads.push(final_byte);
            u8::try_from(self.reads.len()).unwrap()
        }
    }
    #[test]
    fn exact_random_read_sequence_and_final_nack() {
        let mut bus = Fake::default();
        let mut bytes = [0; 3];
        assert_eq!(read(&mut bus, 0x1234, &mut bytes), Ok(()));
        assert_eq!(bus.writes, [0xa0, 0x12, 0x34, 0xa1]);
        assert_eq!((bus.starts, bus.stops), (2, 1));
        assert_eq!(bus.reads, [false, false, true]);
        assert_eq!(bytes, [1, 2, 3]);
    }
    #[test]
    fn refuses_empty_and_cross_capacity_reads_without_bus_activity() {
        let mut bus = Fake::default();
        assert_eq!(read(&mut bus, 0, &mut []), Err(Error::OutOfRange));
        assert_eq!(
            read(&mut bus, u16::try_from(CAPACITY - 1).unwrap(), &mut [0; 2]),
            Err(Error::OutOfRange)
        );
        assert_eq!((bus.starts, bus.stops), (0, 0));
    }
}
