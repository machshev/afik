//! Bounded AFIK K1 serial-witness framing.

#![allow(clippy::identity_op)]

/// Encoded request body size: eight payload bytes plus two CRC bytes.
pub const REQUEST_BODY_BYTES: usize = 10;
/// Complete encoded response size for the fixed 40-byte hello payload.
pub const RESPONSE_FRAME_BYTES: usize = 48;
/// Complete encoded response size for the 16-byte keypad diagnostic payload.
pub const KEYPAD_RESPONSE_FRAME_BYTES: usize = 24;
/// Complete encoded response size for the 24-byte clock diagnostic payload.
pub const CLOCK_RESPONSE_FRAME_BYTES: usize = 32;
/// Complete encoded response size for one 12-byte RCC register payload.
pub const CLOCK_REGISTER_RESPONSE_FRAME_BYTES: usize = 20;

const COMMAND_HELLO_REQUEST: u16 = 0x0514;
const COMMAND_HELLO_RESPONSE: u16 = 0x0515;
const COMMAND_KEYPAD_REQUEST: u16 = 0x7F10;
const COMMAND_KEYPAD_RESPONSE: u16 = 0x7F11;
const COMMAND_CLOCK_REQUEST: u16 = 0x7F12;
const COMMAND_CLOCK_RESPONSE: u16 = 0x7F13;
const COMMAND_CLOCK_REGISTER_REQUESTS: [u16; 4] = [0x7F14, 0x7F16, 0x7F18, 0x7F1A];
const COMMAND_CLOCK_REGISTER_RESPONSES: [u16; 4] = [0x7F15, 0x7F17, 0x7F19, 0x7F1B];
const COMMAND_CLOCK_CONTROL_REQUEST: u16 = 0x7F1C;
const COMMAND_CLOCK_CONTROL_RESPONSE: u16 = 0x7F1D;
/// Fixed no-MMIO marker returned by the clock-path control request.
pub const CLOCK_CONTROL_MARKER: u32 = 0x4B31_434C;
const SESSION_WORD: u32 = 0x6457_396A;
const RESPONSE_PAYLOAD_BYTES: usize = 40;
const RESPONSE_DECLARED_BYTES: u16 = 36;
const RESPONSE_TRAILER: u16 = 0xFFFF;
const XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

/// Printable identity returned by the first AFIK K1 application.
pub const APPLICATION_VERSION: &[u8] = b"AFIK-K1-0.3";

/// One accepted read-only normal-mode request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Existing AFIK application identity probe.
    Hello,
    /// Raw main-key matrix observation.
    KeypadMatrix,
    /// Raw inherited RCC clock observation.
    ClockSnapshot,
    /// One individually identified raw RCC register observation.
    ClockRegister(u8),
    /// No-MMIO control for the clock diagnostic command/response path.
    ClockControl,
}

/// Decodes one bounded normal-mode request body.
pub fn decode_request(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> Option<Request> {
    xor(encoded_body);
    let payload = &encoded_body[..8];
    let expected_crc = u16::from_le_bytes([encoded_body[8], encoded_body[9]]);
    let command = u16::from_le_bytes([payload[0], payload[1]]);
    let declared = u16::from_le_bytes([payload[2], payload[3]]);
    let session = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if declared != 4 || session != SESSION_WORD || crc16_xmodem(payload) != expected_crc {
        return None;
    }
    match command {
        COMMAND_HELLO_REQUEST => Some(Request::Hello),
        COMMAND_KEYPAD_REQUEST => Some(Request::KeypadMatrix),
        COMMAND_CLOCK_REQUEST => Some(Request::ClockSnapshot),
        command if command == COMMAND_CLOCK_REGISTER_REQUESTS[0] => Some(Request::ClockRegister(0)),
        command if command == COMMAND_CLOCK_REGISTER_REQUESTS[1] => Some(Request::ClockRegister(1)),
        command if command == COMMAND_CLOCK_REGISTER_REQUESTS[2] => Some(Request::ClockRegister(2)),
        command if command == COMMAND_CLOCK_REGISTER_REQUESTS[3] => Some(Request::ClockRegister(3)),
        COMMAND_CLOCK_CONTROL_REQUEST => Some(Request::ClockControl),
        _ => None,
    }
}

/// Encodes the no-MMIO clock-path control response.
pub fn encode_clock_control_response(frame: &mut [u8; CLOCK_REGISTER_RESPONSE_FRAME_BYTES]) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&12_u16.to_le_bytes());
    let payload = &mut frame[4..16];
    payload[0..2].copy_from_slice(&COMMAND_CLOCK_CONTROL_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
    payload[4] = 0xA5;
    payload[8..12].copy_from_slice(&CLOCK_CONTROL_MARKER.to_le_bytes());
    frame[16..18].copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..18]);
    frame[18..20].copy_from_slice(&[0xDC, 0xBA]);
}

