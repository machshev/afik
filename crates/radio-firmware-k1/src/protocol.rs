//! Bounded AFIK K1 serial-witness framing.

#![allow(clippy::identity_op)]

/// Encoded request body size: eight payload bytes plus two CRC bytes.
pub const REQUEST_BODY_BYTES: usize = 10;
/// Complete encoded response size for the fixed 40-byte hello payload.
pub const RESPONSE_FRAME_BYTES: usize = 48;
/// Complete encoded response size for the 12-byte keypad diagnostic payload.
pub const KEYPAD_RESPONSE_FRAME_BYTES: usize = 20;

const COMMAND_HELLO_REQUEST: u16 = 0x0514;
const COMMAND_HELLO_RESPONSE: u16 = 0x0515;
const COMMAND_KEYPAD_REQUEST: u16 = 0x7F10;
const COMMAND_KEYPAD_RESPONSE: u16 = 0x7F11;
const SESSION_WORD: u32 = 0x6457_396A;
const RESPONSE_PAYLOAD_BYTES: usize = 40;
const RESPONSE_DECLARED_BYTES: u16 = 36;
const RESPONSE_TRAILER: u16 = 0xFFFF;
const XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

/// Printable identity returned by the first AFIK K1 application.
pub const APPLICATION_VERSION: &[u8] = b"AFIK-K1-0.2";

/// One accepted read-only normal-mode request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Existing AFIK application identity probe.
    Hello,
    /// Raw main-key matrix observation.
    KeypadMatrix,
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
        _ => None,
    }
}

/// Decodes and validates one bounded normal-mode hello request body.
pub fn is_valid_hello_request(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> bool {
    decode_request(encoded_body) == Some(Request::Hello)
}

/// Encodes one raw, read-only main-key matrix response.
pub fn encode_keypad_response(
    frame: &mut [u8; KEYPAD_RESPONSE_FRAME_BYTES],
    row_low_by_column: [u8; 4],
    scan_valid: bool,
    captured: bool,
) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&12_u16.to_le_bytes());
    let payload = &mut frame[4..16];
    payload[0..2].copy_from_slice(&COMMAND_KEYPAD_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
    payload[4..8].copy_from_slice(&row_low_by_column);
    payload[8] = u8::from(scan_valid);
    payload[9] = u8::from(captured);
    frame[16..18].copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..18]);
    frame[18..20].copy_from_slice(&[0xDC, 0xBA]);
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
        decode_request, encode_hello_response, encode_keypad_response, is_valid_hello_request,
        Request, APPLICATION_VERSION,
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

        let mut frame = [0_u8; 20];
        encode_keypad_response(&mut frame, [1, 2, 4, 8], true, true);
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 12, 0]);
        assert_eq!(&frame[18..], &[0xDC, 0xBA]);
        let key = [
            0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03,
            0xE9, 0x80,
        ];
        for (index, byte) in frame[4..18].iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(
            &frame[4..18],
            &[0x11, 0x7F, 8, 0, 1, 2, 4, 8, 1, 1, 0, 0, 0xFF, 0xFF]
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
