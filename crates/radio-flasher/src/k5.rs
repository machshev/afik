//! Bounded, read-only common-application control for the K5 normal envelope.
//!
//! The target envelope is deliberately kept here, beside the existing K5
//! flasher codec, rather than in the shared application crate.  A request is
//! one fixed-session exchange: the caller sends exactly one command and reads
//! exactly one response.  No retry, EEPROM operation, persistence operation,
//! flash operation, or transmit operation is reachable from this module.

use std::io::{Read, Write};

use crate::{codec::receive_packet, codec::send_packet, workflow::FlashError};

/// Fixed session word accepted by the observed K5 normal-mode envelope.
pub const K5_SESSION_WORD: u32 = 0x6457_396A;
/// Command returned by all common K5 application responses.
pub const K5_COMMON_RESPONSE_COMMAND: u16 = 0x0620;
/// Common K5 semantic-view query command.
pub const K5_VIEW_COMMAND: u16 = 0x0600;
/// Common K5 capability query command.
pub const K5_CAPABILITIES_COMMAND: u16 = 0x0601;
/// Common K5 explicit audio-toggle command.
pub const K5_TOGGLE_AUDIO_COMMAND: u16 = 0x0615;
/// Common K5 Up key command.
pub const K5_UP_COMMAND: u16 = 0x0606;
/// Common K5 Down key command.
pub const K5_DOWN_COMMAND: u16 = 0x0607;

const K5_REQUEST_PAYLOAD_BYTES: usize = 8;
const K5_RESPONSE_PAYLOAD_BYTES: usize = 40;
const K5_RESPONSE_DATA_OFFSET: usize = 4;

/// One safe common command available in K5 normal mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlRequest {
    /// Read the shared semantic receive view.
    View,
    /// Read the target's shared application capability bits.
    Capabilities,
    /// Toggle application audio explicitly; this is not the shell Menu key.
    ToggleAudio,
    /// Apply one logical key through the shared receive application.
    Key(LogicalKey),
}

/// The bounded logical-key subset exposed by the host K5 control command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogicalKey {
    /// Shared Up key, which selects the previous channel.
    Up,
    /// Shared Down key, which selects the next channel.
    Down,
}

/// Shared semantic view returned by K5.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct View {
    /// PMR446 example channel number selected by the application.
    pub channel: u8,
    /// Whether application audio is enabled.
    pub audio: bool,
    /// Whether the sampled squelch is open.
    pub squelch_open: bool,
    /// Whether the operator monitor override is active.
    pub monitoring: bool,
    /// Whether the receiver is in a healthy state.
    pub receiver_ok: bool,
}

/// One validated response to a K5 common command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlResponse {
    /// Semantic view, returned by View and Key commands.
    View(View),
    /// Shared application capability bitset.
    Capabilities(u16),
}

/// Sends exactly one K5 normal-mode common command and validates its response.
///
/// [`send_packet`] validates and emits the fixed-session request CRC and
/// footer. [`receive_packet`] bounds synchronisation, checks the response
/// footer and decoded device marker, and validates the response CRC trailer.
/// The remaining checks below validate the exact `0x0620` response command,
/// inner length, reserved bytes, and common payload.
pub fn control<T: Read + Write>(
    transport: &mut T,
    request: ControlRequest,
) -> Result<ControlResponse, FlashError> {
    let command = request.command();
    let mut payload = [0_u8; K5_REQUEST_PAYLOAD_BYTES];
    payload[0..2].copy_from_slice(&command.to_le_bytes());
    payload[2..4].copy_from_slice(&4_u16.to_le_bytes());
    payload[4..8].copy_from_slice(&K5_SESSION_WORD.to_le_bytes());
    send_packet(transport, &payload)?;

    let response = receive_packet(transport)?;
    parse_response(&response, request)
}

impl ControlRequest {
    const fn command(self) -> u16 {
        match self {
            Self::View => K5_VIEW_COMMAND,
            Self::Capabilities => K5_CAPABILITIES_COMMAND,
            Self::ToggleAudio => K5_TOGGLE_AUDIO_COMMAND,
            Self::Key(LogicalKey::Up) => K5_UP_COMMAND,
            Self::Key(LogicalKey::Down) => K5_DOWN_COMMAND,
        }
    }
}

