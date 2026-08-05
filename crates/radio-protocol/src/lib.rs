//! Bounded, transport-independent radio control and programming protocol.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

/// Current protocol version.
pub const PROTOCOL_VERSION: u8 = 1;
/// Maximum payload bytes in one protocol frame.
pub const MAX_PAYLOAD: usize = 128;
/// Maximum encoded packet size including the COBS delimiter.
pub const MAX_ENCODED_FRAME: usize = 144;
/// Flag set on response frames.
pub const FLAG_RESPONSE: u8 = 1 << 0;
/// Flag set when a response carries a device error.
pub const FLAG_ERROR: u8 = 1 << 1;

const MAGIC: [u8; 2] = *b"UR";
const HEADER_LEN: usize = 10;
const CRC_LEN: usize = 2;
const MAX_DECODED_FRAME: usize = HEADER_LEN + MAX_PAYLOAD + CRC_LEN;

/// Protocol services negotiated over one transport family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Service {
    /// Device identity and capability discovery.
    DeviceInfo = 1,
    /// Runtime radio control.
    RuntimeControl = 2,
    /// Transactional configuration objects.
    Configuration = 3,
    /// Firmware update and recovery.
    FirmwareUpdate = 4,
    /// Diagnostics and trace collection.
    Diagnostics = 5,
}

impl TryFrom<u8> for Service {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::DeviceInfo),
            2 => Ok(Self::RuntimeControl),
            3 => Ok(Self::Configuration),
            4 => Ok(Self::FirmwareUpdate),
            5 => Ok(Self::Diagnostics),
            _ => Err(ProtocolError::UnknownService),
        }
    }
}

/// Commands implemented or reserved by protocol version 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Command {
    /// Establish protocol-version compatibility.
    Hello = 0x01,
    /// Read bounded device capabilities.
    GetCapabilities = 0x02,
    /// Read stable device identity.
    GetDeviceInfo = 0x03,
    /// List active configuration objects.
    ListObjects = 0x20,
    /// Read one active configuration object.
    ReadObject = 0x21,
    /// Begin a candidate configuration transaction.
    BeginTransaction = 0x22,
    /// Add or replace a candidate configuration object.
    WriteObject = 0x23,
    /// Validate the complete candidate configuration.
    ValidateTransaction = 0x24,
    /// Atomically activate the candidate configuration.
    CommitTransaction = 0x25,
    /// Discard the candidate configuration.
    AbortTransaction = 0x26,
    /// Error response containing the rejected command and error code.
    Error = 0x7f,
}

impl TryFrom<u8> for Command {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0x01 => Ok(Self::Hello),
            0x02 => Ok(Self::GetCapabilities),
            0x03 => Ok(Self::GetDeviceInfo),
            0x20 => Ok(Self::ListObjects),
            0x21 => Ok(Self::ReadObject),
            0x22 => Ok(Self::BeginTransaction),
            0x23 => Ok(Self::WriteObject),
            0x24 => Ok(Self::ValidateTransaction),
            0x25 => Ok(Self::CommitTransaction),
            0x26 => Ok(Self::AbortTransaction),
            0x7f => Ok(Self::Error),
            _ => Err(ProtocolError::UnknownCommand),
        }
    }
}

/// Stable device-side error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceErrorCode {
    /// The service is not implemented by the target.
    UnsupportedService = 1,
    /// The command is not implemented for the selected service.
    UnsupportedCommand = 2,
    /// Command payload bytes are invalid.
    MalformedPayload = 3,
    /// The requested object does not exist.
    ObjectNotFound = 4,
    /// A transaction is already active.
    TransactionAlreadyOpen = 5,
    /// The request does not match an active transaction.
    NoTransaction = 6,
    /// The candidate configuration failed validation.
    ValidationFailed = 7,
    /// The target lacks capacity for the request.
    CapacityExceeded = 8,
    /// A commit was requested before successful validation.
    NotValidated = 9,
    /// An unspecified device-side failure occurred.
    Internal = 255,
}