/// Encodes one individually identified raw RCC register observation.
pub fn encode_clock_register_response(
    frame: &mut [u8; CLOCK_REGISTER_RESPONSE_FRAME_BYTES],
    register: u8,
    value: u32,
) {
    let index = usize::from(register);
    if index >= COMMAND_CLOCK_REGISTER_RESPONSES.len() {
        return;
    }
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&12_u16.to_le_bytes());
    let payload = &mut frame[4..16];
    payload[0..2].copy_from_slice(&COMMAND_CLOCK_REGISTER_RESPONSES[index].to_le_bytes());
    payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
    payload[4] = register;
    payload[8..12].copy_from_slice(&value.to_le_bytes());
    frame[16..18].copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..18]);
    frame[18..20].copy_from_slice(&[0xDC, 0xBA]);
}

/// Encodes one raw, read-only inherited RCC observation.
pub fn encode_clock_response(
    frame: &mut [u8; CLOCK_RESPONSE_FRAME_BYTES],
    registers: [u32; 4],
    contract_valid: bool,
) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&24_u16.to_le_bytes());
    let payload = &mut frame[4..28];
    payload[0..2].copy_from_slice(&COMMAND_CLOCK_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&20_u16.to_le_bytes());
    for (index, register) in registers.into_iter().enumerate() {
        payload[4 + index * 4..8 + index * 4].copy_from_slice(&register.to_le_bytes());
    }
    payload[20] = u8::from(contract_valid);
    frame[28..30].copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..30]);
    frame[30..32].copy_from_slice(&[0xDC, 0xBA]);
}

/// Decodes and validates one bounded normal-mode hello request body.
pub fn is_valid_hello_request(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> bool {
    decode_request(encoded_body) == Some(Request::Hello)
}

/// Encodes one raw, read-only main-key matrix response.
pub fn encode_keypad_response(
    frame: &mut [u8; KEYPAD_RESPONSE_FRAME_BYTES],
    gpio_b_idr_by_column: [u16; 4],
    scan_valid: bool,
    captured: bool,
) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&16_u16.to_le_bytes());
    let payload = &mut frame[4..20];
    payload[0..2].copy_from_slice(&COMMAND_KEYPAD_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&12_u16.to_le_bytes());
    for (index, idr) in gpio_b_idr_by_column.into_iter().enumerate() {
        payload[4 + index * 2..6 + index * 2].copy_from_slice(&idr.to_le_bytes());
    }
    payload[12] = u8::from(scan_valid);
    payload[13] = u8::from(captured);
    frame[20..22].copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..22]);
    frame[22..24].copy_from_slice(&[0xDC, 0xBA]);
}

/// Encodes one normal-mode hello response into a caller-provided frame.
///
/// # Panics
///
/// This cannot panic because the fixed response payload length fits in `u16`.
pub fn encode_hello_response(frame: &mut [u8; RESPONSE_FRAME_BYTES]) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    let payload_length = u16::try_from(RESPONSE_PAYLOAD_BYTES).expect("bounded response");
    frame[2..4].copy_from_slice(&payload_length.to_le_bytes());

    let payload = &mut frame[4..4 + RESPONSE_PAYLOAD_BYTES];
    payload[0..2].copy_from_slice(&COMMAND_HELLO_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&RESPONSE_DECLARED_BYTES.to_le_bytes());
    payload[4..4 + APPLICATION_VERSION.len()].copy_from_slice(APPLICATION_VERSION);
    frame[4 + RESPONSE_PAYLOAD_BYTES..6 + RESPONSE_PAYLOAD_BYTES]
        .copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..6 + RESPONSE_PAYLOAD_BYTES]);
    frame[6 + RESPONSE_PAYLOAD_BYTES..].copy_from_slice(&[0xDC, 0xBA]);
}

