//! The stock memory protocol: hello, read memory, write memory (`ADR-100`).
//!
//! A radio is programmed the way Quansheng radios always have been: the host
//! reads and writes an 8 KiB memory view in small blocks, and owns what the
//! bytes mean. The host compiles and validates the configuration image before
//! it writes anything; the radio validates the image again when it loads it.
//!
//! Requests use the stock envelope (`AB CD`, length, XOR-obfuscated payload
//! and CRC, `DC BA`) and carry the stock session word. Commands:
//!
//! - `0x0514` hello, answered with `0x0515` and the application identity;
//! - `0x051B` read up to [`MAX_READ_BYTES`], answered with `0x051C`;
//! - `0x051D` write [`WRITE_BYTES`] at an aligned address, answered with
//!   `0x051E` once the bytes are stored.

use crate::serial::{
    crc16_xmodem, encode_hello_response as encode_shared_hello, xor, ApplicationIdentity,
    RESPONSE_FRAME_BYTES as HELLO_FRAME_BYTES,
};

/// Bytes in the memory view, as the K5's EEPROM has.
pub const VIEW_BYTES: u16 = 0x2000;
/// First address of the configuration image the host writes.
pub const IMAGE_ORIGIN: u16 = 0x0000;
/// Bytes reserved for the configuration image, header included: everything
/// below the radio's own place record. The radio reads the image from memory
/// as it needs it, so RAM does not bound this.
pub const IMAGE_REGION_BYTES: u16 = 0x1D00;
/// First address of the radio's own record of where the operator left it.
pub const PLACE_ORIGIN: u16 = 0x1D00;
/// First address of the vendor calibration, which a host may read but never
/// write: it is the radio's own measured data and cannot be recreated.
pub const CALIBRATION_ORIGIN: u16 = 0x1E00;

/// Largest block one read request may ask for; hosts read 128 at a time.
pub const MAX_READ_BYTES: usize = 0x80;
/// Bytes one write request carries, the only write shape stock hosts use.
pub const WRITE_BYTES: usize = 8;
const _: () = assert!(WRITE_BYTES == 8);

/// Largest response frame: a full read block with its envelope.
pub const RESPONSE_FRAME_BYTES: usize = 4 + 8 + MAX_READ_BYTES + 2 + 2;

const COMMAND_HELLO: u16 = 0x0514;
const COMMAND_READ: u16 = 0x051B;
const COMMAND_READ_RESPONSE: u16 = 0x051C;
const COMMAND_WRITE: u16 = 0x051D;
const COMMAND_WRITE_RESPONSE: u16 = 0x051E;
const SESSION_WORD: u32 = 0x6457_396A;

/// Payload bytes of each request, before its two CRC bytes.
const HELLO_PAYLOAD: usize = 8;
const READ_PAYLOAD: usize = 12;
const WRITE_PAYLOAD: usize = 20;
/// The largest request body: a write payload and its CRC.
const MAX_BODY: usize = WRITE_PAYLOAD + 2;
/// The largest declared payload read past before resynchronising.
const MAXIMUM_DISCARD_BYTES: usize = 276;

/// Returns whether a host may write `WRITE_BYTES` at `address`.
///
/// Writes must be aligned and below the calibration. The image and the place
/// are both writable, so restoring a whole backup puts both back.
pub const fn host_may_write(address: u16) -> bool {
    address % 8 == 0 && address < CALIBRATION_ORIGIN
}

/// One validated request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Identify the application.
    Hello,
    /// Read `length` bytes from `address`.
    Read {
        /// First address.
        address: u16,
        /// Bytes requested, from one to [`MAX_READ_BYTES`].
        length: u8,
    },
    /// Write one block at `address`.
    Write {
        /// First address.
        address: u16,
        /// The bytes to store.
        data: [u8; WRITE_BYTES],
    },
}

/// What one byte did to the decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Push {
    /// The frame is not complete yet.
    Idle,
    /// A complete, valid request.
    Request(Request),
    /// A complete frame which failed its CRC, session, or command checks.
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Header { previous_was_header: bool },
    Length { first: Option<u8> },
    Body { index: usize, size: usize },
    Footer { first: Option<u8>, size: usize },
    Discard { remaining: usize },
}

/// Bounded request decoder, fed one byte at a time.
#[derive(Clone, Copy, Debug)]
pub struct Decoder {
    state: State,
    body: [u8; MAX_BODY],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Creates a decoder waiting for a frame header.
    pub const fn new() -> Self {
        Self {
            state: State::Header {
                previous_was_header: false,
            },
            body: [0; MAX_BODY],
        }
    }