impl TryFrom<u8> for DeviceErrorCode {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::UnsupportedService),
            2 => Ok(Self::UnsupportedCommand),
            3 => Ok(Self::MalformedPayload),
            4 => Ok(Self::ObjectNotFound),
            5 => Ok(Self::TransactionAlreadyOpen),
            6 => Ok(Self::NoTransaction),
            7 => Ok(Self::ValidationFailed),
            8 => Ok(Self::CapacityExceeded),
            9 => Ok(Self::NotValidated),
            255 => Ok(Self::Internal),
            _ => Err(ProtocolError::MalformedPayload),
        }
    }
}

/// Bounded capabilities returned during connection negotiation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceCapabilities {
    /// Highest protocol version supported by this session.
    pub protocol_version: u8,
    /// Active configuration storage format version.
    pub storage_version: u8,
    /// Maximum frame payload accepted by the device.
    pub max_frame_payload: u16,
    /// Maximum number of active configuration objects.
    pub max_objects: u16,
    /// Maximum encoded bytes in one object.
    pub max_object_size: u16,
    /// Bitset of supported channel plan encodings.
    pub plan_encodings: u16,
}

impl DeviceCapabilities {
    /// Encoded capability payload length.
    pub const ENCODED_LEN: usize = 10;

    /// Encodes capabilities into a command payload.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let mut writer = PayloadWriter::new(output);
        writer.write_u8(self.protocol_version)?;
        writer.write_u8(self.storage_version)?;
        writer.write_u16(self.max_frame_payload)?;
        writer.write_u16(self.max_objects)?;
        writer.write_u16(self.max_object_size)?;
        writer.write_u16(self.plan_encodings)?;
        Ok(writer.len())
    }

    /// Decodes capabilities and rejects trailing bytes.
    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = PayloadReader::new(input);
        let capabilities = Self {
            protocol_version: reader.read_u8()?,
            storage_version: reader.read_u8()?,
            max_frame_payload: reader.read_u16()?,
            max_objects: reader.read_u16()?,
            max_object_size: reader.read_u16()?,
            plan_encodings: reader.read_u16()?,
        };
        reader.finish()?;
        Ok(capabilities)
    }
}

/// A decoded protocol frame with fixed payload capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame {
    service: Service,
    flags: u8,
    sequence: u16,
    command: Command,
    payload_len: u8,
    payload: [u8; MAX_PAYLOAD],
}

impl Frame {
    /// Constructs a frame by copying a bounded payload.
    pub fn new(
        service: Service,
        flags: u8,
        sequence: u16,
        command: Command,
        payload: &[u8],
    ) -> Result<Self, ProtocolError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge);
        }
        let mut bytes = [0_u8; MAX_PAYLOAD];
        bytes[..payload.len()].copy_from_slice(payload);
        Ok(Self {
            service,
            flags,
            sequence,
            command,
            payload_len: u8::try_from(payload.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
            payload: bytes,
        })
    }

    /// Returns the selected service.
    pub const fn service(self) -> Service {
        self.service
    }

    /// Returns raw frame flags.
    pub const fn flags(self) -> u8 {
        self.flags
    }

    /// Returns the request/response sequence number.
    pub const fn sequence(self) -> u16 {
        self.sequence
    }

    /// Returns the command.
    pub const fn command(self) -> Command {
        self.command
    }

    /// Returns the payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload[..usize::from(self.payload_len)]
    }
}

