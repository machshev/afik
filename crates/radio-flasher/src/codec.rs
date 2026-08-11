use std::io::{self, Read, Write};

use crate::workflow::FlashError;

pub(crate) const MAX_PACKET_BYTES: usize = 272;
const MAX_FRAME_BYTES: usize = MAX_PACKET_BYTES + 8;
const MAX_RESPONSE_BODY_BYTES: usize = MAX_PACKET_BYTES + 2;
const MAX_SYNC_BYTES: usize = 1_024;
const MAX_EMPTY_READS: usize = 20;
const XOR_KEY: [u8; 16] = [
    0x16, 0x6C, 0x14, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5, 0x40, 0x13, 0x03, 0xE9, 0x80,
];

/// A decoded legacy serial payload with bounded storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Packet {
    bytes: [u8; MAX_PACKET_BYTES],
    len: usize,
}

impl Packet {
    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, FlashError> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(FlashError::PacketTooLarge(bytes.len()));
        }
        let mut packet = Self {
            bytes: [0; MAX_PACKET_BYTES],
            len: bytes.len(),
        };
        packet.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(packet)
    }

    /// Returns the decoded payload bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Encodes and writes one legacy packet with the observed CRC/XOR envelope.
///
/// # Panics
///
/// This cannot panic for a payload accepted by the bounded packet-size check;
/// the conversion is guarded by the same fixed maximum.
pub fn send_packet<T: Write>(transport: &mut T, payload: &[u8]) -> Result<(), FlashError> {
    if payload.len() > MAX_PACKET_BYTES {
        return Err(FlashError::PacketTooLarge(payload.len()));
    }
    let mut frame = [0_u8; MAX_FRAME_BYTES];
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    let length = u16::try_from(payload.len()).expect("bounded packet length");
    frame[2..4].copy_from_slice(&length.to_le_bytes());
    frame[4..4 + payload.len()].copy_from_slice(payload);
    let crc = crc16_xmodem(payload);
    frame[4 + payload.len()..6 + payload.len()].copy_from_slice(&crc.to_le_bytes());
    xor(&mut frame[4..6 + payload.len()]);
    frame[6 + payload.len()..8 + payload.len()].copy_from_slice(&[0xDC, 0xBA]);
    transport.write_all(&frame[..payload.len() + 8])?;
    transport.flush()?;
    Ok(())
}

/// Decoded trailer produced by a literal on-wire `FF FF` at this payload length.
///
/// A radio in bootloader mode does not include its trailer in the obfuscation
/// stream, so the marker arrives as two literal `0xFF` bytes and deobfuscation
/// turns them into this length-dependent value. `EVID-K5-013` records the
/// observation on two units and two bootloader versions, and K5TOOL derives the
/// same value from the same key table.
fn literal_marker(length: usize) -> u16 {
    let low = XOR_KEY[length % XOR_KEY.len()] ^ 0xFF;
    let high = XOR_KEY[(length + 1) % XOR_KEY.len()] ^ 0xFF;
    u16::from_le_bytes([low, high])
}

/// Whether a decoded trailer is one of the two observed device markers.
///
/// The trailer is a marker rather than a checksum, and which form arrives
/// depends on the mode the radio is in: normal firmware obfuscates it, so it
/// decodes to `0xFFFF`, while a bootloader sends it literally. Both are
/// accepted because a caller reading a frame does not always know the mode, and
/// neither form carries information a checksum would.
pub(crate) fn is_device_marker(decoded: u16, length: usize) -> bool {
    decoded == 0xFFFF || decoded == literal_marker(length)
}

/// Reads and decodes one complete legacy packet with bounded resynchronisation.
pub fn receive_packet<T: Read>(transport: &mut T) -> Result<Packet, FlashError> {
    let (packet, response_crc) = receive_packet_with_response_crc(transport)?;
    if !is_device_marker(response_crc, packet.as_slice().len()) {
        return Err(FlashError::InvalidResponseCrc(response_crc));
    }
    Ok(packet)
}

