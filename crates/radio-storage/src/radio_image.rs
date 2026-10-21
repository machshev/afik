//! The image a radio runs: storage format 8, one block per bank, then the
//! bank groups.
//!
//! The host compiles a project into this image and writes it into the radio's
//! memory (`ADR-100`); the radio reads it in place. Every channel lives in a
//! bank, and each bank is exactly one block carried in its bank-table entry
//! (`ADR-115`), so the radio's RAM index is one small entry per bank and a
//! bank filter is a range of channel numbers. A group (`ADR-116`) is a name
//! and the banks in it; a location's locator stays in the project.
//!
//! A *list* bank holds explicit channels. The values all of its channels
//! share are stored once in the list header; a `fields` bitfield names the
//! values each record carries instead, so a PMR or simplex list stores little
//! more than a short frequency and a name per channel. Records in a list all
//! have the same size, so any channel is read directly by its position.
//!
//! A *linear* bank is a generated plan: a base, a spacing and a count, with
//! no records at all.
//!
//! ```text
//! header  : magic "AFIK", image version 2, format 8, bank count u16,
//!           payload length u32, CRC-32 (as format 4)
//! payload : radio configuration (16 bytes, as its format 4 object)
//!           TX range count u8, then each range's 14 bytes (`ADR-101`)
//!           banks in ascending id, each: id u8, flags u8, kind u8 (1 list,
//!           2 linear), body length u16, name length u8, name bytes, body
//!           group count u8, then per group: name length u8, name bytes,
//!           its banks as 32 bytes, bank n bit n % 8 of byte n / 8 (each a
//!           bank above, at least one)
//! linear  : the generated-bank object bytes, its id the bank's
//! list    : fields u16, name width u8, count u16, base Hz u32, raster Hz u32,
//!           TX offset i32, step Hz u32, RX tone 3, TX tone 3, mode u8,
//!           flags u8, reserved u8 (was squelch); then `count` records
//! record  : name (name width), RX, [TX], [RX tone], [TX tone], [mode],
//!           [flags], [squelch, ignored], [step], as `fields` says
//! ```
//!
//! A frequency is a `u16` count of rasters above the base when
//! [`SHORT_FREQUENCY`] is set, and a `u32` in hertz otherwise. Without
//! [`TRANSMIT`], TX is RX plus the list's TX offset. The mode byte is
//! modulation | bandwidth << 2 | power << 3.

use radio_channel_plan::{
    BankFlags, BankGroup, BankName, BankSet, ChannelBank, ChannelDefinition, ChannelFlags,
    ChannelName, ChannelRecord, GeneratedBank, MAX_BANKS, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN,
    MAX_GROUPS,
};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    RadioFlags, ScanResume, SquelchLevel, Tone,
};

use radio_tx_policy::{RangeError, TxPolicy, TxRange, MAX_TX_RANGES};

use super::{
    configuration_image_crc, crc32_update, decode_generated_bank, decode_radio_config, decode_tone,
    decode_tx_range_body, encode_generated_bank, encode_radio_config, encode_tone,
    encode_tx_range_body, ObjectKey, ObjectKind, ObjectRef, StorageError,
    CONFIGURATION_IMAGE_MAGIC, MAX_GENERATED_BANK_ENCODED_LEN, RADIO_CONFIG_ENCODED_LEN,
    TX_RANGE_ENCODED_LEN,
};

/// Image container version for format 8.
pub const IMAGE_VERSION: u8 = 2;
/// Object format version this module reads and writes.
pub const FORMAT_VERSION: u8 = 8;
/// Bytes in the image header.
pub const HEADER_LEN: usize = 16;
/// The bank byte meaning no bank; no bank may have this identifier.
pub const NO_BANK: u8 = 0xFF;

// Every bank identifier fits the entry's byte and is not `NO_BANK`.
const _: () = assert!(MAX_BANKS < NO_BANK as u16 + 1);

/// Records carry a `u16` count of rasters above the base, not hertz.
pub const SHORT_FREQUENCY: u16 = 1 << 0;
/// Records carry their own TX frequency.
pub const TRANSMIT: u16 = 1 << 1;
/// Records carry their own RX tone.
pub const RX_TONE: u16 = 1 << 2;
/// Records carry their own TX tone.
pub const TX_TONE: u16 = 1 << 3;
/// Records carry their own mode byte.
pub const MODE: u16 = 1 << 4;
/// Records carry their own channel flags.
pub const FLAGS: u16 = 1 << 5;
/// Records carried their own squelch level.
///
/// Squelch is radio-wide, so the compiler never sets it; an image an older
/// studio wrote may, and the radio skips the byte.
pub const SQUELCH: u16 = 1 << 6;
/// Records carry their own tuning step.
pub const STEP: u16 = 1 << 7;
const KNOWN_FIELDS: u16 = (1 << 8) - 1;

const KIND_LIST: u8 = 1;
const KIND_LINEAR: u8 = 2;
const LIST_HEADER_LEN: usize = 30;
/// A bank entry before its name: id, flags, kind, body length, name length.
const BANK_ENTRY_HEADER_LEN: usize = 6;
/// Largest list record: every field present at full width.
const MAX_RECORD_LEN: usize = MAX_CHANNEL_NAME_LEN + 4 + 4 + 3 + 3 + 1 + 1 + 1 + 4;
/// Payload offset meaning "none". The radio configuration sits at offset 0,
/// so nothing an index points to can, and an empty index is all zeros: the
/// firmware's static index then costs no flash for its initial value.
const ABSENT: u16 = 0;
/// The configuration field of an index no image has filled; [`Index::config`]
/// reports [`RadioConfig::conservative`] instead. All zeros, as above.
const UNSET_CONFIG: RadioConfig = RadioConfig {
    squelch: SquelchLevel::OPEN,
    backlight_seconds: 0,
    scan_resume: ScanResume::TimeOut,
    scan_dwell_ms: 0,
    scan_hold_ms: 0,
    dual_watch: false,
    battery_save_ratio: 0,
    flags: match RadioFlags::from_bits(0) {
        Ok(flags) => flags,
        Err(_) => panic!("no flags is a valid flag set"),
    },
    tx_power: 0,
    tx_timeout: 0,
};

/// Why a project cannot become a radio image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    /// Two generated plans claim one bank.
    DuplicatePlan(BankId),
    /// A bank holds both explicit channels and a generated plan.
    MixedBank(BankId),
    /// A channel is in no bank.
    UnbankedChannel(ChannelId),
    /// The image does not fit the output.
    TooLarge,
    /// A TX range is malformed, or there are too many.
    TxRange(RangeError),
    /// An object failed its own encoding.
    Storage(StorageError),
}

impl core::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicatePlan(bank) => {
                write!(formatter, "bank {} holds two generated plans", bank.get())
            }
            Self::MixedBank(bank) => write!(
                formatter,
                "bank {} holds both channels and a generated plan",
                bank.get()
            ),
            Self::UnbankedChannel(channel) => {
                write!(formatter, "channel {} is in no bank", channel.get())
            }
            Self::TooLarge => formatter.write_str("the image does not fit the radio's memory"),
            Self::TxRange(error) => write!(formatter, "{error}"),
            Self::Storage(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<StorageError> for CompileError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Bounded writer over the output buffer.
struct Writer<'a> {
    output: &'a mut [u8],
    at: usize,
}

