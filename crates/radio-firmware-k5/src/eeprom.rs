//! Guarded access to the K5 V1's 8 KiB configuration EEPROM.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

/// Capacity established by the retained complete K5 backup.
pub const CAPACITY: usize = 8 * 1024;
/// Only write size evidenced by the pinned V1 firmware.
pub const WRITE_BLOCK_BYTES: usize = 8;
// The pinned firmware waits 8 ms. Polling remains bounded but permits at least
// that interval at the sourced one-microsecond bit timing.
const READY_POLLS: usize = 512;
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

/// Why an EEPROM operation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The requested range is empty or crosses the evidenced capacity.
    OutOfRange,
    /// The EEPROM did not acknowledge an address-phase byte.
    NotAcknowledged,
    /// A write address was not aligned to the evidenced eight-byte block.
    Unaligned,
    /// Current EEPROM bytes did not match the caller's expected old bytes.
    PreconditionMismatch,
    /// The device did not become ready within the bounded polling window.
    Busy,
    /// Mandatory read-back did not equal the requested replacement.
    VerificationFailed,
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

/// Replaces one aligned eight-byte block after compare and verifies read-back.
///
/// Supplying the expected current bytes makes stale configuration state fail
/// before programming. The fixed block size is the only write shape evidenced
/// on this board; no arbitrary or cross-block write API is exposed.
pub fn write_verified<B: Bus>(
    bus: &mut B,
    address: u16,
    expected: &[u8; WRITE_BLOCK_BYTES],
    replacement: &[u8; WRITE_BLOCK_BYTES],
) -> Result<(), Error> {
    if usize::from(address) % WRITE_BLOCK_BYTES != 0 {
        return Err(Error::Unaligned);
    }
    let mut current = [0_u8; WRITE_BLOCK_BYTES];
    read(bus, address, &mut current)?;
    if current != *expected {
        return Err(Error::PreconditionMismatch);
    }
    if current == *replacement {
        return Ok(());
    }

    bus.start();
    let address_bytes = address.to_be_bytes();
    if !bus.write(WRITE_ADDRESS) || !bus.write(address_bytes[0]) || !bus.write(address_bytes[1]) {
        bus.stop();
        return Err(Error::NotAcknowledged);
    }
    for byte in replacement {
        if !bus.write(*byte) {
            bus.stop();
            return Err(Error::NotAcknowledged);
        }
    }
    bus.stop();

    let mut ready = false;
    for _ in 0..READY_POLLS {
        bus.start();
        ready = bus.write(WRITE_ADDRESS);
        bus.stop();
        if ready {
            break;
        }
    }
    if !ready {
        return Err(Error::Busy);
    }

    let mut verified = [0_u8; WRITE_BLOCK_BYTES];
    read(bus, address, &mut verified)?;
    if verified != *replacement {
        return Err(Error::VerificationFailed);
    }
    Ok(())
}

/// Bit-banged PA10/PA11 adapter for the evidenced V1 board.
pub struct K5Bus;

impl K5Bus {
    /// Leaves both bus lines driven high in the board-proven push-pull mode.
    pub fn initialise() -> Self {
        for pin in [10, 11] {
            portcon::select_gpio(Port::A, pin);
            portcon::enable_input(Port::A, pin);
            portcon::disable_open_drain(Port::A, pin);
            high(pin);
        }
        Self
    }
}

impl Bus for K5Bus {
    fn start(&mut self) {
        high(11);
        delay();
        high(10);
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
        high(10);
        delay();
        high(11);
        delay();
    }
    fn write(&mut self, mut byte: u8) -> bool {
        low(10);
        for _ in 0..8 {
            if byte & 0x80 == 0 {
                low(11);
            } else {
                high(11);
            }
            delay();
            high(10);
            delay();
            low(10);
            byte <<= 1;
        }
        gpio::set_input(Port::A, 11);
        delay();
        high(10);
        delay();
        let acknowledged = !gpio::read_pin(Port::A, 11);
        low(10);
        high(11);
        acknowledged
    }
    fn read(&mut self, final_byte: bool) -> u8 {
        gpio::set_input(Port::A, 11);
        let mut value = 0;
        for _ in 0..8 {
            low(10);
            delay();
            high(10);
            delay();
            value = (value << 1) | u8::from(gpio::read_pin(Port::A, 11));
        }
        low(10);
        gpio::set_output(Port::A, 11);
        if final_byte {
            high(11);
        } else {
            low(11);
        }
        delay();
        high(10);
        delay();
        low(10);
        high(11);
        value
    }
}

