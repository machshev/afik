//! K5 identity supplied to the shared normal-mode serial service.

use radio_platform::receive_app::ReceiveApp;
pub use radio_platform::serial::{
    ApplicationIdentity, HelloService, REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES,
};
use radio_platform::serial_control::wire as shared_wire;
pub use radio_platform::serial_control::wire::Error as SharedWireError;
pub use radio_platform::serial_control::{
    dispatch as dispatch_shared, Capabilities as SerialCapabilities,
    Capability as SerialCapability, Command as SerialCommand, Query as SerialQuery,
    Request as SerialRequest, Result as SerialResult,
};

/// Printable identity: the workspace version, shared by every target and tool.
pub const APPLICATION_VERSION: &[u8] = concat!("AFIK-K5-", env!("CARGO_PKG_VERSION")).as_bytes();
/// Validated identity consumed by the shared application service.
pub const APPLICATION_IDENTITY: ApplicationIdentity<'static> =
    ApplicationIdentity::new(APPLICATION_VERSION).expect("K5 identity is printable and bounded");

/// Capabilities exposed by the current K5 application boundary.
///
/// K5 has the shared receive app, semantic-view query, and common logical-key
/// command boundary. It has no AFIK persistence or object protocol, and its
/// target-native metrics remain an adapter observation rather than a shared
/// serial capability.
pub const fn serial_capabilities() -> SerialCapabilities {
    SerialCapabilities::shared_receive()
}

/// Applies one already-decoded common serial request to the K5 application.
///
/// UART and circular-DMA framing remain in the target image. This adapter is
/// the point where a decoded common command enters the same shared event,
/// effect, and configuration-acknowledgement path used by keypad input.
pub fn apply_shared_serial_request(app: &mut ReceiveApp, request: SerialRequest) -> SerialResult {
    dispatch_shared(app, serial_capabilities(), request)
}

/// Decodes the fixed common payload after K5's normal-mode framing is removed.
/// Circular DMA, UART registers, and the legacy envelope remain K5-owned.
pub fn decode_shared_request(payload: &[u8]) -> Result<shared_wire::Request, SharedWireError> {
    shared_wire::decode_request(payload)
}

/// Routes one decoded common request through the same application as keypad
/// events, without introducing K1 persistence or object services.
pub fn apply_shared_wire_request(
    app: &mut ReceiveApp,
    request: shared_wire::Request,
) -> SerialResult {
    apply_shared_serial_request(app, shared_wire::into_request(request))
}

/// Encodes a semantic response for a target-owned K5 wire envelope.
pub fn encode_shared_response(
    result: SerialResult,
    output: &mut [u8],
) -> Result<usize, SharedWireError> {
    shared_wire::encode_response(result, output)
}

/// Plain-text banner retained for bounded serial diagnostics.
pub const BOOT_BANNER: &[u8] = concat!("AFIK-K5-", env!("CARGO_PKG_VERSION"), " booted").as_bytes();

/// Encodes the existing identity response without changing its envelope.
pub fn encode_hello_response(frame: &mut [u8; RESPONSE_FRAME_BYTES]) {
    radio_platform::serial::encode_hello_response(APPLICATION_IDENTITY, frame);
}

/// Target-owned normal-mode commands for the common application boundary.
///
/// The existing `0x0514` hello envelope remains unchanged. Common requests
/// use the otherwise unassigned `0x0600..=0x0617` command window, with `0x0615`
/// reserved for explicit audio; each supported logical key has its own command
/// because the observed K5 request body has no spare payload bytes after its
/// fixed session word.
pub const COMMON_VIEW_COMMAND: u16 = 0x0600;
/// Command assigned to the common capability query.
pub const COMMON_CAPABILITIES_COMMAND: u16 = 0x0601;
/// First command in the common logical-key assignment window.
pub const COMMON_KEY_FIRST_COMMAND: u16 = 0x0602;
/// Explicit audio preference command; Menu remains shell-owned list confirm.
pub const COMMON_TOGGLE_AUDIO_COMMAND: u16 = 0x0615;
/// Response command assigned to all common application responses.
pub const COMMON_RESPONSE_COMMAND: u16 = 0x0620;

