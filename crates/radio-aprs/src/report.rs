use core::fmt;

use radio_domain::Frequency;

use crate::{parse_ui_frame, Ax25Callsign, Ax25Error, Ax25UiFrame};

const OBJECT_TYPE: u8 = b';';
const ITEM_TYPE: u8 = b')';
const POSITION_LEN: usize = 19;
const MAX_NAME_LEN: usize = 9;

/// APRS entity class, retained as part of discovery identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportKind {
    /// A fixed-width, timestamped APRS Object.
    Object,
    /// A delimiter-terminated APRS Item.
    Item,
}

/// A bounded case-sensitive Object or Item name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReportName {
    bytes: [u8; MAX_NAME_LEN],
    length: u8,
}

impl ReportName {
    /// Returns the exact case-sensitive name without Object padding.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

/// One validated seven-octet APRS Object timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectTimestamp([u8; 7]);

impl ObjectTimestamp {
    /// Returns the timestamp bytes exactly as advertised.
    pub const fn as_bytes(&self) -> &[u8; 7] {
        &self.0
    }
}

/// A validated uncompressed APRS position retained without invented precision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawPosition {
    latitude: [u8; 8],
    symbol_table: u8,
    longitude: [u8; 9],
    symbol_code: u8,
    ambiguity: u8,
}

impl RawPosition {
    /// Returns the exact eight-octet latitude field.
    pub const fn latitude(&self) -> &[u8; 8] {
        &self.latitude
    }

    /// Returns the symbol-table identifier between latitude and longitude.
    pub const fn symbol_table(self) -> u8 {
        self.symbol_table
    }

    /// Returns the exact nine-octet longitude field.
    pub const fn longitude(&self) -> &[u8; 9] {
        &self.longitude
    }

    /// Returns the symbol code after longitude.
    pub const fn symbol_code(self) -> u8 {
        self.symbol_code
    }

    /// Returns the latitude-specified ambiguity level from zero through four.
    pub const fn ambiguity(self) -> u8 {
        self.ambiguity
    }
}

/// One validated APRS Object or Item before frequency interpretation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AprsReport<'a> {
    /// Object or Item identity class.
    pub kind: ReportKind,
    /// Case-sensitive entity name.
    pub name: ReportName,
    /// Whether this is a live report rather than a kill report.
    pub live: bool,
    /// Required Object timestamp, absent for Items.
    pub timestamp: Option<ObjectTimestamp>,
    /// Validated uncompressed position and symbol.
    pub position: RawPosition,
    /// Uninterpreted bytes following the position.
    pub comment: &'a [u8],
    /// Originating AX.25 source retained for attribution.
    pub source: Ax25Callsign,
}

/// Label used by an advertised three-digit CTCSS field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CtcssPrefix {
    /// `Tnnn` frequency-spec label.
    Tone,
    /// `Cnnn` explicit CTCSS label.
    Ctcss,
}

/// An untrusted frequency-spec tone or code token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvertisedTone {
    /// `Toff` explicitly advertises no access tone or code.
    Off,
    /// A three-digit CTCSS value whose omitted fractional digit is not inferred.
    Ctcss {
        /// Whether the sender used the `T` or `C` label.
        prefix: CtcssPrefix,
        /// Three-digit integer portion exactly represented numerically.
        integer_hz: u16,
        /// Lower-case prefix advertised narrow modulation.
        narrow: bool,
    },
    /// A three-digit DCS value not validated against a trusted code table.
    Dcs {
        /// Advertised three-digit value.
        code: u16,
        /// Lower-case prefix advertised narrow modulation.
        narrow: bool,
    },
}

/// An untrusted signed offset in the frequency specification's 10 kHz units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdvertisedOffset {
    tens_khz: i16,
}

impl AdvertisedOffset {
    /// Returns the signed advertised count of 10 kHz units.
    pub const fn tens_khz(self) -> i16 {
        self.tens_khz
    }
}

/// Unit attached to an advertised nominal range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeUnit {
    /// Statute miles, represented by `m`.
    Miles,
    /// Kilometres, represented by `k`.
    Kilometres,
}