fn low(pin: u8) {
    gpio::write_pin(Port::A, pin, false);
    gpio::set_output(Port::A, pin);
}
fn high(pin: u8) {
    gpio::write_pin(Port::A, pin, true);
    gpio::set_output(Port::A, pin);
}
fn delay() {
    for _ in 0..48 {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::{read, write_verified, Bus, Error, CAPACITY, READY_POLLS};

    #[derive(Default)]
    struct Fake {
        writes: std::vec::Vec<u8>,
        starts: usize,
        stops: usize,
        reads: std::vec::Vec<bool>,
        read_bytes: std::collections::VecDeque<u8>,
        acknowledgements: std::collections::VecDeque<bool>,
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
            self.acknowledgements.pop_front().unwrap_or(true)
        }
        fn read(&mut self, final_byte: bool) -> u8 {
            self.reads.push(final_byte);
            self.read_bytes
                .pop_front()
                .unwrap_or_else(|| u8::try_from(self.reads.len()).unwrap())
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

    #[test]
    fn verified_write_compares_programs_polls_and_reads_back() {
        let old = [1, 2, 3, 4, 5, 6, 7, 8];
        let new = [8, 7, 6, 5, 4, 3, 2, 1];
        let mut bus = Fake::default();
        bus.read_bytes.extend(old.into_iter().chain(new));
        assert_eq!(write_verified(&mut bus, 0x0120, &old, &new), Ok(()));
        assert!(bus
            .writes
            .windows(11)
            .any(|bytes| bytes == [0xa0, 0x01, 0x20, 8, 7, 6, 5, 4, 3, 2, 1]));
        assert_eq!(bus.starts, 6);
        assert_eq!(bus.stops, 4);
    }

    #[test]
    fn stale_precondition_and_unaligned_address_write_nothing() {
        let expected = [0; 8];
        let replacement = [1; 8];
        let mut bus = Fake::default();
        bus.read_bytes.extend([2; 8]);
        assert_eq!(
            write_verified(&mut bus, 0x0100, &expected, &replacement),
            Err(Error::PreconditionMismatch)
        );
        assert_eq!(bus.writes, [0xa0, 0x01, 0x00, 0xa1]);

        let mut bus = Fake::default();
        assert_eq!(
            write_verified(&mut bus, 0x0101, &expected, &replacement),
            Err(Error::Unaligned)
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn read_back_mismatch_is_never_reported_as_success() {
        let old = [0; 8];
        let replacement = [1; 8];
        let mut bus = Fake::default();
        bus.read_bytes.extend(old.into_iter().chain([2; 8]));
        assert_eq!(
            write_verified(&mut bus, 0x0100, &old, &replacement),
            Err(Error::VerificationFailed)
        );
    }

    #[test]
    fn ambiguous_program_acknowledgement_stops_without_retry() {
        let old = [0; 8];
        let replacement = [1; 8];
        let mut bus = Fake::default();
        bus.read_bytes.extend(old);
        bus.acknowledgements.extend([true; 7]);
        bus.acknowledgements.push_back(false);
        assert_eq!(
            write_verified(&mut bus, 0x0100, &old, &replacement),
            Err(Error::NotAcknowledged)
        );
        assert_eq!(bus.writes, [0xa0, 0x01, 0x00, 0xa1, 0xa0, 0x01, 0x00, 1]);
        assert_eq!(bus.stops, 2);
    }

    #[test]
    fn device_which_never_becomes_ready_times_out() {
        let old = [0; 8];
        let replacement = [1; 8];
        let mut bus = Fake::default();
        bus.read_bytes.extend(old);
        bus.acknowledgements.extend([true; 15]);
        bus.acknowledgements.extend([false; READY_POLLS]);
        assert_eq!(
            write_verified(&mut bus, 0x0100, &old, &replacement),
            Err(Error::Busy)
        );
    }
}
