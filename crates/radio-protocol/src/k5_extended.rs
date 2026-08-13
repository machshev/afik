//! Simulator/host-only-shared shape for the future K5 extended normal mode.
//!
//! This codec is deliberately not wired into a target or advertised as a
//! capability. It carries one existing AFIK COBS/CRC frame without changing
//! the current `Hello`, common `0x0600..` controls, or object commands.

use crate::{decode_packet, ProtocolError, MAX_ENCODED_FRAME};

/// Fixed outer-envelope magic.
pub const K5_EXTENDED_MAGIC: [u8; 2] = *b"K5";
/// Envelope version.
pub const K5_EXTENDED_VERSION: u8 = 1;
/// Explicit command carrying one existing AFIK protocol frame.
pub const K5_EXTENDED_PROTOCOL_FRAME: u16 = 0x7001;
/// Fixed simulator session used by the explicit K5 adapter.
pub const K5_EXTENDED_SESSION: u16 = 0x4B35;
/// Bytes before the bounded inner frame.
pub const K5_EXTENDED_HEADER_LEN: usize = 9;
/// CRC-32 bytes after the inner frame.
pub const K5_EXTENDED_CRC_LEN: usize = 4;
/// Maximum outer envelope bytes.
pub const K5_EXTENDED_MAX_BYTES: usize =
    K5_EXTENDED_HEADER_LEN + MAX_ENCODED_FRAME + K5_EXTENDED_CRC_LEN;

/// One strictly decoded K5 extended envelope carrying a complete AFIK frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct K5ExtendedFrame<'a> {
    /// Session word checked by the adapter.
    pub session: u16,
    /// Explicit outer command.
    pub command: u16,
    /// Existing complete AFIK COBS/CRC frame.
    pub payload: &'a [u8],
}

/// Encodes exactly one complete bounded AFIK frame into the extended envelope.
pub fn encode_k5_extended(
    session: u16,
    payload: &[u8],
    output: &mut [u8; K5_EXTENDED_MAX_BYTES],
) -> Result<usize, ProtocolError> {
    if payload.len() > MAX_ENCODED_FRAME {
        return Err(ProtocolError::PayloadTooLarge);
    }
    validate_inner(payload)?;
    output[..2].copy_from_slice(&K5_EXTENDED_MAGIC);
    output[2] = K5_EXTENDED_VERSION;
    output[3..5].copy_from_slice(&K5_EXTENDED_PROTOCOL_FRAME.to_le_bytes());
    output[5..7].copy_from_slice(&session.to_le_bytes());
    output[7..9].copy_from_slice(&(u16::try_from(payload.len()).unwrap_or(u16::MAX)).to_le_bytes());
    output[K5_EXTENDED_HEADER_LEN..K5_EXTENDED_HEADER_LEN + payload.len()].copy_from_slice(payload);
    let end = K5_EXTENDED_HEADER_LEN + payload.len();
    let checksum = crc32(&output[..end]);
    output[end..end + K5_EXTENDED_CRC_LEN].copy_from_slice(&checksum.to_le_bytes());
    Ok(end + K5_EXTENDED_CRC_LEN)
}

/// Decodes and strictly validates one extended envelope.
pub fn decode_k5_extended(
    input: &[u8],
    expected_session: u16,
) -> Result<K5ExtendedFrame<'_>, ProtocolError> {
    if input.len() < K5_EXTENDED_HEADER_LEN + K5_EXTENDED_CRC_LEN {
        return Err(ProtocolError::FrameTooShort);
    }
    if input[..2] != K5_EXTENDED_MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    if input[2] != K5_EXTENDED_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let command = u16::from_le_bytes([input[3], input[4]]);
    if command != K5_EXTENDED_PROTOCOL_FRAME {
        return Err(ProtocolError::UnknownCommand);
    }
    let session = u16::from_le_bytes([input[5], input[6]]);
    if session != expected_session {
        return Err(ProtocolError::MalformedPayload);
    }
    let length = usize::from(u16::from_le_bytes([input[7], input[8]]));
    if length > MAX_ENCODED_FRAME {
        return Err(ProtocolError::PayloadTooLarge);
    }
    let end = K5_EXTENDED_HEADER_LEN
        .checked_add(length)
        .ok_or(ProtocolError::LengthMismatch)?;
    let crc_end = end
        .checked_add(K5_EXTENDED_CRC_LEN)
        .ok_or(ProtocolError::LengthMismatch)?;
    if input.len() != crc_end {
        return Err(ProtocolError::LengthMismatch);
    }
    let expected_crc =
        u32::from_le_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
    if crc32(&input[..end]) != expected_crc {
        return Err(ProtocolError::CrcMismatch);
    }
    validate_inner(&input[K5_EXTENDED_HEADER_LEN..end])?;
    Ok(K5ExtendedFrame {
        session,
        command,
        payload: &input[K5_EXTENDED_HEADER_LEN..end],
    })
}

