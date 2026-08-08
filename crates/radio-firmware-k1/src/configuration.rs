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

use radio_channel_control::{ChannelSource, ProgrammedMemory};
use radio_channel_plan::{
    BankName, ChannelBank, GeneratedBank, PlanEncoding, MAX_BANKS as PLAN_MAX_BANKS,
};
use radio_device::{DeviceService, KindLimits};
use radio_domain::{BankId, RadioConfig, SquelchLevel};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_generated_bank, decode_radio_config,
    encode_radio_config, ObjectKind, StorageError, StorageObject, CHANNEL_BANK_ENCODED_LEN,
    CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_OBJECT_HEADER_LEN,
    GENERATED_BANK_ENCODED_LEN, RADIO_CONFIG_ENCODED_LEN,
};

/// Channels this image stores and selects.
///
/// The bound is set by RAM, not by taste: the store holds an active and a
/// candidate copy of every object, the interface holds the decoded channels,
/// and all of it has to fit the evidenced 16 KiB of SRAM beside the executor,
/// the framebuffer, and the retained-image buffer.
pub const MAX_CHANNELS: usize = 12;

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
pub const MAX_GENERATED_BANKS: usize = 2;

/// Channels this image will expand from stored plans.
///
/// Expansion is arithmetic and holds no memory, but selection, the channel
/// list, and scanning all walk the whole space, so the operator interface stays
/// responsive only while that space is bounded.
pub const MAX_EXPANDED_CHANNELS: u16 = 128;

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
    // This image expands linear simplex plans itself, so it advertises that
    // encoding and the host may compile one for it. The stored-configuration
    // bound is the external-memory region the image claimed, so a host can say
    // how much room a project leaves before writing it.
    DeviceService::with_configuration_capacity(
        PlanEncoding::LinearSimplex.capability_bit(),
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
    /// Stored plans expanded to more channels than this image can select.
    TooManyExpandedChannels,
    /// More generated plans were stored than this image can expand.
    TooManyPlans,
    /// A bank identifier was outside the addressable range.
    BankOutOfRange,
    /// The programmed global configuration failed revalidation.
    InvalidConfig,
}

/// One complete programmed configuration ready to drive the receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Programmed {
    memory: ProgrammedMemory<MAX_CHANNELS, MAX_GENERATED_BANKS>,
    banks: [Option<ChannelBank>; MAX_BANKS],
    config: RadioConfig,
}

impl Default for Programmed {
    fn default() -> Self {
        Self::empty()
    }
}

