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
/// Encoded bytes in one object descriptor returned by `LIST_OBJECTS`.
pub const OBJECT_DESCRIPTOR_ENCODED_LEN: usize = 5;
/// Fixed metadata bytes preceding descriptors in a `LIST_OBJECTS` response.
pub const LIST_OBJECTS_RESPONSE_HEADER_LEN: usize = 10;
/// Maximum object descriptors carried by one bounded `LIST_OBJECTS` response.
pub const MAX_LIST_OBJECTS_PER_PAGE: usize =
    (MAX_PAYLOAD - LIST_OBJECTS_RESPONSE_HEADER_LEN) / OBJECT_DESCRIPTOR_ENCODED_LEN;
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
    /// Read what the receiver is currently doing.
    GetReceiveState = 0x10,
    /// Read one raw receive-metrics sample.
    GetReceiveMetrics = 0x11,
    /// Stop a running scan.
    StopScan = 0x12,
    /// Start scanning the current source.
    StartScan = 0x13,
    /// Leave memory channels for the tunable receiver.
    EnterVfo = 0x14,
    /// Leave the tunable receiver for memory channels.
    EnterMemory = 0x15,
    /// Tune the receiver to an exact frequency in hertz.
    TuneTo = 0x16,
    /// Select one memory channel by storage index.
    SelectChannel = 0x17,
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
            0x10 => Ok(Self::GetReceiveState),
            0x11 => Ok(Self::GetReceiveMetrics),
            0x12 => Ok(Self::StopScan),
            0x13 => Ok(Self::StartScan),
            0x14 => Ok(Self::EnterVfo),
            0x15 => Ok(Self::EnterMemory),
            0x16 => Ok(Self::TuneTo),
            0x17 => Ok(Self::SelectChannel),
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
    /// A sequence was reused for request bytes that differ from the cached request.
    SequenceConflict = 10,
    /// The operation is not valid in the state the device is currently in.
    ///
    /// The request was well formed and the device implements it. It cannot be
    /// performed now — tuning while listening to a memory channel, or changing
    /// source while a scan runs — and may succeed after a state change.
    InvalidState = 11,
    /// A well-formed request named a value the device cannot reach.
    ///
    /// A frequency outside the receiver's range, or a channel index which no
    /// stored channel occupies. The bytes were readable; what they asked for
    /// does not exist.
    OutOfRange = 12,
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
            10 => Ok(Self::SequenceConflict),
            11 => Ok(Self::InvalidState),
            12 => Ok(Self::OutOfRange),
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
    /// Bytes the device reserves for a stored configuration image.
    ///
    /// This is the capacity a whole configuration must fit, so a host can say
    /// how much room a project leaves before it writes one. A device which
    /// declares zero is not reporting a bound.
    pub configuration_bytes: u32,
}

impl DeviceCapabilities {
    /// Encoded capability payload length.
    pub const ENCODED_LEN: usize = 14;

    /// Encodes capabilities into a command payload.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let mut writer = PayloadWriter::new(output);
        writer.write_u8(self.protocol_version)?;
        writer.write_u8(self.storage_version)?;
        writer.write_u16(self.max_frame_payload)?;
        writer.write_u16(self.max_objects)?;
        writer.write_u16(self.max_object_size)?;
        writer.write_u16(self.plan_encodings)?;
        writer.write_u32(self.configuration_bytes)?;
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
            configuration_bytes: reader.read_u32()?,
        };
        reader.finish()?;
        Ok(capabilities)
    }
}

/// One active object described by a `LIST_OBJECTS` response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectDescriptor {
    /// Stable object-kind wire value.
    pub kind: u8,
    /// Kind-local object identifier.
    pub id: u16,
    /// Encoded object payload length in bytes.
    pub encoded_len: u16,
}

const EMPTY_OBJECT_DESCRIPTOR: ObjectDescriptor = ObjectDescriptor {
    kind: 0,
    id: 0,
    encoded_len: 0,
};

/// One decoded, fixed-capacity page returned by `LIST_OBJECTS`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectListPage {
    generation: u32,
    total_objects: u16,
    offset: u16,
    len: u8,
    objects: [ObjectDescriptor; MAX_LIST_OBJECTS_PER_PAGE],
}

