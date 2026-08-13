//! The image's serial exchange, driven by the host implementation that will
//! meet it on the wire.
//!
//! Both sides of the K1 evidence were once agreed with themselves — the host
//! encoded a trailer it then expected back, and `EVID-K5-013` records what that
//! cost. So this drives the real `radio-flasher` probe against the real image
//! framing rather than testing either against a fixture written beside it.

use radio_firmware_k5::protocol::{
    apply_shared_serial_request, apply_shared_wire_request, decode_shared_request,
    encode_shared_response, serial_capabilities, HelloService, SerialCapability, SerialCommand,
    SerialRequest, SerialResult, APPLICATION_IDENTITY, APPLICATION_VERSION, RESPONSE_FRAME_BYTES,
};
use radio_flasher::probe_normal_firmware;
use radio_platform::receive_app::{Event, Key, ReceiveApp};
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

#[test]
fn k5_advertises_only_the_shared_receive_serial_boundary() {
    let capabilities = serial_capabilities();
    assert!(capabilities.contains(SerialCapability::SharedReceive));
    assert!(capabilities.contains(SerialCapability::ViewQuery));
    assert!(capabilities.contains(SerialCapability::KeyCommands));
    assert!(!capabilities.contains(SerialCapability::PersistentConfiguration));
    assert!(!capabilities.contains(SerialCapability::ObjectConfiguration));
    assert!(!capabilities.contains(SerialCapability::ArbitraryChannels));
    assert!(!capabilities.contains(SerialCapability::Vfo));
    assert!(!capabilities.contains(SerialCapability::Scanning));
}

#[test]
fn k5_common_serial_key_enters_the_same_shared_app_as_keypad_input() {
    let mut serial = ReceiveApp::new();
    let mut keypad = ReceiveApp::new();
    let serial_result = apply_shared_serial_request(
        &mut serial,
        SerialRequest::Command(SerialCommand::Key(Key::Up)),
    );
    assert_eq!(
        serial_result,
        SerialResult::Effects(keypad.apply(Event::KeyPress(Key::Up)))
    );
    assert_eq!(serial.view(), keypad.view());
}

#[test]
fn k5_common_wire_key_is_decoded_and_returns_the_shared_view() {
    let request = decode_shared_request(&[0x03, 0x04]).expect("common key");
    let mut app = ReceiveApp::new();
    let result = apply_shared_wire_request(&mut app, request);
    assert!(matches!(result, SerialResult::Effects(_)));
    let mut response = [0_u8; 4];
    let length =
        radio_platform::serial_control::wire::encode_key_response(app.view(), &mut response)
            .expect("key response");
    assert_eq!(&response[..length], &[0x83, 2, 1, 2]);

    let mut query_response = [0_u8; 4];
    let length = encode_shared_response(SerialResult::View(app.view()), &mut query_response)
        .expect("view response");
    assert_eq!(&query_response[..length], &[0x81, 2, 1, 2]);
}

#[test]
fn k5_common_wire_capability_query_is_target_declared() {
    let request = decode_shared_request(&[0x02, 0x00]).expect("capability query");
    let mut app = ReceiveApp::new();
    let result = apply_shared_wire_request(&mut app, request);
    assert_eq!(result, SerialResult::Capabilities(serial_capabilities()));
    let mut response = [0_u8; 4];
    let length = encode_shared_response(result, &mut response).expect("capability response");
    assert_eq!(&response[..length], &[0x82, 0x07, 0x00]);
}
