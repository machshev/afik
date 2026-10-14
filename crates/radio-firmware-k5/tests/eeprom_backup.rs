//! The image's EEPROM read service, driven by the host backup that will meet it
//! on the wire.
//!
//! As in `hello.rs`, the real `radio-flasher` workflow runs against the image's
//! own request decoder and response encoder, and the read goes through the
//! image's own EEPROM driver into a simulated two-wire EEPROM.

use radio_firmware_k5::eeprom::{Bus, CAPACITY};
use radio_firmware_k5::protocol::{
    encode_hello_response, serve_eeprom_read, NormalRequest, NormalService, APPLICATION_VERSION,
    EEPROM_RESPONSE_FRAME_BYTES, RESPONSE_FRAME_BYTES,
};
use radio_flasher::backup_eeprom;
use std::io::{Read, Write};

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

/// A 24C64-style EEPROM answering the driver's random reads.
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

/// A radio running the image's serial service over an in-memory wire.
struct SimulatedRadio {
    service: NormalService,
    eeprom: SimulatedEeprom,
    outbound: Vec<u8>,
    read_position: usize,
}

impl Write for SimulatedRadio {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        for byte in buffer {
            match self.service.push(*byte) {
                Some(NormalRequest::Hello) => {
                    let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
                    encode_hello_response(&mut frame);
                    self.outbound.extend_from_slice(&frame);
                }
                Some(NormalRequest::ReadEeprom { offset, length }) => {
                    let mut frame = [0_u8; EEPROM_RESPONSE_FRAME_BYTES];
                    if let Some(size) =
                        serve_eeprom_read(&mut self.eeprom, offset, length, &mut frame)
                    {
                        self.outbound.extend_from_slice(&frame[..size]);
                    }
                }
                Some(NormalRequest::Common(_)) | None => {}
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

#[test]
fn the_host_backup_reads_the_whole_eeprom_through_the_image() {
    let mut radio = SimulatedRadio {
        service: NormalService::new(),
        eeprom: SimulatedEeprom::new(),
        outbound: Vec::new(),
        read_position: 0,
    };
    let (info, backup) = backup_eeprom(&mut radio).expect("the image serves a complete backup");
    assert_eq!(info.version().as_bytes(), APPLICATION_VERSION);
    assert_eq!(backup.bytes(), radio.eeprom.memory.as_slice());
    // One bus transaction per 128-byte block, each closed with a stop.
    assert_eq!(radio.eeprom.transactions, CAPACITY / 0x80);
    assert_eq!(radio.eeprom.phase, Phase::Idle);
}

#[test]
fn a_read_past_the_eeprom_is_not_answered() {
    let mut eeprom = SimulatedEeprom::new();
    let mut frame = [0_u8; EEPROM_RESPONSE_FRAME_BYTES];
    let last = u16::try_from(CAPACITY - 0x40).unwrap();
    assert!(serve_eeprom_read(&mut eeprom, last, 0x80, &mut frame).is_none());
    assert_eq!(eeprom.transactions, 0, "the bus is never touched");
    assert!(serve_eeprom_read(&mut eeprom, last, 0x40, &mut frame).is_some());
}