impl ObjectListPage {
    /// Encodes a page, rejecting oversized, out-of-range, or unordered input.
    pub fn encode(
        generation: u32,
        total_objects: u16,
        offset: u16,
        objects: &[ObjectDescriptor],
        output: &mut [u8],
    ) -> Result<usize, ProtocolError> {
        if objects.len() > MAX_LIST_OBJECTS_PER_PAGE {
            return Err(ProtocolError::PayloadTooLarge);
        }
        validate_object_page(total_objects, offset, objects)?;
        let returned = u16::try_from(objects.len()).map_err(|_| ProtocolError::PayloadTooLarge)?;
        let mut writer = PayloadWriter::new(output);
        writer.write_u32(generation)?;
        writer.write_u16(total_objects)?;
        writer.write_u16(offset)?;
        writer.write_u16(returned)?;
        for object in objects {
            writer.write_u8(object.kind)?;
            writer.write_u16(object.id)?;
            writer.write_u16(object.encoded_len)?;
        }
        Ok(writer.len())
    }

    /// Decodes a page and rejects trailing, oversized, or unordered data.
    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        let mut reader = PayloadReader::new(input);
        let generation = reader.read_u32()?;
        let total_objects = reader.read_u16()?;
        let offset = reader.read_u16()?;
        let count = usize::from(reader.read_u16()?);
        if count > MAX_LIST_OBJECTS_PER_PAGE {
            return Err(ProtocolError::MalformedPayload);
        }
        let mut objects = [EMPTY_OBJECT_DESCRIPTOR; MAX_LIST_OBJECTS_PER_PAGE];
        for object in &mut objects[..count] {
            *object = ObjectDescriptor {
                kind: reader.read_u8()?,
                id: reader.read_u16()?,
                encoded_len: reader.read_u16()?,
            };
        }
        reader.finish()?;
        validate_object_page(total_objects, offset, &objects[..count])?;
        Ok(Self {
            generation,
            total_objects,
            offset,
            len: u8::try_from(count).map_err(|_| ProtocolError::MalformedPayload)?,
            objects,
        })
    }

    /// Returns the active storage generation described by this page.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Returns the complete active object count.
    pub const fn total_objects(self) -> u16 {
        self.total_objects
    }

    /// Returns the zero-based offset of the first descriptor in this page.
    pub const fn offset(self) -> u16 {
        self.offset
    }

    /// Returns the ordered object descriptors in this page.
    pub fn objects(&self) -> &[ObjectDescriptor] {
        &self.objects[..usize::from(self.len)]
    }
}

/// Encodes the exact two-byte request payload for a `LIST_OBJECTS` page.
pub fn encode_list_objects_request(offset: u16, output: &mut [u8]) -> Result<usize, ProtocolError> {
    let mut writer = PayloadWriter::new(output);
    writer.write_u16(offset)?;
    Ok(writer.len())
}

/// Decodes a `LIST_OBJECTS` request and rejects trailing bytes.
pub fn decode_list_objects_request(input: &[u8]) -> Result<u16, ProtocolError> {
    let mut reader = PayloadReader::new(input);
    let offset = reader.read_u16()?;
    reader.finish()?;
    Ok(offset)
}

fn validate_object_page(
    total_objects: u16,
    offset: u16,
    objects: &[ObjectDescriptor],
) -> Result<(), ProtocolError> {
    let returned = u16::try_from(objects.len()).map_err(|_| ProtocolError::MalformedPayload)?;
    if offset
        .checked_add(returned)
        .is_none_or(|end| end > total_objects)
        || objects
            .windows(2)
            .any(|pair| (pair[0].kind, pair[0].id) >= (pair[1].kind, pair[1].id))
    {
        return Err(ProtocolError::MalformedPayload);
    }
    Ok(())
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

/// One decoded runtime-control request.
///
/// Every variant names an operation the receive controller already performs for
/// a decoded key press. Nothing here can ask for transmission, so there is no
/// transmit request to refuse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlRequest {
    /// Report what the receiver is currently doing.
    GetState,
    /// Report one raw metrics sample.
    GetMetrics,
    /// Stop a running scan.
    StopScan,
    /// Start scanning the current source.
    StartScan,
    /// Leave memory channels for the tunable receiver.
    EnterVfo,
    /// Leave the tunable receiver for memory channels.
    EnterMemory,
    /// Tune the receiver to an exact frequency.
    TuneTo {
        /// Requested frequency in hertz.
        frequency_hz: u32,
    },
    /// Select one memory channel by storage index.
    SelectChannel {
        /// Zero-based storage index.
        index: u16,
    },
}

