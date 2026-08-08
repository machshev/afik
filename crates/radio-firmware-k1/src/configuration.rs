//! Bounded programmed configuration this image accepts and retains.
//!
//! The host tooling programs configuration objects through the shared device
//! service. This module turns one active object snapshot into exactly what the
//! receive path and the user interface need: ordered channel records, the named
//! banks they belong to, and the global receive configuration.
//!
//! Nothing here mints transmit authority. Channels keep the transmit class the
//! host programmed, and the image constructs no transmit path, so a channel
//! programmed as transmittable still cannot key this radio.

use radio_channel_plan::{
    BankMask, BankName, ChannelBank, ChannelRecord, GeneratedBank, PlanEncoding,
    MAX_BANKS as PLAN_MAX_BANKS,
};
use radio_device::{DeviceService, KindLimits};
use radio_domain::{BankId, RadioConfig, SquelchLevel};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_generated_bank, decode_radio_config,
    encode_radio_config, ObjectKind, StorageError, StorageObject, CHANNEL_BANK_ENCODED_LEN,
    CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_OBJECT_HEADER_LEN,
    GENERATED_BANK_ENCODED_LEN, RADIO_CONFIG_ENCODED_LEN,
};

/// Explicit channels this image stores and selects.
///
/// The bound is set by RAM, not by taste: the store holds an active and a
/// candidate copy of every object, the interface holds the decoded channels,
/// and all of it has to fit the evidenced 16 KiB of SRAM beside the executor,
/// the framebuffer, and the retained-image buffer.
///
/// This is deliberately smaller than the plan bound below. A stored channel
/// slot and a plan slot cost this image about the same RAM, but a stored slot
/// buys one channel and a plan slot buys a whole band, so the budget is spent
/// where it goes furthest. Explicit records are for the channels a plan cannot
/// describe: a repeater filed with a simplex band, an off-grid calling channel,
/// a one-off frequency.
pub const MAX_CHANNELS: usize = 8;

/// Named banks this image stores.
///
/// Every bank object occupies a full fixed-size slot in both the active and the
/// candidate snapshot, so a named bank costs this image far more RAM than its
/// twenty-two encoded bytes. Eight is what the retained store can afford beside
/// the channels, the plans, and a working stack; the membership mask still
/// addresses sixteen, so a project using a higher identifier is refused at
/// validation rather than silently dropped.
pub const MAX_BANKS: usize = 8;

// A bank this image stores must be addressable by a channel membership mask.
const _: () = assert!(MAX_BANKS <= PLAN_MAX_BANKS as usize);

/// Generated plans this image stores and expands.
///
/// A plan costs one stored object however many channels it holds, so this bound
/// buys channels cheaply. It is set by RAM rather than by the retained-image
/// budget: the configuration is held and copied by value, so every slot is paid
/// for in each copy whether or not a plan occupies it, and the stack headroom
/// the executor and the interrupt frames need is what limits it.
///
/// Six is what the UK and EU simplex set needs with room to spare: PMR446, the
/// 2 m and 70 cm simplex bands, and their repeater sub-bands are five plans and
/// somewhere over a hundred channels for under four hundred stored bytes.
pub const MAX_GENERATED_BANKS: usize = 6;

/// Configuration objects the device advertises and accepts.
///
/// The bound is the sum of what this image can use: every channel, every named
/// bank, every generated plan, and the singleton radio configuration.
pub const MAX_OBJECTS: usize = MAX_CHANNELS + MAX_BANKS + MAX_GENERATED_BANKS + 1;

/// Bytes reserved for the retained canonical configuration image.
///
/// This is a whole number of flash write pages so a retained image can be
/// written without a read-modify-write cycle.
pub const RETAINED_IMAGE_BYTES: usize = 1_280;

