//! The one read-only exchange the first K5 application answers.
//!
//! This is the legacy Quansheng normal-mode framing the host already speaks:
//! an `AB CD` header, a little-endian payload length, an obfuscated body, and a
//! `DC BA` footer. The image answers a hello with its own printable identity
//! and answers nothing else, so a host which reaches it learns that AFIK code
//! is running and learns nothing it could act on by mistake.

/// Encoded request body size: eight payload bytes plus two CRC bytes.
pub const REQUEST_BODY_BYTES: usize = 10;
/// Complete encoded response size for the fixed 40-byte hello payload.
pub const RESPONSE_FRAME_BYTES: usize = 48;

/// Printable identity returned by the first AFIK K5 application.
pub const APPLICATION_VERSION: &[u8] = b"AFIK-K5-1.2";

/// Plain-text banner sent once at boot, before any host has spoken.
///
/// The stock application sends nothing unprompted, so this is the only witness
/// available to an operator who has just power-cycled the radio and is only
/// watching the wire.
pub const BOOT_BANNER: &[u8] = b"AFIK-K5-1.2 booted";

const COMMAND_HELLO_REQUEST: u16 = 0x0514;
const COMMAND_HELLO_RESPONSE: u16 = 0x0515;
const REQUEST_DECLARED_BYTES: u16 = 4;
const RESPONSE_PAYLOAD_BYTES: usize = 40;
/// The same length as [`RESPONSE_PAYLOAD_BYTES`], in the width the frame
/// declares it in. The assertion below keeps the two from drifting apart.
const RESPONSE_PAYLOAD_LENGTH: u16 = 40;
const _: () = assert!(RESPONSE_PAYLOAD_LENGTH as usize == RESPONSE_PAYLOAD_BYTES);
const RESPONSE_DECLARED_BYTES: u16 = 36;
const RESPONSE_TRAILER: u16 = 0xFFFF;
const SESSION_WORD: u32 = 0x6457_396A;
const XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

/// One accepted read-only normal-mode request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Application identity probe.
    Hello,
}

/// Decodes one bounded request body, or rejects it.
///
/// The body is deobfuscated in place, so a rejected frame leaves the caller's
/// buffer scrambled; callers read a fresh body per frame.
pub fn decode_request(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> Option<Request> {
    xor(encoded_body);
    let payload = &encoded_body[..8];
    let expected_crc = u16::from_le_bytes([encoded_body[8], encoded_body[9]]);
    let command = u16::from_le_bytes([payload[0], payload[1]]);
    let declared = u16::from_le_bytes([payload[2], payload[3]]);
    let session = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if declared != REQUEST_DECLARED_BYTES
        || session != SESSION_WORD
        || crc16_xmodem(payload) != expected_crc
    {
        return None;
    }
    match command {
        COMMAND_HELLO_REQUEST => Some(Request::Hello),
        _ => None,
    }
}

/// The largest declared payload this image reads past before resynchronising.
const MAXIMUM_DISCARD_BYTES: usize = 276;

/// Where in a frame the reader currently is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    /// Looking for the two header bytes.
    Header { previous_was_header: bool },
    /// Collecting the declared payload length.
    Length { first: Option<u8> },
    /// Collecting the obfuscated body.
    Body { index: usize },
    /// Collecting the two footer bytes.
    Footer { first: Option<u8> },
    /// Reading past a frame this image does not serve.
    Discard { remaining: usize },
}

/// Reassembles requests from a byte stream.
///
/// A radio cannot choose what arrives on its wire: noise, a frame for another
/// command, and a truncated frame all have to leave the reader looking for the
/// next header rather than stuck or reading a body out of the wrong bytes.
/// Feeding one byte at a time keeps this true without a receive buffer.
#[derive(Clone, Copy, Debug)]
pub struct RequestReader {
    state: State,
    body: [u8; REQUEST_BODY_BYTES],
}

impl Default for RequestReader {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestReader {
    /// Returns a reader waiting for a header.
    pub const fn new() -> Self {
        Self {
            state: State::Header {
                previous_was_header: false,
            },
            body: [0; REQUEST_BODY_BYTES],
        }
    }

