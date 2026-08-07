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

use radio_channel_control::{ChannelMemory, ChannelSource};
use radio_channel_plan::{ChannelBank, MAX_BANKS as PLAN_MAX_BANKS};
use radio_device::{DeviceService, KindLimits};
use radio_domain::{BankId, RadioConfig};
use radio_storage::{
    decode_channel, decode_channel_bank, decode_radio_config, ObjectKind, StorageError,
    StorageObject, CHANNEL_BANK_ENCODED_LEN, CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN,
    CONFIGURATION_IMAGE_OBJECT_HEADER_LEN, RADIO_CONFIG_ENCODED_LEN,
};

/// Channels this image stores and selects.
///
/// The bound is set by RAM, not by taste: the store holds an active and a
/// candidate copy of every object, the interface holds the decoded channels,
/// and all of it has to fit the evidenced 16 KiB of SRAM beside the executor,
/// the framebuffer, and the retained-image buffer.
pub const MAX_CHANNELS: usize = 12;

/// Named banks this image stores.
pub const MAX_BANKS: usize = PLAN_MAX_BANKS as usize;

/// Configuration objects the device advertises and accepts.
///
/// The bound is the sum of what this image can use: every channel, every bank,
/// and the singleton radio configuration.
pub const MAX_OBJECTS: usize = MAX_CHANNELS + MAX_BANKS + 1;

/// Bytes reserved for the retained canonical configuration image.
///
/// This is a whole number of flash write pages so a retained image can be
/// written without a read-modify-write cycle.
pub const RETAINED_IMAGE_BYTES: usize = 1_280;

/// Largest canonical image a full configuration can produce.
pub const MAX_CONFIGURATION_IMAGE_BYTES: usize = CONFIGURATION_IMAGE_HEADER_LEN
    + MAX_CHANNELS * (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + CHANNEL_ENCODED_LEN)
    + MAX_BANKS * (CONFIGURATION_IMAGE_OBJECT_HEADER_LEN + CHANNEL_BANK_ENCODED_LEN)
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
        // This image selects explicit channel records, so it activates no
        // compact generated plan.
        generated_banks: 0,
        channels: u16::try_from(MAX_CHANNELS).unwrap_or(u16::MAX),
        channel_banks: u16::try_from(MAX_BANKS).unwrap_or(u16::MAX),
        radio_configs: 1,
    }
}

/// Constructs the configuration service this image exposes over serial.
#[must_use]
pub fn device_service() -> DeviceService<MAX_OBJECTS> {
    DeviceService::with_limits(0, kind_limits())
}

/// Why an active object snapshot cannot become a programmed configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    /// An object payload failed its own decoder.
    Object(StorageError),
    /// More channels were stored than this image can select.
    TooManyChannels,
    /// A bank identifier was outside the addressable range.
    BankOutOfRange,
    /// The programmed global configuration failed revalidation.
    InvalidConfig,
}

/// One complete programmed configuration ready to drive the receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Programmed {
    memory: ChannelMemory<MAX_CHANNELS>,
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
            memory: ChannelMemory::new(),
            banks: [None; MAX_BANKS],
            config: RadioConfig::conservative(),
        }
    }

    /// Builds a configuration from one active object snapshot.
    ///
    /// Generated banks are accepted by storage but are not expanded here: this
    /// image selects explicit channel records only, so a radio programmed with
    /// a compact plan reports no channels rather than inventing names for them.
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
                ObjectKind::GeneratedBank => {}
            }
        }
        Ok(programmed)
    }

    /// Returns the ordered channel store.
    #[must_use]
    pub const fn memory(&self) -> ChannelMemory<MAX_CHANNELS> {
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

    /// Returns the banks at least one programmed channel belongs to.
    ///
    /// The result is ordered by bank identifier and contains no bank without
    /// members, so a bank filter can never select an empty view.
    #[must_use]
    pub fn populated_banks(&self) -> ([Option<BankId>; MAX_BANKS], usize) {
        let mut banks = [None; MAX_BANKS];
        let mut count = 0;
        for raw in 0..u16::try_from(MAX_BANKS).unwrap_or(u16::MAX) {
            let bank = BankId::new(raw);
            let populated = (0..self.memory.len())
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
    use super::{ConfigurationError, Programmed, MAX_CHANNELS, MAX_OBJECTS};
    use radio_channel_control::ChannelSource;
    use radio_channel_plan::{
        BankFlags, BankMask, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, SquelchLevel, Tone, TxClass,
    };
    use radio_storage::{
        encode_channel, encode_channel_bank, encode_radio_config, ObjectKey, ObjectKind,
        StorageObject,
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