    /// Consumes one byte.
    pub fn push(&mut self, byte: u8) -> Push {
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
                self.state = if matches!(declared, HELLO_PAYLOAD | READ_PAYLOAD | WRITE_PAYLOAD) {
                    State::Body {
                        index: 0,
                        size: declared + 2,
                    }
                } else {
                    State::Discard {
                        remaining: declared.saturating_add(4).min(MAXIMUM_DISCARD_BYTES),
                    }
                };
            }
            State::Body { index, size } => {
                self.body[index] = byte;
                self.state = if index + 1 == size {
                    State::Footer { first: None, size }
                } else {
                    State::Body {
                        index: index + 1,
                        size,
                    }
                };
            }
            State::Footer { first: None, size } => {
                self.state = State::Footer {
                    first: Some(byte),
                    size,
                };
            }
            State::Footer {
                first: Some(low),
                size,
            } => {
                self.state = State::Header {
                    previous_was_header: false,
                };
                if [low, byte] != [0xDC, 0xBA] {
                    return Push::Rejected;
                }
                return decode(&mut self.body[..size]).map_or(Push::Rejected, Push::Request);
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
        Push::Idle
    }
}

/// Validates one body: CRC, command, declared length, and session word.
fn decode(body: &mut [u8]) -> Option<Request> {
    xor(body);
    let (payload, crc) = body.split_at(body.len() - 2);
    if crc16_xmodem(payload) != u16::from_le_bytes([crc[0], crc[1]]) {
        return None;
    }
    let command = u16::from_le_bytes([payload[0], payload[1]]);
    let declared = usize::from(u16::from_le_bytes([payload[2], payload[3]]));
    if declared + 4 != payload.len() {
        return None;
    }
    let session_at = if command == COMMAND_HELLO { 4 } else { 8 };
    let session = u32::from_le_bytes([
        payload[session_at],
        payload[session_at + 1],
        payload[session_at + 2],
        payload[session_at + 3],
    ]);
    if session != SESSION_WORD {
        return None;
    }
    let address = u16::from_le_bytes([payload[4], payload[5]]);
    match (command, payload.len()) {
        (COMMAND_HELLO, HELLO_PAYLOAD) => Some(Request::Hello),
        (COMMAND_READ, READ_PAYLOAD) => {
            let length = payload[6];
            let end = u32::from(address) + u32::from(length);
            (length != 0 && usize::from(length) <= MAX_READ_BYTES && end <= u32::from(VIEW_BYTES))
                .then_some(Request::Read { address, length })
        }
        (COMMAND_WRITE, WRITE_PAYLOAD) => {
            if usize::from(payload[6]) != WRITE_BYTES {
                return None;
            }
            let mut data = [0_u8; WRITE_BYTES];
            data.copy_from_slice(&payload[12..20]);
            Some(Request::Write { address, data })
        }
        _ => None,
    }
}

/// Wraps `payload_len` bytes already placed at `frame[4..]` in the stock
/// response envelope and returns the frame length.
fn seal(frame: &mut [u8; RESPONSE_FRAME_BYTES], payload_len: usize) -> usize {
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&u16::try_from(payload_len).unwrap_or(0).to_le_bytes());
    let trailer = 4 + payload_len;
    frame[trailer..trailer + 2].copy_from_slice(&0xFFFF_u16.to_le_bytes());
    xor(&mut frame[4..trailer + 2]);
    frame[trailer + 2..trailer + 4].copy_from_slice(&[0xDC, 0xBA]);
    trailer + 4
}

/// Encodes the `0x051C` answer to one read and returns its length.
///
/// `fill` reads the data straight into the frame; when it fails nothing is
/// encoded and `None` is returned, so the host's read times out rather than
/// receiving bytes nobody read.
pub fn encode_read_response(
    address: u16,
    length: u8,
    frame: &mut [u8; RESPONSE_FRAME_BYTES],
    fill: impl FnOnce(&mut [u8]) -> bool,
) -> Option<usize> {
    let length = usize::from(length).min(MAX_READ_BYTES);
    frame.fill(0);
    let payload = &mut frame[4..12 + length];
    payload[0..2].copy_from_slice(&COMMAND_READ_RESPONSE.to_le_bytes());
    payload[2..4].copy_from_slice(&u16::try_from(length + 4).unwrap_or(0).to_le_bytes());
    payload[4..6].copy_from_slice(&address.to_le_bytes());
    payload[6] = u8::try_from(length).unwrap_or(0);
    if !fill(&mut payload[8..]) {
        return None;
    }
    Some(seal(frame, 8 + length))
}