fn parse_response(
    packet: &crate::Packet,
    request: ControlRequest,
) -> Result<ControlResponse, FlashError> {
    let payload = packet.as_slice();
    if payload.len() != K5_RESPONSE_PAYLOAD_BYTES {
        return Err(FlashError::UnexpectedPacket {
            expected: "K5 common response payload",
            command: payload_command(payload),
            length: payload.len(),
        });
    }
    let command = u16::from_le_bytes([payload[0], payload[1]]);
    if command != K5_COMMON_RESPONSE_COMMAND {
        return Err(FlashError::UnexpectedPacket {
            expected: "K5 common response command 0x0620",
            command: Some(command),
            length: payload.len(),
        });
    }
    let length = usize::from(u16::from_le_bytes([payload[2], payload[3]]));
    let expected_length = match request {
        ControlRequest::Capabilities => 3,
        ControlRequest::View | ControlRequest::Key(_) | ControlRequest::ToggleAudio => 4,
    };
    if length != expected_length {
        return Err(FlashError::UnexpectedPacket {
            expected: "K5 common response inner length",
            command: Some(command),
            length,
        });
    }
    if payload[K5_RESPONSE_DATA_OFFSET + length..]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err(FlashError::InvalidK5Response("reserved response bytes"));
    }
    let data = &payload[K5_RESPONSE_DATA_OFFSET..K5_RESPONSE_DATA_OFFSET + length];
    match request {
        ControlRequest::Capabilities => parse_capabilities(data),
        ControlRequest::View => parse_view(data, 0x81),
        ControlRequest::Key(_) | ControlRequest::ToggleAudio => {
            let response = parse_view(data, 0x83)?;
            if matches!(
                response,
                ControlResponse::View(View {
                    receiver_ok: false,
                    ..
                })
            ) {
                return Err(FlashError::K5StateChangeRejected);
            }
            Ok(response)
        }
    }
}

fn payload_command(payload: &[u8]) -> Option<u16> {
    (payload.len() >= 2).then(|| u16::from_le_bytes([payload[0], payload[1]]))
}

fn parse_view(data: &[u8], expected_code: u8) -> Result<ControlResponse, FlashError> {
    if data.len() != 4 || data[0] != expected_code {
        return Err(FlashError::InvalidK5Response("view response payload"));
    }
    if data[2] > 1 || data[3] & !0x07 != 0 {
        return Err(FlashError::InvalidK5Response("view response flags"));
    }
    Ok(ControlResponse::View(View {
        channel: data[1],
        audio: data[2] != 0,
        squelch_open: data[3] & 1 != 0,
        monitoring: data[3] & 4 != 0,
        receiver_ok: data[3] & 2 != 0,
    }))
}

fn parse_capabilities(data: &[u8]) -> Result<ControlResponse, FlashError> {
    if data.len() != 3 || data[0] != 0x82 {
        return Err(FlashError::InvalidK5Response("capability response payload"));
    }
    let bits = u16::from_le_bytes([data[1], data[2]]);
    if bits & !0x01FF != 0 {
        return Err(FlashError::InvalidK5Response("unknown capability bits"));
    }
    Ok(ControlResponse::Capabilities(bits))
}

#[cfg(test)]
mod tests {
    use super::{control, ControlRequest, ControlResponse, LogicalKey, View};
    use crate::FlashError;
    use std::io::{self, Cursor, Read, Write};

    const XOR_KEY: [u8; 16] = [
        0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9,
        0x80,
    ];

    fn response(data: &[u8]) -> Vec<u8> {
        let mut payload = [0_u8; 40];
        payload[0..2].copy_from_slice(&0x0620_u16.to_le_bytes());
        payload[2..4].copy_from_slice(&u16::try_from(data.len()).unwrap().to_le_bytes());
        payload[4..4 + data.len()].copy_from_slice(data);
        let mut frame = vec![0_u8; 48];
        frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
        frame[2..4].copy_from_slice(&40_u16.to_le_bytes());
        frame[4..44].copy_from_slice(&payload);
        frame[44..46].copy_from_slice(&0xFFFF_u16.to_le_bytes());
        for (index, byte) in frame[4..46].iter_mut().enumerate() {
            *byte ^= XOR_KEY[index % XOR_KEY.len()];
        }
        frame[46..].copy_from_slice(&[0xDC, 0xBA]);
        frame
    }

    #[derive(Debug)]
    struct Radio {
        input: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl Radio {
        fn new(response: Vec<u8>) -> Self {
            Self {
                input: Cursor::new(response),
                writes: Vec::new(),
            }
        }
    }

    impl Read for Radio {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            self.input.read(output)
        }
    }