/// An untrusted nominal range token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdvertisedRange {
    /// Advertised two-digit distance.
    pub distance: u8,
    /// Advertised distance unit.
    pub unit: RangeUnit,
}

/// One receive-only voice-repeater advertisement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepeaterAdvertisement {
    /// Object or Item identity class.
    pub kind: ReportKind,
    /// Case-sensitive Object or Item name.
    pub name: ReportName,
    /// Originating AX.25 source and SSID.
    pub source: Ax25Callsign,
    /// Raw validated position and advertised repeater symbol.
    pub position: RawPosition,
    /// Advertised repeater output, which a listener would receive.
    pub output_frequency: Frequency,
    /// Optional non-standard/cross-band repeater input advertised in a comment.
    pub alternate_input_frequency: Option<Frequency>,
    /// Optional advertised access token, never a trusted domain tone.
    pub tone: Option<AdvertisedTone>,
    /// Optional advertised signed offset, never automatically applied.
    pub offset: Option<AdvertisedOffset>,
    /// Optional advertised nominal range.
    pub range: Option<AdvertisedRange>,
}

/// A live discovery candidate or same-origin removal event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeaterEvent {
    /// A live untrusted advertisement.
    Live(RepeaterAdvertisement),
    /// A killed report naming only the identity it may remove.
    Killed {
        /// Object or Item identity class.
        kind: ReportKind,
        /// Case-sensitive report name.
        name: ReportName,
        /// Originating AX.25 source and SSID.
        source: Ax25Callsign,
    },
}

/// AX.25 or APRS receive data failed bounded parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AprsError {
    /// The complete AX.25 envelope was invalid.
    Ax25(Ax25Error),
    /// The APRS data-type identifier was not Object or Item.
    UnsupportedDataType(u8),
    /// A mandatory fixed field was truncated.
    Truncated,
    /// The Object or Item name violated its exact syntax.
    InvalidName,
    /// The live/killed marker was invalid.
    InvalidLifecycle,
    /// The required Object timestamp was malformed.
    InvalidTimestamp,
    /// A compressed or otherwise unsupported position was supplied.
    UnsupportedPosition,
    /// The uncompressed latitude, longitude, table, or symbol was malformed.
    InvalidPosition,
    /// The report did not use the voice-repeater symbol code.
    NotVoiceRepeater,
    /// No supported frequency Object name or leading frequency field existed.
    MissingFrequency,
    /// A recognized frequency encoded zero or invalid decimal text.
    InvalidFrequency,
    /// A recognized tone, offset, range, or duplicate field was malformed.
    InvalidAdvertisementField,
}

impl fmt::Display for AprsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ax25(error) => write!(formatter, "invalid AX.25 envelope: {error}"),
            Self::UnsupportedDataType(kind) => {
                write!(formatter, "unsupported APRS data type 0x{kind:02x}")
            }
            Self::Truncated => formatter.write_str("APRS report is truncated"),
            Self::InvalidName => formatter.write_str("APRS report name is invalid"),
            Self::InvalidLifecycle => formatter.write_str("APRS lifecycle marker is invalid"),
            Self::InvalidTimestamp => formatter.write_str("APRS Object timestamp is invalid"),
            Self::UnsupportedPosition => {
                formatter.write_str("APRS position encoding is unsupported")
            }
            Self::InvalidPosition => formatter.write_str("APRS uncompressed position is invalid"),
            Self::NotVoiceRepeater => {
                formatter.write_str("APRS report is not a voice-repeater symbol")
            }
            Self::MissingFrequency => {
                formatter.write_str("APRS report has no supported leading frequency")
            }
            Self::InvalidFrequency => formatter.write_str("APRS frequency field is invalid"),
            Self::InvalidAdvertisementField => {
                formatter.write_str("APRS frequency advertisement field is invalid")
            }
        }
    }
}

impl From<Ax25Error> for AprsError {
    fn from(error: Ax25Error) -> Self {
        Self::Ax25(error)
    }
}