fn validate_inner(payload: &[u8]) -> Result<(), ProtocolError> {
    let frame = payload
        .strip_suffix(&[0])
        .ok_or(ProtocolError::MalformedPayload)?;
    decode_packet(frame).map(|_| ())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let low_bit_mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & low_bit_mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_frame(payload: &[u8]) -> ([u8; MAX_ENCODED_FRAME], usize) {
        let frame = crate::Frame::new(
            crate::Service::Configuration,
            0,
            1,
            crate::Command::ListObjects,
            payload,
        )
        .unwrap();
        let mut encoded = [0; MAX_ENCODED_FRAME];
        let length = crate::encode_frame(&frame, &mut encoded).unwrap();
        (encoded, length)
    }

    #[test]
    fn max_frame_round_trips_and_trailing_is_rejected() {
        let payload = [0xA5; crate::MAX_PAYLOAD];
        let (encoded_frame, frame_length) = encoded_frame(&payload);
        let mut encoded = [0; K5_EXTENDED_MAX_BYTES];
        let length = encode_k5_extended(
            K5_EXTENDED_SESSION,
            &encoded_frame[..frame_length],
            &mut encoded,
        )
        .unwrap();
        assert_eq!(
            decode_k5_extended(&encoded[..length], K5_EXTENDED_SESSION)
                .unwrap()
                .payload,
            &encoded_frame[..frame_length]
        );
        let mut trailing = encoded[..length].to_vec();
        trailing.push(0);
        assert_eq!(
            decode_k5_extended(&trailing, K5_EXTENDED_SESSION),
            Err(ProtocolError::LengthMismatch)
        );
    }

    #[test]
    fn malformed_session_command_crc_and_oversize_fail_closed() {
        let (frame, frame_length) = encoded_frame(b"frame");
        let mut encoded = [0; K5_EXTENDED_MAX_BYTES];
        let length =
            encode_k5_extended(K5_EXTENDED_SESSION, &frame[..frame_length], &mut encoded).unwrap();
        assert_eq!(
            decode_k5_extended(&encoded[..length], 1),
            Err(ProtocolError::MalformedPayload)
        );
        encoded[3] ^= 1;
        assert_eq!(
            decode_k5_extended(&encoded[..length], K5_EXTENDED_SESSION),
            Err(ProtocolError::UnknownCommand)
        );
        let length =
            encode_k5_extended(K5_EXTENDED_SESSION, &frame[..frame_length], &mut encoded).unwrap();
        encoded[length - 1] ^= 1;
        assert_eq!(
            decode_k5_extended(&encoded[..length], K5_EXTENDED_SESSION),
            Err(ProtocolError::CrcMismatch)
        );
        let mut oversized = encoded;
        oversized[7..9].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            decode_k5_extended(&oversized, K5_EXTENDED_SESSION),
            Err(ProtocolError::PayloadTooLarge)
        );
        let mut malformed = [0; K5_EXTENDED_MAX_BYTES];
        let malformed_len = K5_EXTENDED_HEADER_LEN + 5 + K5_EXTENDED_CRC_LEN;
        malformed[..2].copy_from_slice(&K5_EXTENDED_MAGIC);
        malformed[2] = K5_EXTENDED_VERSION;
        malformed[3..5].copy_from_slice(&K5_EXTENDED_PROTOCOL_FRAME.to_le_bytes());
        malformed[5..7].copy_from_slice(&K5_EXTENDED_SESSION.to_le_bytes());
        malformed[7..9].copy_from_slice(&5_u16.to_le_bytes());
        malformed[9..14].copy_from_slice(b"frame");
        let malformed_crc = crc32(&malformed[..14]);
        malformed[14..18].copy_from_slice(&malformed_crc.to_le_bytes());
        assert_eq!(
            decode_k5_extended(&malformed[..malformed_len], K5_EXTENDED_SESSION),
            Err(ProtocolError::MalformedPayload)
        );
    }
}