/// Protocol framing or payload failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// A payload exceeds the fixed frame capacity.
    PayloadTooLarge,
    /// The caller's output buffer is too small.
    OutputTooSmall,
    /// A COBS packet is structurally invalid.
    MalformedCobs,
    /// The decoded packet is shorter than its fixed fields.
    FrameTooShort,
    /// The frame magic does not match this protocol family.
    InvalidMagic,
    /// The protocol version is unsupported.
    UnsupportedVersion,
    /// The encoded payload length does not match the packet.
    LengthMismatch,
    /// The frame checksum is invalid.
    CrcMismatch,
    /// The service byte is unknown.
    UnknownService,
    /// The command byte is unknown.
    UnknownCommand,
    /// A structured payload lacks required bytes.
    MalformedPayload,
    /// Structured payload bytes remained after decoding.
    TrailingPayload,
    /// A stream packet exceeded the fixed decoder buffer.
    StreamOverflow,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge => formatter.write_str("protocol payload too large"),
            Self::OutputTooSmall => formatter.write_str("protocol output buffer too small"),
            Self::MalformedCobs => formatter.write_str("malformed COBS packet"),
            Self::FrameTooShort => formatter.write_str("protocol frame too short"),
            Self::InvalidMagic => formatter.write_str("invalid protocol magic"),
            Self::UnsupportedVersion => formatter.write_str("unsupported protocol version"),
            Self::LengthMismatch => formatter.write_str("protocol frame length mismatch"),
            Self::CrcMismatch => formatter.write_str("protocol CRC mismatch"),
            Self::UnknownService => formatter.write_str("unknown protocol service"),
            Self::UnknownCommand => formatter.write_str("unknown protocol command"),
            Self::MalformedPayload => formatter.write_str("malformed command payload"),
            Self::TrailingPayload => formatter.write_str("unexpected trailing payload bytes"),
            Self::StreamOverflow => formatter.write_str("protocol stream packet overflow"),
        }
    }
}

/// Writes little-endian scalar fields into a bounded command payload.
pub struct PayloadWriter<'a> {
    output: &'a mut [u8],
    position: usize,
}

impl<'a> PayloadWriter<'a> {
    /// Constructs a writer at the beginning of `output`.
    pub fn new(output: &'a mut [u8]) -> Self {
        Self {
            output,
            position: 0,
        }
    }

    /// Appends one byte.
    pub fn write_u8(&mut self, value: u8) -> Result<(), ProtocolError> {
        self.write_bytes(&[value])
    }

    /// Appends a little-endian 16-bit integer.
    pub fn write_u16(&mut self, value: u16) -> Result<(), ProtocolError> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Appends a little-endian 32-bit integer.
    pub fn write_u32(&mut self, value: u32) -> Result<(), ProtocolError> {
        self.write_bytes(&value.to_le_bytes())
    }

    /// Appends raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ProtocolError> {
        let end = self
            .position
            .checked_add(bytes.len())
            .ok_or(ProtocolError::OutputTooSmall)?;
        let destination = self
            .output
            .get_mut(self.position..end)
            .ok_or(ProtocolError::OutputTooSmall)?;
        destination.copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }

    /// Returns the number of written bytes.
    pub const fn len(&self) -> usize {
        self.position
    }

    /// Reports whether no bytes have been written.
    pub const fn is_empty(&self) -> bool {
        self.position == 0
    }
}

/// Reads little-endian scalar fields from a command payload.
pub struct PayloadReader<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> PayloadReader<'a> {
    /// Constructs a reader at the beginning of `input`.
    pub const fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    /// Reads one byte.
    pub fn read_u8(&mut self) -> Result<u8, ProtocolError> {
        let bytes = self.read_bytes(1)?;
        Ok(bytes[0])
    }

    /// Reads a little-endian 16-bit integer.
    pub fn read_u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Reads a little-endian 32-bit integer.
    pub fn read_u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Borrows exactly `length` bytes.
    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::MalformedPayload)?;
        let bytes = self
            .input
            .get(self.position..end)
            .ok_or(ProtocolError::MalformedPayload)?;
        self.position = end;
        Ok(bytes)
    }

    /// Succeeds only when all payload bytes were consumed.
    pub fn finish(self) -> Result<(), ProtocolError> {
        if self.position == self.input.len() {
            Ok(())
        } else {
            Err(ProtocolError::TrailingPayload)
        }
    }

    /// Returns the unread byte count.
    pub fn remaining(&self) -> usize {
        self.input.len() - self.position
    }
}