/// Parses a validated UI frame as one supported APRS Object or Item.
pub fn parse_report<'a>(frame: &Ax25UiFrame<'a>) -> Result<AprsReport<'a>, AprsError> {
    let information = frame.information();
    match information.first().copied() {
        Some(OBJECT_TYPE) => parse_object(frame.source().callsign, information),
        Some(ITEM_TYPE) => parse_item(frame.source().callsign, information),
        Some(kind) => Err(AprsError::UnsupportedDataType(kind)),
        None => Err(AprsError::Truncated),
    }
}

/// Parses one complete AX.25 frame into a receive-only repeater event.
pub fn parse_repeater_event(frame: &[u8]) -> Result<RepeaterEvent, AprsError> {
    let ui_frame = parse_ui_frame(frame)?;
    let report = parse_report(&ui_frame)?;
    repeater_event(report)
}

fn parse_object(source: Ax25Callsign, information: &[u8]) -> Result<AprsReport<'_>, AprsError> {
    const POSITION_START: usize = 18;
    if information
        .get(POSITION_START)
        .is_some_and(|byte| !byte.is_ascii_digit())
    {
        return Err(AprsError::UnsupportedPosition);
    }
    if information.len() < POSITION_START + POSITION_LEN {
        return Err(AprsError::Truncated);
    }
    let name = object_name(&information[1..10])?;
    let live = match information[10] {
        b'*' => true,
        b'_' => false,
        _ => return Err(AprsError::InvalidLifecycle),
    };
    let timestamp = parse_timestamp(&information[11..18])?;
    let position = parse_position(&information[POSITION_START..POSITION_START + POSITION_LEN])?;
    Ok(AprsReport {
        kind: ReportKind::Object,
        name,
        live,
        timestamp: Some(timestamp),
        position,
        comment: &information[POSITION_START + POSITION_LEN..],
        source,
    })
}

fn parse_item(source: Ax25Callsign, information: &[u8]) -> Result<AprsReport<'_>, AprsError> {
    let tail = information.get(1..).ok_or(AprsError::Truncated)?;
    let delimiter = tail
        .iter()
        .take(MAX_NAME_LEN + 1)
        .position(|byte| matches!(*byte, b'!' | b'_'))
        .ok_or(AprsError::InvalidName)?;
    if !(3..=MAX_NAME_LEN).contains(&delimiter) {
        return Err(AprsError::InvalidName);
    }
    let name = item_name(&tail[..delimiter])?;
    let position_start = 1 + delimiter + 1;
    if information
        .get(position_start)
        .is_some_and(|byte| !byte.is_ascii_digit())
    {
        return Err(AprsError::UnsupportedPosition);
    }
    if information.len() < position_start + POSITION_LEN {
        return Err(AprsError::Truncated);
    }
    let position = parse_position(&information[position_start..position_start + POSITION_LEN])?;
    Ok(AprsReport {
        kind: ReportKind::Item,
        name,
        live: tail[delimiter] == b'!',
        timestamp: None,
        position,
        comment: &information[position_start + POSITION_LEN..],
        source,
    })
}

fn object_name(bytes: &[u8]) -> Result<ReportName, AprsError> {
    if bytes.len() != MAX_NAME_LEN
        || !bytes.iter().all(|byte| (b' '..=b'~').contains(byte))
        || bytes.iter().all(|byte| *byte == b' ')
    {
        return Err(AprsError::InvalidName);
    }
    let length = bytes
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    name_from_bytes(&bytes[..length])
}

fn item_name(bytes: &[u8]) -> Result<ReportName, AprsError> {
    if !(3..=MAX_NAME_LEN).contains(&bytes.len())
        || !bytes
            .iter()
            .all(|byte| (b' '..=b'~').contains(byte) && !matches!(*byte, b'!' | b'_'))
        || bytes.iter().all(|byte| *byte == b' ')
    {
        return Err(AprsError::InvalidName);
    }
    name_from_bytes(bytes)
}

fn name_from_bytes(bytes: &[u8]) -> Result<ReportName, AprsError> {
    let mut name = [0_u8; MAX_NAME_LEN];
    let destination = name.get_mut(..bytes.len()).ok_or(AprsError::InvalidName)?;
    destination.copy_from_slice(bytes);
    Ok(ReportName {
        bytes: name,
        length: u8::try_from(bytes.len()).map_err(|_| AprsError::InvalidName)?,
    })
}