    impl Write for Radio {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            self.writes.extend_from_slice(input);
            Ok(input.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn view_request_uses_one_fixed_session_exchange_and_decodes_exact_view() {
        let mut radio = Radio::new(response(&[0x81, 7, 1, 2]));
        assert_eq!(
            control(&mut radio, ControlRequest::View).unwrap(),
            ControlResponse::View(View {
                channel: 7,
                audio: true,
                squelch_open: false,
                monitoring: false,
                receiver_ok: true,
            })
        );
        assert_eq!(radio.writes.len(), 16);
        let request = &radio.writes;
        assert_eq!(&request[..4], &[0xAB, 0xCD, 8, 0]);
        let mut decoded = request[4..14].to_vec();
        for (index, byte) in decoded.iter_mut().enumerate() {
            *byte ^= XOR_KEY[index % XOR_KEY.len()];
        }
        assert_eq!(&decoded[..2], &0x0600_u16.to_le_bytes());
        assert_eq!(&decoded[2..4], &4_u16.to_le_bytes());
        assert_eq!(&decoded[4..8], &0x6457_396A_u32.to_le_bytes());
        assert_eq!(&request[14..], &[0xDC, 0xBA]);
    }

    #[test]
    fn capabilities_and_logical_keys_use_expected_commands() {
        for (request, command, data) in [
            (ControlRequest::Capabilities, 0x0601, &[0x82, 7, 0][..]),
            (ControlRequest::ToggleAudio, 0x0615, &[0x83, 1, 1, 2][..]),
            (
                ControlRequest::Key(LogicalKey::Up),
                0x0606,
                &[0x83, 2, 1, 2][..],
            ),
            (
                ControlRequest::Key(LogicalKey::Down),
                0x0607,
                &[0x83, 3, 1, 2][..],
            ),
        ] {
            let mut radio = Radio::new(response(data));
            let _ = control(&mut radio, request).unwrap();
            let mut decoded = radio.writes[4..14].to_vec();
            for (index, byte) in decoded.iter_mut().enumerate() {
                *byte ^= XOR_KEY[index % XOR_KEY.len()];
            }
            assert_eq!(u16::from_le_bytes([decoded[0], decoded[1]]), command);
        }
    }

    #[test]
    fn malformed_response_crc_footer_command_and_payload_never_decode() {
        let valid = response(&[0x81, 1, 1, 0]);
        for (name, index, value) in [
            ("crc", 40, 1),
            ("footer", 47, 0),
            ("command", 4, 1),
            ("reserved", 12, 1),
        ] {
            let mut frame = valid.clone();
            if name == "footer" {
                frame[index] = value;
            } else {
                frame[index] ^= value;
            }
            let mut radio = Radio::new(frame);
            assert!(control(&mut radio, ControlRequest::View).is_err(), "{name}");
        }
    }

    #[test]
    fn wrong_length_and_unknown_flags_fail_without_a_second_request() {
        let mut frame = response(&[0x81, 1, 1, 8]);
        let mut radio = Radio::new(frame.clone());
        assert!(matches!(
            control(&mut radio, ControlRequest::View),
            Err(FlashError::InvalidK5Response("view response flags"))
        ));
        frame = response(&[0x82, 7, 0]);
        let mut radio = Radio::new(frame);
        assert!(control(&mut radio, ControlRequest::View).is_err());
    }

    #[test]
    fn state_change_fault_view_is_a_typed_host_failure_but_query_reports_it() {
        let mut radio = Radio::new(response(&[0x83, 1, 1, 0]));
        assert!(matches!(
            control(&mut radio, ControlRequest::ToggleAudio),
            Err(FlashError::K5StateChangeRejected)
        ));

        let mut radio = Radio::new(response(&[0x81, 1, 1, 0]));
        assert_eq!(
            control(&mut radio, ControlRequest::View).unwrap(),
            ControlResponse::View(View {
                channel: 1,
                audio: true,
                squelch_open: false,
                monitoring: false,
                receiver_ok: false,
            })
        );
    }

    #[test]
    fn view_and_key_response_opcodes_are_not_interchangeable() {
        let mut radio = Radio::new(response(&[0x83, 1, 1, 0]));
        assert!(control(&mut radio, ControlRequest::View).is_err());
        let mut radio = Radio::new(response(&[0x81, 1, 1, 0]));
        assert!(control(&mut radio, ControlRequest::ToggleAudio).is_err());
    }
}