/// Largest canonical image a full configuration can produce.
pub const MAX_CONFIGURATION_IMAGE_BYTES: usize = CONFIGURATION_IMAGE_HEADER_LEN
    + MAX_CHANNELS * (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + CHANNEL_ENCODED_LEN)
    + MAX_BANKS * (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + CHANNEL_BANK_ENCODED_LEN)
    + MAX_GENERATED_BANKS * (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + GENERATED_BANK_ENCODED_LEN)
    + (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + RADIO_CONFIG_ENCODED_LEN);

// A retained region which cannot hold the largest programmable configuration
// would fail only after the operator had already programmed the radio.
const _: () = assert!(MAX_CONFIGURATION_IMAGE_BYTES <= RETAINED_IMAGE_BYTES);

/// Returns the object counts this image will activate.
///
/// The store is large enough to stage a complete project, but the interface can
/// only select [`MAX_CHANNELS`] channels, so a larger candidate is rejected when
/// the host validates it rather than after it is already running.
#[must_use]
pub fn kind_limits() -> KindLimits {
    KindLimits {
        generated_banks: u16::try_from(MAX_GENERATED_BANKS).unwrap_or(u16::MAX),
        channels: u16::try_from(MAX_CHANNELS).unwrap_or(u16::MAX),
        channel_banks: u16::try_from(MAX_BANKS).unwrap_or(u16::MAX),
        radio_configs: 1,
    }
}

/// Constructs the configuration service this image exposes over serial.
#[must_use]
pub fn device_service(configuration_bytes: u32) -> DeviceService<MAX_OBJECTS> {
    // This image expands both arithmetic plan families itself, so it advertises
    // them and the host may compile either for it. A fixed-offset plan is
    // honestly supported here: its receive frequencies are what this image
    // tunes, and it constructs no transmit path for any channel, stored or
    // expanded. The stored-configuration bound is the external-memory region
    // the image claimed, so a host can say how much room a project leaves
    // before writing it.
    DeviceService::with_configuration_capacity(
        PlanEncoding::LinearSimplex.capability_bit()
            | PlanEncoding::LinearFixedOffset.capability_bit(),
        kind_limits(),
        configuration_bytes,
    )
}

/// Replaces the stored radio-wide squelch level.
///
/// The operator can change squelch on the handset, and a setting which did not
/// survive a battery change would not be worth the menu. The active snapshot is
/// rewritten through the ordinary validating path with one field changed, so a
/// rejected result leaves the radio exactly as it was rather than half
/// reconfigured. A radio which was never programmed gains a configuration
/// object carrying the conservative defaults and the chosen level.
///
/// The caller is responsible for retaining the resulting image; this changes
/// what the radio is running, not what its memory holds.
pub fn store_squelch<const OBJECTS: usize>(
    service: &mut DeviceService<OBJECTS>,
    squelch: SquelchLevel,
) -> Result<u32, StorageError> {
    let mut config = RadioConfig::conservative();
    let mut objects = [None; OBJECTS];
    let mut count = 0;
    for object in service.active_objects() {
        if object.key().kind == ObjectKind::RadioConfig {
            // The one object being replaced is not carried over, so the level
            // is the only field this changes.
            config = decode_radio_config(object)?;
            continue;
        }
        let slot = objects.get_mut(count).ok_or(StorageError::StoreFull)?;
        *slot = Some(*object);
        count += 1;
    }
    config.squelch = squelch;
    let replacement = encode_radio_config(config)?;
    let slot = objects.get_mut(count).ok_or(StorageError::StoreFull)?;
    *slot = Some(replacement);
    service.load(objects.into_iter().flatten())
}

/// Why an active object snapshot cannot become a programmed configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    /// An object payload failed its own decoder.
    Object(StorageError),
    /// More channels were stored than this image can select.
    TooManyChannels,
    /// More generated plans were stored than this image can expand.
    TooManyPlans,
    /// Stored plans expanded past the index space selection can address.
    TooManyExpanded,
    /// A bank identifier was outside the addressable range.
    BankOutOfRange,
    /// The programmed global configuration failed revalidation.
    InvalidConfig,
}

