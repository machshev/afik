//! The image's serial exchange, driven by the host implementation that will
//! meet it on the wire.
//!
//! Both sides of the K1 evidence were once agreed with themselves — the host
//! encoded a trailer it then expected back, and `EVID-K5-013` records what that
//! cost. So this drives the real `radio-flasher` probe against the real image
//! framing rather than testing either against a fixture written beside it.

use radio_firmware_k5::protocol::{
    apply_shared_serial_request, apply_shared_wire_request, decode_shared_request,
    encode_common_view_response, encode_shared_response, serial_capabilities, HelloService,
    NormalRequest, NormalService, SerialCapability, SerialCommand, SerialRequest, SerialResult,
    APPLICATION_IDENTITY, APPLICATION_VERSION, RESPONSE_FRAME_BYTES,
};
use radio_flasher::probe_normal_firmware;
use radio_platform::operator_shell::{Capabilities as ShellCapabilities, Context, Intent, Shell};
use radio_platform::receive_app::{Event, Key, ReceiveApp};
use std::io::{Read, Write};

const NORMAL_XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

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
    let mut shell = Shell::new();
    shell.set_capabilities(ShellCapabilities::K5);
    shell.select_memory();
    let mut physical = ReceiveApp::new();
    let mut serial = ReceiveApp::new();
    let up_context = Context {
        visible_channels: 16,
        active_index: 0,
        scanning: false,
    };
    assert_eq!(shell.press(Key::Up, 0, up_context), Intent::SelectPrevious);
    let physical_result = physical.apply(Event::PreviousChannel);
    let serial_result = apply_shared_serial_request(
        &mut serial,
        SerialRequest::Command(SerialCommand::Key(Key::Up)),
    );
    assert_eq!(serial_result, SerialResult::Effects(physical_result));
    assert_eq!(serial.view(), physical.view());
    assert_eq!(serial.view().channel, 16);

    let down_context = Context {
        active_index: 15,
        ..up_context
    };
    assert_eq!(shell.press(Key::Down, 1, down_context), Intent::SelectNext);
    let physical_result = physical.apply(Event::NextChannel);
    let serial_result = apply_shared_serial_request(
        &mut serial,
        SerialRequest::Command(SerialCommand::Key(Key::Down)),
    );
    assert_eq!(serial_result, SerialResult::Effects(physical_result));
    assert_eq!(serial.view(), physical.view());
    assert_eq!(serial.view().channel, 1);

    assert_eq!(
        shell.press(Key::Side2, 2, down_context),
        Intent::ToggleMonitor
    );
    let physical_result = physical.apply(Event::ToggleMonitor);
    let serial_result = apply_shared_serial_request(
        &mut serial,
        SerialRequest::Command(SerialCommand::Key(Key::Side2)),
    );
    assert_eq!(serial_result, SerialResult::Effects(physical_result));
    assert_eq!(serial.view(), physical.view());
    assert!(serial.view().monitoring);
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
    assert_eq!(&response[..length], &[0x83, 16, 1, 2]);

    let mut query_response = [0_u8; 4];
    let length = encode_shared_response(SerialResult::View(app.view()), &mut query_response)
        .expect("view response");
    assert_eq!(&query_response[..length], &[0x81, 16, 1, 2]);
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

#[test]
fn malformed_common_payload_is_rejected_without_mutating_receive_state() {
    let app = ReceiveApp::new();
    let before = app.view();
    assert!(decode_shared_request(&[0x03]).is_err());
    assert!(decode_shared_request(&[0x03, 0xFF]).is_err());
    assert!(decode_shared_request(&[0x01, 0x01]).is_err());
    assert_eq!(app.view(), before);
}

fn normal_request(command: u16) -> [u8; 16] {
    let mut frame = [0_u8; 16];
    frame[..4].copy_from_slice(&[0xAB, 0xCD, 8, 0]);
    frame[4..6].copy_from_slice(&command.to_le_bytes());
    frame[6..8].copy_from_slice(&4_u16.to_le_bytes());
    frame[8..12].copy_from_slice(&0x6457_396A_u32.to_le_bytes());
    let mut crc = 0_u16;
    for byte in &frame[4..12] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    frame[12..14].copy_from_slice(&crc.to_le_bytes());
    for (index, byte) in frame[4..14].iter_mut().enumerate() {
        *byte ^= NORMAL_XOR_KEY[index];
    }
    frame[14..].copy_from_slice(&[0xDC, 0xBA]);
    frame
}

#[test]
fn k5_normal_envelope_dispatches_a_key_into_receive_app_and_encodes_response() {
    let mut service = NormalService::new();
    let mut request = None;
    for byte in normal_request(0x0606) {
        request = service.push(byte).or(request);
    }
    assert_eq!(
        request,
        Some(NormalRequest::Common(
            radio_platform::serial_control::wire::Request::Key(
                radio_platform::serial_control::wire::KeyCode::Up,
            )
        ))
    );
    let NormalRequest::Common(request) = request.expect("common request") else {
        panic!("expected common request");
    };
    let mut app = ReceiveApp::new();
    let result = apply_shared_wire_request(&mut app, request);
    assert!(matches!(result, SerialResult::Effects(_)));
    let mut response = [0_u8; RESPONSE_FRAME_BYTES];
    assert_eq!(
        encode_common_view_response(app.view(), request, &mut response),
        48
    );
    // The target-owned response is an exact fixed frame; the unit protocol
    // tests assert its de-obfuscated command/payload fields.
    assert_eq!(&response[..4], &[0xAB, 0xCD, 40, 0]);
    assert_eq!(&response[46..], &[0xDC, 0xBA]);
    // The fixed response envelope retains the legacy inner-length
    // convention: this is the four-byte common response data area, not the
    // command and metadata size.
    let mut decoded = response;
    for (index, byte) in decoded[4..46].iter_mut().enumerate() {
        *byte ^= NORMAL_XOR_KEY[index % NORMAL_XOR_KEY.len()];
    }
    assert_eq!(&decoded[6..8], &[4, 0]);
}

#[test]
fn k5_toggle_audio_dispatches_and_emits_the_bounded_view_response() {
    let mut service = NormalService::new();
    let mut request = None;
    for byte in normal_request(0x0615) {
        request = service.push(byte).or(request);
    }
    assert_eq!(
        request,
        Some(NormalRequest::Common(
            radio_platform::serial_control::wire::Request::ToggleAudio,
        ))
    );
    let NormalRequest::Common(request) = request.expect("common request") else {
        panic!("expected common request");
    };
    let mut app = ReceiveApp::new();
    let result = apply_shared_wire_request(&mut app, request);
    assert!(matches!(result, SerialResult::Effects(_)));
    assert!(!app.view().audio);
    let mut response = [0_u8; RESPONSE_FRAME_BYTES];
    assert_eq!(
        encode_common_view_response(app.view(), request, &mut response),
        48
    );
    assert_eq!(&response[..4], &[0xAB, 0xCD, 40, 0]);
    assert_eq!(&response[46..], &[0xDC, 0xBA]);
}