impl Writer<'_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), CompileError> {
        let end = self.at + bytes.len();
        self.output
            .get_mut(self.at..end)
            .ok_or(CompileError::TooLarge)?
            .copy_from_slice(bytes);
        self.at = end;
        Ok(())
    }

    fn u8(&mut self, value: u8) -> Result<(), CompileError> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), CompileError> {
        self.bytes(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> Result<(), CompileError> {
        self.bytes(&value.to_le_bytes())
    }

    fn tone(&mut self, tone: Tone) -> Result<(), CompileError> {
        let mut field = [0_u8; 3];
        encode_tone(tone, &mut field);
        self.bytes(&field)
    }
}

/// The bank a channel is stored in, or [`NO_BANK`].
fn bank_of(channel: &ChannelRecord) -> u8 {
    channel
        .bank()
        .and_then(|bank| u8::try_from(bank.get()).ok())
        .unwrap_or(NO_BANK)
}

const fn mode_byte(channel: &ChannelRecord) -> u8 {
    channel.modulation() as u8 | (channel.bandwidth() as u8) << 2 | (channel.power() as u8) << 3
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    a
}

/// How one list block lays out its records.
#[derive(Clone, Copy, Debug)]
struct ListLayout {
    fields: u16,
    name_width: u8,
    base: u32,
    raster: u32,
    tx_offset: i32,
    template: ChannelRecord,
}

impl ListLayout {
    /// Chooses the shared values and the per-record fields for `members`.
    fn choose<'a>(mut members: impl Iterator<Item = &'a ChannelRecord> + Clone) -> Option<Self> {
        let template = *members.clone().next()?;
        let mut fields = 0;
        let mut name_width = 0;
        let offset = |channel: &ChannelRecord| {
            i64::from(channel.transmit().as_hz()) - i64::from(channel.receive().as_hz())
        };
        let mut base = u32::MAX;
        for channel in members.clone() {
            name_width = name_width.max(channel.name().len());
            if offset(channel) != offset(&template) {
                fields |= TRANSMIT;
            }
            for (differs, field) in [
                (channel.rx_tone() != template.rx_tone(), RX_TONE),
                (channel.tx_tone() != template.tx_tone(), TX_TONE),
                (mode_byte(channel) != mode_byte(&template), MODE),
                (channel.flags() != template.flags(), FLAGS),
                (channel.step() != template.step(), STEP),
            ] {
                if differs {
                    fields |= field;
                }
            }
            base = base.min(channel.receive().as_hz());
            if fields & TRANSMIT != 0 {
                base = base.min(channel.transmit().as_hz());
            }
        }
        // A second pass, because TRANSMIT may have been found only late.
        if fields & TRANSMIT != 0 {
            for channel in members.clone() {
                base = base.min(channel.transmit().as_hz());
            }
        }
        let mut raster = 0;
        let mut span = 0;
        for channel in members.by_ref() {
            for hz in [
                Some(channel.receive().as_hz()),
                (fields & TRANSMIT != 0).then(|| channel.transmit().as_hz()),
            ]
            .into_iter()
            .flatten()
            {
                raster = gcd(raster, hz - base);
                span = span.max(hz - base);
            }
        }
        let raster = raster.max(1);
        if u16::try_from(span / raster).is_ok() {
            fields |= SHORT_FREQUENCY;
        }
        Some(Self {
            fields,
            name_width,
            base,
            raster,
            tx_offset: i32::try_from(offset(&template)).unwrap_or(0),
            template,
        })
    }
}

/// Bytes one record of a list block occupies.
fn record_len(fields: u16, name_width: u8) -> usize {
    let frequency = if fields & SHORT_FREQUENCY != 0 { 2 } else { 4 };
    let mut len = usize::from(name_width) + frequency;
    for (field, bytes) in [
        (TRANSMIT, frequency),
        (RX_TONE, 3),
        (TX_TONE, 3),
        (MODE, 1),
        (FLAGS, 1),
        (SQUELCH, 1),
        (STEP, 4),
    ] {
        if fields & field != 0 {
            len += bytes;
        }
    }
    len
}

/// Bytes the list body of `members` occupies.
fn list_len<'a>(members: impl Iterator<Item = &'a ChannelRecord> + Clone) -> Option<usize> {
    let layout = ListLayout::choose(members.clone())?;
    Some(LIST_HEADER_LEN + members.count() * record_len(layout.fields, layout.name_width))
}

/// Writes the list body of `members`, which must not be empty.
fn write_list<'a>(
    writer: &mut Writer<'_>,
    members: impl Iterator<Item = &'a ChannelRecord> + Clone,
) -> Result<(), CompileError> {
    let Some(layout) = ListLayout::choose(members.clone()) else {
        return Ok(());
    };
    let count = members.clone().count();
    let template = layout.template;
    writer.u16(layout.fields)?;
    writer.u8(layout.name_width)?;
    writer.u16(u16::try_from(count).map_err(|_| CompileError::TooLarge)?)?;
    writer.u32(layout.base)?;
    writer.u32(layout.raster)?;
    writer.bytes(&layout.tx_offset.to_le_bytes())?;
    writer.u32(template.step().as_hz())?;
    writer.tone(template.rx_tone())?;
    writer.tone(template.tx_tone())?;
    writer.u8(mode_byte(&template))?;
    writer.u8(template.flags().bits())?;
    // Squelch is radio-wide; this byte held a level and is reserved.
    writer.u8(0)?;
    let frequency = |writer: &mut Writer<'_>, hz: u32| {
        if layout.fields & SHORT_FREQUENCY != 0 {
            writer.u16(u16::try_from((hz - layout.base) / layout.raster).unwrap_or(0))
        } else {
            writer.u32(hz)
        }
    };
    for channel in members {
        let name = channel.name().field();
        writer.bytes(&name[..usize::from(layout.name_width)])?;
        frequency(writer, channel.receive().as_hz())?;
        let fields = layout.fields;
        if fields & TRANSMIT != 0 {
            frequency(writer, channel.transmit().as_hz())?;
        }
        if fields & RX_TONE != 0 {
            writer.tone(channel.rx_tone())?;
        }
        if fields & TX_TONE != 0 {
            writer.tone(channel.tx_tone())?;
        }
        if fields & MODE != 0 {
            writer.u8(mode_byte(channel))?;
        }
        if fields & FLAGS != 0 {
            writer.u8(channel.flags().bits())?;
        }
        if fields & STEP != 0 {
            writer.u32(channel.step().as_hz())?;
        }
    }
    Ok(())
}

/// Compiles a validated project into a radio image and returns its length.
///
/// Banks are written in identifier order, each with its explicit channels in
/// the order given or its generated plan; a bank with neither is left out.
/// The radio numbers channels in this order.
pub fn compile(
    config: RadioConfig,
    tx_ranges: &[TxRange],
    banks: &[ChannelBank],
    channels: &[ChannelRecord],
    plans: &[GeneratedBank],
    groups: &[BankGroup],
    output: &mut [u8],
) -> Result<usize, CompileError> {
    for (index, plan) in plans.iter().enumerate() {
        if plans[..index].iter().any(|other| other.id() == plan.id()) {
            return Err(CompileError::DuplicatePlan(plan.id()));
        }
    }
    if let Some(channel) = channels.iter().find(|channel| bank_of(channel) == NO_BANK) {
        return Err(CompileError::UnbankedChannel(channel.id()));
    }
    let mut writer = Writer {
        output,
        at: HEADER_LEN,
    };
    writer.bytes(encode_radio_config(config)?.data())?;
    TxPolicy::new(tx_ranges).map_err(CompileError::TxRange)?;
    writer.u8(u8::try_from(tx_ranges.len()).map_err(|_| CompileError::TooLarge)?)?;
    for range in tx_ranges {
        writer.bytes(&encode_tx_range_body(*range))?;
    }
    let mut bank_count = 0_u16;
    let mut held = BankSet::EMPTY;
    for bank in 0..u8::try_from(MAX_BANKS).unwrap_or(0) {
        let members = channels
            .iter()
            .filter(move |channel| bank_of(channel) == bank);
        let plan = plans
            .iter()
            .find(|plan| u8::try_from(plan.id().get()).ok() == Some(bank));
        let (kind, body_len, plan) = match (list_len(members.clone()), plan) {
            (Some(_), Some(plan)) => return Err(CompileError::MixedBank(plan.id())),
            (Some(len), None) => (KIND_LIST, len, None),
            (None, Some(plan)) => {
                let object = encode_generated_bank(*plan)?;
                (KIND_LINEAR, object.len(), Some(object))
            }
            (None, None) => continue,
        };
        let named = banks
            .iter()
            .find(|named| u8::try_from(named.id().get()).ok() == Some(bank));
        let name = named.map(|named| named.name());
        writer.u8(bank)?;
        writer.u8(named.map_or(0, |named| named.flags().bits()))?;
        writer.u8(kind)?;
        writer.u16(u16::try_from(body_len).map_err(|_| CompileError::TooLarge)?)?;
        let name_len = name.map_or(0, BankName::len);
        writer.u8(name_len)?;
        if let Some(name) = name {
            writer.bytes(&name.field()[..usize::from(name_len)])?;
        }
        match plan {
            Some(object) => writer.bytes(object.data())?,
            None => write_list(&mut writer, members)?,
        }
        bank_count += 1;
        held.insert(BankId::new(u16::from(bank)));
    }
    // Groups in identifier order, each keeping only banks the image holds; a
    // group left with none is dropped, so the radio never offers an empty one.
    let mut ordered: [Option<&BankGroup>; MAX_GROUPS as usize] = [None; MAX_GROUPS as usize];
    for group in groups {
        if let Some(slot) = ordered.get_mut(usize::from(group.id())) {
            *slot = Some(group);
        }
    }
    let kept = || {
        ordered.iter().flatten().filter_map(|group| {
            let banks: BankSet = group
                .banks()
                .iter()
                .filter(|bank| held.contains(*bank))
                .fold(BankSet::EMPTY, BankSet::with);
            (!banks.is_empty()).then_some((group.name(), banks))
        })
    };
    writer.u8(u8::try_from(kept().count()).map_err(|_| CompileError::TooLarge)?)?;
    for (name, banks) in kept() {
        writer.u8(name.len())?;
        writer.bytes(&name.field()[..usize::from(name.len())])?;
        writer.bytes(&banks.to_bytes())?;
    }
    let len = writer.at;
    let output = writer.output;
    output[..4].copy_from_slice(&CONFIGURATION_IMAGE_MAGIC);
    output[4] = IMAGE_VERSION;
    output[5] = FORMAT_VERSION;
    output[6..8].copy_from_slice(&bank_count.to_le_bytes());
    output[8..12].copy_from_slice(&u32::try_from(len - HEADER_LEN).unwrap_or(0).to_le_bytes());
    let crc = configuration_image_crc(&output[..12], &output[HEADER_LEN..len]);
    output[12..16].copy_from_slice(&crc.to_le_bytes());
    Ok(len)
}