impl ControlRequest {
    /// Largest encoded request payload.
    pub const MAX_PAYLOAD_LEN: usize = 4;

    /// Returns the command byte which carries this request.
    pub const fn command(&self) -> Command {
        match self {
            Self::GetState => Command::GetReceiveState,
            Self::GetMetrics => Command::GetReceiveMetrics,
            Self::StopScan => Command::StopScan,
            Self::StartScan => Command::StartScan,
            Self::EnterVfo => Command::EnterVfo,
            Self::EnterMemory => Command::EnterMemory,
            Self::TuneTo { .. } => Command::TuneTo,
            Self::SelectChannel { .. } => Command::SelectChannel,
        }
    }

    /// Writes this request's payload, which is empty for most requests.
    pub fn encode_payload(&self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        match self {
            Self::TuneTo { frequency_hz } => {
                let buffer = output.get_mut(..4).ok_or(ProtocolError::OutputTooSmall)?;
                buffer.copy_from_slice(&frequency_hz.to_le_bytes());
                Ok(4)
            }
            Self::SelectChannel { index } => {
                let buffer = output.get_mut(..2).ok_or(ProtocolError::OutputTooSmall)?;
                buffer.copy_from_slice(&index.to_le_bytes());
                Ok(2)
            }
            _ => Ok(0),
        }
    }

    /// Reads one request from a command byte and exactly its payload bytes.
    ///
    /// A command outside the runtime-control range is not this service's, and a
    /// payload of the wrong length is refused rather than padded or truncated.
    pub fn decode(command: Command, payload: &[u8]) -> Result<Self, ProtocolError> {
        let empty = |request| {
            if payload.is_empty() {
                Ok(request)
            } else {
                Err(ProtocolError::TrailingPayload)
            }
        };
        match command {
            Command::GetReceiveState => empty(Self::GetState),
            Command::GetReceiveMetrics => empty(Self::GetMetrics),
            Command::StopScan => empty(Self::StopScan),
            Command::StartScan => empty(Self::StartScan),
            Command::EnterVfo => empty(Self::EnterVfo),
            Command::EnterMemory => empty(Self::EnterMemory),
            Command::TuneTo => {
                let bytes: [u8; 4] = payload
                    .try_into()
                    .map_err(|_| ProtocolError::MalformedPayload)?;
                Ok(Self::TuneTo {
                    frequency_hz: u32::from_le_bytes(bytes),
                })
            }
            Command::SelectChannel => {
                let bytes: [u8; 2] = payload
                    .try_into()
                    .map_err(|_| ProtocolError::MalformedPayload)?;
                Ok(Self::SelectChannel {
                    index: u16::from_le_bytes(bytes),
                })
            }
            _ => Err(ProtocolError::UnknownCommand),
        }
    }
}

/// Which source the receiver is currently listening to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReceiveMode {
    /// One stored memory channel.
    Memory = 0,
    /// The tunable receiver.
    Vfo = 1,
}

impl TryFrom<u8> for ReceiveMode {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Memory),
            1 => Ok(Self::Vfo),
            _ => Err(ProtocolError::MalformedPayload),
        }
    }
}

/// Whether a scan is running, and what it is doing if so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ScanActivity {
    /// No scan is running.
    Idle = 0,
    /// Waiting out the no-signal dwell on one channel.
    Dwell = 1,
    /// Holding on a channel which was found busy.
    Hold = 2,
}

impl TryFrom<u8> for ScanActivity {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            0 => Ok(Self::Idle),
            1 => Ok(Self::Dwell),
            2 => Ok(Self::Hold),
            _ => Err(ProtocolError::MalformedPayload),
        }
    }
}