/// Reads a device packet without interpreting its decoded trailer.
///
/// The K1 command paths validate the complete envelope and payload structure and
/// then apply their own command-specific checks, so they have no use for the
/// marker. This is not a K1 peculiarity: `EVID-K5-013` records that every
/// observed bootloader sends the literal form, and that the exemption here
/// happened to be the only reason the K1 path worked.
pub(crate) fn receive_packet_without_response_crc<T: Read>(
    transport: &mut T,
) -> Result<Packet, FlashError> {
    receive_packet_with_response_crc(transport).map(|(packet, _)| packet)
}

pub(crate) fn receive_packet_with_response_crc<T: Read>(
    transport: &mut T,
) -> Result<(Packet, u16), FlashError> {
    find_header(transport)?;
    let length_low = read_byte(transport)?;
    let length_high = read_byte(transport)?;
    let length = usize::from(u16::from_le_bytes([length_low, length_high]));
    if length > MAX_PACKET_BYTES {
        return Err(FlashError::PacketTooLarge(length));
    }

    let mut encoded = [0_u8; MAX_RESPONSE_BODY_BYTES];
    read_complete(transport, &mut encoded[..length + 2])?;
    let footer = [read_byte(transport)?, read_byte(transport)?];
    if footer != [0xDC, 0xBA] {
        return Err(FlashError::InvalidFooter(footer));
    }
    xor(&mut encoded[..length + 2]);
    let response_crc = u16::from_le_bytes([encoded[length], encoded[length + 1]]);
    let packet = Packet::from_slice(&encoded[..length])?;
    Ok((packet, response_crc))
}

fn find_header<T: Read>(transport: &mut T) -> Result<(), FlashError> {
    let mut saw_ab = false;
    for _ in 0..MAX_SYNC_BYTES {
        let byte = read_byte(transport)?;
        if saw_ab && byte == 0xCD {
            return Ok(());
        }
        saw_ab = byte == 0xAB;
    }
    Err(FlashError::SyncLimit)
}

fn read_complete<T: Read>(transport: &mut T, buffer: &mut [u8]) -> Result<(), FlashError> {
    for byte in buffer {
        *byte = read_byte(transport)?;
    }
    Ok(())
}

fn read_byte<T: Read>(transport: &mut T) -> Result<u8, FlashError> {
    let mut byte = [0_u8; 1];
    let mut empty_reads = 0;
    loop {
        match transport.read(&mut byte) {
            Ok(0) => {
                empty_reads += 1;
                if empty_reads >= MAX_EMPTY_READS {
                    return Err(FlashError::Timeout);
                }
            }
            Ok(1) => return Ok(byte[0]),
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(FlashError::Io(error)),
        }
    }
}