/// Encodes a frame as one COBS packet including its zero delimiter.
pub fn encode_frame(frame: &Frame, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let payload = frame.payload();
    let decoded_len = HEADER_LEN + payload.len() + CRC_LEN;
    let mut decoded = [0_u8; MAX_DECODED_FRAME];
    decoded[0..2].copy_from_slice(&MAGIC);
    decoded[2] = PROTOCOL_VERSION;
    decoded[3] = frame.service as u8;
    decoded[4] = frame.flags;
    decoded[5..7].copy_from_slice(&frame.sequence.to_le_bytes());
    decoded[7] = frame.command as u8;
    let payload_len = u16::try_from(payload.len()).map_err(|_| ProtocolError::PayloadTooLarge)?;
    decoded[8..10].copy_from_slice(&payload_len.to_le_bytes());
    decoded[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    let crc_offset = HEADER_LEN + payload.len();
    let crc = crc16_ccitt_false(&decoded[..crc_offset]);
    decoded[crc_offset..decoded_len].copy_from_slice(&crc.to_le_bytes());

    if output.is_empty() {
        return Err(ProtocolError::OutputTooSmall);
    }
    let encoded_len = cobs_encode(&decoded[..decoded_len], output)?;
    let delimiter = output
        .get_mut(encoded_len)
        .ok_or(ProtocolError::OutputTooSmall)?;
    *delimiter = 0;
    Ok(encoded_len + 1)
}

/// Decodes one COBS packet without its zero delimiter.
pub fn decode_packet(packet: &[u8]) -> Result<Frame, ProtocolError> {
    let mut decoded = [0_u8; MAX_DECODED_FRAME];
    let decoded_len = cobs_decode(packet, &mut decoded)?;
    if decoded_len < HEADER_LEN + CRC_LEN {
        return Err(ProtocolError::FrameTooShort);
    }
    if decoded[..2] != MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    if decoded[2] != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let payload_len = usize::from(u16::from_le_bytes([decoded[8], decoded[9]]));
    if payload_len > MAX_PAYLOAD || decoded_len != HEADER_LEN + payload_len + CRC_LEN {
        return Err(ProtocolError::LengthMismatch);
    }
    let crc_offset = HEADER_LEN + payload_len;
    let expected_crc = u16::from_le_bytes([decoded[crc_offset], decoded[crc_offset + 1]]);
    if crc16_ccitt_false(&decoded[..crc_offset]) != expected_crc {
        return Err(ProtocolError::CrcMismatch);
    }
    Frame::new(
        Service::try_from(decoded[3])?,
        decoded[4],
        u16::from_le_bytes([decoded[5], decoded[6]]),
        Command::try_from(decoded[7])?,
        &decoded[HEADER_LEN..crc_offset],
    )
}

/// Incremental COBS stream decoder with delimiter-based error recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamDecoder {
    packet: [u8; MAX_ENCODED_FRAME],
    len: usize,
    discarding: bool,
}

impl Default for StreamDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamDecoder {
    /// Constructs an empty decoder.
    pub const fn new() -> Self {
        Self {
            packet: [0; MAX_ENCODED_FRAME],
            len: 0,
            discarding: false,
        }
    }

    /// Pushes one stream byte and returns a frame or packet error at a delimiter.
    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, ProtocolError>> {
        if byte == 0 {
            if self.discarding {
                self.reset();
                return Some(Err(ProtocolError::StreamOverflow));
            }
            if self.len == 0 {
                return None;
            }
            let result = decode_packet(&self.packet[..self.len]);
            self.reset();
            return Some(result);
        }
        if self.discarding {
            return None;
        }
        if self.len == self.packet.len() {
            self.discarding = true;
            return None;
        }
        self.packet[self.len] = byte;
        self.len += 1;
        None
    }

    fn reset(&mut self) {
        self.len = 0;
        self.discarding = false;
    }
}

/// Calculates CRC-16/CCITT-FALSE for protocol and fixture use.
pub fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn cobs_encode(input: &[u8], output: &mut [u8]) -> Result<usize, ProtocolError> {
    if output.is_empty() {
        return Err(ProtocolError::OutputTooSmall);
    }
    let mut code_index = 0;
    let mut write_index = 1;
    let mut code = 1_u8;

    for byte in input {
        if *byte == 0 {
            *output
                .get_mut(code_index)
                .ok_or(ProtocolError::OutputTooSmall)? = code;
            code_index = write_index;
            write_index = write_index
                .checked_add(1)
                .ok_or(ProtocolError::OutputTooSmall)?;
            code = 1;
        } else {
            *output
                .get_mut(write_index)
                .ok_or(ProtocolError::OutputTooSmall)? = *byte;
            write_index += 1;
            code = code.wrapping_add(1);
            if code == 0xff {
                *output
                    .get_mut(code_index)
                    .ok_or(ProtocolError::OutputTooSmall)? = code;
                code_index = write_index;
                write_index = write_index
                    .checked_add(1)
                    .ok_or(ProtocolError::OutputTooSmall)?;
                code = 1;
            }
        }
    }
    *output
        .get_mut(code_index)
        .ok_or(ProtocolError::OutputTooSmall)? = code;
    Ok(write_index)
}