/// One completed K5 normal-mode request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalRequest {
    /// The existing identity request.
    Hello,
    /// A common request mapped through the target-owned command assignment.
    Common(shared_wire::Request),
    /// The stock `0x051B` EEPROM read, which is how a host takes a backup.
    ReadEeprom {
        /// First EEPROM address.
        offset: u16,
        /// Bytes requested, from one to [`MAX_EEPROM_READ_BYTES`].
        length: u8,
    },
}

/// Stock read-EEPROM request command.
pub const READ_EEPROM_COMMAND: u16 = 0x051B;
/// Stock read-EEPROM response command.
pub const READ_EEPROM_RESPONSE_COMMAND: u16 = 0x051C;
/// Largest block one read request may ask for, as hosts read 128 at a time.
pub const MAX_EEPROM_READ_BYTES: usize = 0x80;
/// Complete encoded frame for the largest EEPROM read response.
///
/// Header and length (4), command and declared length (4), offset, length and
/// pad (4), the data, the trailer (2), and the footer (2).
pub const EEPROM_RESPONSE_FRAME_BYTES: usize = 4 + 8 + MAX_EEPROM_READ_BYTES + 2 + 2;
/// Declared length of a read request: offset, length, pad, and session word.
const READ_EEPROM_DECLARED_BYTES: u16 = 8;
/// Encoded read-request body: twelve payload bytes plus two CRC bytes.
const READ_EEPROM_BODY_BYTES: usize = 14;
/// Session word every observed host request carries.
const SESSION_WORD: u32 = 0x6457_396A;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Header { previous_was_header: bool },
    Length { first: Option<u8> },
    Body { index: usize, size: usize },
    Footer { first: Option<u8>, size: usize },
    Discard { remaining: usize },
}

/// Bounded K5 normal-mode envelope decoder for hello, common, and EEPROM-read
/// requests.
#[derive(Clone, Copy, Debug)]
pub struct NormalService {
    state: State,
    body: [u8; READ_EEPROM_BODY_BYTES],
}

impl NormalService {
    /// Creates a decoder waiting for a normal-mode frame header.
    pub const fn new() -> Self {
        Self {
            state: State::Header {
                previous_was_header: false,
            },
            body: [0; READ_EEPROM_BODY_BYTES],
        }
    }