/// One bank as the radio indexes it: four bytes.
///
/// Every bank the image holds has at least one channel and they number in
/// identifier order, so a bank's channels run from its `first` to the next
/// held bank's.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct BankSlot {
    /// Payload offset of the bank's entry; [`ABSENT`] for a bank the image
    /// does not hold.
    entry: u16,
    /// Number of the bank's first channel on the radio.
    first: u16,
}

/// A bank's entry as read from the image.
#[derive(Clone, Copy, Debug)]
struct BankEntry {
    flags: u8,
    kind: u8,
    body_len: usize,
    name_len: u8,
    /// Payload offset of the name.
    name: usize,
    /// Payload offset of the body.
    body: usize,
}

/// What a radio keeps in RAM about the image it runs: one entry per bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Index {
    config: RadioConfig,
    /// Whether `config` came from an image.
    configured: bool,
    /// Payload offset of the first TX range and how many there are.
    tx_ranges: u16,
    tx_range_count: u8,
    banks: [BankSlot; MAX_BANKS as usize],
    /// Payload offset of each group's entry.
    groups: [u16; MAX_GROUPS as usize],
    group_count: u8,
    len: u16,
    populated: BankSet,
    /// Bytes the image occupies, header included.
    bytes: u16,
}

impl Default for Index {
    fn default() -> Self {
        Self::empty()
    }
}

/// Bytes a [`Stream`] reads from memory at once.
const STREAM_WINDOW_BYTES: usize = 64;

/// A payload read in order, one window at a time, and checksummed as it
/// arrives.
///
/// Decoding reads the payload front to back in small pieces. Serving those
/// from a window, and checksumming each window as it is fetched, reads the
/// payload from memory once and in few transfers, where checking it and then
/// decoding it read it twice, the second time in hundreds of small reads.
struct Stream<'a, R> {
    read: &'a mut R,
    len: usize,
    crc: u32,
    window: [u8; STREAM_WINDOW_BYTES],
    /// Payload offset of the window's first byte.
    start: usize,
    /// Payload offset just past the last byte fetched.
    end: usize,
}

impl<R: FnMut(usize, &mut [u8]) -> bool> Stream<'_, R> {
    /// Fetches the next window, checksumming it.
    fn advance(&mut self) -> bool {
        let len = STREAM_WINDOW_BYTES.min(self.len - self.end);
        if len == 0 || !(self.read)(self.end, &mut self.window[..len]) {
            return false;
        }
        self.crc = crc32_update(self.crc, &self.window[..len]);
        self.start = self.end;
        self.end += len;
        true
    }

    /// Fills `buffer` from `offset`, which must lie inside the payload.
    fn read(&mut self, offset: usize, buffer: &mut [u8]) -> bool {
        if offset < self.start {
            // Behind the window: read directly. The checksum already has it.
            return offset + buffer.len() <= self.len && (self.read)(offset, buffer);
        }
        for (position, byte) in (offset..).zip(buffer.iter_mut()) {
            while position >= self.end {
                if !self.advance() {
                    return false;
                }
            }
            *byte = self.window[position - self.start];
        }
        true
    }

    /// Fetches whatever decoding did not reach and returns the checksum.
    fn finish(&mut self) -> Result<u32, StorageError> {
        while self.end < self.len {
            if !self.advance() {
                return Err(StorageError::MalformedImage);
            }
        }
        Ok(self.crc)
    }
}

/// Reads exactly `N` bytes at a payload offset.
fn read_array<const N: usize>(
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    offset: usize,
) -> Result<[u8; N], StorageError> {
    let mut bytes = [0_u8; N];
    if read(offset, &mut bytes) {
        Ok(bytes)
    } else {
        Err(StorageError::MalformedImage)
    }
}

/// Builds a policy from decoded ranges, denying everything if one is absent.
fn policy_of(ranges: &[Option<TxRange>]) -> TxPolicy {
    let mut flat = [TxRange {
        class: radio_domain::TxClass::LicenceFreePlan,
        first_hz: 1,
        last_hz: 1,
        max_mw: 1,
        wide_fm: false,
        raster_hz: 0,
    }; MAX_TX_RANGES];
    for (slot, range) in flat.iter_mut().zip(ranges) {
        match range {
            Some(range) => *slot = *range,
            None => return TxPolicy::deny_all(),
        }
    }
    TxPolicy::new(&flat[..ranges.len()]).unwrap_or_default()
}

/// A list block's header, decoded.
#[derive(Clone, Copy, Debug)]
struct ListHeader {
    fields: u16,
    name_width: u8,
    count: u16,
    base: u32,
    raster: u32,
    tx_offset: i32,
    step: u32,
    rx_tone: [u8; 3],
    tx_tone: [u8; 3],
    mode: u8,
    flags: u8,
}

impl ListHeader {
    fn decode(bytes: &[u8; LIST_HEADER_LEN]) -> Result<Self, StorageError> {
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let header = Self {
            fields: u16::from_le_bytes([bytes[0], bytes[1]]),
            name_width: bytes[2],
            count: u16::from_le_bytes([bytes[3], bytes[4]]),
            base: u32_at(5),
            raster: u32_at(9),
            tx_offset: i32::from_le_bytes([bytes[13], bytes[14], bytes[15], bytes[16]]),
            step: u32_at(17),
            rx_tone: [bytes[21], bytes[22], bytes[23]],
            tx_tone: [bytes[24], bytes[25], bytes[26]],
            mode: bytes[27],
            flags: bytes[28],
        };
        if header.fields & !KNOWN_FIELDS != 0
            || usize::from(header.name_width) > MAX_CHANNEL_NAME_LEN
            || header.raster == 0
        {
            return Err(StorageError::MalformedObject);
        }
        Ok(header)
    }

    /// Decodes one record into a channel with identifier `id` in `bank`.
    fn channel(&self, record: &[u8], id: u16, bank: u8) -> Result<ChannelRecord, StorageError> {
        let mut cursor = Cursor {
            bytes: record,
            at: 0,
        };
        let width = usize::from(self.name_width);
        let mut name = [0_u8; MAX_CHANNEL_NAME_LEN];
        name[..width].copy_from_slice(cursor.take(width));
        let name_len = name.iter().position(|byte| *byte == 0).unwrap_or(width);
        let name = ChannelName::from_field(name, u8::try_from(name_len).unwrap_or(0))
            .map_err(malformed)?;
        let receive = self.frequency(&mut cursor)?;
        let transmit = if self.fields & TRANSMIT != 0 {
            self.frequency(&mut cursor)?
        } else {
            u32::try_from(i64::from(receive) + i64::from(self.tx_offset)).map_err(malformed)?
        };
        let rx_tone = cursor.field(self.fields & RX_TONE != 0, &self.rx_tone);
        let tx_tone = cursor.field(self.fields & TX_TONE != 0, &self.tx_tone);
        let [mode] = cursor.field(self.fields & MODE != 0, &[self.mode]);
        let [flags] = cursor.field(self.fields & FLAGS != 0, &[self.flags]);
        // An older studio may have stored a squelch level per record; squelch
        // is radio-wide, so it is skipped.
        let _ = cursor.field(self.fields & SQUELCH != 0, &[0]);
        let step =
            u32::from_le_bytes(cursor.field(self.fields & STEP != 0, &self.step.to_le_bytes()));
        let bank = (bank != NO_BANK).then(|| BankId::new(u16::from(bank)));
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name,
            receive: Frequency::from_hz(receive).map_err(malformed)?,
            transmit: Frequency::from_hz(transmit).map_err(malformed)?,
            rx_tone: decode_tone(&rx_tone)?,
            tx_tone: decode_tone(&tx_tone)?,
            modulation: Modulation::try_from(mode & 0b11).map_err(malformed)?,
            bandwidth: Bandwidth::try_from((mode >> 2) & 1).map_err(malformed)?,
            power: PowerLevel::try_from(mode >> 3).map_err(malformed)?,
            step: FrequencyStep::from_hz(step).map_err(malformed)?,
            flags: ChannelFlags::from_bits(flags).map_err(malformed)?,
            bank,
        })
        .map_err(malformed)
    }

    /// Reads one frequency in the block's form.
    fn frequency(&self, cursor: &mut Cursor<'_>) -> Result<u32, StorageError> {
        if self.fields & SHORT_FREQUENCY != 0 {
            let [low, high] = cursor.array();
            u32::from(u16::from_le_bytes([low, high]))
                .checked_mul(self.raster)
                .and_then(|hz| hz.checked_add(self.base))
                .ok_or(StorageError::MalformedObject)
        } else {
            Ok(u32::from_le_bytes(cursor.array()))
        }
    }
}