fn cobs_decode(input: &[u8], output: &mut [u8]) -> Result<usize, ProtocolError> {
    if input.is_empty() {
        return Err(ProtocolError::MalformedCobs);
    }
    let mut read_index: usize = 0;
    let mut write_index: usize = 0;
    while read_index < input.len() {
        let code = input[read_index];
        if code == 0 {
            return Err(ProtocolError::MalformedCobs);
        }
        read_index += 1;
        let block_len = usize::from(code - 1);
        let block_end = read_index
            .checked_add(block_len)
            .ok_or(ProtocolError::MalformedCobs)?;
        let block = input
            .get(read_index..block_end)
            .ok_or(ProtocolError::MalformedCobs)?;
        let output_end = write_index
            .checked_add(block_len)
            .ok_or(ProtocolError::OutputTooSmall)?;
        output
            .get_mut(write_index..output_end)
            .ok_or(ProtocolError::OutputTooSmall)?
            .copy_from_slice(block);
        write_index = output_end;
        read_index = block_end;

        if code != 0xff && read_index < input.len() {
            *output
                .get_mut(write_index)
                .ok_or(ProtocolError::OutputTooSmall)? = 0;
            write_index += 1;
        }
    }
    Ok(write_index)
}

#[cfg(test)]
mod tests {
    use super::{
        decode_packet, encode_frame, Command, DeviceCapabilities, Frame, ProtocolError, Service,
        StreamDecoder, FLAG_RESPONSE, MAX_ENCODED_FRAME,
    };

    #[test]
    fn frame_with_zero_bytes_round_trips() {
        let expected = Frame::new(
            Service::Configuration,
            FLAG_RESPONSE,
            0x1200,
            Command::ReadObject,
            &[0, 1, 0, 2, 3, 0],
        )
        .unwrap();
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let len = encode_frame(&expected, &mut encoded).unwrap();
        assert_eq!(encoded[len - 1], 0);
        assert_eq!(decode_packet(&encoded[..len - 1]).unwrap(), expected);
    }

    #[test]
    fn corrupt_packet_is_rejected_and_stream_recovers() {
        let expected = Frame::new(Service::DeviceInfo, 0, 9, Command::Hello, &[1]).unwrap();
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let len = encode_frame(&expected, &mut encoded).unwrap();
        let mut corrupt = encoded;
        corrupt[len - 2] ^= 0x40;

        let mut decoder = StreamDecoder::new();
        let mut error = None;
        for byte in &corrupt[..len] {
            if let Some(result) = decoder.push(*byte) {
                error = Some(result.unwrap_err());
            }
        }
        assert_eq!(error, Some(ProtocolError::CrcMismatch));

        let mut recovered = None;
        for byte in &encoded[..len] {
            if let Some(result) = decoder.push(*byte) {
                recovered = Some(result.unwrap());
            }
        }
        assert_eq!(recovered, Some(expected));
    }

    #[test]
    fn capabilities_round_trip_without_trailing_bytes() {
        let expected = DeviceCapabilities {
            protocol_version: 1,
            storage_version: 1,
            max_frame_payload: 128,
            max_objects: 8,
            max_object_size: 64,
            plan_encodings: 1,
        };
        let mut bytes = [0_u8; DeviceCapabilities::ENCODED_LEN];
        let len = expected.encode(&mut bytes).unwrap();
        assert_eq!(DeviceCapabilities::decode(&bytes[..len]).unwrap(), expected);
    }
}