fn parse_timestamp(bytes: &[u8]) -> Result<ObjectTimestamp, AprsError> {
    let timestamp: [u8; 7] = bytes.try_into().map_err(|_| AprsError::InvalidTimestamp)?;
    if !timestamp[..6].iter().all(u8::is_ascii_digit) {
        return Err(AprsError::InvalidTimestamp);
    }
    let first = decimal_pair(timestamp[0], timestamp[1]);
    let second = decimal_pair(timestamp[2], timestamp[3]);
    let third = decimal_pair(timestamp[4], timestamp[5]);
    let valid = match timestamp[6] {
        b'z' | b'/' => (1..=31).contains(&first) && second <= 23 && third <= 59,
        b'h' => first <= 23 && second <= 59 && third <= 59,
        _ => false,
    };
    if !valid {
        return Err(AprsError::InvalidTimestamp);
    }
    Ok(ObjectTimestamp(timestamp))
}

fn parse_position(bytes: &[u8]) -> Result<RawPosition, AprsError> {
    if bytes.len() != POSITION_LEN {
        return Err(AprsError::Truncated);
    }
    if !bytes[0].is_ascii_digit() {
        return Err(AprsError::UnsupportedPosition);
    }
    let mut latitude = [0_u8; 8];
    latitude.copy_from_slice(&bytes[..8]);
    let symbol_table = bytes[8];
    let mut longitude = [0_u8; 9];
    longitude.copy_from_slice(&bytes[9..18]);
    let symbol_code = bytes[18];

    let ambiguity = validate_latitude(latitude)?;
    validate_longitude(&longitude)?;
    if !matches!(symbol_table, b'/' | b'\\' | b'0'..=b'9' | b'A'..=b'Z')
        || !(b'!'..=b'~').contains(&symbol_code)
    {
        return Err(AprsError::InvalidPosition);
    }
    Ok(RawPosition {
        latitude,
        symbol_table,
        longitude,
        symbol_code,
        ambiguity,
    })
}

fn validate_latitude(latitude: [u8; 8]) -> Result<u8, AprsError> {
    if latitude[4] != b'.' || !matches!(latitude[7], b'N' | b'S') {
        return Err(AprsError::InvalidPosition);
    }
    if !latitude[..2].iter().all(u8::is_ascii_digit) {
        return Err(AprsError::InvalidPosition);
    }
    let mut ambiguity = 0_u8;
    let mut significant_digit_seen = false;
    for index in [6, 5, 3, 2] {
        match latitude[index] {
            b' ' if !significant_digit_seen => ambiguity += 1,
            byte if byte.is_ascii_digit() => significant_digit_seen = true,
            _ => return Err(AprsError::InvalidPosition),
        }
    }
    if latitude[2] != b' ' && latitude[2] > b'5' {
        return Err(AprsError::InvalidPosition);
    }
    let degrees = decimal_pair(latitude[0], latitude[1]);
    if degrees > 90
        || (degrees == 90
            && [2, 3, 5, 6]
                .iter()
                .any(|index| !matches!(latitude[*index], b'0' | b' ')))
    {
        return Err(AprsError::InvalidPosition);
    }
    Ok(ambiguity)
}

fn validate_longitude(longitude: &[u8; 9]) -> Result<(), AprsError> {
    if longitude[5] != b'.'
        || !matches!(longitude[8], b'E' | b'W')
        || !longitude[..5].iter().all(u8::is_ascii_digit)
        || !longitude[6..8].iter().all(u8::is_ascii_digit)
        || longitude[3] > b'5'
    {
        return Err(AprsError::InvalidPosition);
    }
    let degrees = u16::from(longitude[0] - b'0') * 100
        + u16::from(longitude[1] - b'0') * 10
        + u16::from(longitude[2] - b'0');
    if degrees > 180
        || (degrees == 180 && [3, 4, 6, 7].iter().any(|index| longitude[*index] != b'0'))
    {
        return Err(AprsError::InvalidPosition);
    }
    Ok(())
}