/// Any decoding failure is a malformed object.
fn malformed<E>(_: E) -> StorageError {
    StorageError::MalformedObject
}

/// Reads one record front to back.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> &'a [u8] {
        let field = &self.bytes[self.at..self.at + len];
        self.at += len;
        field
    }

    fn array<const N: usize>(&mut self) -> [u8; N] {
        let mut value = [0_u8; N];
        value.copy_from_slice(self.take(N));
        value
    }

    /// The record's own value when it carries one, else the block's.
    fn field<const N: usize>(&mut self, own: bool, shared: &[u8; N]) -> [u8; N] {
        if own {
            self.array()
        } else {
            *shared
        }
    }
}

impl Index {
    /// An index of no channels and the conservative configuration.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            config: UNSET_CONFIG,
            configured: false,
            tx_ranges: 0,
            tx_range_count: 0,
            banks: [BankSlot {
                entry: ABSENT,
                first: 0,
            }; MAX_BANKS as usize],
            groups: [ABSENT; MAX_GROUPS as usize],
            group_count: 0,
            bytes: 0,
            len: 0,
            populated: BankSet::EMPTY,
        }
    }

    /// Validates an image held outside RAM and indexes it.
    ///
    /// `read` fills a buffer from a payload offset. The checksum is checked
    /// first, then every bank and every channel is decoded once, so a radio
    /// never runs on an image any part of which is malformed.
    pub fn build(
        header: &[u8; HEADER_LEN],
        read: impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<Self, StorageError> {
        let mut index = Self::empty();
        index.rebuild(header, read)?;
        Ok(index)
    }

    /// Indexes an image in place, as [`Self::build`] does, leaving this index
    /// empty if the image is refused.
    ///
    /// The index is a kilobyte; a radio keeps one in a static and rebuilds it
    /// there rather than building another on its small stack.
    pub fn rebuild(
        &mut self,
        header: &[u8; HEADER_LEN],
        read: impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), StorageError> {
        *self = Self::empty();
        let built = self.index(header, read);
        if built.is_err() {
            *self = Self::empty();
        }
        built
    }

    fn index(
        &mut self,
        header: &[u8; HEADER_LEN],
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), StorageError> {
        if header[..4] != CONFIGURATION_IMAGE_MAGIC {
            return Err(StorageError::MalformedImage);
        }
        if header[4] != IMAGE_VERSION || header[5] != FORMAT_VERSION {
            return Err(StorageError::UnsupportedImageVersion);
        }
        let bank_count = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let payload_len = usize::try_from(u32::from_le_bytes([
            header[8], header[9], header[10], header[11],
        ]))
        .map_err(|_| StorageError::ImageTooLarge)?;
        if bank_count > usize::from(MAX_BANKS) || payload_len > usize::from(u16::MAX) {
            return Err(StorageError::ImageTooLarge);
        }
        let expected = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let mut stream = Stream {
            read: &mut read,
            len: payload_len,
            crc: crc32_update(u32::MAX, &header[..12]),
            window: [0; STREAM_WINDOW_BYTES],
            start: 0,
            end: 0,
        };
        let decoded = self.decode(
            &mut |offset, buffer| stream.read(offset, buffer),
            bank_count,
            payload_len,
        );
        // Nothing decoded is used unless the whole payload checks, and a
        // damaged image reports the damage whichever part of it failed.
        if !stream.finish()? != expected {
            return Err(StorageError::ImageIntegrity);
        }
        decoded?;
        // Saturates only for an image no memory region could hold.
        self.bytes = u16::try_from(HEADER_LEN + payload_len).unwrap_or(u16::MAX);
        Ok(())
    }

    /// Returns the bytes the image occupies, header included; zero for none.
    #[must_use]
    pub const fn image_bytes(&self) -> u16 {
        self.bytes
    }

    /// Decodes and checks every object in a payload of `payload_len` bytes
    /// into this index, which starts empty.
    fn decode(
        &mut self,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        bank_count: usize,
        payload_len: usize,
    ) -> Result<(), StorageError> {
        let index = self;
        let config = read_array::<RADIO_CONFIG_ENCODED_LEN>(read, 0)?;
        index.configured = true;
        index.config = decode_radio_config(&ObjectRef::new(
            ObjectKey {
                kind: ObjectKind::RadioConfig,
                id: 0,
            },
            &config,
        ))?
        .validate()
        .map_err(|_| StorageError::MalformedObject)?;
        let mut at = RADIO_CONFIG_ENCODED_LEN;
        let [range_count] = read_array::<1>(read, at)?;
        at += 1;
        index.tx_ranges = u16::try_from(at).map_err(|_| StorageError::ImageTooLarge)?;
        index.tx_range_count = range_count;
        if usize::from(range_count) > MAX_TX_RANGES {
            return Err(StorageError::MalformedObject);
        }
        for _ in 0..range_count {
            decode_tx_range_body(&read_array(read, at)?)?;
            at += TX_RANGE_ENCODED_LEN;
        }
        let mut first = 0_u32;
        let mut previous: Option<u8> = None;
        for _ in 0..bank_count {
            let (id, entry) = Self::read_entry(read, at)?;
            // Ascending, so each bank appears once and channels number in
            // bank order.
            if previous.is_some_and(|previous| previous >= id) {
                return Err(StorageError::NonCanonicalImage);
            }
            previous = Some(id);
            Self::read_bank(read, id, &entry)?;
            let count = Self::check_body(read, id, &entry)?;
            index.banks[usize::from(id)] = BankSlot {
                entry: u16::try_from(at).map_err(|_| StorageError::ImageTooLarge)?,
                first: u16::try_from(first).map_err(|_| StorageError::ImageTooLarge)?,
            };
            first += u32::from(count);
            if first > u32::from(u16::MAX) {
                return Err(StorageError::ImageTooLarge);
            }
            index.populated.insert(BankId::new(u16::from(id)));
            at = entry.body + entry.body_len;
        }
        let [group_count] = read_array::<1>(read, at)?;
        at += 1;
        if group_count > MAX_GROUPS {
            return Err(StorageError::MalformedObject);
        }
        // Only the layout is checked here, to keep the radio's loader small:
        // a group's name and banks are checked when it is read (`Self::group`).
        for slot in 0..usize::from(group_count) {
            index.groups[slot] = u16::try_from(at).map_err(|_| StorageError::ImageTooLarge)?;
            let [name_len] = read_array::<1>(read, at)?;
            if name_len == 0 || usize::from(name_len) > MAX_BANK_NAME_LEN {
                return Err(StorageError::MalformedObject);
            }
            at += 1 + usize::from(name_len) + 32;
        }
        index.group_count = group_count;
        if at != payload_len {
            return Err(StorageError::MalformedImage);
        }
        index.len = u16::try_from(first).unwrap_or(u16::MAX);
        // A slot the image does not hold takes the number of the next held
        // bank's first channel, so `first` never falls across the table and
        // the lookups below search it rather than walk it.
        let mut next = index.len;
        for slot in index.banks.iter_mut().rev() {
            if slot.entry == ABSENT {
                slot.first = next;
            } else {
                next = slot.first;
            }
        }
        Ok(())
    }

    /// Reads the bank entry at `at`: its identifier, and where its name and
    /// body lie.
    fn read_entry(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        at: usize,
    ) -> Result<(u8, BankEntry), StorageError> {
        let [id, flags, kind, low, high, name_len] = read_array::<BANK_ENTRY_HEADER_LEN>(read, at)?;
        if u16::from(id) >= MAX_BANKS || usize::from(name_len) > MAX_BANK_NAME_LEN {
            return Err(StorageError::MalformedObject);
        }
        let name = at + BANK_ENTRY_HEADER_LEN;
        Ok((
            id,
            BankEntry {
                flags,
                kind,
                body_len: usize::from(u16::from_le_bytes([low, high])),
                name_len,
                name,
                body: name + usize::from(name_len),
            },
        ))
    }

    /// Validates the body of bank `id` and every channel in it, and returns
    /// how many channels it holds.
    // `#[inline(never)]` here and on the functions below keeps one copy of a
    // function the K5 build otherwise inlines at every caller (`RISK-049`).
    #[inline(never)]
    fn check_body(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        id: u8,
        entry: &BankEntry,
    ) -> Result<u16, StorageError> {
        match entry.kind {
            KIND_LINEAR => {
                Ok(Self::read_plan(read, entry.body, entry.body_len, id)?.channel_count())
            }
            KIND_LIST => {
                let list = ListHeader::decode(&read_array(read, entry.body)?)?;
                let stride = record_len(list.fields, list.name_width);
                if list.count == 0
                    || LIST_HEADER_LEN + usize::from(list.count) * stride != entry.body_len
                {
                    return Err(StorageError::MalformedObject);
                }
                for position in 0..list.count {
                    let mut bytes = [0_u8; MAX_RECORD_LEN];
                    let offset = entry.body + LIST_HEADER_LEN + usize::from(position) * stride;
                    if !read(offset, &mut bytes[..stride]) {
                        return Err(StorageError::MalformedImage);
                    }
                    list.channel(&bytes[..stride], position + 1, id)?;
                }
                Ok(list.count)
            }
            _ => Err(StorageError::UnsupportedObject),
        }
    }

    /// The named bank an entry describes, or `None` for an unnamed one.
    fn read_bank(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        id: u8,
        entry: &BankEntry,
    ) -> Result<Option<ChannelBank>, StorageError> {
        let flags = BankFlags::from_bits(entry.flags).map_err(malformed)?;
        if entry.name_len == 0 {
            return Ok(None);
        }
        let len = usize::from(entry.name_len);
        let mut name = [0_u8; MAX_BANK_NAME_LEN];
        if !read(entry.name, &mut name[..len]) {
            return Err(StorageError::MalformedImage);
        }
        ChannelBank::new(
            BankId::new(u16::from(id)),
            BankName::from_field(name, entry.name_len).map_err(malformed)?,
            flags,
        )
        .map(Some)
        .map_err(malformed)
    }

    fn read_plan(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        body: usize,
        body_len: usize,
        bank: u8,
    ) -> Result<GeneratedBank, StorageError> {
        let mut bytes = [0_u8; MAX_GENERATED_BANK_ENCODED_LEN];
        let data = bytes
            .get_mut(..body_len)
            .ok_or(StorageError::MalformedObject)?;
        if !read(body, data) {
            return Err(StorageError::MalformedImage);
        }
        let plan = decode_generated_bank(&ObjectRef::new(
            ObjectKey {
                kind: ObjectKind::GeneratedBank,
                id: u16::from(bank),
            },
            data,
        ))?;
        if plan.id().get() != u16::from(bank) {
            return Err(StorageError::MalformedObject);
        }
        Ok(plan)
    }

    /// The programmed TX ranges as a policy.
    ///
    /// They were validated when the index was built; a range that no longer
    /// reads back (the memory changed underneath) leaves nothing permitted.
    #[inline(never)]
    pub fn tx_policy(&self, mut read: impl FnMut(usize, &mut [u8]) -> bool) -> TxPolicy {
        let mut ranges = [None; MAX_TX_RANGES];
        let count = usize::from(self.tx_range_count).min(MAX_TX_RANGES);
        for (position, slot) in ranges.iter_mut().take(count).enumerate() {
            let at = usize::from(self.tx_ranges) + position * TX_RANGE_ENCODED_LEN;
            match read_array(&mut read, at).and_then(|body| decode_tx_range_body(&body)) {
                Ok(range) => *slot = Some(range),
                Err(_) => return TxPolicy::deny_all(),
            }
        }
        policy_of(&ranges[..count])
    }

    /// The global receive configuration.
    #[must_use]
    pub const fn config(&self) -> RadioConfig {
        if self.configured {
            self.config
        } else {
            RadioConfig::conservative()
        }
    }

    /// Channels the image selects.
    #[must_use]
    pub const fn len(&self) -> u16 {
        self.len
    }

    /// Whether the image selects no channel.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Banks with at least one channel: every bank the image holds.
    #[must_use]
    pub const fn populated(&self) -> BankSet {
        self.populated
    }

    /// The channel numbers of `bank`: from its first to the next held
    /// bank's, or to the end.
    fn span(&self, bank: usize) -> Option<core::ops::Range<u16>> {
        let slot = self.banks.get(bank)?;
        if slot.entry == ABSENT {
            return None;
        }
        let end = self.banks.get(bank + 1).map_or(self.len, |next| next.first);
        Some(slot.first..end)
    }

    /// The bank and position within it of channel number `index`.
    #[must_use]
    pub fn place_of(&self, index: u16) -> Option<(BankId, u16)> {
        if index >= self.len {
            return None;
        }
        // The last slot starting at or before `index`: the bank holding it,
        // since every slot after that bank starts where the bank ends. This
        // runs for every channel a bank-set view counts, on every interface
        // pass, so it searches rather than walks all the slots.
        let bank = self
            .banks
            .partition_point(|slot| slot.first <= index)
            .checked_sub(1)?;
        let slot = self.banks[bank];
        if slot.entry == ABSENT {
            return None;
        }
        Some((BankId::new(u16::try_from(bank).ok()?), index - slot.first))
    }

    /// The channel number of position `position` in `bank`, if it has one.
    #[must_use]
    pub fn index_of(&self, bank: BankId, position: u16) -> Option<u16> {
        let span = self.span(usize::from(bank.get()))?;
        let index = span.start.checked_add(position)?;
        span.contains(&index).then_some(index)
    }

    /// Whether channel number `index` is stored in `bank`, from RAM alone.
    #[must_use]
    pub fn member_at(&self, index: u16, bank: BankId) -> bool {
        self.span(usize::from(bank.get()))
            .is_some_and(|span| span.contains(&index))
    }

    /// Reads the entry of `bank`, if the image holds it.
    fn entry_of(
        &self,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<(u8, BankEntry)> {
        let slot = self.banks.get(usize::from(bank.get()))?;
        if slot.entry == ABSENT {
            return None;
        }
        Self::read_entry(read, usize::from(slot.entry)).ok()
    }

    /// Reads and decodes channel number `index`.
    ///
    /// A list channel's identifier is its position in its bank plus one; a
    /// plan's channels carry the plan's own identifiers.
    pub fn channel_at(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        index: u16,
    ) -> Option<ChannelRecord> {
        let (bank, position) = self.place_of(index)?;
        let (id, entry) = self.entry_of(&mut read, bank)?;
        if entry.kind == KIND_LINEAR {
            return Self::read_plan(&mut read, entry.body, entry.body_len, id)
                .ok()?
                .channel_record(position)
                .ok();
        }
        let list = ListHeader::decode(&read_array(&mut read, entry.body).ok()?).ok()?;
        let stride = record_len(list.fields, list.name_width);
        let mut record = [0_u8; MAX_RECORD_LEN];
        let offset = entry.body + LIST_HEADER_LEN + usize::from(position) * stride;
        read(offset, &mut record[..stride]).then_some(())?;
        list.channel(&record[..stride], position + 1, id).ok()
    }

    /// The named bank `bank`, as its entry stores it.
    pub fn bank(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<ChannelBank> {
        let (id, entry) = self.entry_of(&mut read, bank)?;
        Self::read_bank(&mut read, id, &entry).ok().flatten()
    }

    /// The generated plan stored in `bank`, if it is one.
    pub fn plan(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<GeneratedBank> {
        let (id, entry) = self.entry_of(&mut read, bank)?;
        (entry.kind == KIND_LINEAR)
            .then(|| Self::read_plan(&mut read, entry.body, entry.body_len, id).ok())
            .flatten()
    }

    /// How many groups the image holds; the radio numbers them from zero.
    #[must_use]
    pub const fn group_count(&self) -> u8 {
        self.group_count
    }

    /// Payload offset of group `group`'s entry and its name's length.
    fn group_entry(
        &self,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        group: u8,
    ) -> Option<(usize, usize)> {
        if group >= self.group_count {
            return None;
        }
        let at = usize::from(*self.groups.get(usize::from(group))?);
        let [len] = read_array::<1>(read, at).ok()?;
        Some((at, usize::from(len)))
    }

    /// Reads group `group`'s name.
    pub fn group_name(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        group: u8,
    ) -> Option<BankName> {
        let (at, len) = self.group_entry(&mut read, group)?;
        let mut name = [0_u8; MAX_BANK_NAME_LEN];
        read(at + 1, name.get_mut(..len)?).then_some(())?;
        BankName::from_field(name, u8::try_from(len).ok()?).ok()
    }

    /// Reads group `group`'s banks, narrowed to those the image holds so a
    /// group can never select a bank that is not there.
    pub fn group_banks(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        group: u8,
    ) -> Option<BankSet> {
        let (at, len) = self.group_entry(&mut read, group)?;
        let banks = read_array::<32>(&mut read, at + 1 + len).ok()?;
        Some(BankSet::from_bytes(banks).ok()?.within(&self.populated))
    }

    /// Reads group `group`'s name and banks; `None` for a group with no bank
    /// the image holds.
    pub fn group(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        group: u8,
    ) -> Option<(BankName, BankSet)> {
        let banks = self.group_banks(&mut read, group)?;
        let name = self.group_name(&mut read, group)?;
        (!banks.is_empty()).then_some((name, banks))
    }

    /// The name to show for `bank`: its own name, else its plan's.
    pub fn bank_name(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<BankName> {
        self.bank(&mut read, bank)
            .map(ChannelBank::name)
            .or_else(|| self.plan(&mut read, bank).map(GeneratedBank::name))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{compile, CompileError, Index, HEADER_LEN, NO_BANK};
    use crate::{
        configuration_image_crc, StorageError, RADIO_CONFIG_ENCODED_LEN, TX_RANGE_ENCODED_LEN,
    };
    use radio_channel_plan::{
        BankFlags, BankGroup, BankName, BankSet, ChannelBank, ChannelDefinition, ChannelFlags,
        ChannelName, ChannelRecord, ChannelTemplate, GeneratedBank, Locator, MAX_BANKS,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, Tone, TxClass,
    };
    use radio_tx_policy::{RangeError, TxPolicy, TxRange, MAX_TX_RANGES};
    use std::vec::Vec;

    fn channel(id: u16, name: &str, rx: u32, tx: u32, bank: Option<u16>) -> ChannelDefinition {
        ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new(name).unwrap(),
            receive: Frequency::from_hz(rx).unwrap(),
            transmit: Frequency::from_hz(tx).unwrap(),
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low1,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            flags: ChannelFlags::default(),
            bank: bank.map(BankId::new),
        }
    }

    fn record(definition: ChannelDefinition) -> ChannelRecord {
        ChannelRecord::new(definition).unwrap()
    }

    /// Sixteen PMR446 channels in bank 0, all sharing everything but the
    /// frequency and the name.
    fn pmr() -> Vec<ChannelRecord> {
        (0..16_u16)
            .map(|n| {
                let hz = 446_006_250 + u32::from(n) * 12_500;
                let name = std::format!("PMR{}", n + 1);
                record(channel(n + 1, &name, hz, hz, Some(0)))
            })
            .collect()
    }

    /// Repeaters in bank 1 whose offsets and tones differ channel to channel.
    fn repeaters() -> Vec<ChannelRecord> {
        let mut one = channel(100, "GB3AA", 145_600_000, 145_000_000, Some(1));
        one.rx_tone = Tone::ctcss(770).unwrap();
        one.tx_tone = Tone::ctcss(770).unwrap();
        let mut two = channel(101, "GB3BB", 433_050_000, 434_650_000, Some(1));
        two.tx_tone = Tone::ctcss(1188).unwrap();
        two.power = PowerLevel::High;
        vec_of([one, two])
    }

    fn vec_of<const N: usize>(definitions: [ChannelDefinition; N]) -> Vec<ChannelRecord> {
        definitions.into_iter().map(record).collect()
    }

    fn plan() -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(2),
            BankName::new("MARINE").unwrap(),
            Frequency::from_hz(156_000_000).unwrap(),
            FrequencyStep::from_hz(25_000).unwrap(),
            40,
        )
        .unwrap()
    }

    fn banks() -> Vec<ChannelBank> {
        std::vec![ChannelBank::new(
            BankId::new(0),
            BankName::new("PMR446").unwrap(),
            BankFlags::default(),
        )
        .unwrap()]
    }

    fn compiled(channels: &[ChannelRecord], plans: &[GeneratedBank]) -> Vec<u8> {
        compiled_with(&[], channels, plans)
    }

    fn compiled_with(
        ranges: &[TxRange],
        channels: &[ChannelRecord],
        plans: &[GeneratedBank],
    ) -> Vec<u8> {
        let mut image = std::vec![0_u8; 8192];
        let len = compile(
            RadioConfig::conservative(),
            ranges,
            &banks(),
            channels,
            plans,
            &[],
            &mut image,
        )
        .expect("compile");
        image.truncate(len);
        image
    }

    const RANGES: [TxRange; 2] = [
        TxRange {
            class: TxClass::LicenceFreePlan,
            first_hz: 446_006_250,
            last_hz: 446_193_750,
            max_mw: 500,
            wide_fm: false,
            raster_hz: 12_500,
        },
        TxRange {
            class: TxClass::Amateur,
            first_hz: 144_000_000,
            last_hz: 146_000_000,
            max_mw: 5_000,
            wide_fm: true,
            raster_hz: 0,
        },
    ];

    /// Rewrites the header checksum after a test edits the payload.
    fn reseal(image: &mut [u8]) {
        let (header, payload) = image.split_at_mut(HEADER_LEN);
        let crc = configuration_image_crc(&header[..12], payload);
        header[12..16].copy_from_slice(&crc.to_le_bytes());
    }

    #[test]
    fn tx_ranges_round_trip_into_the_radio_policy() {
        let image = compiled_with(&RANGES, &pmr(), &[plan()]);
        let index = index(&image).expect("index");
        assert_eq!(
            index.tx_policy(reader(&image)),
            TxPolicy::new(&RANGES).unwrap()
        );
        // The ranges cost a count and 14 bytes each.
        assert_eq!(
            image.len(),
            compiled(&pmr(), &[plan()]).len() + 2 * TX_RANGE_ENCODED_LEN
        );
        // No range programmed: nothing may transmit.
        let bare = compiled(&pmr(), &[]);
        assert_eq!(
            index_of(&bare).tx_policy(reader(&bare)),
            TxPolicy::deny_all()
        );
    }

    fn index_of(image: &[u8]) -> Index {
        index(image).expect("index")
    }

    #[test]
    fn a_malformed_or_extra_tx_range_is_refused_by_compiler_and_radio() {
        let mut image = [0_u8; 1024];
        let empty = TxRange {
            max_mw: 0,
            ..RANGES[0]
        };
        assert_eq!(
            compile(
                RadioConfig::conservative(),
                &[empty],
                &[],
                &[],
                &[],
                &[],
                &mut image
            ),
            Err(CompileError::TxRange(RangeError::Power))
        );
        assert_eq!(
            compile(
                RadioConfig::conservative(),
                &[RANGES[0]; MAX_TX_RANGES + 1],
                &[],
                &[],
                &[],
                &[],
                &mut image
            ),
            Err(CompileError::TxRange(RangeError::TooMany))
        );

        // The radio checks again: a class byte no TxClass has, then a
        // reversed span, each with a valid checksum.
        let first = RADIO_CONFIG_ENCODED_LEN + 1;
        for (offset, value) in [(0, 0), (0, 9), (1, 2), (2, 0xFF)] {
            let mut image = compiled_with(&RANGES, &[], &[]);
            image[HEADER_LEN + first + offset] = value;
            if offset == 2 {
                // first Hz above last Hz
                image[HEADER_LEN + first + 5] = 0xFF;
            }
            reseal(&mut image);
            assert_eq!(
                index(&image),
                Err(StorageError::MalformedObject),
                "offset {offset} value {value}"
            );
        }
    }

    fn index(image: &[u8]) -> Result<Index, StorageError> {
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&image[..HEADER_LEN]);
        Index::build(&header, reader(image))
    }

    fn reader(image: &[u8]) -> impl FnMut(usize, &mut [u8]) -> bool + '_ {
        let payload = &image[HEADER_LEN..];
        move |offset, buffer| {
            payload
                .get(offset..offset + buffer.len())
                .is_some_and(|bytes| {
                    buffer.copy_from_slice(bytes);
                    true
                })
        }
    }

    /// Everything but the identifier, which the radio numbers by position.
    fn same_channel(a: &ChannelRecord, b: &ChannelRecord) {
        assert_eq!(a.name(), b.name());
        assert_eq!(a.receive(), b.receive());
        assert_eq!(a.transmit(), b.transmit());
        assert_eq!(a.rx_tone(), b.rx_tone());
        assert_eq!(a.tx_tone(), b.tx_tone());
        assert_eq!(a.modulation(), b.modulation());
        assert_eq!(a.bandwidth(), b.bandwidth());
        assert_eq!(a.power(), b.power());
        assert_eq!(a.step(), b.step());
        assert_eq!(a.flags(), b.flags());
        assert_eq!(a.bank(), b.bank());
    }

    #[test]
    fn every_channel_comes_back_bank_by_bank() {
        let air = record(channel(200, "AIR", 118_100_000, 118_100_000, Some(3)));
        let mut channels = repeaters();
        channels.push(air);
        channels.extend(pmr());
        let image = compiled(&channels, &[plan()]);
        let index = index(&image).expect("index");

        // Bank 0's list, bank 1's list, bank 2's plan, then bank 3's list.
        assert_eq!(index.len(), 16 + 2 + 40 + 1);
        let expected: Vec<ChannelRecord> = pmr()
            .into_iter()
            .chain(repeaters())
            .chain((0..40).map(|n| plan().channel_record(n).unwrap()))
            .chain([air])
            .collect();
        for (position, want) in expected.iter().enumerate() {
            let position = u16::try_from(position).unwrap();
            let got = index.channel_at(reader(&image), position).expect("channel");
            same_channel(&got, want);
            for bank in 0..4 {
                let bank = BankId::new(bank);
                assert_eq!(index.member_at(position, bank), want.is_member_of(bank));
            }
        }
        let held: Vec<u16> = index.populated().iter().map(BankId::get).collect();
        assert_eq!(held, [0, 1, 2, 3]);
        // A channel is named by its bank and its position there.
        assert_eq!(index.place_of(0), Some((BankId::new(0), 0)));
        assert_eq!(index.place_of(17), Some((BankId::new(1), 1)));
        assert_eq!(index.place_of(58), Some((BankId::new(3), 0)));
        assert_eq!(index.place_of(59), None);
        assert_eq!(index.index_of(BankId::new(2), 39), Some(57));
        assert_eq!(index.index_of(BankId::new(2), 40), None);
        assert_eq!(index.index_of(BankId::new(4), 0), None);
        // A list channel's identifier is its position in its bank plus one.
        assert_eq!(
            index.channel_at(reader(&image), 17).unwrap().id(),
            ChannelId::new(2)
        );
        assert_eq!(
            index
                .bank_name(reader(&image), BankId::new(0))
                .unwrap()
                .as_str(),
            "PMR446"
        );
        assert_eq!(
            index
                .bank_name(reader(&image), BankId::new(2))
                .unwrap()
                .as_str(),
            "MARINE",
            "a plan names its own bank"
        );
        assert!(index.bank_name(reader(&image), BankId::new(1)).is_none());
    }

    #[test]
    fn broadcast_stations_and_plans_come_back_as_broadcast() {
        let mut station = channel(1, "RADIO 4", 93_500_000, 93_500_000, Some(0));
        station.modulation = Modulation::Broadcast;
        station.bandwidth = Bandwidth::Wide;
        let station = record(station);
        let spacing = FrequencyStep::from_hz(100_000).unwrap();
        let mut template = ChannelTemplate::narrow_fm(spacing);
        template.modulation = Modulation::Broadcast;
        let band = GeneratedBank::linear_simplex_with(
            BankId::new(2),
            BankName::new("FM").unwrap(),
            Frequency::from_hz(87_500_000).unwrap(),
            spacing,
            206,
            template,
        )
        .unwrap();
        let image = compiled(&[station], &[band]);
        let index = index(&image).expect("index");
        same_channel(&index.channel_at(reader(&image), 0).unwrap(), &station);
        let last = index.channel_at(reader(&image), 206).unwrap();
        assert_eq!(last.modulation(), Modulation::Broadcast);
        assert_eq!(last.receive().as_hz(), 108_000_000);
    }

    #[test]
    fn a_host_reads_back_each_bank_and_plan_it_wrote() {
        let image = compiled(&pmr(), &[plan()]);
        let index = index(&image).expect("index");
        assert_eq!(index.bank(reader(&image), BankId::new(0)), Some(banks()[0]));
        assert_eq!(index.bank(reader(&image), BankId::new(2)), None);
        assert_eq!(index.plan(reader(&image), BankId::new(2)), Some(plan()));
        assert_eq!(index.plan(reader(&image), BankId::new(0)), None);
        assert_eq!(index.plan(reader(&image), BankId::new(1)), None);
    }

    #[test]
    fn every_channel_must_be_in_a_bank_and_a_bank_one_kind() {
        let mut image = std::vec![0_u8; 8192];
        let unbanked = record(channel(9, "AIR", 118_100_000, 118_100_000, None));
        assert_eq!(
            compile(
                RadioConfig::conservative(),
                &[],
                &banks(),
                &[unbanked],
                &[],
                &[],
                &mut image
            ),
            Err(CompileError::UnbankedChannel(ChannelId::new(9)))
        );
        let in_plan_bank = record(channel(9, "AIR", 118_100_000, 118_100_000, Some(2)));
        assert_eq!(
            compile(
                RadioConfig::conservative(),
                &[],
                &banks(),
                &[in_plan_bank],
                &[plan()],
                &[],
                &mut image
            ),
            Err(CompileError::MixedBank(BankId::new(2)))
        );
        // A named bank with nothing in it is left out.
        let image = compiled(&[], &[plan()]);
        let index = index(&image).expect("index");
        assert_eq!(index.bank(reader(&image), BankId::new(0)), None);
        assert_eq!(index.populated(), BankSet::EMPTY.with(BankId::new(2)));
    }

    #[test]
    fn the_last_bank_identifier_holds_a_list_and_a_plan_bank_beside_it() {
        let last = MAX_BANKS - 1;
        let channels = vec_of([
            channel(1, "LOW", 145_500_000, 145_500_000, Some(1)),
            channel(2, "HIGH", 433_500_000, 433_500_000, Some(last)),
        ]);
        let wide_plan = GeneratedBank::linear_simplex(
            BankId::new(200),
            BankName::new("AIR").unwrap(),
            Frequency::from_hz(118_000_000).unwrap(),
            FrequencyStep::from_hz(25_000).unwrap(),
            760,
        )
        .unwrap();
        let image = compiled(&channels, &[wide_plan]);
        let index = index(&image).expect("index");
        assert_eq!(index.len(), 1 + 760 + 1);
        assert_eq!(index.place_of(761), Some((BankId::new(last), 0)));
        // Every channel across the gaps names its own bank and position.
        for number in 0..index.len() {
            let want = match number {
                0 => (BankId::new(1), 0),
                761 => (BankId::new(last), 0),
                _ => (BankId::new(200), number - 1),
            };
            assert_eq!(index.place_of(number), Some(want), "channel {number}");
            assert!(index.member_at(number, want.0), "channel {number}");
        }
        assert_eq!(index.place_of(762), None);
        assert_eq!(index.index_of(BankId::new(200), 759), Some(760));
        assert!(index.member_at(761, BankId::new(last)));
        assert!(!index.member_at(760, BankId::new(last)));
        let top = index.channel_at(reader(&image), 761).unwrap();
        same_channel(&top, &channels[1]);
        let plan = index.channel_at(reader(&image), 760).unwrap();
        assert_eq!(plan.bank(), Some(BankId::new(200)));
        assert_eq!(index.populated().len(), 3);
    }

    fn group(id: u8, name: &str, banks: &[u16]) -> BankGroup {
        let set = banks
            .iter()
            .fold(BankSet::EMPTY, |set, bank| set.with(BankId::new(*bank)));
        BankGroup::new(id, BankName::new(name).unwrap(), set).unwrap()
    }

    #[test]
    fn groups_reach_the_radio_with_their_held_banks_only() {
        // Banks 0 (PMR), 1 (repeaters) and 2 (the plan) are held; bank 7 is
        // not, and a group of nothing but it is dropped.
        let mut channels = pmr();
        channels.extend(repeaters());
        let groups = [
            group(4, "AIRBAND", &[7]),
            group(3, "HOME", &[0, 1, 7]).with_locator(Some(Locator::new("JO02EH").unwrap())),
            group(9, "ALL", &[0, 1, 2]),
        ];
        let mut image = std::vec![0_u8; 8192];
        let len = compile(
            RadioConfig::conservative(),
            &[],
            &banks(),
            &channels,
            &[plan()],
            &groups,
            &mut image,
        )
        .unwrap();
        image.truncate(len);
        let index = index(&image).expect("index");
        // In identifier order, renumbered from zero; the locator is not sent.
        assert_eq!(index.group_count(), 2);
        let (name, members) = index.group(reader(&image), 0).unwrap();
        assert_eq!(name.as_str(), "HOME");
        assert_eq!(
            members,
            BankSet::EMPTY.with(BankId::new(0)).with(BankId::new(1))
        );
        let (name, members) = index.group(reader(&image), 1).unwrap();
        assert_eq!(name.as_str(), "ALL");
        assert_eq!(members.len(), 3);
        assert_eq!(index.group(reader(&image), 2), None);

        // A bank the image does not hold is never offered: add bank 7 to
        // the last group's set, whose first byte is 32 from the end.
        let first_byte = image.len() - 32;
        assert_eq!(image[first_byte], 0b111);
        image[first_byte] |= 1 << 7;
        reseal(&mut image);
        let index = super::Index::build(&header_of(&image), reader(&image)).unwrap();
        assert_eq!(index.group(reader(&image), 1).unwrap().1.len(), 3);
        // A name length past the field is refused when the radio loads.
        let mut long = image.clone();
        let name_at = long.len() - 32 - 3 - 1;
        assert_eq!(long[name_at], 3);
        long[name_at] = 17;
        reseal(&mut long);
        assert!(index_result(&long).is_err());
    }

    fn header_of(image: &[u8]) -> [u8; HEADER_LEN] {
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&image[..HEADER_LEN]);
        header
    }

    fn index_result(image: &[u8]) -> Result<(), StorageError> {
        index(image).map(|_| ())
    }

    #[test]
    fn banks_out_of_order_are_refused() {
        // Two banks of one channel each, then their entries swapped.
        let channels = vec_of([
            channel(1, "A", 145_500_000, 145_500_000, Some(0)),
            channel(2, "B", 145_500_000, 145_500_000, Some(1)),
        ]);
        let mut image = compiled(&channels, &[]);
        let first = HEADER_LEN + RADIO_CONFIG_ENCODED_LEN + 1;
        // Bank 0's entry is named PMR446: 6 + 6 + 30 + 1 + 2 bytes; bank 1's
        // is unnamed: 6 + 30 + 1 + 2.
        let (zero, one) = (6 + 6 + 30 + 3, 6 + 30 + 3);
        assert_eq!(image.len(), first + zero + one + 1, "and no groups");
        let mut swapped = image[first + zero..first + zero + one].to_vec();
        swapped.extend_from_slice(&image[first..first + zero]);
        image[first..first + zero + one].copy_from_slice(&swapped);
        reseal(&mut image);
        assert_eq!(index(&image), Err(StorageError::NonCanonicalImage));
    }

    #[test]
    fn a_pmr_list_costs_its_names_and_short_frequencies() {
        let image = compiled(&pmr(), &[]);
        // Header 16, config 16, no TX ranges 1, the bank's entry 6 and name
        // 6, list header 30, then 16 records of a 5-byte name and a 2-byte
        // frequency, and no groups 1.
        assert_eq!(image.len(), 16 + 16 + 1 + 6 + 6 + 30 + 16 * (5 + 2) + 1);
    }

    #[test]
    fn an_image_with_per_record_squelch_still_loads_and_ignores_it() {
        // An older studio could store a squelch level in each record. Rebuild
        // the PMR list that way: the level is the last field of each record,
        // so one byte follows each 7-byte record.
        let image = compiled(&pmr(), &[]);
        let entry = 16 + 16 + 1;
        let list = entry + 6 + 6;
        let records = list + 30;
        let mut old = image[..records].to_vec();
        let fields = u16::from_le_bytes([old[list], old[list + 1]]) | super::SQUELCH;
        old[list..list + 2].copy_from_slice(&fields.to_le_bytes());
        let body = u16::from_le_bytes([old[entry + 3], old[entry + 4]]) + 16;
        old[entry + 3..entry + 5].copy_from_slice(&body.to_le_bytes());
        let payload = u32::from_le_bytes([old[8], old[9], old[10], old[11]]) + 16;
        old[8..12].copy_from_slice(&payload.to_le_bytes());
        let groups = image.len() - 1;
        for record in image[records..groups].chunks(7) {
            old.extend_from_slice(record);
            old.push(9);
        }
        old.push(image[groups]);
        reseal(&mut old);

        let index = index(&old).expect("index");
        for (position, want) in pmr().iter().enumerate() {
            let position = u16::try_from(position).unwrap();
            let got = index.channel_at(reader(&old), position).expect("channel");
            same_channel(&got, want);
        }
    }

    #[test]
    fn frequencies_too_far_apart_for_rasters_are_stored_in_hertz() {
        let channels = vec_of([
            channel(1, "LOW", 30_000_001, 30_000_001, Some(4)),
            channel(2, "HIGH", 470_000_000, 470_000_000, Some(4)),
        ]);
        let image = compiled(&channels, &[]);
        let index = index(&image).expect("index");
        for (position, want) in channels.iter().enumerate() {
            let got = index
                .channel_at(reader(&image), u16::try_from(position).unwrap())
                .unwrap();
            same_channel(&got, want);
        }
    }

    #[test]
    fn a_damaged_erased_or_unreadable_image_is_refused() {
        let image = compiled(&pmr(), &[plan()]);
        let mut damaged = image.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert_eq!(index(&damaged), Err(StorageError::ImageIntegrity));
        assert_eq!(
            index(&[0xFF; 64]).map(|_| ()),
            Err(StorageError::MalformedImage)
        );
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&image[..HEADER_LEN]);
        assert_eq!(
            Index::build(&header, |_, _| false),
            Err(StorageError::MalformedImage)
        );
        assert!(Index::empty().is_empty());
        assert_eq!(NO_BANK, 0xFF);
    }

    /// Booting reads the image over a slow bus, so it is read once, in
    /// window-sized pieces, and a damaged image still reports the damage
    /// rather than whatever part of it failed to decode first.
    #[test]
    fn building_reads_the_payload_once_in_windows() {
        let image = compiled(&pmr(), &[plan()]);
        let payload_len = image.len() - HEADER_LEN;
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&image[..HEADER_LEN]);
        let (mut calls, mut bytes) = (0, 0);
        let mut inner = reader(&image);
        Index::build(&header, |offset, buffer: &mut [u8]| {
            calls += 1;
            bytes += buffer.len();
            inner(offset, buffer)
        })
        .expect("a sound image indexes");
        assert_eq!(bytes, payload_len);
        assert_eq!(calls, payload_len.div_ceil(super::STREAM_WINDOW_BYTES));
        assert_eq!(
            usize::from(index(&image).expect("indexes").image_bytes()),
            image.len()
        );
        assert_eq!(Index::empty().image_bytes(), 0);

        let mut damaged = image.clone();
        damaged[HEADER_LEN] ^= 0xFF;
        assert_eq!(index(&damaged), Err(StorageError::ImageIntegrity));
    }

    #[test]
    fn an_empty_index_holds_zeros_and_reports_the_conservative_config() {
        // Zeros let the firmware's static index cost no flash; the accessors
        // still read as an unprogrammed radio.
        let empty = Index::empty();
        assert_eq!(empty.config, super::UNSET_CONFIG);
        assert!(empty
            .banks
            .iter()
            .all(|bank| *bank == super::BankSlot::default()));
        assert_eq!(empty.config(), radio_domain::RadioConfig::conservative());
        assert_ne!(
            super::UNSET_CONFIG,
            radio_domain::RadioConfig::conservative()
        );
        assert_eq!(super::ABSENT, 0);
    }
}