/// What the receiver is currently doing, as the display would show it.
///
/// This is live state. It says where the receiver is pointed and whether a scan
/// is running; it carries no transmit frequency, class or authority, because
/// nothing in the runtime-control service can ask for one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveStateReport {
    /// Whether a memory channel or the tunable receiver is selected.
    pub mode: ReceiveMode,
    /// Whether a scan is running, and its phase if it is.
    pub scan: ScanActivity,
    /// Selected bank, absent when every programmed channel is in scope.
    pub bank: Option<u16>,
    /// Storage index of the selected channel; meaningless in VFO mode.
    pub index: u16,
    /// Stable identifier of the selected channel; zero in VFO mode.
    pub channel_id: u16,
    /// Channels the current bank filter admits.
    pub visible_channels: u16,
    /// Exact receive frequency in hertz.
    pub frequency_hz: u32,
}

impl ReceiveStateReport {
    /// Encoded length of one report.
    pub const ENCODED_LEN: usize = 15;

    /// Writes the report into a caller-supplied buffer.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let buffer = output
            .get_mut(..Self::ENCODED_LEN)
            .ok_or(ProtocolError::OutputTooSmall)?;
        buffer[0] = self.mode as u8;
        buffer[1] = self.scan as u8;
        // A bank is present or it is not, and the identifier is only meaningful
        // when it is; encoding them separately keeps every byte value legal.
        buffer[2] = u8::from(self.bank.is_some());
        buffer[3..5].copy_from_slice(&self.bank.unwrap_or(0).to_le_bytes());
        buffer[5..7].copy_from_slice(&self.index.to_le_bytes());
        buffer[7..9].copy_from_slice(&self.channel_id.to_le_bytes());
        buffer[9..11].copy_from_slice(&self.visible_channels.to_le_bytes());
        buffer[11..15].copy_from_slice(&self.frequency_hz.to_le_bytes());
        Ok(Self::ENCODED_LEN)
    }

    /// Reads one report from exactly its encoded bytes.
    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        if input.len() < Self::ENCODED_LEN {
            return Err(ProtocolError::MalformedPayload);
        }
        if input.len() > Self::ENCODED_LEN {
            return Err(ProtocolError::TrailingPayload);
        }
        let bank = match input[2] {
            0 => None,
            1 => Some(u16::from_le_bytes([input[3], input[4]])),
            _ => return Err(ProtocolError::MalformedPayload),
        };
        Ok(Self {
            mode: ReceiveMode::try_from(input[0])?,
            scan: ScanActivity::try_from(input[1])?,
            bank,
            index: u16::from_le_bytes([input[5], input[6]]),
            channel_id: u16::from_le_bytes([input[7], input[8]]),
            visible_channels: u16::from_le_bytes([input[9], input[10]]),
            frequency_hz: u32::from_le_bytes([input[11], input[12], input[13], input[14]]),
        })
    }
}

/// A report a frame cannot carry is a report a host cannot read.
const _: () = assert!(ReceiveStateReport::ENCODED_LEN <= MAX_PAYLOAD);

/// One raw receive-metrics sample and the frequency it was taken at.
///
/// Every field is raw, in the chip's own units. `samples` is what makes a
/// reading usable: the receiver needs an unmeasured settling time after a
/// retune, so a host which wants a settled reading waits for this counter to
/// advance past the value it saw when it asked for the frequency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveMetricsReport {
    /// Frequency the sample was taken at, in hertz.
    pub frequency_hz: u32,
    /// Completed metric samples since boot, saturating.
    pub samples: u16,
    /// Approximate RSSI multiplied by two, preserving the 0.5 dB step.
    pub rssi_dbm_x2: i16,
    /// Raw glitch indicator; lower values indicate a cleaner signal.
    pub glitch: u8,
    /// Raw excess-noise indicator; lower values indicate a cleaner signal.
    pub noise: u8,
    /// Whether the carrier squelch link reads open.
    pub squelch_open: bool,
}

impl ReceiveMetricsReport {
    /// Encoded length of one report.
    pub const ENCODED_LEN: usize = 11;