fn repeater_event(report: AprsReport<'_>) -> Result<RepeaterEvent, AprsError> {
    if report.position.symbol_code() != b'r' {
        return Err(AprsError::NotVoiceRepeater);
    }
    if !report.live {
        return Ok(RepeaterEvent::Killed {
            kind: report.kind,
            name: report.name,
            source: report.source,
        });
    }

    let name_frequency = parse_name_frequency(report.name.as_bytes())?;
    let comment_frequency = parse_comment_frequency(report.comment)?;
    let (output_frequency, alternate_input_frequency, field_start) =
        match (name_frequency, comment_frequency) {
            (Some(output), Some(input)) => (output, Some(input), 10),
            (Some(output), None) => (output, None, 0),
            (None, Some(output)) => (output, None, 10),
            (None, None) => return Err(AprsError::MissingFrequency),
        };
    let fields = parse_advertisement_fields(&report.comment[field_start..])?;
    Ok(RepeaterEvent::Live(RepeaterAdvertisement {
        kind: report.kind,
        name: report.name,
        source: report.source,
        position: report.position,
        output_frequency,
        alternate_input_frequency,
        tone: fields.tone,
        offset: fields.offset,
        range: fields.range,
    }))
}

fn parse_name_frequency(name: &[u8]) -> Result<Option<Frequency>, AprsError> {
    if name.len() != MAX_NAME_LEN {
        return Ok(None);
    }
    if name[3] != b'.' || !name[..3].iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    if name[4..7].iter().all(u8::is_ascii_digit) {
        return frequency(name, 3).map(Some);
    }
    if name[4..6].iter().all(u8::is_ascii_digit) {
        return frequency(name, 2).map(Some);
    }
    Ok(None)
}

fn parse_comment_frequency(comment: &[u8]) -> Result<Option<Frequency>, AprsError> {
    let Some(field) = comment.get(..10) else {
        return Ok(None);
    };
    let suffix_matches = field[7..].eq_ignore_ascii_case(b"MHz");
    if field[3] == b'.'
        && field[..3].iter().all(u8::is_ascii_digit)
        && field[4..7].iter().all(u8::is_ascii_digit)
        && suffix_matches
    {
        return frequency(field, 3).map(Some);
    }
    if field[3] == b'.'
        && field[..3].iter().all(u8::is_ascii_digit)
        && field[4..6].iter().all(u8::is_ascii_digit)
        && field[6] == b' '
        && field[7..].eq_ignore_ascii_case(b"MHz")
    {
        return frequency(field, 2).map(Some);
    }
    Ok(None)
}

fn frequency(bytes: &[u8], fractional_digits: usize) -> Result<Frequency, AprsError> {
    let whole = u32::from(bytes[0] - b'0') * 100
        + u32::from(bytes[1] - b'0') * 10
        + u32::from(bytes[2] - b'0');
    let fraction = if fractional_digits == 3 {
        u32::from(bytes[4] - b'0') * 100
            + u32::from(bytes[5] - b'0') * 10
            + u32::from(bytes[6] - b'0')
    } else {
        u32::from(bytes[4] - b'0') * 10 + u32::from(bytes[5] - b'0')
    };
    let scale = if fractional_digits == 3 {
        1_000
    } else {
        10_000
    };
    let hz = whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fraction * scale))
        .ok_or(AprsError::InvalidFrequency)?;
    Frequency::from_hz(hz).map_err(|_| AprsError::InvalidFrequency)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AdvertisementFields {
    tone: Option<AdvertisedTone>,
    offset: Option<AdvertisedOffset>,
    range: Option<AdvertisedRange>,
}

