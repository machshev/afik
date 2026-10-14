//! The K5's memory view on the wire: the host backup reads it and host writes
//! change it, through the shared protocol and the K5's own EEPROM driver into
//! a simulated two-wire EEPROM.

use std::future::Future;
use std::io::{Read, Write};
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use radio_app::drivers::Memory;
use radio_firmware_k5::eeprom::{Bus, CAPACITY};
use radio_firmware_k5::memory::K5Memory;
use radio_flasher::backup_eeprom;
use radio_platform::memory_link::host::{self, REQUEST_FRAME_BYTES};
use radio_platform::memory_link::{
    encode_hello_response, encode_read_response, encode_write_response, host_may_write, Decoder,
    Push, Request, RESPONSE_FRAME_BYTES,
};
use radio_platform::serial::{xor, ApplicationIdentity};

const IDENTITY: &[u8] = b"AFIK-K5-7.0.15";

/// Polls a future that never waits, as the K5's EEPROM writes do not.
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("an EEPROM write waited"),
    }
}

/// What the simulated EEPROM expects next within one bus transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Idle,
    DeviceAddress,
    AddressHigh,
    AddressLow,
    Data,
    Reading,
}

/// A 24C64-style EEPROM answering the driver's reads and page writes.
struct SimulatedEeprom {
    memory: Vec<u8>,
    pointer: usize,
    phase: Phase,
    transactions: usize,
}

impl SimulatedEeprom {
    fn new() -> Self {
        Self {
            memory: (0_u8..=u8::MAX).cycle().take(CAPACITY).collect(),
            pointer: 0,
            phase: Phase::Idle,
            transactions: 0,
        }
    }
}

impl Bus for SimulatedEeprom {
    fn start(&mut self) {
        self.phase = Phase::DeviceAddress;
    }

    fn stop(&mut self) {
        self.phase = Phase::Idle;
        self.transactions += 1;
    }

    fn write(&mut self, byte: u8) -> bool {
        match (self.phase, byte) {
            (Phase::DeviceAddress, 0xA0) => self.phase = Phase::AddressHigh,
            (Phase::DeviceAddress, 0xA1) => self.phase = Phase::Reading,
            (Phase::AddressHigh, high) => {
                self.pointer = usize::from(high) << 8;
                self.phase = Phase::AddressLow;
            }
            (Phase::AddressLow, low) => {
                self.pointer |= usize::from(low);
                self.phase = Phase::Data;
            }
            (Phase::Data, data) => {
                self.memory[self.pointer % CAPACITY] = data;
                self.pointer += 1;
            }
            _ => return false,
        }
        true
    }

    fn read(&mut self, _final_byte: bool) -> u8 {
        assert_eq!(self.phase, Phase::Reading, "a read outside a read phase");
        let byte = self.memory[self.pointer % CAPACITY];
        self.pointer += 1;
        byte
    }
}

/// A radio answering the memory protocol over an in-memory wire, as
/// `radio_app::serial` does.
struct SimulatedRadio {
    decoder: Decoder,
    memory: K5Memory<SimulatedEeprom>,
    outbound: Vec<u8>,
    read_position: usize,
}

impl SimulatedRadio {
    fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            memory: K5Memory::new(SimulatedEeprom::new()),
            outbound: Vec::new(),
            read_position: 0,
        }
    }

    fn eeprom(&mut self) -> &mut SimulatedEeprom {
        self.memory.bus()
    }

    /// Takes everything the radio has sent since the last call.
    fn take_outbound(&mut self) -> Vec<u8> {
        let taken = self.outbound[self.read_position..].to_vec();
        self.read_position = self.outbound.len();
        taken
    }
}