fn crc16_xmodem(bytes: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

fn xor(bytes: &mut [u8]) {
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte ^= XOR_KEY[index % XOR_KEY.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_request, encode_clock_control_response, encode_clock_register_response,
        encode_clock_response, encode_hello_response, encode_keypad_response,
        is_valid_hello_request, Request, APPLICATION_VERSION, CLOCK_CONTROL_MARKER,
    };

    #[test]
    fn hello_request_validation_accepts_only_the_fixed_session() {
        let mut request = [0x14, 0x05, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64, 0x2C, 0xB7];
        assert!(!is_valid_hello_request(&mut request));

        // The request body is the encoded form used by the host. Construct it
        // through the same fixed-key envelope so this test stays wire-exact.
        let payload = [0x14, 0x05, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64];
        let mut crc = 0_u16;
        for byte in payload {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 == 0 {
                    crc << 1
                } else {
                    (crc << 1) ^ 0x1021
                };
            }
        }
        let mut encoded = [0_u8; 10];
        encoded[..8].copy_from_slice(&payload);
        encoded[8..].copy_from_slice(&crc.to_le_bytes());
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in encoded.iter_mut().enumerate() {
            *byte ^= key[index];
        }
        assert!(is_valid_hello_request(&mut encoded));
    }

    #[test]
    fn hello_response_has_the_observed_envelope_and_afik_identity() {
        let mut frame = [0_u8; 48];
        encode_hello_response(&mut frame);
        assert_eq!(&frame[..2], &[0xAB, 0xCD]);
        assert_eq!(&frame[2..4], &[40, 0]);
        assert_eq!(&frame[46..], &[0xDC, 0xBA]);

        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in frame[4..46].iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(&frame[4..6], &[0x15, 0x05]);
        assert_eq!(&frame[6..8], &[36, 0]);
        assert_eq!(
            &frame[8..8 + APPLICATION_VERSION.len()],
            APPLICATION_VERSION
        );
        assert_eq!(&frame[48 - 4..48 - 2], &[0xFF, 0xFF]);
    }

    #[test]
    fn keypad_request_and_raw_response_are_wire_exact() {
        let payload = [0x10, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64];
        let mut encoded = encode_request_for_test(payload);
        assert_eq!(decode_request(&mut encoded), Some(Request::KeypadMatrix));

        let mut frame = [0_u8; 24];
        encode_keypad_response(&mut frame, [0x1001, 0x2002, 0x4004, 0x8008], true, true);
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 16, 0]);
        assert_eq!(&frame[22..], &[0xDC, 0xBA]);
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in frame[4..22].iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(
            &frame[4..22],
            &[0x11, 0x7F, 12, 0, 1, 0x10, 2, 0x20, 4, 0x40, 8, 0x80, 1, 1, 0, 0, 0xFF, 0xFF]
        );
    }

    #[test]
    fn clock_request_and_raw_response_are_wire_exact() {
        let payload = [0x12, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64];
        let mut encoded = encode_request_for_test(payload);
        assert_eq!(decode_request(&mut encoded), Some(Request::ClockSnapshot));

        let mut frame = [0_u8; 32];
        encode_clock_response(&mut frame, [0x0300_0500, 0x0000_8000, 0x0000_0012, 0], true);
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 24, 0]);
        assert_eq!(&frame[30..], &[0xDC, 0xBA]);
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in frame[4..30].iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(&frame[4..8], &[0x13, 0x7F, 20, 0]);
        assert_eq!(
            &frame[8..24],
            &[0, 5, 0, 3, 0, 128, 0, 0, 18, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(&frame[24..30], &[1, 0, 0, 0, 0xFF, 0xFF]);
    }

    #[test]
    fn individual_clock_register_responses_are_identified_and_bounded() {
        for (register, request_command) in
            [0x7F14_u16, 0x7F16, 0x7F18, 0x7F1A].into_iter().enumerate()
        {
            let mut payload = [0_u8; 8];
            payload[0..2].copy_from_slice(&request_command.to_le_bytes());
            payload[2..4].copy_from_slice(&4_u16.to_le_bytes());
            payload[4..8].copy_from_slice(&0x6457_396A_u32.to_le_bytes());
            let mut encoded = encode_request_for_test(payload);
            assert_eq!(
                decode_request(&mut encoded),
                Some(Request::ClockRegister(u8::try_from(register).unwrap()))
            );

            let mut frame = [0_u8; 20];
            encode_clock_register_response(
                &mut frame,
                u8::try_from(register).unwrap(),
                0x1234_0000 | u32::try_from(register).unwrap(),
            );
            assert_eq!(&frame[..4], &[0xAB, 0xCD, 12, 0]);
            assert_eq!(&frame[18..], &[0xDC, 0xBA]);
        }
    }

    #[test]
    fn no_mmio_clock_control_is_wire_exact() {
        let payload = [0x1C, 0x7F, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64];
        let mut encoded = encode_request_for_test(payload);
        assert_eq!(decode_request(&mut encoded), Some(Request::ClockControl));

        let mut frame = [0_u8; 20];
        encode_clock_control_response(&mut frame);
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in frame[4..18].iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(&frame[4..8], &[0x1D, 0x7F, 8, 0]);
        assert_eq!(frame[8], 0xA5);
        assert_eq!(
            u32::from_le_bytes(frame[12..16].try_into().unwrap()),
            CLOCK_CONTROL_MARKER
        );
    }

    fn encode_request_for_test(payload: [u8; 8]) -> [u8; 10] {
        let mut encoded = [0_u8; 10];
        encoded[..8].copy_from_slice(&payload);
        encoded[8..].copy_from_slice(&super::crc16_xmodem(&payload).to_le_bytes());
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in encoded.iter_mut().enumerate() {
            *byte ^= key[index];
        }
        encoded
    }
}