fn parse_advertisement_fields(comment: &[u8]) -> Result<AdvertisementFields, AprsError> {
    let mut fields = AdvertisementFields::default();
    let mut remainder = comment;
    loop {
        remainder = trim_spaces(remainder);
        if remainder.is_empty() {
            return Ok(fields);
        }
        let token_end = remainder
            .iter()
            .position(|byte| *byte == b' ')
            .unwrap_or(remainder.len());
        let token = &remainder[..token_end];
        let recognized = match token.first().copied() {
            Some(b'T' | b't' | b'C' | b'c' | b'D' | b'd')
                if token.len() == 4 || token == b"Toff" =>
            {
                if fields.tone.is_some() {
                    return Err(AprsError::InvalidAdvertisementField);
                }
                fields.tone = Some(parse_tone(token)?);
                true
            }
            Some(b'+' | b'-') if token.len() == 4 => {
                if fields.offset.is_some() {
                    return Err(AprsError::InvalidAdvertisementField);
                }
                fields.offset = Some(parse_offset(token)?);
                true
            }
            Some(b'R') if token.len() == 4 => {
                if fields.range.is_some() {
                    return Err(AprsError::InvalidAdvertisementField);
                }
                fields.range = Some(parse_range(token)?);
                true
            }
            _ => false,
        };
        if !recognized {
            return Ok(fields);
        }
        remainder = &remainder[token_end..];
    }
}

fn parse_tone(token: &[u8]) -> Result<AdvertisedTone, AprsError> {
    if token == b"Toff" {
        return Ok(AdvertisedTone::Off);
    }
    if token.len() != 4 || !token[1..].iter().all(u8::is_ascii_digit) {
        return Err(AprsError::InvalidAdvertisementField);
    }
    let value = three_digits(&token[1..]);
    match token[0] {
        b'T' | b't' => Ok(AdvertisedTone::Ctcss {
            prefix: CtcssPrefix::Tone,
            integer_hz: value,
            narrow: token[0].is_ascii_lowercase(),
        }),
        b'C' | b'c' => Ok(AdvertisedTone::Ctcss {
            prefix: CtcssPrefix::Ctcss,
            integer_hz: value,
            narrow: token[0].is_ascii_lowercase(),
        }),
        b'D' | b'd' => Ok(AdvertisedTone::Dcs {
            code: value,
            narrow: token[0].is_ascii_lowercase(),
        }),
        _ => Err(AprsError::InvalidAdvertisementField),
    }
}

fn parse_offset(token: &[u8]) -> Result<AdvertisedOffset, AprsError> {
    if token.len() != 4 || !token[1..].iter().all(u8::is_ascii_digit) {
        return Err(AprsError::InvalidAdvertisementField);
    }
    let magnitude = i16::try_from(three_digits(&token[1..]))
        .map_err(|_| AprsError::InvalidAdvertisementField)?;
    let tens_khz = if token[0] == b'-' {
        -magnitude
    } else {
        magnitude
    };
    Ok(AdvertisedOffset { tens_khz })
}

fn parse_range(token: &[u8]) -> Result<AdvertisedRange, AprsError> {
    if token.len() != 4 || !token[1..3].iter().all(u8::is_ascii_digit) {
        return Err(AprsError::InvalidAdvertisementField);
    }
    let distance = (token[1] - b'0') * 10 + (token[2] - b'0');
    let unit = match token[3] {
        b'm' => RangeUnit::Miles,
        b'k' => RangeUnit::Kilometres,
        _ => return Err(AprsError::InvalidAdvertisementField),
    };
    Ok(AdvertisedRange { distance, unit })
}

fn trim_spaces(mut bytes: &[u8]) -> &[u8] {
    while bytes.first() == Some(&b' ') {
        bytes = &bytes[1..];
    }
    bytes
}

const fn decimal_pair(high: u8, low: u8) -> u8 {
    (high - b'0') * 10 + (low - b'0')
}