impl Write for SimulatedRadio {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let identity = ApplicationIdentity::new(IDENTITY).expect("identity");
        for byte in buffer {
            let Push::Request(request) = self.decoder.push(*byte) else {
                continue;
            };
            let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
            let length = match request {
                Request::Hello => Some(encode_hello_response(identity, &mut frame)),
                Request::Read { address, length } => {
                    let memory = &mut self.memory;
                    encode_read_response(address, length, &mut frame, |data| {
                        memory.read(address, data)
                    })
                }
                Request::Write { address, data } => (host_may_write(address)
                    && ready(self.memory.write(address, &data)))
                .then(|| encode_write_response(address, &mut frame)),
            };
            if let Some(length) = length {
                self.outbound.extend_from_slice(&frame[..length]);
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for SimulatedRadio {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let available = &self.outbound[self.read_position..];
        if available.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the radio sent nothing",
            ));
        }
        let taken = available.len().min(buffer.len());
        buffer[..taken].copy_from_slice(&available[..taken]);
        self.read_position += taken;
        Ok(taken)
    }
}

/// Sends one write and returns the offset the radio acknowledged, if any.
fn write_block(radio: &mut SimulatedRadio, address: u16, data: [u8; 8]) -> Option<u16> {
    let mut frame = [0_u8; REQUEST_FRAME_BYTES];
    let length = host::write(address, data, &mut frame);
    radio.write_all(&frame[..length]).expect("wire");
    let mut answer = radio.take_outbound();
    if answer.is_empty() {
        return None;
    }
    assert_eq!(answer.len(), 14, "one acknowledgement frame");
    xor(&mut answer[4..12]);
    assert_eq!(&answer[4..6], &0x051E_u16.to_le_bytes());
    Some(u16::from_le_bytes([answer[8], answer[9]]))
}

#[test]
fn the_host_backup_reads_the_whole_eeprom_through_the_image() {
    let mut radio = SimulatedRadio::new();
    let (info, backup) = backup_eeprom(&mut radio).expect("the image serves a complete backup");
    assert_eq!(info.version().as_bytes(), IDENTITY);
    assert_eq!(backup.bytes(), radio.eeprom().memory.as_slice());
    // One bus transaction per 128-byte block, each closed with a stop.
    assert_eq!(radio.eeprom().transactions, CAPACITY / 0x80);
    assert_eq!(radio.eeprom().phase, Phase::Idle);
}

#[test]
fn a_host_write_reaches_the_eeprom_and_is_acknowledged() {
    let mut radio = SimulatedRadio::new();
    let data = [0xA5, 0x5A, 1, 2, 3, 4, 5, 6];
    assert_eq!(write_block(&mut radio, 0x0040, data), Some(0x0040));
    assert_eq!(&radio.eeprom().memory[0x40..0x48], &data);

    // Writing the same bytes again costs no programming cycle.
    let before = radio.eeprom().transactions;
    assert_eq!(write_block(&mut radio, 0x0040, data), Some(0x0040));
    assert_eq!(
        radio.eeprom().transactions - before,
        1,
        "only the comparing read"
    );
}

#[test]
fn calibration_and_unaligned_writes_are_refused_unanswered() {
    let mut radio = SimulatedRadio::new();
    let calibration = radio.eeprom().memory[0x1F40..0x1F48].to_vec();
    assert_eq!(write_block(&mut radio, 0x1F40, [0; 8]), None);
    assert_eq!(
        &radio.eeprom().memory[0x1F40..0x1F48],
        calibration.as_slice()
    );
    assert_eq!(write_block(&mut radio, 0x0004, [0; 8]), None);
    assert_eq!(radio.eeprom().transactions, 0, "the bus is never touched");
}

#[test]
fn a_read_past_the_eeprom_is_not_answered() {
    let mut radio = SimulatedRadio::new();
    let mut frame = [0_u8; REQUEST_FRAME_BYTES];
    let length = host::read(0x1FC0, 0x80, &mut frame);
    radio.write_all(&frame[..length]).expect("wire");
    assert!(radio.take_outbound().is_empty());
    assert_eq!(radio.eeprom().transactions, 0, "the bus is never touched");
}
