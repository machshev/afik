//! The image a radio runs: storage format 5, grouped into bank blocks.
//!
//! The host compiles a project into this image and writes it into the radio's
//! memory (`ADR-100`); the radio reads it in place. Every channel lives in one
//! block, and every block belongs to one bank (or to none), so a bank filter
//! is a range and the radio's RAM index is one entry per block.
//!
//! A *list* block holds explicit channels. The values all of its channels
//! share are stored once in the block header; a `fields` bitfield names the
//! values each record carries instead, so a PMR or simplex list stores little
//! more than a short frequency and a name per channel. Records in a block all
//! have the same size, so any channel is read directly by index.
//!
//! A *linear* block is a generated plan: a base, a spacing and a count, with
//! no records at all.
//!
//! ```text
//! header  : magic "AFIK", image version 2, format 5, block count u16,
//!           payload length u32, CRC-32 (as format 4)
//! payload : radio configuration (16 bytes, as its format 4 object)
//!           bank count u8, then per bank: id u8, flags u8, name length u8,
//!           name bytes
//!           blocks, each: kind u8 (1 list, 2 linear), bank u8 (0xFF none),
//!           body length u16, body
//! linear  : the format 4 generated-bank object bytes
//! list    : fields u16, name width u8, count u16, base Hz u32, raster Hz u32,
//!           TX offset i32, step Hz u32, RX tone 3, TX tone 3, mode u8,
//!           flags u8, squelch u8, TX class u8; then `count` records
//! record  : name (name width), RX, [TX], [RX tone], [TX tone], [mode],
//!           [flags], [squelch], [TX class], [step], as `fields` says
//! ```
//!
//! A frequency is a `u16` count of rasters above the base when
//! [`SHORT_FREQUENCY`] is set, and a `u32` in hertz otherwise. Without
//! [`TRANSMIT`], TX is RX plus the block's TX offset. The mode byte is
//! modulation | bandwidth << 2 | power << 3.

use radio_channel_plan::{
    BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
    ChannelRecord, GeneratedBank, MAX_BANKS, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN,
};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig,
    SquelchLevel, Tone, TxClass,
};

use super::{
    configuration_image_crc, crc32_update, decode_generated_bank, decode_radio_config, decode_tone,
    encode_generated_bank, encode_radio_config, encode_tone, ObjectKey, ObjectKind, ObjectRef,
    StorageError, CONFIGURATION_IMAGE_MAGIC, MAX_GENERATED_BANK_ENCODED_LEN,
    RADIO_CONFIG_ENCODED_LEN,
};

/// Image container version for format 5.
pub const IMAGE_VERSION: u8 = 2;
/// Object format version this module reads and writes.
pub const FORMAT_VERSION: u8 = 5;
/// Bytes in the image header.
pub const HEADER_LEN: usize = 16;
/// Most blocks one image may hold, which bounds the radio's RAM index.
pub const MAX_BLOCKS: usize = 48;
/// The bank of a block whose channels are in no bank.
pub const NO_BANK: u8 = 0xFF;

// Every bank, and the bankless channels, can hold a list and a plan.
const _: () = assert!(MAX_BLOCKS > MAX_BANKS as usize * 2);

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
/// Records carry their own squelch level.
pub const SQUELCH: u16 = 1 << 6;
/// Records carry their own TX class.
pub const TX_CLASS: u16 = 1 << 7;
/// Records carry their own tuning step.
pub const STEP: u16 = 1 << 8;
const KNOWN_FIELDS: u16 = (1 << 9) - 1;

const KIND_LIST: u8 = 1;
const KIND_LINEAR: u8 = 2;
const BLOCK_HEADER_LEN: usize = 4;
const LIST_HEADER_LEN: usize = 31;
const BANK_ENTRY_HEADER_LEN: usize = 3;
/// Largest list record: every field present at full width.
const MAX_RECORD_LEN: usize = MAX_CHANNEL_NAME_LEN + 4 + 4 + 3 + 3 + 1 + 1 + 1 + 1 + 4;
/// Payload offset meaning "none".
const ABSENT: u16 = u16::MAX;

/// Why a project cannot become a radio image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileError {
    /// A channel is in more than one bank; in this format a channel belongs to
    /// the one bank it is stored in.
    MultipleBanks(ChannelId),
    /// Two generated plans claim one bank.
    DuplicatePlan(BankId),
    /// More blocks than a radio indexes.
    TooManyBlocks,
    /// The image does not fit the output.
    TooLarge,
    /// An object failed its own encoding.
    Storage(StorageError),
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