pub(crate) fn crc16_xmodem(bytes: &[u8]) -> u16 {
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
pub(crate) fn encode_response(payload: &[u8]) -> Vec<u8> {
    encode_response_with_trailer(payload, 0xFFFF)
}

#[cfg(test)]
pub(crate) fn encode_response_with_trailer(payload: &[u8], trailer: u16) -> Vec<u8> {
    let mut frame = vec![0_u8; payload.len() + 8];
    frame[0..2].copy_from_slice(&[0xAB, 0xCD]);
    frame[2..4].copy_from_slice(
        &u16::try_from(payload.len())
            .expect("test response is bounded")
            .to_le_bytes(),
    );
    frame[4..4 + payload.len()].copy_from_slice(payload);
    frame[4 + payload.len()..6 + payload.len()].copy_from_slice(&trailer.to_le_bytes());
    xor(&mut frame[4..6 + payload.len()]);
    frame[6 + payload.len()..].copy_from_slice(&[0xDC, 0xBA]);
    frame
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        crc16_xmodem, encode_response, encode_response_with_trailer, receive_packet,
        receive_packet_without_response_crc, send_packet,
    };
    use crate::workflow::FlashError;

    const SOURCED_V2_BEACON: [u8; 44] = [
        0xAB, 0xCD, 0x24, 0x00, 0x0E, 0x69, 0x34, 0xE6, 0x2F, 0x93, 0x0F, 0x46, 0x3D, 0x66, 0x85,
        0x0A, 0x24, 0x44, 0x16, 0x8F, 0x9A, 0x6C, 0x47, 0xE6, 0x1C, 0xBF, 0x3D, 0x70, 0x0F, 0x05,
        0xE3, 0x40, 0x27, 0x09, 0xE9, 0x80, 0x16, 0x6C, 0x14, 0xC6, 0xD1, 0x6E, 0xDC, 0xBA,
    ];

    const CLEAR_V2_BEACON: [u8; 36] = [
        0x18, 0x05, 0x20, 0x00, 0x01, 0x02, 0x02, 0x06, 0x1C, 0x53, 0x50, 0x4A, 0x37, 0x47, 0xFF,
        0x0F, 0x8C, 0x00, 0x53, 0x00, 0x32, 0x2E, 0x30, 0x30, 0x2E, 0x30, 0x36, 0x00, 0x34, 0x0A,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x20,
    ];

    /// Exact wire frame captured from the `EVID-K5-013` UV-K6, bootloader
    /// `2.00.06`, with the identity field at offsets 4..20 zeroed because
    /// `EVID-K5-014` redacts it. Command, lengths, version, trailer and footer
    /// are as observed.
    const CAPTURED_K6_BEACON: [u8; 44] = [
        0xAB, 0xCD, 0x24, 0x00, 0x0E, 0x69, 0x34, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5,
        0x40, 0x13, 0x03, 0xE9, 0x80, 0x16, 0x6C, 0x14, 0xE6, 0x1C, 0xBF, 0x3D, 0x70, 0x0F, 0x05,
        0xE3, 0x40, 0x27, 0x09, 0xE9, 0x80, 0x16, 0x6C, 0x14, 0xC6, 0xFF, 0xFF, 0xDC, 0xBA,
    ];

    /// The same, captured from the `EVID-K5-012` UV-5R Plus, bootloader
    /// `4.00.01`, identically redacted.
    const CAPTURED_UV5R_BEACON: [u8; 44] = [
        0xAB, 0xCD, 0x24, 0x00, 0x0E, 0x69, 0x34, 0xE6, 0x2E, 0x91, 0x0D, 0x40, 0x21, 0x35, 0xD5,
        0x40, 0x13, 0x03, 0xE9, 0x80, 0x16, 0x6C, 0x14, 0xE6, 0x1A, 0xBF, 0x3D, 0x70, 0x0F, 0x05,
        0xE4, 0x40, 0x27, 0x09, 0xE9, 0x80, 0x16, 0x6C, 0x14, 0xC6, 0xFF, 0xFF, 0xDC, 0xBA,
    ];

    #[test]
    fn sourced_beacon_vector_decodes_exactly() {
        let packet = receive_packet(&mut Cursor::new(SOURCED_V2_BEACON)).unwrap();
        assert_eq!(packet.as_slice(), CLEAR_V2_BEACON);
    }

    #[test]
    fn a_literal_trailer_from_a_bootloader_is_accepted() {
        // Both units send `FF FF` unobfuscated, so the decoded value is
        // length-dependent rather than the running-mode marker. The frames
        // differ only in their version string.
        for frame in [CAPTURED_K6_BEACON, CAPTURED_UV5R_BEACON] {
            let packet = receive_packet(&mut Cursor::new(frame)).unwrap();
            assert_eq!(packet.as_slice().len(), 36);
            assert_eq!(&packet.as_slice()[0..2], &[0x18, 0x05]);
        }

        let versions = [CAPTURED_K6_BEACON, CAPTURED_UV5R_BEACON].map(|frame| {
            let packet = receive_packet(&mut Cursor::new(frame)).unwrap();
            packet.as_slice()[20..27].to_vec()
        });
        assert_eq!(versions[0], b"2.00.06");
        assert_eq!(versions[1], b"4.00.01");
    }

    #[test]
    fn the_literal_marker_is_the_value_measured_on_hardware() {
        // `EVID-K5-013`: a 36-byte payload puts the trailer at key offsets 4
        // and 5, which is the 0x6ED1 both radios produced and the value K5TOOL
        // derives from the same table.
        assert_eq!(super::literal_marker(36), 0x6ED1);
        assert!(super::is_device_marker(0x6ED1, 36));
        assert!(super::is_device_marker(0xFFFF, 36));
    }

    #[test]
    fn accepting_both_markers_does_not_accept_any_trailer() {
        assert!(!super::is_device_marker(0x1234, 36));
        // The literal marker is only valid at the length that produces it.
        assert!(!super::is_device_marker(0x6ED1, 34));

        let frame = encode_response_with_trailer(&CLEAR_V2_BEACON, 0x1234);
        assert!(matches!(
            receive_packet(&mut Cursor::new(frame)),
            Err(FlashError::InvalidResponseCrc(0x1234))
        ));
    }

    #[test]
    fn scanner_resynchronises_before_a_fragmented_response() {
        let mut input = vec![0x00, 0xAB, 0x00, 0xAB];
        input.extend(encode_response(&CLEAR_V2_BEACON));
        let packet = receive_packet(&mut Cursor::new(input)).unwrap();
        assert_eq!(packet.as_slice(), CLEAR_V2_BEACON);
    }

    #[test]
    fn request_encoding_has_exact_envelope_crc_and_footer() {
        let payload = [0x14, 0x05, 0x04, 0x00, 0x6A, 0x39, 0x57, 0x64];
        let mut encoded = Vec::new();
        send_packet(&mut encoded, &payload).unwrap();
        assert_eq!(&encoded[..4], &[0xAB, 0xCD, 0x08, 0x00]);
        assert_eq!(&encoded[encoded.len() - 2..], &[0xDC, 0xBA]);
        assert_eq!(encoded.len(), payload.len() + 8);
        assert_ne!(&encoded[4..12], &payload);
    }

    #[test]
    fn corrupt_crc_footer_and_oversize_fail_closed() {
        let mut corrupt_crc = encode_response(&CLEAR_V2_BEACON);
        corrupt_crc[40] ^= 1;
        assert!(matches!(
            receive_packet(&mut Cursor::new(corrupt_crc)),
            Err(FlashError::InvalidResponseCrc(_))
        ));

        let mut corrupt_footer = encode_response(&CLEAR_V2_BEACON);
        *corrupt_footer.last_mut().unwrap() = 0;
        assert!(matches!(
            receive_packet(&mut Cursor::new(corrupt_footer)),
            Err(FlashError::InvalidFooter(_))
        ));

        let oversized = [0xAB, 0xCD, 0x11, 0x01];
        assert!(matches!(
            receive_packet(&mut Cursor::new(oversized)),
            Err(FlashError::PacketTooLarge(273))
        ));
    }

    #[test]
    fn explicit_k1_path_decodes_a_frame_the_marker_check_now_also_accepts() {
        // This test previously asserted that 0x6ED1 was a non-marker trailer
        // only the K1 path would tolerate. `EVID-K5-013` shows it is what every
        // observed bootloader sends at this length, so the ordinary path accepts
        // it too and the explicit path remains equivalent for these frames.
        let frame = encode_response_with_trailer(&CLEAR_V2_BEACON, 0x6ED1);
        assert_eq!(
            receive_packet(&mut Cursor::new(frame.clone()))
                .unwrap()
                .as_slice(),
            CLEAR_V2_BEACON
        );
        assert_eq!(
            receive_packet_without_response_crc(&mut Cursor::new(frame))
                .unwrap()
                .as_slice(),
            CLEAR_V2_BEACON
        );
    }

    #[test]
    fn crc16_matches_xmodem_check_vector() {
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    }
}