    /// Writes the report into a caller-supplied buffer.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, ProtocolError> {
        let buffer = output
            .get_mut(..Self::ENCODED_LEN)
            .ok_or(ProtocolError::OutputTooSmall)?;
        buffer[0..4].copy_from_slice(&self.frequency_hz.to_le_bytes());
        buffer[4..6].copy_from_slice(&self.samples.to_le_bytes());
        buffer[6..8].copy_from_slice(&self.rssi_dbm_x2.to_le_bytes());
        buffer[8] = self.glitch;
        buffer[9] = self.noise;
        buffer[10] = u8::from(self.squelch_open);
        Ok(Self::ENCODED_LEN)
    }

    /// Reads one report from exactly its encoded bytes.
    pub fn decode(input: &[u8]) -> Result<Self, ProtocolError> {
        if input.len() < Self::ENCODED_LEN {
            return Err(ProtocolError::MalformedPayload);
        }
        if input.len() > Self::ENCODED_LEN {
            return Err(ProtocolError::TrailingPayload);
        }
        let squelch_open = match input[10] {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::MalformedPayload),
        };
        Ok(Self {
            frequency_hz: u32::from_le_bytes([input[0], input[1], input[2], input[3]]),
            samples: u16::from_le_bytes([input[4], input[5]]),
            rssi_dbm_x2: i16::from_le_bytes([input[6], input[7]]),
            glitch: input[8],
            noise: input[9],
            squelch_open,
        })
    }
}

const _: () = assert!(ReceiveMetricsReport::ENCODED_LEN <= MAX_PAYLOAD);

#[cfg(test)]
mod tests {
    use super::{
        cobs_encode, crc16_ccitt_false, decode_list_objects_request, decode_packet, encode_frame,
        encode_list_objects_request, Command, ControlRequest, DeviceCapabilities, Frame,
        ObjectDescriptor, ObjectListPage, ProtocolError, ReceiveMetricsReport, ReceiveMode,
        ReceiveStateReport, ScanActivity, Service, StreamDecoder, CRC_LEN, FLAG_RESPONSE,
        HEADER_LEN, MAGIC, MAX_ENCODED_FRAME, MAX_LIST_OBJECTS_PER_PAGE, MAX_PAYLOAD,
        PROTOCOL_VERSION,
    };

