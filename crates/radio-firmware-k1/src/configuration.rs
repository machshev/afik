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
    BankName, ChannelBank, ChannelRecord, GeneratedBank, PlanEncoding, MAX_BANKS,
};
use radio_device::DeviceService;
use radio_domain::{BankId, RadioConfig, SquelchLevel};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_generated_bank, decode_radio_config,
    encode_radio_config, Object, ObjectKind, ObjectRef, StorageError,
    CONFIGURATION_IMAGE_HEADER_LEN,
};

/// Packed configuration bytes this image stores.
///
/// This is the whole bound, and the only one the radio declares. What it buys
/// is decided by the operator rather than by the firmware: a project may spend
/// these bytes on explicit channels, on named banks, or — far more cheaply — on
/// generated plans, each of which costs one object however many channels it
/// expands to. Nothing here counts objects of a kind.
///
/// The number is set by RAM. The store holds an active and a candidate copy,
/// the interface holds a third for the channels it is showing, and all of it
/// has to fit the evidenced 16 KiB of SRAM beside the executor, the
/// framebuffer, and the retained-image buffer.
pub const CONFIGURATION_STORE_BYTES: usize = 1_264;

/// Bytes reserved for the retained canonical configuration image.
///
/// This is a whole number of write pages so a retained image can be written
/// without a read-modify-write cycle, and it is exactly what the store it
/// retains can hold.
pub const RETAINED_IMAGE_BYTES: usize = CONFIGURATION_IMAGE_HEADER_LEN + CONFIGURATION_STORE_BYTES;

// A retained region which cannot hold the largest programmable configuration
// would fail only after the operator had already programmed the radio.
const _: () = assert!(RETAINED_IMAGE_BYTES == 1_280);

/// The configuration service this image exposes over serial.
pub type K1DeviceService = DeviceService<CONFIGURATION_STORE_BYTES>;

/// Constructs the configuration service this image exposes over serial.
#[must_use]
pub fn device_service() -> K1DeviceService {
    // This image expands both arithmetic plan families itself, so it advertises
    // them and the host may compile either for it. A fixed-offset plan is
    // honestly supported here: its receive frequencies are what this image
    // tunes, and it constructs no transmit path for any channel, stored or
    // expanded. Everything else the device advertises — the stored-byte bound
    // it accepts and the object count that bound implies — is derived from the
    // store itself, so it cannot claim a capacity it does not have.
    DeviceService::with_plan_encodings(
        PlanEncoding::LinearSimplex.capability_bit()
            | PlanEncoding::LinearFixedOffset.capability_bit(),
    )
}

/// One radio-wide setting the operator changed on the handset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingChange {
    /// The squelch level applied when no channel overrides it.
    Squelch(SquelchLevel),
    /// How long a scan listens to a channel before it moves on.
    ScanDwellMs(u32),
}