/// The single bank a channel is stored in, or [`NO_BANK`].
fn bank_of(channel: &ChannelRecord) -> Result<u8, CompileError> {
    let bits = channel.banks().bits();
    match bits.count_ones() {
        0 => Ok(NO_BANK),
        1 => Ok(u8::try_from(bits.trailing_zeros()).unwrap_or(NO_BANK)),
        _ => Err(CompileError::MultipleBanks(channel.id())),
    }
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
                (channel.squelch() != template.squelch(), SQUELCH),
                (channel.tx_class() != template.tx_class(), TX_CLASS),
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
        (TX_CLASS, 1),
        (STEP, 4),
    ] {
        if fields & field != 0 {
            len += bytes;
        }
    }
    len
}

fn write_list<'a>(
    writer: &mut Writer<'_>,
    bank: u8,
    members: impl Iterator<Item = &'a ChannelRecord> + Clone,
) -> Result<bool, CompileError> {
    let Some(layout) = ListLayout::choose(members.clone()) else {
        return Ok(false);
    };
    let count = members.clone().count();
    let body = LIST_HEADER_LEN + count * record_len(layout.fields, layout.name_width);
    writer.u8(KIND_LIST)?;
    writer.u8(bank)?;
    writer.u16(u16::try_from(body).map_err(|_| CompileError::TooLarge)?)?;
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
    writer.u8(template.squelch().get())?;
    writer.u8(template.tx_class() as u8)?;
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
        if fields & SQUELCH != 0 {
            writer.u8(channel.squelch().get())?;
        }
        if fields & TX_CLASS != 0 {
            writer.u8(channel.tx_class() as u8)?;
        }
        if fields & STEP != 0 {
            writer.u32(channel.step().as_hz())?;
        }
    }
    Ok(true)
}

/// Compiles a validated project into a radio image and returns its length.
///
/// Blocks are written bank by bank: a bank's explicit channels, in the order
/// given, then its generated plan; channels in no bank come last. The radio
/// numbers channels in this order.
pub fn compile(
    config: RadioConfig,
    banks: &[ChannelBank],
    channels: &[ChannelRecord],
    plans: &[GeneratedBank],
    output: &mut [u8],
) -> Result<usize, CompileError> {
    for channel in channels {
        bank_of(channel)?;
    }
    for (index, plan) in plans.iter().enumerate() {
        if plans[..index].iter().any(|other| other.id() == plan.id()) {
            return Err(CompileError::DuplicatePlan(plan.id()));
        }
    }
    let mut writer = Writer {
        output,
        at: HEADER_LEN,
    };
    writer.bytes(encode_radio_config(config)?.data())?;
    writer.u8(u8::try_from(banks.len()).map_err(|_| CompileError::TooLarge)?)?;
    for bank in banks {
        let name = bank.name();
        writer.u8(u8::try_from(bank.id().get()).map_err(|_| CompileError::TooLarge)?)?;
        writer.u8(bank.flags().bits())?;
        writer.u8(name.len())?;
        writer.bytes(&name.field()[..usize::from(name.len())])?;
    }
    let mut blocks = 0_usize;
    let bank_ids = (0..u8::try_from(MAX_BANKS).unwrap_or(0)).chain(core::iter::once(NO_BANK));
    for bank in bank_ids {
        let members = channels
            .iter()
            .filter(move |channel| bank_of(channel).ok() == Some(bank));
        if write_list(&mut writer, bank, members)? {
            blocks += 1;
        }
        if let Some(plan) = plans
            .iter()
            .find(|plan| u8::try_from(plan.id().get()).ok() == Some(bank))
        {
            let object = encode_generated_bank(*plan)?;
            writer.u8(KIND_LINEAR)?;
            writer.u8(bank)?;
            writer.u16(u16::try_from(object.len()).map_err(|_| CompileError::TooLarge)?)?;
            writer.bytes(object.data())?;
            blocks += 1;
        }
    }
    if blocks > MAX_BLOCKS {
        return Err(CompileError::TooManyBlocks);
    }
    let len = writer.at;
    let output = writer.output;
    output[..4].copy_from_slice(&CONFIGURATION_IMAGE_MAGIC);
    output[4] = IMAGE_VERSION;
    output[5] = FORMAT_VERSION;
    output[6..8].copy_from_slice(&u16::try_from(blocks).unwrap_or(0).to_le_bytes());
    output[8..12].copy_from_slice(&u32::try_from(len - HEADER_LEN).unwrap_or(0).to_le_bytes());
    let crc = configuration_image_crc(&output[..12], &output[HEADER_LEN..len]);
    output[12..16].copy_from_slice(&crc.to_le_bytes());
    Ok(len)
}