/// One stored channel's identity and bank membership.
///
/// Held so that selection counting and bank filtering — the walks which touch
/// every channel — need no decode and no lock. Four bytes against the
/// forty-four a decoded record costs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StoredEntry {
    id: u16,
    banks: BankMask,
}

/// One installed plan's bank and size.
///
/// A plan expands into its own bank and no other, so this is everything bank
/// filtering needs to know about it without decoding the plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PlanEntry {
    bank: u16,
    count: u16,
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
/// The consequence is that this type costs the same whether a radio holds four
/// channels or four thousand, and the object bounds size stored bytes rather
/// than SRAM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Programmed {
    stored: [StoredEntry; MAX_CHANNELS],
    stored_len: u16,
    plans: [PlanEntry; MAX_GENERATED_BANKS],
    plan_len: u8,
    expanded: u16,
    /// Bank identifiers a named bank object defines.
    named: u16,
    config: RadioConfig,
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
            stored: [StoredEntry {
                id: 0,
                banks: BankMask::from_bits(0),
            }; MAX_CHANNELS],
            stored_len: 0,
            plans: [PlanEntry { bank: 0, count: 0 }; MAX_GENERATED_BANKS],
            plan_len: 0,
            expanded: 0,
            named: 0,
            config: RadioConfig::conservative(),
        }
    }

    /// Indexes one active object snapshot without retaining any of it.
    ///
    /// Every object is decoded once, here, so that a malformed one is refused
    /// before the radio runs on it. Nothing decoded is kept: what survives is
    /// each stored channel's identifier and membership, each plan's bank and
    /// size, and the global configuration.
    ///
    /// Both channel kinds land in one selection space: an explicit record costs
    /// one stored object, and a generated plan costs one stored object however
    /// many channels it expands to. There is deliberately no bound on expanded
    /// channels, because holding one would cost the operator channels for
    /// nothing.
    pub fn index<'a, I>(objects: I) -> Result<Self, ConfigurationError>
    where
        I: IntoIterator<Item = &'a StorageObject>,
    {
        let mut programmed = Self::empty();
        for object in objects {
            match object.key().kind {
                ObjectKind::Channel => {
                    let channel = decode_channel(object).map_err(ConfigurationError::Object)?;
                    programmed.insert_stored(StoredEntry {
                        id: channel.id().get(),
                        banks: channel.banks(),
                    })?;
                }
                ObjectKind::ChannelBank => {
                    let bank = decode_channel_bank(object).map_err(ConfigurationError::Object)?;
                    if usize::from(bank.id().get()) >= MAX_BANKS {
                        return Err(ConfigurationError::BankOutOfRange);
                    }
                    programmed.named |= 1 << bank.id().get();
                }
                ObjectKind::RadioConfig => {
                    let config = decode_radio_config(object).map_err(ConfigurationError::Object)?;
                    programmed.config = config
                        .validate()
                        .map_err(|_| ConfigurationError::InvalidConfig)?;
                }
                ObjectKind::GeneratedBank => {
                    let plan = decode_generated_bank(object).map_err(ConfigurationError::Object)?;
                    programmed.insert_plan(PlanEntry {
                        bank: plan.id().get(),
                        count: plan.channel_count(),
                    })?;
                }
            }
        }
        Ok(programmed)
    }

    /// Inserts one stored-channel entry, keeping identifier order.
    fn insert_stored(&mut self, entry: StoredEntry) -> Result<(), ConfigurationError> {
        let len = usize::from(self.stored_len);
        if len >= MAX_CHANNELS {
            return Err(ConfigurationError::TooManyChannels);
        }
        let mut position = len;
        for index in 0..len {
            if self.stored[index].id == entry.id {
                self.stored[index] = entry;
                return Ok(());
            }
            if self.stored[index].id > entry.id {
                position = index;
                break;
            }
        }
        let mut index = len;
        while index > position {
            self.stored[index] = self.stored[index - 1];
            index -= 1;
        }
        self.stored[position] = entry;
        self.stored_len += 1;
        Ok(())
    }

    /// Inserts one plan entry, keeping bank order and checking the index space.
    fn insert_plan(&mut self, entry: PlanEntry) -> Result<(), ConfigurationError> {
        let len = usize::from(self.plan_len);
        let mut position = len;
        for index in 0..len {
            if self.plans[index].bank == entry.bank {
                let without = self.expanded - self.plans[index].count;
                self.expanded = Self::checked_total(self.stored_len, without, entry.count)?;
                self.plans[index] = entry;
                return Ok(());
            }
            if self.plans[index].bank > entry.bank {
                position = index;
                break;
            }
        }
        if len >= MAX_GENERATED_BANKS {
            return Err(ConfigurationError::TooManyPlans);
        }
        let expanded = Self::checked_total(self.stored_len, self.expanded, entry.count)?;
        let mut index = len;
        while index > position {
            self.plans[index] = self.plans[index - 1];
            index -= 1;
        }
        self.plans[position] = entry;
        self.plan_len += 1;
        self.expanded = expanded;
        Ok(())
    }

    /// Returns the expanded total, refusing one the index space cannot address.
    fn checked_total(stored: u16, expanded: u16, added: u16) -> Result<u16, ConfigurationError> {
        let total = expanded
            .checked_add(added)
            .ok_or(ConfigurationError::TooManyExpanded)?;
        if u32::from(stored) + u32::from(total) > u32::from(u16::MAX) {
            return Err(ConfigurationError::TooManyExpanded);
        }
        Ok(total)
    }

    /// Returns the global receive configuration.
    #[must_use]
    pub const fn config(&self) -> RadioConfig {
        self.config
    }

    /// Returns the number of channels this configuration selects.
    #[must_use]
    pub const fn len(&self) -> u16 {
        self.stored_len.saturating_add(self.expanded)
    }

    /// Returns the number of programmed channels.
    #[must_use]
    pub const fn channel_count(&self) -> u16 {
        self.len()
    }

    /// Returns the number of channels which cost one stored object each.
    #[must_use]
    pub const fn stored_channels(&self) -> u16 {
        self.stored_len
    }

    /// Returns the number of channels expanded from stored plans.
    #[must_use]
    pub const fn expanded_channels(&self) -> u16 {
        self.expanded
    }

    /// Reports whether a named bank object defines one bank identifier.
    #[must_use]
    pub const fn is_named(&self, bank: BankId) -> bool {
        self.named & (1 << bank.get()) != 0
    }

    /// Reports whether any channel was programmed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reports whether the channel at one index belongs to one bank.
    ///
    /// Answered from the index alone: a stored channel carries its membership
    /// here, and an expanded one belongs to its plan's bank and no other. So the
    /// filtered walks never decode and never lock, which is what keeps a
    /// band-sized plan cheap to select within.
    #[must_use]
    pub fn member_at(&self, index: u16, bank: BankId) -> bool {
        match index.checked_sub(self.stored_len) {
            None => self
                .stored
                .get(usize::from(index))
                .is_some_and(|entry| entry.banks.contains(bank)),
            Some(offset) => self
                .plan_at(offset)
                .is_some_and(|(plan, _)| plan.bank == bank.get()),
        }
    }

    /// Returns the plan owning one plan-space offset, and the index within it.
    fn plan_at(&self, offset: u16) -> Option<(PlanEntry, u16)> {
        let mut remaining = offset;
        for entry in self.plans.iter().take(usize::from(self.plan_len)) {
            if remaining < entry.count {
                return Some((*entry, remaining));
            }
            remaining -= entry.count;
        }
        None
    }

    /// Expands or decodes the channel at one index from the stored objects.
    ///
    /// This is the only path which touches storage, and it is taken once per
    /// channel the operator actually sees or tunes rather than once per channel
    /// walked. Nothing it decodes is retained.
    #[must_use]
    pub fn channel_at<'a, I>(&self, objects: I, index: u16) -> Option<ChannelRecord>
    where
        I: IntoIterator<Item = &'a StorageObject>,
    {
        match index.checked_sub(self.stored_len) {
            None => {
                let id = self.stored.get(usize::from(index))?.id;
                let object = find(objects, ObjectKind::Channel, id)?;
                decode_channel(object).ok()
            }
            Some(offset) => {
                let (entry, inner) = self.plan_at(offset)?;
                let object = find(objects, ObjectKind::GeneratedBank, entry.bank)?;
                decode_generated_bank(object)
                    .ok()?
                    .channel_record(inner)
                    .ok()
            }
        }
    }

    /// Returns the name to show for one bank.
    ///
    /// A generated plan names its own bank, so a radio programmed with plans
    /// alone still shows the operator what each filter selects.
    #[must_use]
    pub fn bank_name<'a, I>(&self, objects: I, bank: BankId) -> Option<BankName>
    where
        I: IntoIterator<Item = &'a StorageObject>,
    {
        // One pass: a named bank and a plan may share an identifier, and the
        // name an operator chose wins over the one a plan derived.
        let mut named = None;
        let mut plan = None;
        for object in objects {
            if object.key().id != bank.get() {
                continue;
            }
            match object.key().kind {
                ObjectKind::ChannelBank => named = decode_channel_bank(object).ok(),
                ObjectKind::GeneratedBank => plan = decode_generated_bank(object).ok(),
                _ => {}
            }
        }
        named
            .map(ChannelBank::name)
            .or_else(|| plan.map(GeneratedBank::name))
    }

    /// Returns the banks at least one selectable channel belongs to.
    ///
    /// The result is ordered by bank identifier and contains no bank without
    /// members, so a bank filter can never select an empty view. A generated
    /// plan populates its own bank, because every channel it expands to is a
    /// member of it. Answered from the index alone.
    #[must_use]
    pub fn populated_banks(&self) -> ([Option<BankId>; MAX_BANKS], usize) {
        let mut banks = [None; MAX_BANKS];
        let mut count = 0;
        for raw in 0..u16::try_from(MAX_BANKS).unwrap_or(u16::MAX) {
            let bank = BankId::new(raw);
            let populated = self
                .plans
                .iter()
                .take(usize::from(self.plan_len))
                .any(|entry| entry.bank == raw)
                || self
                    .stored
                    .iter()
                    .take(usize::from(self.stored_len))
                    .any(|entry| entry.banks.contains(bank));
            if populated {
                banks[count] = Some(bank);
                count += 1;
            }
        }
        (banks, count)
    }
}

