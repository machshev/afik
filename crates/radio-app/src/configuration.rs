//! Bounded programmed configuration this image accepts and retains.
//!
//! The host compiles and validates a configuration image and writes it into
//! the radio's memory (`ADR-100`). This module turns the loaded objects into
//! exactly what the receive path and the user interface need: ordered channel
//! records, the named banks they belong to, and the global receive
//! configuration.
//!
//! Nothing here mints transmit authority. Channels keep the transmit class the
//! host programmed, and the image constructs no transmit path, so a channel
//! programmed as transmittable still cannot key this radio.

use radio_channel_plan::{BankName, ChannelBank, ChannelRecord, GeneratedBank, MAX_BANKS};
use radio_domain::{BankId, RadioConfig, SquelchLevel};
use radio_platform::memory_link::IMAGE_REGION_BYTES;
use radio_storage::{
    decode_channel, decode_channel_bank, decode_generated_bank, decode_radio_config,
    read_entry_header, walk_image, ObjectKind, ObjectRef, StorageError, CHANNEL_ENCODED_LEN,
    CONFIGURATION_IMAGE_HEADER_LEN, MAX_OBJECT_DATA, OBJECT_ENTRY_HEADER_LEN,
};

/// Packed configuration bytes this image stores.
///
/// This is the whole bound, and the only one the radio declares. What it buys
/// is decided by the operator rather than by the firmware: a project may spend
/// these bytes on explicit channels, on named banks, or — far more cheaply — on
/// generated plans, each of which costs one object however many channels it
/// expands to. Nothing here counts objects of a kind.
///
/// It is the image region of the memory view less the image header. The
/// image stays in memory and is read an object at a time; RAM holds only
/// [`Programmed`] and one bank mask per explicit channel.
pub const CONFIGURATION_STORE_BYTES: usize =
    IMAGE_REGION_BYTES as usize - CONFIGURATION_IMAGE_HEADER_LEN;

/// The most explicit channels the image region can hold, which is how many
/// bank masks the radio keeps in RAM.
pub const MAX_STORED_CHANNELS: usize =
    CONFIGURATION_STORE_BYTES / (OBJECT_ENTRY_HEADER_LEN + CHANNEL_ENCODED_LEN);

/// A payload offset meaning "no such object".
const ABSENT: u16 = u16::MAX;

/// Bytes one object entry can occupy.
const ENTRY_BYTES: usize = OBJECT_ENTRY_HEADER_LEN + MAX_OBJECT_DATA;

/// Why an active object snapshot cannot become a programmed configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    /// An object payload failed its own decoder.
    Object(StorageError),
    /// Stored plans expanded past the index space selection can address.
    TooManyExpanded,
    /// A bank identifier was outside the addressable range.
    BankOutOfRange,
    /// The programmed global configuration failed revalidation.
    InvalidConfig,
    /// More explicit channels than the radio keeps bank masks for.
    TooManyChannels,
    /// Explicit channels of different encoded sizes, which the offset
    /// arithmetic cannot address.
    UnevenChannels,
}

/// An index over the programmed configuration, holding no configuration.
///
/// This is the channelised model applied all the way up. A generated plan does
/// not materialise its channels; neither does the radio materialise its
/// configuration. What is held here is what selection and filtering need to
/// answer without touching storage — how many channels there are, which bank
/// each belongs to, and the global receive settings. Everything else is decoded
/// from the stored objects on the lookup that needs it and dropped again.
///
/// It costs the same whether the radio holds four channels or four thousand,
/// and every array in it is sized by the sixteen banks a membership mask can
/// address rather than by any bound on what may be stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Programmed {
    /// Explicit channel records, which occupy the first selection indices.
    stored_len: u16,
    /// Expanded channels each bank's plan contributes, in bank order.
    expanded: [u16; MAX_BANKS as usize],
    /// Bank identifiers a named bank object defines.
    named: u16,
    /// Bank identifiers at least one selectable channel belongs to.
    populated: u16,
    config: RadioConfig,
    /// Payload offset of the first explicit channel entry.
    channel_base: u16,
    /// Bytes from one explicit channel entry to the next.
    channel_stride: u16,
    /// Payload offset of each bank's generated plan.
    plans: [u16; MAX_BANKS as usize],
    /// Payload offset of each bank's named-bank object.
    names: [u16; MAX_BANKS as usize],
}