    fn encode_raw_frame(service: u8, command: u8) -> ([u8; MAX_ENCODED_FRAME], usize) {
        let mut decoded = [0_u8; HEADER_LEN + CRC_LEN];
        decoded[0..2].copy_from_slice(&MAGIC);
        decoded[2] = PROTOCOL_VERSION;
        decoded[3] = service;
        decoded[5..7].copy_from_slice(&1_u16.to_le_bytes());
        decoded[7] = command;
        let crc = crc16_ccitt_false(&decoded[..HEADER_LEN]);
        decoded[HEADER_LEN..].copy_from_slice(&crc.to_le_bytes());

        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let packet_len = cobs_encode(&decoded, &mut encoded).unwrap();
        encoded[packet_len] = 0;
        (encoded, packet_len + 1)
    }

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
            configuration_bytes: 4_096,
        };
        let mut bytes = [0_u8; DeviceCapabilities::ENCODED_LEN];
        let len = expected.encode(&mut bytes).unwrap();
        assert_eq!(DeviceCapabilities::decode(&bytes[..len]).unwrap(), expected);
    }

    #[test]
    fn object_list_page_is_bounded_ordered_and_exact() {
        let expected = [
            ObjectDescriptor {
                kind: 1,
                id: 2,
                encoded_len: 31,
            },
            ObjectDescriptor {
                kind: 1,
                id: 7,
                encoded_len: 31,
            },
        ];
        let mut bytes = [0_u8; MAX_PAYLOAD];
        let len = ObjectListPage::encode(4, 5, 2, &expected, &mut bytes).unwrap();
        let page = ObjectListPage::decode(&bytes[..len]).unwrap();
        assert_eq!(page.generation(), 4);
        assert_eq!(page.total_objects(), 5);
        assert_eq!(page.offset(), 2);
        assert_eq!(page.objects(), expected);

        let mut too_many = [ObjectDescriptor {
            kind: 1,
            id: 1,
            encoded_len: 31,
        }; MAX_LIST_OBJECTS_PER_PAGE + 1];
        for (index, object) in too_many.iter_mut().enumerate() {
            object.id = u16::try_from(index).unwrap();
        }
        assert_eq!(
            ObjectListPage::encode(
                0,
                u16::try_from(too_many.len()).unwrap(),
                0,
                &too_many,
                &mut bytes,
            ),
            Err(ProtocolError::PayloadTooLarge)
        );
        assert_eq!(
            ObjectListPage::encode(0, 2, 0, &[expected[1], expected[0]], &mut bytes),
            Err(ProtocolError::MalformedPayload)
        );
    }

    #[test]
    fn object_list_request_rejects_trailing_bytes() {
        let mut bytes = [0_u8; 2];
        let len = encode_list_objects_request(17, &mut bytes).unwrap();
        assert_eq!(decode_list_objects_request(&bytes[..len]).unwrap(), 17);
        assert_eq!(
            decode_list_objects_request(&[17, 0, 0]),
            Err(ProtocolError::TrailingPayload)
        );
    }

    #[test]
    fn unknown_wire_values_are_discarded_and_stream_recovers() {
        let (unknown_service, unknown_service_len) = encode_raw_frame(0xfe, Command::Hello as u8);
        let (unknown_command, unknown_command_len) =
            encode_raw_frame(Service::DeviceInfo as u8, 0x55);
        let expected = Frame::new(Service::DeviceInfo, 0, 2, Command::Hello, &[1]).unwrap();
        let mut valid = [0_u8; MAX_ENCODED_FRAME];
        let valid_len = encode_frame(&expected, &mut valid).unwrap();

        let mut decoder = StreamDecoder::new();
        let mut errors = [None; 2];
        let mut error_count = 0;
        let mut recovered = None;
        for packet in [
            &unknown_service[..unknown_service_len],
            &unknown_command[..unknown_command_len],
            &valid[..valid_len],
        ] {
            for byte in packet {
                if let Some(result) = decoder.push(*byte) {
                    match result {
                        Ok(frame) => recovered = Some(frame),
                        Err(error) => {
                            errors[error_count] = Some(error);
                            error_count += 1;
                        }
                    }
                }
            }
        }

        assert_eq!(
            errors,
            [
                Some(ProtocolError::UnknownService),
                Some(ProtocolError::UnknownCommand),
            ]
        );
        assert_eq!(recovered, Some(expected));
    }

    #[test]
    fn runtime_control_commands_decode_from_their_wire_bytes() {
        let expected = [
            (0x10_u8, Command::GetReceiveState),
            (0x11, Command::GetReceiveMetrics),
            (0x12, Command::StopScan),
            (0x13, Command::StartScan),
            (0x14, Command::EnterVfo),
            (0x15, Command::EnterMemory),
            (0x16, Command::TuneTo),
            (0x17, Command::SelectChannel),
        ];
        for (byte, command) in expected {
            assert_eq!(Command::try_from(byte), Ok(command));
            assert_eq!(command as u8, byte);
        }
        assert_eq!(Command::try_from(0x18), Err(ProtocolError::UnknownCommand));
    }

    #[test]
    fn every_control_request_round_trips_through_its_command_and_payload() {
        let requests = [
            ControlRequest::GetState,
            ControlRequest::GetMetrics,
            ControlRequest::StopScan,
            ControlRequest::StartScan,
            ControlRequest::EnterVfo,
            ControlRequest::EnterMemory,
            ControlRequest::TuneTo {
                frequency_hz: 145_512_500,
            },
            ControlRequest::SelectChannel { index: 399 },
        ];
        for request in requests {
            let mut payload = [0_u8; ControlRequest::MAX_PAYLOAD_LEN];
            let length = request
                .encode_payload(&mut payload)
                .expect("the payload encodes");
            assert_eq!(
                ControlRequest::decode(request.command(), &payload[..length]),
                Ok(request)
            );
        }
    }

    #[test]
    fn a_control_request_refuses_a_payload_of_the_wrong_length() {
        // An operation which takes no argument is not given one.
        assert_eq!(
            ControlRequest::decode(Command::StopScan, &[0]),
            Err(ProtocolError::TrailingPayload)
        );
        // A frequency is four bytes or it is not a frequency.
        assert_eq!(
            ControlRequest::decode(Command::TuneTo, &[0, 0, 0]),
            Err(ProtocolError::MalformedPayload)
        );
        assert_eq!(
            ControlRequest::decode(Command::TuneTo, &[0, 0, 0, 0, 0]),
            Err(ProtocolError::MalformedPayload)
        );
        assert_eq!(
            ControlRequest::decode(Command::SelectChannel, &[0]),
            Err(ProtocolError::MalformedPayload)
        );
    }

    #[test]
    fn a_configuration_command_is_not_a_control_request() {
        assert_eq!(
            ControlRequest::decode(Command::ListObjects, &[]),
            Err(ProtocolError::UnknownCommand)
        );
        assert_eq!(
            ControlRequest::decode(Command::Hello, &[1]),
            Err(ProtocolError::UnknownCommand)
        );
    }

    #[test]
    fn a_receive_state_report_round_trips_in_both_bank_forms() {
        for bank in [None, Some(7)] {
            let report = ReceiveStateReport {
                mode: ReceiveMode::Memory,
                scan: ScanActivity::Hold,
                bank,
                index: 300,
                channel_id: 41,
                visible_channels: 16,
                frequency_hz: 145_500_000,
            };
            let mut buffer = [0_u8; ReceiveStateReport::ENCODED_LEN];
            let length = report.encode(&mut buffer).expect("the report encodes");
            assert_eq!(length, ReceiveStateReport::ENCODED_LEN);
            assert_eq!(ReceiveStateReport::decode(&buffer), Ok(report));
        }
    }

    #[test]
    fn a_receive_state_report_rejects_wrong_lengths_and_illegal_bytes() {
        let report = ReceiveStateReport {
            mode: ReceiveMode::Vfo,
            scan: ScanActivity::Idle,
            bank: None,
            index: 0,
            channel_id: 0,
            visible_channels: 0,
            frequency_hz: 433_000_000,
        };
        let mut buffer = [0_u8; ReceiveStateReport::ENCODED_LEN + 1];
        report.encode(&mut buffer).expect("the report encodes");

        assert_eq!(
            ReceiveStateReport::decode(&buffer[..ReceiveStateReport::ENCODED_LEN - 1]),
            Err(ProtocolError::MalformedPayload)
        );
        assert_eq!(
            ReceiveStateReport::decode(&buffer),
            Err(ProtocolError::TrailingPayload)
        );

        let mut illegal = buffer;
        illegal[0] = 2;
        assert_eq!(
            ReceiveStateReport::decode(&illegal[..ReceiveStateReport::ENCODED_LEN]),
            Err(ProtocolError::MalformedPayload)
        );

        let mut illegal = buffer;
        illegal[1] = 3;
        assert_eq!(
            ReceiveStateReport::decode(&illegal[..ReceiveStateReport::ENCODED_LEN]),
            Err(ProtocolError::MalformedPayload)
        );

        // A bank presence byte which is neither absent nor present names no
        // bank, so it cannot be silently read as one.
        let mut illegal = buffer;
        illegal[2] = 2;
        assert_eq!(
            ReceiveStateReport::decode(&illegal[..ReceiveStateReport::ENCODED_LEN]),
            Err(ProtocolError::MalformedPayload)
        );
    }

    #[test]
    fn a_metrics_report_round_trips_including_a_negative_reading() {
        let report = ReceiveMetricsReport {
            frequency_hz: 145_512_500,
            samples: 65_535,
            rssi_dbm_x2: -238,
            glitch: 12,
            noise: 41,
            squelch_open: true,
        };
        let mut buffer = [0_u8; ReceiveMetricsReport::ENCODED_LEN];
        let length = report.encode(&mut buffer).expect("the report encodes");
        assert_eq!(length, ReceiveMetricsReport::ENCODED_LEN);
        assert_eq!(ReceiveMetricsReport::decode(&buffer), Ok(report));
    }

    #[test]
    fn a_metrics_report_rejects_wrong_lengths_and_an_illegal_squelch_byte() {
        let report = ReceiveMetricsReport {
            frequency_hz: 145_000_000,
            samples: 1,
            rssi_dbm_x2: -300,
            glitch: 0,
            noise: 0,
            squelch_open: false,
        };
        let mut buffer = [0_u8; ReceiveMetricsReport::ENCODED_LEN + 1];
        report.encode(&mut buffer).expect("the report encodes");

        assert_eq!(
            ReceiveMetricsReport::decode(&buffer[..ReceiveMetricsReport::ENCODED_LEN - 1]),
            Err(ProtocolError::MalformedPayload)
        );
        assert_eq!(
            ReceiveMetricsReport::decode(&buffer),
            Err(ProtocolError::TrailingPayload)
        );

        let mut illegal = buffer;
        illegal[10] = 2;
        assert_eq!(
            ReceiveMetricsReport::decode(&illegal[..ReceiveMetricsReport::ENCODED_LEN]),
            Err(ProtocolError::MalformedPayload)
        );
    }
}