    /// Consumes one byte and returns a complete validated request.
    pub fn push(&mut self, byte: u8) -> Option<NormalRequest> {
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
                // The frame length counts the payload; the body adds its CRC.
                let size = declared + 2;
                self.state = if size == REQUEST_BODY_BYTES || size == READ_EEPROM_BODY_BYTES {
                    State::Body { index: 0, size }
                } else {
                    State::Discard {
                        remaining: declared.saturating_add(4).min(276),
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
                    return None;
                }
                if size == READ_EEPROM_BODY_BYTES {
                    return decode_read_eeprom(&mut self.body);
                }
                let mut body = [0_u8; REQUEST_BODY_BYTES];
                body.copy_from_slice(&self.body[..REQUEST_BODY_BYTES]);
                let command = radio_platform::serial::decode_command(&mut body)?;
                return decode_normal_command(command);
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

impl Default for NormalService {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates one read-EEPROM body: CRC, declared length, session word, and a
/// length the response can carry. The address range is the driver's to check.
fn decode_read_eeprom(body: &mut [u8; READ_EEPROM_BODY_BYTES]) -> Option<NormalRequest> {
    xor(body);
    let (payload, crc) = body.split_at(12);
    if radio_platform::serial::crc16_xmodem(payload) != u16::from_le_bytes([crc[0], crc[1]]) {
        return None;
    }
    let command = u16::from_le_bytes([payload[0], payload[1]]);
    let declared = u16::from_le_bytes([payload[2], payload[3]]);
    let session = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    let length = payload[6];
    if command != READ_EEPROM_COMMAND
        || declared != READ_EEPROM_DECLARED_BYTES
        || session != SESSION_WORD
        || length == 0
        || usize::from(length) > MAX_EEPROM_READ_BYTES
    {
        return None;
    }
    Some(NormalRequest::ReadEeprom {
        offset: u16::from_le_bytes([payload[4], payload[5]]),
        length,
    })
}

/// Encodes the stock `0x051C` response to one EEPROM read and returns its size.
///
/// The payload echoes the offset and length before the data, which is what a
/// host checks every block against.
pub fn encode_eeprom_response(
    offset: u16,
    data: &[u8],
    frame: &mut [u8; EEPROM_RESPONSE_FRAME_BYTES],
) -> usize {
    let length = data.len().min(MAX_EEPROM_READ_BYTES);
    let payload_bytes = 8 + length;
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&u16::try_from(payload_bytes).unwrap_or(0).to_le_bytes());
    let payload = &mut frame[4..4 + payload_bytes];
    payload[0..2].copy_from_slice(&READ_EEPROM_RESPONSE_COMMAND.to_le_bytes());
    payload[2..4].copy_from_slice(&u16::try_from(length + 4).unwrap_or(0).to_le_bytes());
    payload[4..6].copy_from_slice(&offset.to_le_bytes());
    payload[6] = u8::try_from(length).unwrap_or(0);
    payload[8..].copy_from_slice(&data[..length]);
    let trailer = 4 + payload_bytes;
    frame[trailer..trailer + 2].copy_from_slice(&0xFFFF_u16.to_le_bytes());
    xor(&mut frame[4..trailer + 2]);
    frame[trailer + 2..trailer + 4].copy_from_slice(&[0xDC, 0xBA]);
    trailer + 4
}

/// Serves one validated EEPROM read: reads the block and encodes the response.
///
/// Returns the frame length, or `None` when the driver refused the range or the
/// device did not acknowledge, in which case nothing is sent and the host's
/// read times out rather than receiving data it did not ask for.
pub fn serve_eeprom_read<B: crate::eeprom::Bus>(
    bus: &mut B,
    offset: u16,
    length: u8,
    frame: &mut [u8; EEPROM_RESPONSE_FRAME_BYTES],
) -> Option<usize> {
    let mut data = [0_u8; MAX_EEPROM_READ_BYTES];
    let data = data.get_mut(..usize::from(length))?;
    crate::eeprom::read(bus, offset, data).ok()?;
    Some(encode_eeprom_response(offset, data, frame))
}

fn decode_normal_command(command: u16) -> Option<NormalRequest> {
    if command == 0x0514 {
        return Some(NormalRequest::Hello);
    }
    if command == COMMON_VIEW_COMMAND {
        return Some(NormalRequest::Common(shared_wire::Request::View));
    }
    if command == COMMON_CAPABILITIES_COMMAND {
        return Some(NormalRequest::Common(shared_wire::Request::Capabilities));
    }
    if command == COMMON_TOGGLE_AUDIO_COMMAND {
        return Some(NormalRequest::Common(shared_wire::Request::ToggleAudio));
    }
    let code = command.checked_sub(COMMON_KEY_FIRST_COMMAND)?;
    let key = match code {
        1 => shared_wire::KeyCode::Side2,
        4 => shared_wire::KeyCode::Up,
        5 => shared_wire::KeyCode::Down,
        _ => return None,
    };
    Some(NormalRequest::Common(shared_wire::Request::Key(key)))
}

/// Encodes one common response in the observed fixed K5 response envelope.
pub fn encode_common_response(
    result: SerialResult,
    request: shared_wire::Request,
    frame: &mut [u8; RESPONSE_FRAME_BYTES],
) -> Result<(), SharedWireError> {
    let mut common = [0_u8; shared_wire::MAX_RESPONSE_BYTES];
    let length = match result {
        SerialResult::Effects(_) => return Err(SharedWireError::UnsupportedResult),
        result => shared_wire::encode_response(result, &mut common)?,
    };
    if matches!(request, shared_wire::Request::Key(_)) {
        return Err(SharedWireError::UnsupportedResult);
    }
    encode_common_payload(&common[..length], frame);
    Ok(())
}

/// Encodes the bounded view response to a common state-changing command.
///
/// Both logical keys and the explicit audio command return the same validated
/// view payload; the caller does not need a second response envelope.
pub fn encode_common_view_response(
    view: radio_platform::receive_app::View,
    request: shared_wire::Request,
    frame: &mut [u8; RESPONSE_FRAME_BYTES],
) -> usize {
    if !matches!(
        request,
        shared_wire::Request::Key(_) | shared_wire::Request::ToggleAudio
    ) {
        return 0;
    }
    let mut common = [0_u8; shared_wire::MAX_RESPONSE_BYTES];
    let Ok(length) = shared_wire::encode_key_response(view, &mut common) else {
        return 0;
    };
    encode_common_payload(&common[..length], frame);
    RESPONSE_FRAME_BYTES
}

fn encode_common_payload(common: &[u8], frame: &mut [u8; RESPONSE_FRAME_BYTES]) {
    frame.fill(0);
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(&40_u16.to_le_bytes());
    let payload = &mut frame[4..44];
    payload[0..2].copy_from_slice(&COMMON_RESPONSE_COMMAND.to_le_bytes());
    // The inner length is the number of response data bytes after the
    // command/length words.  This is the same convention as the existing
    // hello/read envelope: it is not the command plus header size.
    payload[2..4].copy_from_slice(&u16::try_from(common.len()).unwrap_or(0).to_le_bytes());
    payload[4..4 + common.len()].copy_from_slice(common);
    frame[44..46].copy_from_slice(&0xFFFF_u16.to_le_bytes());
    xor(&mut frame[4..46]);
    frame[46..48].copy_from_slice(&[0xDC, 0xBA]);
}

fn xor(bytes: &mut [u8]) {
    const KEY: [u8; 16] = [
        0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9,
        0x80,
    ];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte ^= KEY[index % KEY.len()];
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        apply_shared_wire_request, encode_common_response, encode_common_view_response,
        encode_eeprom_response, NormalRequest, NormalService, COMMON_RESPONSE_COMMAND,
        EEPROM_RESPONSE_FRAME_BYTES,
    };
    use radio_platform::receive_app::{ReceiveApp, View};
    use radio_platform::serial_control::{wire, Capabilities, Result as SerialResult};
    use std::vec;

    fn crc16(bytes: &[u8]) -> u16 {
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

    fn request(command: u16) -> [u8; 16] {
        let mut frame = [0_u8; 16];
        frame[..4].copy_from_slice(&[0xAB, 0xCD, 8, 0]);
        frame[4..6].copy_from_slice(&command.to_le_bytes());
        frame[6..8].copy_from_slice(&4_u16.to_le_bytes());
        frame[8..12].copy_from_slice(&0x6457_396A_u32.to_le_bytes());
        let crc = crc16(&frame[4..12]);
        frame[12..14].copy_from_slice(&crc.to_le_bytes());
        super::xor(&mut frame[4..14]);
        frame[14..].copy_from_slice(&[0xDC, 0xBA]);
        frame
    }

    fn read_eeprom_request(offset: u16, length: u8, session: u32) -> std::vec::Vec<u8> {
        let mut frame = vec![0_u8; 20];
        frame[..4].copy_from_slice(&[0xAB, 0xCD, 12, 0]);
        frame[4..6].copy_from_slice(&0x051B_u16.to_le_bytes());
        frame[6..8].copy_from_slice(&8_u16.to_le_bytes());
        frame[8..10].copy_from_slice(&offset.to_le_bytes());
        frame[10] = length;
        frame[12..16].copy_from_slice(&session.to_le_bytes());
        let crc = crc16(&frame[4..16]);
        frame[16..18].copy_from_slice(&crc.to_le_bytes());
        super::xor(&mut frame[4..18]);
        frame[18..].copy_from_slice(&[0xDC, 0xBA]);
        frame
    }

    fn decode(stream: &[u8]) -> Option<NormalRequest> {
        let mut service = NormalService::new();
        let mut seen = None;
        for byte in stream {
            seen = service.push(*byte).or(seen);
        }
        seen
    }

    #[test]
    fn the_stock_eeprom_read_request_is_decoded_and_bounded() {
        assert_eq!(
            decode(&read_eeprom_request(0x1F80, 0x80, 0x6457_396A)),
            Some(NormalRequest::ReadEeprom {
                offset: 0x1F80,
                length: 0x80
            })
        );
        // Lengths the response cannot carry, and a foreign session, are dropped.
        assert_eq!(decode(&read_eeprom_request(0, 0, 0x6457_396A)), None);
        assert_eq!(decode(&read_eeprom_request(0, 0x81, 0x6457_396A)), None);
        assert_eq!(decode(&read_eeprom_request(0, 0x80, 0x1234_5678)), None);
        let mut corrupt = read_eeprom_request(0, 0x80, 0x6457_396A);
        corrupt[16] ^= 1;
        assert_eq!(decode(&corrupt), None);

        // A hello still decodes after a read, on the same service.
        let mut stream = read_eeprom_request(0x40, 8, 0x6457_396A);
        stream.extend(request(0x0514));
        let mut service = NormalService::new();
        let seen: std::vec::Vec<_> = stream
            .iter()
            .filter_map(|byte| service.push(*byte))
            .collect();
        assert_eq!(
            seen,
            [
                NormalRequest::ReadEeprom {
                    offset: 0x40,
                    length: 8
                },
                NormalRequest::Hello
            ]
        );
    }

    #[test]
    fn the_eeprom_response_is_the_stock_051c_envelope() {
        let data: std::vec::Vec<u8> = (0..0x80).collect();
        let mut frame = [0_u8; EEPROM_RESPONSE_FRAME_BYTES];
        let length = encode_eeprom_response(0x0100, &data, &mut frame);
        assert_eq!(length, EEPROM_RESPONSE_FRAME_BYTES);
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 0x88, 0x00]);
        assert_eq!(&frame[length - 2..length], &[0xDC, 0xBA]);
        super::xor(&mut frame[4..length - 2]);
        assert_eq!(&frame[4..8], &[0x1C, 0x05, 0x84, 0x00]);
        assert_eq!(&frame[8..12], &[0x00, 0x01, 0x80, 0x00]);
        assert_eq!(&frame[12..12 + 0x80], data.as_slice());
        assert_eq!(&frame[length - 4..length - 2], &[0xFF, 0xFF]);

        // A short read is framed at its own size.
        let short = encode_eeprom_response(8, &data[..8], &mut frame);
        assert_eq!(short, 4 + 8 + 8 + 4);
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 16, 0]);
    }

    #[test]
    fn command_assignment_decodes_common_view_capabilities_and_key() {
        let mut service = NormalService::new();
        let mut seen = None;
        for byte in request(0x0600) {
            seen = service.push(byte).or(seen);
        }
        assert_eq!(seen, Some(NormalRequest::Common(wire::Request::View)));

        let mut service = NormalService::new();
        let mut seen = None;
        for byte in request(0x0601) {
            seen = service.push(byte).or(seen);
        }
        assert_eq!(
            seen,
            Some(NormalRequest::Common(wire::Request::Capabilities))
        );

        let mut service = NormalService::new();
        let mut seen = None;
        for byte in request(0x0606) {
            seen = service.push(byte).or(seen);
        }
        assert_eq!(
            seen,
            Some(NormalRequest::Common(wire::Request::Key(wire::KeyCode::Up)))
        );
    }

    #[test]
    fn malformed_or_reserved_command_recovers_at_the_next_header() {
        let mut service = NormalService::new();
        let mut stream = vec![0xAB, 0xCD, 0x40, 0x00];
        stream.extend([0_u8; 68]);
        stream.extend(request(0x0615));
        let mut seen = None;
        for byte in stream {
            seen = service.push(byte).or(seen);
        }
        assert_eq!(
            seen,
            Some(NormalRequest::Common(wire::Request::ToggleAudio))
        );

        let mut service = NormalService::new();
        let mut seen = None;
        for byte in request(0x0616) {
            seen = service.push(byte).or(seen);
        }
        assert_eq!(seen, None);
    }

    #[test]
    fn unsupported_shell_keys_are_rejected_before_receive_app_dispatch() {
        let unsupported = [0x0602, 0x0604, 0x0608, 0x0613, 0x0614];
        for command in unsupported {
            let mut service = NormalService::new();
            let mut seen = None;
            for byte in request(command) {
                seen = service.push(byte).or(seen);
            }
            assert_eq!(seen, None, "unsupported command {command:#06x}");
        }
    }

    #[test]
    fn bad_crc_and_footer_are_discarded_without_mutating_the_shared_app() {
        let mut app = ReceiveApp::new();
        let before = app.view();

        let mut bad_crc = request(0x0606);
        bad_crc[12] ^= 0x01;
        let mut service = NormalService::new();
        for byte in bad_crc {
            assert_eq!(service.push(byte), None);
        }
        assert_eq!(app.view(), before);

        let mut bad_footer = request(0x0606);
        bad_footer[15] = 0;
        let mut service = NormalService::new();
        for byte in bad_footer {
            assert_eq!(service.push(byte), None);
        }
        assert_eq!(app.view(), before);

        // Only a validated command is allowed to cross into ReceiveApp.  This
        // also proves a malformed frame cannot be retried as an application
        // event by the target adapter.
        let mut service = NormalService::new();
        let mut seen = None;
        for byte in request(0x0606) {
            seen = service.push(byte).or(seen);
        }
        let Some(NormalRequest::Common(common)) = seen else {
            panic!("valid command was lost after malformed frames");
        };
        let result = apply_shared_wire_request(&mut app, common);
        assert!(matches!(result, SerialResult::Effects(_)));
        assert_ne!(app.view(), before);
    }

    #[test]
    fn common_response_is_exactly_the_existing_fixed_k5_envelope() {
        let view = View {
            channel: 7,
            audio: true,
            monitoring: false,
            squelch_open: false,
            receiver_ok: true,
        };
        let mut frame = [0_u8; 48];
        encode_common_response(SerialResult::View(view), wire::Request::View, &mut frame)
            .expect("view response");
        assert_eq!(&frame[..4], &[0xAB, 0xCD, 40, 0]);
        assert_eq!(&frame[46..], &[0xDC, 0xBA]);
        super::xor(&mut frame[4..46]);
        assert_eq!(&frame[4..6], &COMMON_RESPONSE_COMMAND.to_le_bytes());
        assert_eq!(&frame[6..8], &[4, 0]);
        assert_eq!(&frame[8..12], &[0x81, 7, 1, 2]);

        let mut key_frame = [0_u8; 48];
        assert_eq!(
            encode_common_view_response(
                view,
                wire::Request::Key(wire::KeyCode::Up),
                &mut key_frame,
            ),
            48
        );
        super::xor(&mut key_frame[4..46]);
        assert_eq!(&key_frame[8..12], &[0x83, 7, 1, 2]);
        assert_eq!(
            Capabilities::shared_receive().bits(),
            0x0007,
            "the K5 capability response remains the common receive boundary"
        );
    }
}