impl Default for Programmed {
    fn default() -> Self {
        Self::empty()
    }
}

impl Programmed {
    /// Returns an empty index carrying the conservative defaults.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            stored_len: 0,
            expanded: [0; MAX_BANKS as usize],
            named: 0,
            populated: 0,
            config: RadioConfig::conservative(),
            channel_base: 0,
            channel_stride: 0,
            plans: [ABSENT; MAX_BANKS as usize],
            names: [ABSENT; MAX_BANKS as usize],
        }
    }

    /// Validates the image held in memory and indexes it, keeping none of it.
    ///
    /// `read` fills a buffer from a payload offset. Every object is decoded
    /// once, here, so a malformed one is refused before the radio runs on it.
    /// What survives is how many channels there are, where each kind of
    /// object starts, which banks each explicit channel is in (written to
    /// `masks`), and the global configuration.
    ///
    /// Both channel kinds land in one selection space: an explicit record costs
    /// one stored object, and a generated plan costs one stored object however
    /// many channels it expands to.
    pub fn from_image(
        header: &[u8; CONFIGURATION_IMAGE_HEADER_LEN],
        read: impl FnMut(usize, &mut [u8]) -> bool,
        masks: &mut [u16],
    ) -> Result<Self, ConfigurationError> {
        let mut programmed = Self::empty();
        let mut expanded_total = 0_u32;
        let mut failure = None;
        let walked = walk_image(header, read, |offset, object| {
            match programmed.visit(offset, &object, masks, &mut expanded_total) {
                Ok(()) => true,
                Err(error) => {
                    failure = Some(error);
                    false
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        walked.map_err(ConfigurationError::Object)?;
        if expanded_total + u32::from(programmed.stored_len) > u32::from(u16::MAX) {
            return Err(ConfigurationError::TooManyExpanded);
        }
        Ok(programmed)
    }

    /// Indexes one object found at `offset`.
    fn visit(
        &mut self,
        offset: usize,
        object: &ObjectRef<'_>,
        masks: &mut [u16],
        expanded_total: &mut u32,
    ) -> Result<(), ConfigurationError> {
        let offset = u16::try_from(offset).map_err(|_| ConfigurationError::TooManyExpanded)?;
        match object.key().kind {
            ObjectKind::Channel => {
                let channel = decode_channel(object).map_err(ConfigurationError::Object)?;
                let stride = u16::try_from(OBJECT_ENTRY_HEADER_LEN + object.len())
                    .map_err(|_| ConfigurationError::UnevenChannels)?;
                // Channel objects sort together, so they sit end to end.
                if self.stored_len == 0 {
                    self.channel_base = offset;
                    self.channel_stride = stride;
                } else if stride != self.channel_stride {
                    return Err(ConfigurationError::UnevenChannels);
                }
                let mask = masks
                    .get_mut(usize::from(self.stored_len))
                    .ok_or(ConfigurationError::TooManyChannels)?;
                *mask = channel.banks().bits();
                self.stored_len += 1;
                self.populated |= channel.banks().bits();
            }
            ObjectKind::ChannelBank => {
                let bank = decode_channel_bank(object).map_err(ConfigurationError::Object)?;
                self.named |= bit(bank.id())?;
                self.names[usize::from(bank.id().get())] = offset;
            }
            ObjectKind::RadioConfig => {
                let config = decode_radio_config(object).map_err(ConfigurationError::Object)?;
                self.config = config
                    .validate()
                    .map_err(|_| ConfigurationError::InvalidConfig)?;
            }
            ObjectKind::GeneratedBank => {
                let plan = decode_generated_bank(object).map_err(ConfigurationError::Object)?;
                self.populated |= bit(plan.id())?;
                let bank = usize::from(plan.id().get());
                self.expanded[bank] = plan.channel_count();
                self.plans[bank] = offset;
                *expanded_total += u32::from(plan.channel_count());
            }
        }
        Ok(())
    }

    /// Returns this index with the radio-wide squelch replaced.
    ///
    /// The operator's handset choice is the radio's own and is kept beside
    /// the place, not written into the host's image.
    #[must_use]
    pub const fn with_squelch(mut self, squelch: SquelchLevel) -> Self {
        self.config.squelch = squelch;
        self
    }

    /// Returns the global receive configuration.
    #[must_use]
    pub const fn config(&self) -> RadioConfig {
        self.config
    }

    /// Returns the number of channels this configuration selects.
    #[must_use]
    pub fn len(&self) -> u16 {
        self.stored_len.saturating_add(self.expanded_channels())
    }

    /// Returns the number of programmed channels.
    #[must_use]
    pub fn channel_count(&self) -> u16 {
        self.len()
    }

    /// Returns the number of channels which cost one stored object each.
    #[must_use]
    pub const fn stored_channels(&self) -> u16 {
        self.stored_len
    }

    /// Returns the number of channels expanded from stored plans.
    #[must_use]
    pub fn expanded_channels(&self) -> u16 {
        self.expanded
            .iter()
            .fold(0_u16, |total, count| total.saturating_add(*count))
    }

    /// Reports whether a named bank object defines one bank identifier.
    #[must_use]
    pub const fn is_named(&self, bank: BankId) -> bool {
        self.named & (1 << bank.get()) != 0
    }

    /// Reports whether any channel was programmed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the plan bank owning one plan-space offset, and the index in it.
    ///
    /// Plans are held in bank order and expand in bank order, so the plan a
    /// selection index falls in is found by accumulating sixteen counts and
    /// touching no storage at all.
    fn plan_at(&self, offset: u16) -> Option<(BankId, u16)> {
        let mut remaining = offset;
        for (bank, count) in self.expanded.iter().enumerate() {
            if remaining < *count {
                return Some((BankId::new(u16::try_from(bank).ok()?), remaining));
            }
            remaining -= *count;
        }
        None
    }

    /// Reports whether the channel at one index belongs to one bank.
    ///
    /// An expanded channel belongs to its plan's bank and no other, which the
    /// index answers alone. A stored channel's banks are in `masks`, kept in
    /// RAM because a bank filter asks about every channel on every pass.
    #[must_use]
    pub fn member_at(&self, masks: &[u16], index: u16, bank: BankId) -> bool {
        match index.checked_sub(self.stored_len) {
            None => masks
                .get(usize::from(index))
                .is_some_and(|mask| bank.get() < MAX_BANKS && mask & (1 << bank.get()) != 0),
            Some(offset) => self
                .plan_at(offset)
                .is_some_and(|(plan_bank, _)| plan_bank == bank),
        }
    }

    /// Reads and decodes the channel at one index from memory.
    ///
    /// This is the only path which reads a channel, and it is taken once per
    /// channel the operator actually sees or tunes rather than once per channel
    /// walked. Nothing it decodes is retained.
    #[must_use]
    pub fn channel_at(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        index: u16,
    ) -> Option<ChannelRecord> {
        let mut entry = [0_u8; ENTRY_BYTES];
        match index.checked_sub(self.stored_len) {
            None => {
                let offset = usize::from(self.channel_base)
                    + usize::from(index) * usize::from(self.channel_stride);
                decode_channel(&read_object(&mut read, offset, &mut entry)?).ok()
            }
            Some(offset) => {
                let (bank, inner) = self.plan_at(offset)?;
                let at = self.plans[usize::from(bank.get())];
                let object = read_object(&mut read, usize::from(at), &mut entry)?;
                decode_generated_bank(&object)
                    .ok()?
                    .channel_record(inner)
                    .ok()
            }
        }
    }

    /// Returns the name to show for one bank.
    ///
    /// A generated plan names its own bank, so a radio programmed with plans
    /// alone still shows the operator what each filter selects. The name an
    /// operator chose wins over the one a plan derived.
    #[must_use]
    pub fn bank_name(
        &self,
        mut read: impl FnMut(usize, &mut [u8]) -> bool,
        bank: BankId,
    ) -> Option<BankName> {
        let slot = usize::from(bank.get());
        let mut entry = [0_u8; ENTRY_BYTES];
        if let Some(&at) = self.names.get(slot).filter(|at| **at != ABSENT) {
            let object = read_object(&mut read, usize::from(at), &mut entry)?;
            return decode_channel_bank(&object).ok().map(ChannelBank::name);
        }
        let at = *self.plans.get(slot).filter(|at| **at != ABSENT)?;
        let object = read_object(&mut read, usize::from(at), &mut entry)?;
        decode_generated_bank(&object).ok().map(GeneratedBank::name)
    }

    /// Returns the banks at least one selectable channel belongs to.
    ///
    /// The result is ordered by bank identifier and contains no bank without
    /// members, so a bank filter can never select an empty view. A generated
    /// plan populates its own bank, because every channel it expands to is a
    /// member of it. Answered from the index alone.
    #[must_use]
    pub fn populated_banks(&self) -> ([Option<BankId>; MAX_BANKS as usize], usize) {
        let mut banks = [None; MAX_BANKS as usize];
        let mut count = 0;
        for raw in 0..MAX_BANKS {
            if self.populated & (1 << raw) != 0 {
                banks[count] = Some(BankId::new(raw));
                count += 1;
            }
        }
        (banks, count)
    }
}

/// Returns the membership bit of one bank identifier.
fn bit(bank: BankId) -> Result<u16, ConfigurationError> {
    if bank.get() >= MAX_BANKS {
        return Err(ConfigurationError::BankOutOfRange);
    }
    Ok(1 << bank.get())
}

/// Reads the object entry at one payload offset into `entry`.
fn read_object<'a>(
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    offset: usize,
    entry: &'a mut [u8; ENTRY_BYTES],
) -> Option<ObjectRef<'a>> {
    let mut header = [0_u8; OBJECT_ENTRY_HEADER_LEN];
    if !read(offset, &mut header) {
        return None;
    }
    let (key, len) = read_entry_header(&header).ok()?;
    let data = &mut entry[..len];
    if !read(offset + OBJECT_ENTRY_HEADER_LEN, data) {
        return None;
    }
    Some(ObjectRef::new(key, data))
}

#[cfg(test)]
mod tests {
    use super::{ConfigurationError, Programmed, CONFIGURATION_STORE_BYTES, MAX_STORED_CHANNELS};
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord, GeneratedBank,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, SquelchLevel, Tone, TxClass,
    };
    use radio_storage::{
        encode_channel, encode_channel_bank, encode_configuration_image, encode_generated_bank,
        encode_radio_config, ObjectArena, StorageObject, CONFIGURATION_IMAGE_HEADER_LEN,
        OBJECT_ENTRY_HEADER_LEN,
    };
    use std::vec;
    use std::vec::Vec;

    /// A configuration image as the host writes it into memory.
    struct Image {
        header: [u8; CONFIGURATION_IMAGE_HEADER_LEN],
        payload: Vec<u8>,
        masks: [u16; MAX_STORED_CHANNELS],
    }

    impl Image {
        /// Encodes whatever an arena holds, in its canonical order.
        fn from_arena(arena: &ObjectArena<CONFIGURATION_STORE_BYTES>) -> Self {
            let objects: Vec<_> = arena.iter().collect();
            let mut bytes = vec![0_u8; CONFIGURATION_IMAGE_HEADER_LEN + CONFIGURATION_STORE_BYTES];
            let len = encode_configuration_image(&objects, &mut bytes).expect("image");
            let mut header = [0_u8; CONFIGURATION_IMAGE_HEADER_LEN];
            header.copy_from_slice(&bytes[..CONFIGURATION_IMAGE_HEADER_LEN]);
            Self {
                header,
                payload: bytes[CONFIGURATION_IMAGE_HEADER_LEN..len].to_vec(),
                masks: [0; MAX_STORED_CHANNELS],
            }
        }

        fn index(&mut self) -> Result<Programmed, ConfigurationError> {
            let payload = &self.payload;
            Programmed::from_image(
                &self.header,
                |offset, buffer| {
                    payload
                        .get(offset..offset + buffer.len())
                        .is_some_and(|bytes| {
                            buffer.copy_from_slice(bytes);
                            true
                        })
                },
                &mut self.masks,
            )
        }

        fn reader(&self) -> impl FnMut(usize, &mut [u8]) -> bool + '_ {
            |offset, buffer| {
                self.payload
                    .get(offset..offset + buffer.len())
                    .is_some_and(|bytes| {
                        buffer.copy_from_slice(bytes);
                        true
                    })
            }
        }
    }

    /// The image a host would write for these objects, in canonical order.
    fn store(objects: &[StorageObject]) -> Image {
        let mut arena = ObjectArena::new();
        for object in objects {
            arena.write(object).expect("store");
        }
        Image::from_arena(&arena)
    }

    fn channel(id: u16, hz: u32, banks: BankMask) -> ChannelRecord {
        let receive = Frequency::from_hz(hz).expect("frequency");
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("CH").expect("name"),
            receive,
            transmit: receive,
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step: FrequencyStep::from_hz(12_500).expect("step"),
            squelch: SquelchLevel::new(3).expect("squelch"),
            flags: ChannelFlags::default(),
            banks,
            tx_class: TxClass::Never,
        })
        .expect("channel")
    }

    #[test]
    fn objects_become_ordered_channels_named_banks_and_a_config() {
        let bank = BankId::new(2);
        let mask = BankMask::default().with(bank, true).expect("mask");
        let objects = [
            encode_channel(channel(7, 433_500_000, mask)).expect("channel"),
            encode_channel(channel(3, 145_500_000, mask)).expect("channel"),
            encode_channel_bank(
                ChannelBank::new(
                    bank,
                    BankName::new("UHF").expect("name"),
                    BankFlags::default(),
                )
                .expect("bank"),
            )
            .expect("bank object"),
            encode_radio_config(RadioConfig {
                backlight_seconds: 30,
                ..RadioConfig::conservative()
            })
            .expect("config"),
        ];
        let programmed = store(&objects).index().expect("programmed");
        assert_eq!(programmed.channel_count(), 2);
        assert_eq!(
            programmed
                .channel_at(store(&objects).reader(), 0)
                .expect("first")
                .id()
                .get(),
            3,
            "channels are ordered by stable identifier, not write order"
        );
        assert_eq!(programmed.config().backlight_seconds, 30);
        assert!(programmed.is_named(bank));
        assert_eq!(
            programmed
                .bank_name(store(&objects).reader(), bank)
                .expect("bank")
                .as_str(),
            "UHF"
        );
        let (banks, count) = programmed.populated_banks();
        assert_eq!(count, 1);
        assert_eq!(banks[0], Some(bank));
    }

    #[test]
    fn stored_channels_and_expanded_plans_occupy_one_selection_space() {
        let plan = GeneratedBank::linear_simplex(
            BankId::new(5),
            BankName::new("PMR446").expect("name"),
            Frequency::from_hz(446_006_250).expect("frequency"),
            FrequencyStep::from_hz(12_500).expect("step"),
            16,
            TxClass::Never,
        )
        .expect("plan");
        let objects = [
            encode_generated_bank(plan).expect("plan object"),
            encode_channel(channel(1, 145_500_000, BankMask::default())).expect("channel"),
        ];
        let programmed = store(&objects).index().expect("programmed");

        assert_eq!(programmed.channel_count(), 17);
        assert_eq!(programmed.stored_channels(), 1);
        assert_eq!(programmed.expanded_channels(), 16);
        assert_eq!(
            programmed
                .channel_at(store(&objects).reader(), 0)
                .expect("stored")
                .id()
                .get(),
            1
        );
        assert_eq!(
            programmed
                .channel_at(store(&objects).reader(), 1)
                .expect("expanded")
                .name()
                .as_str(),
            "PMR 1"
        );

        // The plan names and populates its own bank without a named-bank object.
        let (banks, count) = programmed.populated_banks();
        assert_eq!(count, 1);
        assert_eq!(banks[0], Some(BankId::new(5)));
        assert_eq!(
            programmed
                .bank_name(store(&objects).reader(), BankId::new(5))
                .expect("plan name")
                .as_str(),
            "PMR446"
        );
        assert!(
            !programmed.is_named(BankId::new(5)),
            "the plan names its bank with no named-bank object beside it"
        );
    }

    #[test]
    fn a_band_sized_plan_costs_one_object_and_is_accepted_whole() {
        // The civil airband at 25 kHz is 760 channels. It is one stored object,
        // so no bound on expanded channels may refuse it: doing so would cost
        // the operator 759 channels to save nothing.
        let objects = [encode_generated_bank(
            GeneratedBank::linear_simplex(
                BankId::new(0),
                BankName::new("AIRBAND").expect("name"),
                Frequency::from_hz(118_000_000).expect("frequency"),
                FrequencyStep::from_hz(25_000).expect("step"),
                760,
                TxClass::Never,
            )
            .expect("plan"),
        )
        .expect("plan object")];
        let programmed = store(&objects).index().expect("programmed");
        assert_eq!(programmed.len(), 760);
        let last = programmed
            .channel_at(store(&objects).reader(), 759)
            .expect("last channel");
        assert_eq!(last.active().receive.as_hz(), 136_975_000);
    }

    #[test]
    fn an_unprogrammed_snapshot_is_empty_and_conservative() {
        let programmed = store(&[]).index().expect("programmed");
        assert!(programmed.is_empty());
        assert_eq!(programmed.config(), RadioConfig::conservative());
        assert_eq!(programmed.populated_banks().1, 0);
        assert!(!programmed.is_named(BankId::new(0)));
    }

    /// There is no channel count. What the operator gets for the bytes is
    /// whatever mixture of objects fits them, and the store is what refuses.
    #[test]
    fn channels_are_bounded_by_bytes_and_by_nothing_else() {
        let mut arena = ObjectArena::<CONFIGURATION_STORE_BYTES>::new();
        let mut stored = 0_u16;
        while arena
            .write(&encode_channel(channel(stored + 1, 145_000_000, BankMask::default())).unwrap())
            .is_ok()
        {
            stored += 1;
        }
        assert!(
            stored > 8,
            "the store holds {stored} explicit channels, far more than a fixed table did"
        );
        let programmed = Image::from_arena(&arena).index().expect("programmed");
        assert_eq!(programmed.channel_count(), stored);
        assert_eq!(
            programmed
                .channel_at(Image::from_arena(&arena).reader(), stored - 1)
                .expect("last")
                .id()
                .get(),
            stored,
            "the last channel the bytes had room for is selectable"
        );
    }

    #[test]
    fn a_malformed_object_payload_is_rejected_rather_than_guessed() {
        let mut image =
            store(
                &[encode_channel(channel(1, 145_000_000, BankMask::default())).expect("channel")],
            );
        // Zero the channel's payload, past its entry header: a zero frequency
        // is no channel, and the memory holding it must not be trusted.
        image.payload[OBJECT_ENTRY_HEADER_LEN..].fill(0);
        assert!(matches!(image.index(), Err(ConfigurationError::Object(_))));
    }
}
