//! The image's serial exchange, driven by the host implementation that will
//! meet it on the wire.
//!
//! Both sides of the K1 evidence were once agreed with themselves — the host
//! encoded a trailer it then expected back, and `EVID-K5-013` records what that
//! cost. So this drives the real `radio-flasher` probe against the real image
//! framing rather than testing either against a fixture written beside it.

use radio_firmware_k5::protocol::{
    HelloService, APPLICATION_IDENTITY, APPLICATION_VERSION, RESPONSE_FRAME_BYTES,
};
use radio_flasher::probe_normal_firmware;
use std::io::{Read, Write};

/// A radio which runs the image's own protocol code over an in-memory wire.
struct SimulatedRadio {
    service: HelloService<'static>,
    outbound: Vec<u8>,
    read_position: usize,
    answered: usize,
}

impl Default for SimulatedRadio {
    fn default() -> Self {
        Self {
            service: HelloService::new(APPLICATION_IDENTITY),
            outbound: Vec::new(),
            read_position: 0,
            answered: 0,
        }
    }
}

impl Write for SimulatedRadio {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        for byte in buffer {
            let mut response = [0_u8; RESPONSE_FRAME_BYTES];
            if let Some(length) = self.service.push(*byte, &mut response) {
                self.outbound.extend_from_slice(&response[..length]);
                self.answered += 1;
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
fn the_host_probe_reads_the_images_identity() {
    let mut radio = SimulatedRadio::default();
    let info = probe_normal_firmware(&mut radio).expect("the image answers the host's hello");
    assert_eq!(
        info.version(),
        std::str::from_utf8(APPLICATION_VERSION).expect("printable identity")
    );
    assert_eq!(radio.answered, 1);
}

#[test]
fn line_noise_before_a_request_does_not_stop_the_image_answering() {
    let mut radio = SimulatedRadio::default();
    radio
        .write_all(&[0x00, 0xAB, 0xAB, 0xFF, 0xDC, 0xBA, 0xAB])
        .expect("noise is accepted");
    let info = probe_normal_firmware(&mut radio).expect("the image resynchronises");
    assert_eq!(
        info.version(),
        std::str::from_utf8(APPLICATION_VERSION).expect("printable identity")
    );
}

#[test]
fn a_second_request_is_answered_as_well_as_the_first() {
    let mut radio = SimulatedRadio::default();
    probe_normal_firmware(&mut radio).expect("first exchange");
    probe_normal_firmware(&mut radio).expect("second exchange");
    assert_eq!(radio.answered, 2);
}

#[test]
fn a_longer_frame_is_read_past_rather_than_partly_answered() {
    let mut radio = SimulatedRadio::default();
    let mut frame = vec![0xAB, 0xCD, 0x40, 0x00];
    frame.extend(std::iter::repeat_n(0x00, 0x40 + 2));
    frame.extend_from_slice(&[0xDC, 0xBA]);
    radio.write_all(&frame).expect("the longer frame is read");
    assert_eq!(radio.answered, 0);
    probe_normal_firmware(&mut radio).expect("the next request still lands");
    assert_eq!(radio.answered, 1);
}
