//! Bounded normal-mode serial service shared by target applications.

/// Encoded request body size: eight payload bytes plus two CRC bytes.
pub const REQUEST_BODY_BYTES: usize = 10;
/// Complete encoded response size for the fixed 40-byte hello payload.
pub const RESPONSE_FRAME_BYTES: usize = 48;
/// Maximum printable application identity carried by a hello response.
pub const MAX_IDENTITY_BYTES: usize = 32;

const COMMAND_HELLO_REQUEST: u16 = 0x0514;
const COMMAND_HELLO_RESPONSE: u16 = 0x0515;
const REQUEST_DECLARED_BYTES: u16 = 4;
const RESPONSE_PAYLOAD_BYTES: usize = 40;
const RESPONSE_PAYLOAD_LENGTH: u16 = 40;
const _: () = assert!(RESPONSE_PAYLOAD_LENGTH as usize == RESPONSE_PAYLOAD_BYTES);
const RESPONSE_DECLARED_BYTES: u16 = 36;
const RESPONSE_TRAILER: u16 = 0xFFFF;
const SESSION_WORD: u32 = 0x6457_396A;
const XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

/// A validated, bounded application identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationIdentity<'a>(&'a [u8]);

impl<'a> ApplicationIdentity<'a> {
    /// Validates one printable identity for the fixed hello response.
    pub const fn new(bytes: &'a [u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_IDENTITY_BYTES {
            return None;
        }
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] < 0x20 || bytes[index] > 0x7E {
                return None;
            }
            index += 1;
        }
        Some(Self(bytes))
    }

    /// Returns the printable wire bytes.
    pub const fn bytes(self) -> &'a [u8] {
        self.0
    }
}

/// Decodes a validated request body into its command word.
///
/// The body is deobfuscated in place, so a rejected body must not be reused.
pub fn decode_command(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> Option<u16> {
    xor(encoded_body);
    let payload = &encoded_body[..8];
    let expected_crc = u16::from_le_bytes([encoded_body[8], encoded_body[9]]);
    let declared = u16::from_le_bytes([payload[2], payload[3]]);
    let session = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if declared != REQUEST_DECLARED_BYTES
        || session != SESSION_WORD
        || crc16_xmodem(payload) != expected_crc
    {
        return None;
    }
    Some(u16::from_le_bytes([payload[0], payload[1]]))
}

/// Returns whether a validated request body asks for application identity.
pub fn decode_hello_request(encoded_body: &mut [u8; REQUEST_BODY_BYTES]) -> bool {
    decode_command(encoded_body) == Some(COMMAND_HELLO_REQUEST)
}

/// Encodes the common hello response with a target-supplied identity.
pub fn encode_hello_response(
    identity: ApplicationIdentity<'_>,
    frame: &mut [u8; RESPONSE_FRAME_BYTES],
) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&RESPONSE_PAYLOAD_LENGTH.to_le_bytes());
    let payload = &mut frame[4..4 + RESPONSE_PAYLOAD_BYTES];
    payload[0..2].copy_from_slice(&COMMAND_HELLO_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&RESPONSE_DECLARED_BYTES.to_le_bytes());
    payload[4..4 + identity.bytes().len()].copy_from_slice(identity.bytes());
    frame[4 + RESPONSE_PAYLOAD_BYTES..6 + RESPONSE_PAYLOAD_BYTES]
        .copy_from_slice(&RESPONSE_TRAILER.to_le_bytes());
    xor(&mut frame[4..6 + RESPONSE_PAYLOAD_BYTES]);
    frame[6 + RESPONSE_PAYLOAD_BYTES..].copy_from_slice(&[0xDC, 0xBA]);
}

/// The largest declared payload read past before resynchronising.
const MAXIMUM_DISCARD_BYTES: usize = 276;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Header { previous_was_header: bool },
    Length { first: Option<u8> },
    Body { index: usize },
    Footer { first: Option<u8> },
    Discard { remaining: usize },
}