/// Encodes the `0x0515` hello answer carrying `identity`.
pub fn encode_hello_response(
    identity: ApplicationIdentity<'_>,
    frame: &mut [u8; RESPONSE_FRAME_BYTES],
) -> usize {
    let mut hello = [0_u8; HELLO_FRAME_BYTES];
    encode_shared_hello(identity, &mut hello);
    frame.fill(0);
    frame[..HELLO_FRAME_BYTES].copy_from_slice(&hello);
    HELLO_FRAME_BYTES
}

/// Encodes the `0x051E` acknowledgement of one stored write.
pub fn encode_write_response(address: u16, frame: &mut [u8; RESPONSE_FRAME_BYTES]) -> usize {
    frame.fill(0);
    frame[4..6].copy_from_slice(&COMMAND_WRITE_RESPONSE.to_le_bytes());
    frame[6..8].copy_from_slice(&2_u16.to_le_bytes());
    frame[8..10].copy_from_slice(&address.to_le_bytes());
    seal(frame, 6)
}

/// Host side: builds one request frame. Used by the host tools and tests.
pub mod host {
    use super::{
        COMMAND_HELLO, COMMAND_READ, COMMAND_WRITE, HELLO_PAYLOAD, READ_PAYLOAD, SESSION_WORD,
        WRITE_BYTES, WRITE_PAYLOAD,
    };
    use crate::serial::{crc16_xmodem, xor};

    /// Largest request frame: a write.
    pub const REQUEST_FRAME_BYTES: usize = 4 + WRITE_PAYLOAD + 2 + 2;

    fn frame(payload: &[u8], output: &mut [u8; REQUEST_FRAME_BYTES]) -> usize {
        let len = payload.len();
        output[0..2].copy_from_slice(&[0xAB, 0xCD]);
        output[2..4].copy_from_slice(&u16::try_from(len).unwrap_or(0).to_le_bytes());
        output[4..4 + len].copy_from_slice(payload);
        output[4 + len..6 + len].copy_from_slice(&crc16_xmodem(payload).to_le_bytes());
        xor(&mut output[4..6 + len]);
        output[6 + len..8 + len].copy_from_slice(&[0xDC, 0xBA]);
        8 + len
    }

    /// Encodes a hello request.
    pub fn hello(output: &mut [u8; REQUEST_FRAME_BYTES]) -> usize {
        let mut payload = [0_u8; HELLO_PAYLOAD];
        payload[0..2].copy_from_slice(&COMMAND_HELLO.to_le_bytes());
        payload[2..4].copy_from_slice(&4_u16.to_le_bytes());
        payload[4..8].copy_from_slice(&SESSION_WORD.to_le_bytes());
        frame(&payload, output)
    }

    /// Encodes a read request.
    pub fn read(address: u16, length: u8, output: &mut [u8; REQUEST_FRAME_BYTES]) -> usize {
        let mut payload = [0_u8; READ_PAYLOAD];
        payload[0..2].copy_from_slice(&COMMAND_READ.to_le_bytes());
        payload[2..4].copy_from_slice(&8_u16.to_le_bytes());
        payload[4..6].copy_from_slice(&address.to_le_bytes());
        payload[6] = length;
        payload[8..12].copy_from_slice(&SESSION_WORD.to_le_bytes());
        frame(&payload, output)
    }

    /// Encodes a write request.
    pub fn write(
        address: u16,
        data: [u8; WRITE_BYTES],
        output: &mut [u8; REQUEST_FRAME_BYTES],
    ) -> usize {
        let mut payload = [0_u8; WRITE_PAYLOAD];
        payload[0..2].copy_from_slice(&COMMAND_WRITE.to_le_bytes());
        payload[2..4].copy_from_slice(&16_u16.to_le_bytes());
        payload[4..6].copy_from_slice(&address.to_le_bytes());
        payload[6] = u8::try_from(WRITE_BYTES).unwrap_or(0);
        payload[8..12].copy_from_slice(&SESSION_WORD.to_le_bytes());
        payload[12..20].copy_from_slice(&data);
        frame(&payload, output)
    }
}

#[cfg(test)]
mod tests {
    use super::host::{self, REQUEST_FRAME_BYTES};
    use super::{
        encode_read_response, encode_write_response, host_may_write, Decoder, Push, Request,
        CALIBRATION_ORIGIN, RESPONSE_FRAME_BYTES,
    };
    use crate::serial::xor;