/// One block as the radio indexes it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Block {
    kind: u8,
    bank: u8,
    /// Channels in the block.
    count: u16,
    /// Index of the block's first channel.
    first: u16,
    /// Payload offset of the block's body.
    body: u16,
    /// Bytes per record in a list block; body bytes in a linear one.
    stride: u16,
}

/// What a radio keeps in RAM about the image it runs: a few bytes per block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Index {
    config: RadioConfig,
    blocks: [Block; MAX_BLOCKS],
    block_count: usize,
    names: [u16; MAX_BANKS as usize],
    len: u16,
    populated: u16,
}

impl Default for Index {
    fn default() -> Self {
        Self::empty()
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
    squelch: u8,
    tx_class: u8,
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
            squelch: bytes[29],
            tx_class: bytes[30],
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
        let [squelch] = cursor.field(self.fields & SQUELCH != 0, &[self.squelch]);
        let [tx_class] = cursor.field(self.fields & TX_CLASS != 0, &[self.tx_class]);
        let step =
            u32::from_le_bytes(cursor.field(self.fields & STEP != 0, &self.step.to_le_bytes()));
        let banks = if bank == NO_BANK {
            BankMask::default()
        } else {
            BankMask::default()
                .with(BankId::new(u16::from(bank)), true)
                .map_err(malformed)?
        };
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
            squelch: SquelchLevel::new(squelch).map_err(malformed)?,
            flags: ChannelFlags::from_bits(flags).map_err(malformed)?,
            banks,
            tx_class: TxClass::try_from(tx_class).map_err(malformed)?,
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
            config: RadioConfig::conservative(),
            blocks: [Block {
                kind: 0,
                bank: 0,
                count: 0,
                first: 0,
                body: 0,
                stride: 0,
            }; MAX_BLOCKS],
            block_count: 0,
            names: [ABSENT; MAX_BANKS as usize],
            len: 0,
            populated: 0,
        }
    }

    /// Validates an image held outside RAM and indexes it.
    ///
    /// `read` fills a buffer from a payload offset. The checksum is checked
    /// first, then every block and every channel is decoded once, so a radio
    /// never runs on an image any part of which is malformed.
    pub fn build(
        header: &[u8; HEADER_LEN],
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<Self, StorageError> {
        if header[..4] != CONFIGURATION_IMAGE_MAGIC {
            return Err(StorageError::MalformedImage);
        }
        if header[4] != IMAGE_VERSION || header[5] != FORMAT_VERSION {
            return Err(StorageError::UnsupportedImageVersion);
        }
        let block_count = usize::from(u16::from_le_bytes([header[6], header[7]]));
        let payload_len = usize::try_from(u32::from_le_bytes([
            header[8], header[9], header[10], header[11],
        ]))
        .map_err(|_| StorageError::ImageTooLarge)?;
        if block_count > MAX_BLOCKS || payload_len > usize::from(u16::MAX) {
            return Err(StorageError::ImageTooLarge);
        }
        let expected = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
        let mut crc = crc32_update(u32::MAX, &header[..12]);
        let mut chunk = [0_u8; 64];
        let mut at = 0;
        while at < payload_len {
            let len = chunk.len().min(payload_len - at);
            if !read(at, &mut chunk[..len]) {
                return Err(StorageError::MalformedImage);
            }
            crc = crc32_update(crc, &chunk[..len]);
            at += len;
        }
        if !crc != expected {
            return Err(StorageError::ImageIntegrity);
        }

        let mut index = Self::empty();
        let config = read_array::<RADIO_CONFIG_ENCODED_LEN>(&mut read, 0)?;
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
        let [bank_count] = read_array::<1>(&mut read, at)?;
        at += 1;
        for _ in 0..bank_count {
            let entry = read_array::<BANK_ENTRY_HEADER_LEN>(&mut read, at)?;
            let bank = Self::read_bank(&mut read, at, entry)?;
            let slot = &mut index.names[usize::from(bank.id().get())];
            if *slot != ABSENT {
                return Err(StorageError::NonCanonicalImage);
            }
            *slot = u16::try_from(at).map_err(|_| StorageError::ImageTooLarge)?;
            at += BANK_ENTRY_HEADER_LEN + usize::from(entry[2]);
        }
        let mut first = 0_u32;
        for slot in 0..block_count {
            let (block, body_end) = Self::read_block(&mut read, at, first)?;
            index.blocks[slot] = block;
            let (count, bank) = (block.count, block.bank);
            at = body_end;
            first += u32::from(count);
            if first > u32::from(u16::MAX) {
                return Err(StorageError::ImageTooLarge);
            }
            if bank != NO_BANK && count > 0 {
                index.populated |= 1 << bank;
            }
        }
        if at != payload_len {
            return Err(StorageError::MalformedImage);
        }
        index.block_count = block_count;
        index.len = u16::try_from(first).unwrap_or(u16::MAX);
        Ok(index)
    }

    /// Reads and validates the block at `at`, whose first channel is number
    /// `first`, and returns it with the offset just past it.
    fn read_block(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        at: usize,
        first: u32,
    ) -> Result<(Block, usize), StorageError> {
        let [kind, bank, low, high] = read_array::<BLOCK_HEADER_LEN>(read, at)?;
        let body_len = usize::from(u16::from_le_bytes([low, high]));
        let body = at + BLOCK_HEADER_LEN;
        if bank != NO_BANK && u16::from(bank) >= MAX_BANKS {
            return Err(StorageError::MalformedObject);
        }
        let (count, stride) = match kind {
            KIND_LINEAR => {
                let plan = Self::read_plan(read, body, body_len, bank)?;
                (plan.channel_count(), body_len)
            }
            KIND_LIST => {
                let list = ListHeader::decode(&read_array(read, body)?)?;
                let stride = record_len(list.fields, list.name_width);
                if LIST_HEADER_LEN + usize::from(list.count) * stride != body_len {
                    return Err(StorageError::MalformedObject);
                }
                for record in 0..list.count {
                    let mut bytes = [0_u8; MAX_RECORD_LEN];
                    let offset = body + LIST_HEADER_LEN + usize::from(record) * stride;
                    if !read(offset, &mut bytes[..stride]) {
                        return Err(StorageError::MalformedImage);
                    }
                    let id = u16::try_from(first + u32::from(record) + 1)
                        .map_err(|_| StorageError::ImageTooLarge)?;
                    list.channel(&bytes[..stride], id, bank)?;
                }
                (list.count, stride)
            }
            _ => return Err(StorageError::UnsupportedObject),
        };
        let block = Block {
            kind,
            bank,
            count,
            first: u16::try_from(first).map_err(|_| StorageError::ImageTooLarge)?,
            body: u16::try_from(body).map_err(|_| StorageError::ImageTooLarge)?,
            stride: u16::try_from(stride).map_err(|_| StorageError::ImageTooLarge)?,
        };
        Ok((block, body + body_len))
    }

    fn read_bank(
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        at: usize,
        entry: [u8; BANK_ENTRY_HEADER_LEN],
    ) -> Result<ChannelBank, StorageError> {
        let len = usize::from(entry[2]);
        if len > MAX_BANK_NAME_LEN {
            return Err(StorageError::MalformedObject);
        }
        let mut name = [0_u8; MAX_BANK_NAME_LEN];
        if !read(at + BANK_ENTRY_HEADER_LEN, &mut name[..len]) {
            return Err(StorageError::MalformedImage);
        }
        ChannelBank::new(
            BankId::new(u16::from(entry[0])),
            BankName::from_field(name, entry[2]).map_err(malformed)?,
            BankFlags::from_bits(entry[1]).map_err(malformed)?,
        )
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

    /// The global receive configuration.
    #[must_use]
    pub const fn config(&self) -> RadioConfig {
        self.config
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

    /// Banks with at least one channel, as a bit per bank.
    #[must_use]
    pub const fn populated(&self) -> u16 {
        self.populated
    }

    fn block_of(&self, index: u16) -> Option<(&Block, u16)> {
        self.blocks[..self.block_count]
            .iter()
            .find(|block| index >= block.first && index - block.first < block.count)
            .map(|block| (block, index - block.first))
    }

    /// Whether the channel at `index` is stored in `bank`, from RAM alone.
    #[must_use]
    pub fn member_at(&self, index: u16, bank: BankId) -> bool {
        self.block_of(index)
            .is_some_and(|(block, _)| u16::from(block.bank) == bank.get())
    }

    /// Reads and decodes the channel at `index`.
    pub fn channel_at(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        index: u16,
    ) -> Option<ChannelRecord> {
        let (block, inner) = self.block_of(index)?;
        let body = usize::from(block.body);
        if block.kind == KIND_LINEAR {
            return Self::read_plan(&mut read, body, usize::from(block.stride), block.bank)
                .ok()?
                .channel_record(inner)
                .ok();
        }
        let list = ListHeader::decode(&read_array(&mut read, body).ok()?).ok()?;
        let stride = usize::from(block.stride);
        let mut record = [0_u8; MAX_RECORD_LEN];
        let offset = body + LIST_HEADER_LEN + usize::from(inner) * stride;
        read(offset, &mut record[..stride]).then_some(())?;
        list.channel(&record[..stride], index + 1, block.bank).ok()
    }

    /// The name to show for `bank`: its own name, else its plan's.
    pub fn bank_name(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<BankName> {
        let slot = usize::from(bank.get());
        if let Some(&at) = self.names.get(slot).filter(|at| **at != ABSENT) {
            let at = usize::from(at);
            let entry = read_array::<BANK_ENTRY_HEADER_LEN>(&mut read, at).ok()?;
            return Self::read_bank(&mut read, at, entry)
                .ok()
                .map(ChannelBank::name);
        }
        let block = self.blocks[..self.block_count]
            .iter()
            .find(|block| block.kind == KIND_LINEAR && u16::from(block.bank) == bank.get())?;
        Self::read_plan(
            &mut read,
            usize::from(block.body),
            usize::from(block.stride),
            block.bank,
        )
        .ok()
        .map(GeneratedBank::name)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{compile, CompileError, Index, HEADER_LEN, NO_BANK};
    use crate::StorageError;
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord, GeneratedBank,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, SquelchLevel, Tone, TxClass,
    };
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
            power: PowerLevel::Low,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            squelch: SquelchLevel::new(3).unwrap(),
            flags: ChannelFlags::default(),
            banks: bank.map_or(BankMask::default(), |bank| {
                BankMask::default().with(BankId::new(bank), true).unwrap()
            }),
            tx_class: TxClass::Never,
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
        two.tx_class = TxClass::Amateur;
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
            TxClass::Never,
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
        let mut image = std::vec![0_u8; 8192];
        let len = compile(
            RadioConfig::conservative(),
            &banks(),
            channels,
            plans,
            &mut image,
        )
        .expect("compile");
        image.truncate(len);
        image
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
        assert_eq!(a.squelch(), b.squelch());
        assert_eq!(a.flags(), b.flags());
        assert_eq!(a.banks(), b.banks());
        assert_eq!(a.tx_class(), b.tx_class());
    }

    #[test]
    fn every_channel_comes_back_bank_by_bank() {
        let unbanked = record(channel(200, "AIR", 118_100_000, 118_100_000, None));
        let mut channels = repeaters();
        channels.push(unbanked);
        channels.extend(pmr());
        let image = compiled(&channels, &[plan()]);
        let index = index(&image).expect("index");

        // Bank 0's list, bank 1's list, bank 2's plan, then the unbanked list.
        assert_eq!(index.len(), 16 + 2 + 40 + 1);
        let expected: Vec<ChannelRecord> = pmr()
            .into_iter()
            .chain(repeaters())
            .chain((0..40).map(|n| plan().channel_record(n).unwrap()))
            .chain([unbanked])
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
        assert_eq!(index.populated(), 0b111);
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
    fn a_pmr_list_costs_its_names_and_short_frequencies() {
        let image = compiled(&pmr(), &[]);
        // Header 16, config 16, one named bank 3 + 6, block header 4, list
        // header 31, then 16 records of a 5-byte name and a 2-byte frequency.
        assert_eq!(image.len(), 16 + 16 + 1 + 3 + 6 + 4 + 31 + 16 * (5 + 2));
    }

    #[test]
    fn a_channel_in_two_banks_is_refused() {
        let mut definition = channel(1, "BOTH", 446_006_250, 446_006_250, Some(0));
        definition.banks = definition.banks.with(BankId::new(3), true).unwrap();
        let mut image = [0_u8; 512];
        assert_eq!(
            compile(
                RadioConfig::conservative(),
                &[],
                &[record(definition)],
                &[],
                &mut image
            ),
            Err(CompileError::MultipleBanks(ChannelId::new(1)))
        );
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
}