/// Reassembles and answers hello requests independently of a target UART.
#[derive(Clone, Copy, Debug)]
pub struct HelloService<'a> {
    identity: ApplicationIdentity<'a>,
    state: State,
    body: [u8; REQUEST_BODY_BYTES],
}

impl<'a> HelloService<'a> {
    /// Creates a service waiting for a frame header.
    pub const fn new(identity: ApplicationIdentity<'a>) -> Self {
        Self {
            identity,
            state: State::Header {
                previous_was_header: false,
            },
            body: [0; REQUEST_BODY_BYTES],
        }
    }

    /// Consumes one adapter-provided byte and writes a response when complete.
    pub fn push(&mut self, byte: u8, response: &mut [u8; RESPONSE_FRAME_BYTES]) -> Option<usize> {
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
            }
            State::Length { first: None } => self.state = State::Length { first: Some(byte) },
            State::Length { first: Some(low) } => {
                let declared = usize::from(u16::from_le_bytes([low, byte]));
                self.state = if declared == REQUEST_BODY_BYTES - 2 {
                    State::Body { index: 0 }
                } else {
                    State::Discard {
                        remaining: declared.saturating_add(4).min(MAXIMUM_DISCARD_BYTES),
                    }
                };
            }
            State::Body { index } => {
                self.body[index] = byte;
                self.state = if index + 1 == REQUEST_BODY_BYTES {
                    State::Footer { first: None }
                } else {
                    State::Body { index: index + 1 }
                };
            }
            State::Footer { first: None } => self.state = State::Footer { first: Some(byte) },
            State::Footer { first: Some(low) } => {
                self.state = State::Header {
                    previous_was_header: false,
                };
                if [low, byte] == [0xDC, 0xBA] && decode_hello_request(&mut self.body) {
                    encode_hello_response(self.identity, response);
                    return Some(RESPONSE_FRAME_BYTES);
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
            }
        }
        None
    }
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
    use super::{crc16_xmodem, xor, ApplicationIdentity, HelloService, RESPONSE_FRAME_BYTES};

    const IDENTITY: ApplicationIdentity<'static> =
        ApplicationIdentity::new(b"AFIK-TEST-1.0").unwrap();

    fn hello() -> [u8; 16] {
        let mut frame = [0_u8; 16];
        frame[..4].copy_from_slice(&[0xAB, 0xCD, 8, 0]);
        frame[4..6].copy_from_slice(&0x0514_u16.to_le_bytes());
        frame[6..8].copy_from_slice(&4_u16.to_le_bytes());
        frame[8..12].copy_from_slice(&0x6457_396A_u32.to_le_bytes());
        let crc = crc16_xmodem(&frame[4..12]);
        frame[12..14].copy_from_slice(&crc.to_le_bytes());
        xor(&mut frame[4..14]);
        frame[14..].copy_from_slice(&[0xDC, 0xBA]);
        frame
    }

    #[test]
    fn one_service_logic_answers_adapter_bytes_and_resynchronises() {
        for prefix in [&[][..], &[0, 0xAB, 0xFF][..]] {
            let mut service = HelloService::new(IDENTITY);
            let mut response = [0_u8; RESPONSE_FRAME_BYTES];
            let mut answered = None;
            for byte in prefix.iter().chain(hello().iter()) {
                answered = service.push(*byte, &mut response).or(answered);
            }
            assert_eq!(answered, Some(RESPONSE_FRAME_BYTES));
            xor(&mut response[4..46]);
            assert_eq!(&response[8..8 + IDENTITY.bytes().len()], IDENTITY.bytes());
        }
    }

    #[test]
    fn identities_are_printable_and_bounded() {
        assert!(ApplicationIdentity::new(b"").is_none());
        assert!(ApplicationIdentity::new(b"bad\nidentity").is_none());
        assert!(ApplicationIdentity::new(&[b'X'; 33]).is_none());
    }
}