impl Programmed {
    /// Returns an empty configuration carrying the conservative defaults.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            memory: ProgrammedMemory::new(),
            banks: [None; MAX_BANKS],
            config: RadioConfig::conservative(),
        }
    }

    /// Builds a configuration from one active object snapshot.
    ///
    /// Both channel kinds land in one selection space: an explicit record costs
    /// one stored object, and a generated plan costs one stored object for
    /// every channel it expands to. Nothing is expanded eagerly; only the
    /// resulting channel count is checked against what this image can select.
    pub fn from_objects<'a, I>(objects: I) -> Result<Self, ConfigurationError>
    where
        I: IntoIterator<Item = &'a StorageObject>,
    {
        let mut programmed = Self::empty();
        for object in objects {
            match object.key().kind {
                ObjectKind::Channel => {
                    let channel = decode_channel(object).map_err(ConfigurationError::Object)?;
                    programmed
                        .memory
                        .insert(channel)
                        .map_err(|_| ConfigurationError::TooManyChannels)?;
                }
                ObjectKind::ChannelBank => {
                    let bank = decode_channel_bank(object).map_err(ConfigurationError::Object)?;
                    let slot = programmed
                        .banks
                        .get_mut(usize::from(bank.id().get()))
                        .ok_or(ConfigurationError::BankOutOfRange)?;
                    *slot = Some(bank);
                }
                ObjectKind::RadioConfig => {
                    let config = decode_radio_config(object).map_err(ConfigurationError::Object)?;
                    programmed.config = config
                        .validate()
                        .map_err(|_| ConfigurationError::InvalidConfig)?;
                }
                ObjectKind::GeneratedBank => {
                    let plan = decode_generated_bank(object).map_err(ConfigurationError::Object)?;
                    programmed
                        .memory
                        .install(plan)
                        .map_err(|_| ConfigurationError::TooManyPlans)?;
                }
            }
        }
        if programmed.memory.expanded_len() > MAX_EXPANDED_CHANNELS {
            return Err(ConfigurationError::TooManyExpandedChannels);
        }
        Ok(programmed)
    }

    /// Returns the ordered channel store, stored channels and plans together.
    #[must_use]
    pub const fn memory(&self) -> ProgrammedMemory<MAX_CHANNELS, MAX_GENERATED_BANKS> {
        self.memory
    }

    /// Returns the global receive configuration.
    #[must_use]
    pub const fn config(&self) -> RadioConfig {
        self.config
    }

    /// Returns the number of programmed channels.
    #[must_use]
    pub fn channel_count(&self) -> u16 {
        self.memory.len()
    }

    /// Reports whether any channel was programmed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.memory.is_empty()
    }

    /// Returns the programmed bank, if the host named it.
    #[must_use]
    pub fn bank(&self, bank: BankId) -> Option<ChannelBank> {
        self.banks.get(usize::from(bank.get())).copied().flatten()
    }

    /// Returns the name to show for one bank.
    ///
    /// A generated plan names its own bank, so a radio programmed with plans
    /// alone still shows the operator what each filter selects.
    #[must_use]
    pub fn bank_name(&self, bank: BankId) -> Option<BankName> {
        self.bank(bank)
            .map(ChannelBank::name)
            .or_else(|| self.memory.plan(bank).map(GeneratedBank::name))
    }

    /// Returns the banks at least one selectable channel belongs to.
    ///
    /// The result is ordered by bank identifier and contains no bank without
    /// members, so a bank filter can never select an empty view. A generated
    /// plan populates its own bank, because every channel it expands to is a
    /// member of it.
    #[must_use]
    pub fn populated_banks(&self) -> ([Option<BankId>; MAX_BANKS], usize) {
        let mut banks = [None; MAX_BANKS];
        let mut count = 0;
        for raw in 0..u16::try_from(MAX_BANKS).unwrap_or(u16::MAX) {
            let bank = BankId::new(raw);
            let populated = self.memory.plan(bank).is_some()
                || (0..self.memory.stored_len())
                    .filter_map(|index| self.memory.get(index))
                    .any(|channel| channel.banks().contains(bank));
            if populated {
                banks[count] = Some(bank);
                count += 1;
            }
        }
        (banks, count)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        device_service, store_squelch, ConfigurationError, Programmed, MAX_CHANNELS,
        MAX_EXPANDED_CHANNELS, MAX_OBJECTS,
    };
    use radio_channel_control::ChannelSource;
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
        let programmed = Programmed::from_objects(objects.iter()).expect("programmed");
        assert_eq!(programmed.channel_count(), 2);
        assert_eq!(
            programmed.memory().get(0).expect("first").id().get(),
            3,
            "channels are ordered by stable identifier, not write order"
        );
        assert_eq!(programmed.config().backlight_seconds, 30);
        assert_eq!(programmed.bank(bank).expect("bank").name().as_str(), "UHF");
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
        let programmed = Programmed::from_objects(objects.iter()).expect("programmed");

        assert_eq!(programmed.channel_count(), 17);
        assert_eq!(programmed.memory().stored_len(), 1);
        assert_eq!(programmed.memory().expanded_len(), 16);
        assert_eq!(programmed.memory().get(0).expect("stored").id().get(), 1);
        assert_eq!(
            programmed
                .memory()
                .get(1)
                .expect("expanded")
                .name()
                .as_str(),
            "PMR446 01"
        );

        // The plan names and populates its own bank without a named-bank object.
        let (banks, count) = programmed.populated_banks();
        assert_eq!(count, 1);
        assert_eq!(banks[0], Some(BankId::new(5)));
        assert_eq!(
            programmed
                .bank_name(BankId::new(5))
                .expect("plan name")
                .as_str(),
            "PMR446"
        );
        assert_eq!(programmed.bank(BankId::new(5)), None);
    }

    #[test]
    fn plans_expanding_past_what_the_interface_selects_are_refused() {
        let objects = [encode_generated_bank(
            GeneratedBank::linear_simplex(
                BankId::new(0),
                BankName::new("BIG").expect("name"),
                Frequency::from_hz(400_000_000).expect("frequency"),
                FrequencyStep::from_hz(12_500).expect("step"),
                MAX_EXPANDED_CHANNELS + 1,
                TxClass::Never,
            )
            .expect("plan"),
        )
        .expect("plan object")];
        assert_eq!(
            Programmed::from_objects(objects.iter()),
            Err(ConfigurationError::TooManyExpandedChannels)
        );
    }

    #[test]
    fn an_unprogrammed_snapshot_is_empty_and_conservative() {
        let programmed = Programmed::from_objects([].iter()).expect("programmed");
        assert!(programmed.is_empty());
        assert_eq!(programmed.config(), RadioConfig::conservative());
        assert_eq!(programmed.populated_banks().1, 0);
        assert_eq!(programmed.bank(BankId::new(0)), None);
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
            Programmed::from_objects(objects.iter()),
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

        let programmed = Programmed::from_objects(service.active_objects()).expect("programmed");
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

        let programmed = Programmed::from_objects(service.active_objects()).expect("programmed");
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
            Programmed::from_objects([malformed].iter()),
            Err(ConfigurationError::Object(_))
        ));
    }
}