    fn decode_all(bytes: &[u8]) -> Push {
        let mut decoder = Decoder::new();
        let mut last = Push::Idle;
        for byte in bytes {
            last = decoder.push(*byte);
            if last != Push::Idle {
                break;
            }
        }
        last
    }

    #[test]
    fn host_requests_decode_to_what_was_sent() {
        let mut frame = [0_u8; REQUEST_FRAME_BYTES];
        let len = host::hello(&mut frame);
        assert_eq!(decode_all(&frame[..len]), Push::Request(Request::Hello));
        let len = host::read(0x0F80, 0x80, &mut frame);
        assert_eq!(
            decode_all(&frame[..len]),
            Push::Request(Request::Read {
                address: 0x0F80,
                length: 0x80
            })
        );
        let len = host::write(0x0010, [1, 2, 3, 4, 5, 6, 7, 8], &mut frame);
        assert_eq!(
            decode_all(&frame[..len]),
            Push::Request(Request::Write {
                address: 0x0010,
                data: [1, 2, 3, 4, 5, 6, 7, 8]
            })
        );
    }

    #[test]
    fn a_write_frame_matches_the_stock_host_encoding() {
        // Command, declared 16, offset, length 8, pad, session, data: the
        // shape `radio-flasher` has always sent to stock radios.
        let mut frame = [0_u8; REQUEST_FRAME_BYTES];
        let len = host::write(0x1234, [0xAA; 8], &mut frame);
        assert_eq!(len, 28);
        let mut payload = [0_u8; 22];
        payload.copy_from_slice(&frame[4..26]);
        xor(&mut payload);
        assert_eq!(&payload[..8], &[0x1D, 0x05, 16, 0, 0x34, 0x12, 8, 0]);
        assert_eq!(&payload[8..12], &0x6457_396A_u32.to_le_bytes());
    }

    #[test]
    fn damaged_or_oversized_requests_are_rejected() {
        let mut frame = [0_u8; REQUEST_FRAME_BYTES];
        let len = host::read(0, 0x80, &mut frame);
        frame[6] ^= 0x01;
        assert_eq!(decode_all(&frame[..len]), Push::Rejected);
        let len = host::read(0, 0x81, &mut frame);
        assert_eq!(decode_all(&frame[..len]), Push::Rejected);
        let len = host::read(0x1FF0, 0x20, &mut frame);
        assert_eq!(decode_all(&frame[..len]), Push::Rejected, "past the view");
    }

    #[test]
    fn the_decoder_resynchronises_after_an_unknown_length() {
        let mut stream = std::vec![0xAB, 0xCD, 3, 0, 1, 2, 3, 0, 0, 0xDC, 0xBA];
        let mut frame = [0_u8; REQUEST_FRAME_BYTES];
        let len = host::hello(&mut frame);
        stream.extend_from_slice(&frame[..len]);
        let mut decoder = Decoder::new();
        let pushes: std::vec::Vec<_> = stream.iter().map(|byte| decoder.push(*byte)).collect();
        assert_eq!(pushes.last(), Some(&Push::Request(Request::Hello)));
    }

    #[test]
    fn calibration_and_unaligned_addresses_are_not_writable() {
        assert!(host_may_write(0));
        assert!(host_may_write(CALIBRATION_ORIGIN - 8));
        assert!(!host_may_write(CALIBRATION_ORIGIN));
        assert!(!host_may_write(0x1F40));
        assert!(!host_may_write(4));
    }

    #[test]
    fn responses_carry_the_address_the_host_checks() {
        let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
        let len = encode_read_response(0x0100, 4, &mut frame, |data| {
            data.copy_from_slice(&[9, 8, 7, 6]);
            true
        })
        .unwrap();
        assert_eq!(len, 4 + 12 + 2 + 2);
        let mut payload = [0_u8; 14];
        payload.copy_from_slice(&frame[4..18]);
        xor(&mut payload);
        assert_eq!(
            &payload[..12],
            &[0x1C, 0x05, 8, 0, 0x00, 0x01, 4, 0, 9, 8, 7, 6]
        );
        assert!(encode_read_response(0, 4, &mut frame, |_| false).is_none());

        let len = encode_write_response(0x0108, &mut frame);
        assert_eq!(len, 4 + 6 + 2 + 2);
        let mut payload = [0_u8; 8];
        payload.copy_from_slice(&frame[4..12]);
        xor(&mut payload);
        assert_eq!(&payload[..6], &[0x1E, 0x05, 2, 0, 0x08, 0x01]);
    }
}