/// Applies one handset setting to the stored radio-wide configuration.
///
/// The operator can change these on the handset, and a setting which did not
/// survive a battery change would not be worth the menu. The one object being
/// changed is rewritten through the ordinary validating path, so a rejected
/// value leaves the radio exactly as it was rather than half reconfigured — and
/// a zero dwell is rejected there rather than here, because the domain owns
/// what a valid configuration is. A radio which was never programmed gains a
/// configuration object carrying the conservative defaults and the one chosen
/// value.
///
/// The caller is responsible for retaining the resulting image; this changes
/// what the radio is running, not what its memory holds.
pub fn store_setting<const BYTES: usize>(
    service: &mut DeviceService<BYTES>,
    change: SettingChange,
) -> Result<u32, StorageError> {
    let mut config = service
        .active_objects()
        .find(|object| object.key().kind == ObjectKind::RadioConfig)
        .map_or_else(
            || Ok(RadioConfig::conservative()),
            |object| decode_radio_config(&object),
        )?;
    match change {
        SettingChange::Squelch(squelch) => config.squelch = squelch,
        SettingChange::ScanDwellMs(milliseconds) => config.scan_dwell_ms = milliseconds,
    }
    let replacement = encode_radio_config(config)?;
    service.store_object(&replacement)
}

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
        }
    }

    /// Indexes one active object snapshot without retaining any of it.
    ///
    /// Every object is decoded once, here, so that a malformed one is refused
    /// before the radio runs on it. Nothing decoded is kept: what survives is
    /// how many channels there are, which bank each is in, and the global
    /// configuration.
    ///
    /// Both channel kinds land in one selection space: an explicit record costs
    /// one stored object, and a generated plan costs one stored object however
    /// many channels it expands to. There is deliberately no bound on expanded
    /// channels, because holding one would cost the operator channels for
    /// nothing.
    pub fn index<'a, I>(objects: I) -> Result<Self, ConfigurationError>
    where
        I: IntoIterator<Item = ObjectRef<'a>>,
    {
        let mut programmed = Self::empty();
        let mut expanded_total = 0_u32;
        for object in objects {
            match object.key().kind {
                ObjectKind::Channel => {
                    let channel = decode_channel(&object).map_err(ConfigurationError::Object)?;
                    programmed.stored_len = programmed
                        .stored_len
                        .checked_add(1)
                        .ok_or(ConfigurationError::TooManyExpanded)?;
                    programmed.populated |= channel.banks().bits();
                }
                ObjectKind::ChannelBank => {
                    let bank = decode_channel_bank(&object).map_err(ConfigurationError::Object)?;
                    programmed.named |= bit(bank.id())?;
                }
                ObjectKind::RadioConfig => {
                    let config =
                        decode_radio_config(&object).map_err(ConfigurationError::Object)?;
                    programmed.config = config
                        .validate()
                        .map_err(|_| ConfigurationError::InvalidConfig)?;
                }
                ObjectKind::GeneratedBank => {
                    let plan =
                        decode_generated_bank(&object).map_err(ConfigurationError::Object)?;
                    programmed.populated |= bit(plan.id())?;
                    let slot = programmed
                        .expanded
                        .get_mut(usize::from(plan.id().get()))
                        .ok_or(ConfigurationError::BankOutOfRange)?;
                    // A plan owns its bank identifier, so a second plan for the
                    // same bank replaces rather than adds to it.
                    expanded_total -= u32::from(*slot);
                    *slot = plan.channel_count();
                    expanded_total += u32::from(plan.channel_count());
                }
            }
        }
        if expanded_total + u32::from(programmed.stored_len) > u32::from(u16::MAX) {
            return Err(ConfigurationError::TooManyExpanded);
        }
        Ok(programmed)
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
    /// index answers alone. A stored channel carries its own membership, which
    /// costs one decode — and there is one decode per explicit record, not per
    /// channel a plan expands to, which is what keeps a band-sized plan cheap
    /// to filter.
    #[must_use]
    pub fn member_at<'a, I>(&self, objects: I, index: u16, bank: BankId) -> bool
    where
        I: IntoIterator<Item = ObjectRef<'a>>,
    {
        match index.checked_sub(self.stored_len) {
            None => stored_at(objects, index)
                .and_then(|object| decode_channel(&object).ok())
                .is_some_and(|channel| channel.is_member_of(bank)),
            Some(offset) => self
                .plan_at(offset)
                .is_some_and(|(plan_bank, _)| plan_bank == bank),
        }
    }

    /// Expands or decodes the channel at one index from the stored objects.
    ///
    /// This is the only path which touches storage, and it is taken once per
    /// channel the operator actually sees or tunes rather than once per channel
    /// walked. Nothing it decodes is retained.
    #[must_use]
    pub fn channel_at<'a, I>(&self, objects: I, index: u16) -> Option<ChannelRecord>
    where
        I: IntoIterator<Item = ObjectRef<'a>>,
    {
        match index.checked_sub(self.stored_len) {
            None => decode_channel(&stored_at(objects, index)?).ok(),
            Some(offset) => {
                let (bank, inner) = self.plan_at(offset)?;
                let object = find(objects, ObjectKind::GeneratedBank, bank.get())?;
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
    /// alone still shows the operator what each filter selects.
    #[must_use]
    pub fn bank_name<'a, I>(&self, objects: I, bank: BankId) -> Option<BankName>
    where
        I: IntoIterator<Item = ObjectRef<'a>>,
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
                ObjectKind::ChannelBank => named = decode_channel_bank(&object).ok(),
                ObjectKind::GeneratedBank => plan = decode_generated_bank(&object).ok(),
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

/// Returns the explicit channel object at one selection index.
///
/// Objects are held in stable-key order, so the index-th channel object is the
/// index-th selectable stored channel.
fn stored_at<'a, I>(objects: I, index: u16) -> Option<ObjectRef<'a>>
where
    I: IntoIterator<Item = ObjectRef<'a>>,
{
    objects
        .into_iter()
        .filter(|object| object.key().kind == ObjectKind::Channel)
        .nth(usize::from(index))
}

/// Returns the stored object with one kind and identifier.
fn find<'a, I>(objects: I, kind: ObjectKind, id: u16) -> Option<ObjectRef<'a>>
where
    I: IntoIterator<Item = ObjectRef<'a>>,
{
    objects
        .into_iter()
        .find(|object| object.key().kind == kind && object.key().id == id)
}

#[cfg(test)]
mod tests {
    use super::{
        device_service, store_setting, ConfigurationError, Programmed, SettingChange,
        CONFIGURATION_STORE_BYTES,
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
        encode_channel, encode_channel_bank, encode_generated_bank, encode_radio_config, Object,
        ObjectArena, ObjectKey, ObjectKind, StorageObject,
    };

    /// The radio's own store, holding exactly what a test wrote to it.
    ///
    /// Tests index what the device would run rather than a slice standing in
    /// for it, so ordering and lookup are the store's rather than the test's.
    fn store(objects: &[StorageObject]) -> ObjectArena<CONFIGURATION_STORE_BYTES> {
        let mut arena = ObjectArena::new();
        for object in objects {
            arena.write(object).expect("store");
        }
        arena
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
        let programmed = Programmed::index(&store(&objects)).expect("programmed");
        assert_eq!(programmed.channel_count(), 2);
        assert_eq!(
            programmed
                .channel_at(&store(&objects), 0)
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
                .bank_name(&store(&objects), bank)
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
        let programmed = Programmed::index(&store(&objects)).expect("programmed");

        assert_eq!(programmed.channel_count(), 17);
        assert_eq!(programmed.stored_channels(), 1);
        assert_eq!(programmed.expanded_channels(), 16);
        assert_eq!(
            programmed
                .channel_at(&store(&objects), 0)
                .expect("stored")
                .id()
                .get(),
            1
        );
        assert_eq!(
            programmed
                .channel_at(&store(&objects), 1)
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
                .bank_name(&store(&objects), BankId::new(5))
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
        let programmed = Programmed::index(&store(&objects)).expect("programmed");
        assert_eq!(programmed.len(), 760);
        let last = programmed
            .channel_at(&store(&objects), 759)
            .expect("last channel");
        assert_eq!(last.active().receive.as_hz(), 136_975_000);
    }

    #[test]
    fn an_unprogrammed_snapshot_is_empty_and_conservative() {
        let programmed = Programmed::index(&store(&[])).expect("programmed");
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
        let programmed = Programmed::index(&arena).expect("programmed");
        assert_eq!(programmed.channel_count(), stored);
        assert_eq!(
            programmed
                .channel_at(&arena, stored - 1)
                .expect("last")
                .id()
                .get(),
            stored,
            "the last channel the bytes had room for is selectable"
        );
    }

    /// A level chosen on the handset has to become part of what is stored.
    #[test]
    fn storing_a_squelch_level_keeps_every_other_object_and_field() {
        let mut service = device_service();
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

        store_setting(
            &mut service,
            SettingChange::Squelch(SquelchLevel::new(8).expect("level")),
        )
        .expect("store");
        assert_ne!(service.generation(), before, "the change is a new snapshot");

        // A second setting changes its own field and leaves the first alone.
        store_setting(&mut service, SettingChange::ScanDwellMs(40)).expect("store");

        let programmed = Programmed::index(service.active_objects()).expect("programmed");
        assert_eq!(programmed.config().squelch, SquelchLevel::new(8).unwrap());
        assert_eq!(programmed.config().scan_dwell_ms, 40);
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
        let mut service = device_service();
        assert_eq!(service.active_objects().count(), 0);

        store_setting(
            &mut service,
            SettingChange::Squelch(SquelchLevel::new(6).expect("level")),
        )
        .expect("store");

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
            Programmed::index(&store(&[malformed])),
            Err(ConfigurationError::Object(_))
        ));
    }
}