/// Returns the stored object with one kind and identifier.
fn find<'a, I>(objects: I, kind: ObjectKind, id: u16) -> Option<&'a StorageObject>
where
    I: IntoIterator<Item = &'a StorageObject>,
{
    objects
        .into_iter()
        .find(|object| object.key().kind == kind && object.key().id == id)
}

#[cfg(test)]
mod tests {
    use super::{
        device_service, store_squelch, ConfigurationError, Programmed, MAX_CHANNELS, MAX_OBJECTS,
    };
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord, GeneratedBank,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, SquelchLevel, Tone, TxClass,
    };
    use radio_storage::{
        encode_channel, encode_channel_bank, encode_generated_bank, encode_radio_config, ObjectKey,
        ObjectKind, StorageObject,
    };

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
        let programmed = Programmed::index(objects.iter()).expect("programmed");
        assert_eq!(programmed.channel_count(), 2);
        assert_eq!(
            programmed
                .channel_at(objects.iter(), 0)
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
                .bank_name(objects.iter(), bank)
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
        let programmed = Programmed::index(objects.iter()).expect("programmed");

        assert_eq!(programmed.channel_count(), 17);
        assert_eq!(programmed.stored_channels(), 1);
        assert_eq!(programmed.expanded_channels(), 16);
        assert_eq!(
            programmed
                .channel_at(objects.iter(), 0)
                .expect("stored")
                .id()
                .get(),
            1
        );
        assert_eq!(
            programmed
                .channel_at(objects.iter(), 1)
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
                .bank_name(objects.iter(), BankId::new(5))
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
        let programmed = Programmed::index(objects.iter()).expect("programmed");
        assert_eq!(programmed.len(), 760);
        let last = programmed
            .channel_at(objects.iter(), 759)
            .expect("last channel");
        assert_eq!(last.active().receive.as_hz(), 136_975_000);
    }

    #[test]
    fn an_unprogrammed_snapshot_is_empty_and_conservative() {
        let programmed = Programmed::index([].iter()).expect("programmed");
        assert!(programmed.is_empty());
        assert_eq!(programmed.config(), RadioConfig::conservative());
        assert_eq!(programmed.populated_banks().1, 0);
        assert!(!programmed.is_named(BankId::new(0)));
    }

    #[test]
    fn more_channels_than_the_image_selects_is_rejected() {
        let mut objects = std::vec::Vec::new();
        for id in 0..=u16::try_from(MAX_CHANNELS).unwrap() {
            objects.push(
                encode_channel(channel(id + 1, 145_000_000, BankMask::default())).expect("channel"),
            );
        }
        assert!(objects.len() <= MAX_OBJECTS);
        assert_eq!(
            Programmed::index(objects.iter()),
            Err(ConfigurationError::TooManyChannels)
        );
    }

    /// A level chosen on the handset has to become part of what is stored.
    #[test]
    fn storing_a_squelch_level_keeps_every_other_object_and_field() {
        let mut service = device_service(4_096);
        let bank = BankId::new(2);
        let mask = BankMask::default().with(bank, true).expect("mask");
        service
            .load([
                encode_channel(channel(7, 433_500_000, mask)).expect("channel"),
                encode_channel(channel(3, 145_500_000, mask)).expect("channel"),
                encode_radio_config(RadioConfig {
                    backlight_seconds: 30,
                    squelch: SquelchLevel::new(2).expect("level"),
                    ..RadioConfig::conservative()
                })
                .expect("config"),
            ])
            .expect("load");
        let before = service.generation();

        store_squelch(&mut service, SquelchLevel::new(8).expect("level")).expect("store");
        assert_ne!(service.generation(), before, "the change is a new snapshot");

        let programmed = Programmed::index(service.active_objects()).expect("programmed");
        assert_eq!(programmed.config().squelch, SquelchLevel::new(8).unwrap());
        assert_eq!(
            programmed.config().backlight_seconds,
            30,
            "no other field is disturbed"
        );
        assert_eq!(programmed.channel_count(), 2, "no channel is lost");
        assert_eq!(
            service
                .active_objects()
                .filter(|object| object.key().kind == ObjectKind::RadioConfig)
                .count(),
            1,
            "the configuration is replaced, not duplicated"
        );
    }

    /// An unprogrammed radio still has to be able to set its own squelch.
    #[test]
    fn storing_a_squelch_level_on_an_empty_radio_creates_the_configuration() {
        let mut service = device_service(4_096);
        assert_eq!(service.active_objects().count(), 0);

        store_squelch(&mut service, SquelchLevel::new(6).expect("level")).expect("store");

        let programmed = Programmed::index(service.active_objects()).expect("programmed");
        assert_eq!(programmed.config().squelch, SquelchLevel::new(6).unwrap());
        assert_eq!(
            RadioConfig {
                squelch: programmed.config().squelch,
                ..programmed.config()
            },
            RadioConfig {
                squelch: SquelchLevel::new(6).unwrap(),
                ..RadioConfig::conservative()
            },
            "everything else stays at the conservative defaults"
        );
    }

    #[test]
    fn a_malformed_object_payload_is_rejected_rather_than_guessed() {
        let malformed = StorageObject::new(
            ObjectKey {
                kind: ObjectKind::Channel,
                id: 1,
            },
            &[0_u8; 8],
        )
        .expect("object");
        assert!(matches!(
            Programmed::index([malformed].iter()),
            Err(ConfigurationError::Object(_))
        ));
    }
}