    /// Accepts one received byte, returning a request when one completes.
    pub fn push(&mut self, byte: u8) -> Option<Request> {
        match self.state {
            State::Header {
                previous_was_header,
            } => {
                self.state = if previous_was_header && byte == 0xCD {
                    State::Length { first: None }
                } else {
                    State::Header {
                        previous_was_header: byte == 0xAB,
                    }
                };
                None
            }
            State::Length { first: None } => {
                self.state = State::Length { first: Some(byte) };
                None
            }
            State::Length { first: Some(low) } => {
                let declared = usize::from(u16::from_le_bytes([low, byte]));
                self.state = if declared == REQUEST_BODY_BYTES - 2 {
                    State::Body { index: 0 }
                } else {
                    // The footer is read past too, so a longer frame cannot
                    // leave its own tail looking like the next header.
                    State::Discard {
                        remaining: declared.saturating_add(4).min(MAXIMUM_DISCARD_BYTES),
                    }
                };
                None
            }
            State::Body { index } => {
                self.body[index] = byte;
                self.state = if index + 1 == REQUEST_BODY_BYTES {
                    State::Footer { first: None }
                } else {
                    State::Body { index: index + 1 }
                };
                None
            }
            State::Footer { first: None } => {
                self.state = State::Footer { first: Some(byte) };
                None
            }
            State::Footer { first: Some(low) } => {
                self.state = State::Header {
                    previous_was_header: false,
                };
                if [low, byte] == [0xDC, 0xBA] {
                    decode_request(&mut self.body)
                } else {
                    None
                }
            }
            State::Discard { remaining } => {
                self.state = if remaining <= 1 {
                    State::Header {
                        previous_was_header: false,
                    }
                } else {
                    State::Discard {
                        remaining: remaining - 1,
                    }
                };
                None
            }
        }
    }
}

/// Encodes the hello response into a caller-provided frame.
pub fn encode_hello_response(frame: &mut [u8; RESPONSE_FRAME_BYTES]) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&RESPONSE_PAYLOAD_LENGTH.to_le_bytes());

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
        crc16_xmodem, decode_request, encode_hello_response, xor, Request, APPLICATION_VERSION,
        REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES,
    };

    fn request_body(command: u16, session: u32) -> [u8; REQUEST_BODY_BYTES] {
        let mut body = [0_u8; REQUEST_BODY_BYTES];
        body[0..2].copy_from_slice(&command.to_le_bytes());
        body[2..4].copy_from_slice(&4_u16.to_le_bytes());
        body[4..8].copy_from_slice(&session.to_le_bytes());
        let crc = crc16_xmodem(&body[..8]);
        body[8..10].copy_from_slice(&crc.to_le_bytes());
        xor(&mut body);
        body
    }

    #[test]
    fn a_hello_request_is_accepted() {
        let mut body = request_body(0x0514, 0x6457_396A);
        assert_eq!(decode_request(&mut body), Some(Request::Hello));
    }

    #[test]
    fn another_command_is_refused_rather_than_answered() {
        let mut body = request_body(0x051D, 0x6457_396A);
        assert_eq!(decode_request(&mut body), None);
    }

    #[test]
    fn a_frame_from_another_session_is_refused() {
        let mut body = request_body(0x0514, 0x1234_5678);
        assert_eq!(decode_request(&mut body), None);
    }

    #[test]
    fn a_corrupted_body_is_refused() {
        let mut body = request_body(0x0514, 0x6457_396A);
        body[5] ^= 0xFF;
        assert_eq!(decode_request(&mut body), None);
    }

    #[test]
    fn the_response_carries_the_identity_inside_the_declared_frame() {
        let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
        encode_hello_response(&mut frame);
        assert_eq!(&frame[0..2], &[0xAB, 0xCD]);
        assert_eq!(&frame[2..4], &40_u16.to_le_bytes());
        assert_eq!(&frame[46..48], &[0xDC, 0xBA]);

        let mut body = [0_u8; 42];
        body.copy_from_slice(&frame[4..46]);
        xor(&mut body);
        assert_eq!(&body[0..2], &0x0515_u16.to_le_bytes());
        assert_eq!(&body[2..4], &36_u16.to_le_bytes());
        assert_eq!(&body[4..4 + APPLICATION_VERSION.len()], APPLICATION_VERSION);
        assert_eq!(&body[40..42], &[0xFF, 0xFF]);
    }
}