fn three_digits(bytes: &[u8]) -> u16 {
    u16::from(bytes[0] - b'0') * 100 + u16::from(bytes[1] - b'0') * 10 + u16::from(bytes[2] - b'0')
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        parse_repeater_event, AdvertisedRange, AdvertisedTone, AprsError, CtcssPrefix, RangeUnit,
        RepeaterEvent, ReportKind,
    };
    use std::vec::Vec;

    fn fcs_accumulator(bytes: &[u8]) -> u16 {
        let mut fcs = 0xffff_u16;
        for byte in bytes {
            fcs ^= u16::from(*byte);
            for _ in 0..8 {
                fcs = if fcs & 1 == 0 {
                    fcs >> 1
                } else {
                    (fcs >> 1) ^ 0x8408
                };
            }
        }
        fcs
    }

    fn address(callsign: &[u8], ssid: u8, final_address: bool) -> [u8; 7] {
        let mut encoded = [b' ' << 1; 7];
        for (destination, source) in encoded.iter_mut().zip(callsign.iter().copied()) {
            *destination = source << 1;
        }
        encoded[6] = 0x60 | (ssid << 1) | u8::from(final_address);
        encoded
    }

    fn frame(source: &[u8], ssid: u8, information: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&address(b"APRS", 0, false));
        bytes.extend_from_slice(&address(source, ssid, true));
        bytes.extend_from_slice(&[0x03, 0xf0]);
        bytes.extend_from_slice(information);
        let fcs = !fcs_accumulator(&bytes);
        bytes.extend_from_slice(&fcs.to_le_bytes());
        bytes
    }

    #[test]
    fn frequency_object_preserves_untrusted_fields_without_deriving_tx() {
        let bytes = frame(
            b"DIGI",
            2,
            b";146.940-A*111111z4903.50N/07201.75WrT107 -060 R25m Local",
        );
        let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
            panic!("expected live candidate");
        };
        assert_eq!(candidate.kind, ReportKind::Object);
        assert_eq!(candidate.name.as_bytes(), b"146.940-A");
        assert_eq!(candidate.source.as_bytes(), b"DIGI");
        assert_eq!(candidate.source.ssid(), 2);
        assert_eq!(candidate.output_frequency.as_hz(), 146_940_000);
        assert_eq!(candidate.alternate_input_frequency, None);
        assert_eq!(
            candidate.tone,
            Some(AdvertisedTone::Ctcss {
                prefix: CtcssPrefix::Tone,
                integer_hz: 107,
                narrow: false,
            })
        );
        assert_eq!(candidate.offset.unwrap().tens_khz(), -60);
        assert_eq!(
            candidate.range,
            Some(AdvertisedRange {
                distance: 25,
                unit: RangeUnit::Miles,
            })
        );
        assert_eq!(candidate.position.ambiguity(), 0);
    }

    #[test]
    fn object_comment_frequency_is_an_alternate_input_and_receive_is_case_insensitive() {
        let bytes = frame(
            b"DIGI",
            0,
            b";146.940-A*111111z4903.50N/07201.75Wr147.540mHz c067 +060 R10k",
        );
        let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
            panic!("expected live candidate");
        };
        assert_eq!(candidate.output_frequency.as_hz(), 146_940_000);
        assert_eq!(
            candidate.alternate_input_frequency.unwrap().as_hz(),
            147_540_000
        );
        assert_eq!(
            candidate.tone,
            Some(AdvertisedTone::Ctcss {
                prefix: CtcssPrefix::Ctcss,
                integer_hz: 67,
                narrow: true,
            })
        );
        assert_eq!(candidate.offset.unwrap().tens_khz(), 60);
        assert_eq!(candidate.range.unwrap().unit, RangeUnit::Kilometres);
    }

    #[test]
    fn item_leading_frequency_and_latitude_ambiguity_are_bounded() {
        let bytes = frame(
            b"N0CALL",
            7,
            b")LOCAL!4903.5 N/07201.75Wr146.52 MHz Toff -000 R05m nearby",
        );
        let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
            panic!("expected live candidate");
        };
        assert_eq!(candidate.kind, ReportKind::Item);
        assert_eq!(candidate.name.as_bytes(), b"LOCAL");
        assert_eq!(candidate.position.ambiguity(), 1);
        assert_eq!(candidate.output_frequency.as_hz(), 146_520_000);
        assert_eq!(candidate.tone, Some(AdvertisedTone::Off));
        assert_eq!(candidate.offset.unwrap().tens_khz(), 0);
    }

    #[test]
    fn every_uncompressed_latitude_ambiguity_level_is_preserved() {
        for (latitude, expected) in [
            (b"4903.50N".as_slice(), 0),
            (b"4903.5 N".as_slice(), 1),
            (b"4903.  N".as_slice(), 2),
            (b"490 .  N".as_slice(), 3),
            (b"49  .  N".as_slice(), 4),
        ] {
            let mut information = b")MAP  !".to_vec();
            information.extend_from_slice(latitude);
            information.extend_from_slice(b"/07201.75Wr146.52 MHz");
            let bytes = frame(b"N0CALL", 0, &information);
            let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
                panic!("expected live candidate");
            };
            assert_eq!(candidate.name.as_bytes(), b"MAP  ");
            assert_eq!(candidate.position.ambiguity(), expected);
        }
    }

    #[test]
    fn dcs_and_two_decimal_frequency_object_remain_advertised_only() {
        let bytes = frame(
            b"DIGI",
            0,
            b";146.94-xy*111111z4903.50N/07201.75Wrd256 Repeater",
        );
        let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
            panic!("expected live candidate");
        };
        assert_eq!(candidate.output_frequency.as_hz(), 146_940_000);
        assert_eq!(
            candidate.tone,
            Some(AdvertisedTone::Dcs {
                code: 256,
                narrow: true,
            })
        );
    }

    #[test]
    fn killed_report_needs_no_frequency_but_retains_same_origin_identity() {
        let bytes = frame(b"DIGI", 4, b";LOCAL    _111111z4903.50N/07201.75Wrretired");
        assert!(matches!(
            parse_repeater_event(&bytes).unwrap(),
            RepeaterEvent::Killed {
                kind: ReportKind::Object,
                name,
                source,
            } if name.as_bytes() == b"LOCAL" && source.as_bytes() == b"DIGI" && source.ssid() == 4
        ));
    }

    #[test]
    fn unsupported_and_malformed_position_cases_fail_explicitly() {
        let compressed = frame(b"DIGI", 0, b";146.940-A*111111z/5L!!<*e7rT107");
        assert_eq!(
            parse_repeater_event(&compressed),
            Err(AprsError::UnsupportedPosition)
        );

        let non_progressive = frame(b"DIGI", 0, b";146.940-A*111111z4903. 5N/07201.75Wr");
        assert_eq!(
            parse_repeater_event(&non_progressive),
            Err(AprsError::InvalidPosition)
        );

        let ambiguous_longitude = frame(b"DIGI", 0, b";146.940-A*111111z4903.  N/07201.  Wr");
        assert_eq!(
            parse_repeater_event(&ambiguous_longitude),
            Err(AprsError::InvalidPosition)
        );

        let wrong_symbol = frame(b"DIGI", 0, b";146.940-A*111111z4903.50N/07201.75W-");
        assert_eq!(
            parse_repeater_event(&wrong_symbol),
            Err(AprsError::NotVoiceRepeater)
        );
    }

    #[test]
    fn malformed_time_frequency_and_recognized_fields_are_rejected() {
        let timestamp = frame(b"DIGI", 0, b";146.940-A*321111z4903.50N/07201.75Wr");
        assert_eq!(
            parse_repeater_event(&timestamp),
            Err(AprsError::InvalidTimestamp)
        );

        let zero_frequency = frame(b"DIGI", 0, b";000.000-A*111111z4903.50N/07201.75Wr");
        assert_eq!(
            parse_repeater_event(&zero_frequency),
            Err(AprsError::InvalidFrequency)
        );

        let malformed_tone = frame(b"DIGI", 0, b";146.940-A*111111z4903.50N/07201.75WrT1O7");
        assert_eq!(
            parse_repeater_event(&malformed_tone),
            Err(AprsError::InvalidAdvertisementField)
        );

        let duplicate_range = frame(
            b"DIGI",
            0,
            b";146.940-A*111111z4903.50N/07201.75WrR10m R20m",
        );
        assert_eq!(
            parse_repeater_event(&duplicate_range),
            Err(AprsError::InvalidAdvertisementField)
        );
    }

    #[test]
    fn unrelated_trailing_comment_does_not_become_policy_data() {
        let bytes = frame(
            b"DIGI",
            0,
            b";146.940-A*111111z4903.50N/07201.75WrRepeater +999 R99m",
        );
        let RepeaterEvent::Live(candidate) = parse_repeater_event(&bytes).unwrap() else {
            panic!("expected live candidate");
        };
        assert_eq!(candidate.tone, None);
        assert_eq!(candidate.offset, None);
        assert_eq!(candidate.range, None);
    }
}
