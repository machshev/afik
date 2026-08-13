//! Hardware-independent configuration activation values.
//!
//! A target may retain a complete configuration in any storage and expose
//! arbitrary channels, VFO state, or scanning through its own adapter. Once a
//! selected channel is one of the common receive examples, the adapter hands
//! this small value to the shared application. It carries no storage, serial,
//! target, or persistence behavior.

/// Number of channels in the shared PMR446 example raster.
pub const PMR446_CHANNELS: u8 = 16;
const PMR446_FIRST_HZ: u32 = 446_006_250;
const PMR446_STEP_HZ: u32 = 12_500;

/// A validated one-based channel in the shared PMR446 example raster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pmr446Channel(u8);

impl Pmr446Channel {
    /// The first channel in the shared raster.
    pub const FIRST: Self = Self(1);

    /// Validates a one-based channel number.
    pub const fn new(number: u8) -> Option<Self> {
        if number == 0 || number > PMR446_CHANNELS {
            None
        } else {
            Some(Self(number))
        }
    }

    /// Returns the one-based channel number.
    pub const fn number(self) -> u8 {
        self.0
    }

    /// Returns the exact receive frequency in hertz.
    pub const fn frequency_hz(self) -> u32 {
        PMR446_FIRST_HZ + (self.0 as u32 - 1) * PMR446_STEP_HZ
    }

    /// Returns the next channel, wrapping after the last channel.
    #[must_use]
    pub const fn next(self) -> Self {
        if self.0 == PMR446_CHANNELS {
            Self::FIRST
        } else {
            Self(self.0 + 1)
        }
    }

    /// Returns the previous channel, wrapping before the first channel.
    #[must_use]
    pub const fn previous(self) -> Self {
        if self.0 == 1 {
            Self(PMR446_CHANNELS)
        } else {
            Self(self.0 - 1)
        }
    }

    /// Identifies a frequency in the shared raster, if it is exact.
    pub fn from_frequency_hz(frequency_hz: u32) -> Option<Self> {
        if frequency_hz < PMR446_FIRST_HZ {
            return None;
        }
        let offset = frequency_hz - PMR446_FIRST_HZ;
        if offset % PMR446_STEP_HZ != 0 {
            return None;
        }
        Self::new(u8::try_from(offset / PMR446_STEP_HZ + 1).ok()?)
    }
}

/// A validated shared receive configuration selected by a target adapter.
///
/// `generation` is an opaque host/storage generation. K5 can use zero while
/// it has no persistence, whereas K1 supplies its retained object generation.
/// The shared application never writes or interprets persistence; it only
/// carries the identity so an adapter cannot accidentally apply a stale
/// activation after replacing its active snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivatedConfiguration {
    generation: u32,
    channel: Pmr446Channel,
    audio: bool,
}

impl ActivatedConfiguration {
    /// Constructs an activation from a validated common channel.
    pub const fn new(generation: u32, channel: Pmr446Channel, audio: bool) -> Self {
        Self {
            generation,
            channel,
            audio,
        }
    }

    /// Constructs an activation from a one-based common channel number.
    pub const fn from_channel(generation: u32, channel: u8, audio: bool) -> Option<Self> {
        match Pmr446Channel::new(channel) {
            Some(channel) => Some(Self::new(generation, channel, audio)),
            None => None,
        }
    }

    /// Returns the opaque active configuration generation.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Returns the selected common channel.
    pub const fn channel(self) -> Pmr446Channel {
        self.channel
    }

    /// Returns the selected one-based channel number.
    pub const fn channel_number(self) -> u8 {
        self.channel.number()
    }

    /// Returns the exact receive frequency in hertz.
    pub const fn frequency_hz(self) -> u32 {
        self.channel.frequency_hz()
    }

    /// Returns the operator's chip-audio preference.
    pub const fn audio(self) -> bool {
        self.audio
    }

    /// Returns a copy selecting another common channel in the same snapshot.
    #[must_use]
    pub const fn select(self, channel: Pmr446Channel) -> Self {
        Self { channel, ..self }
    }

    /// Returns a copy carrying a changed operator audio preference.
    #[must_use]
    pub const fn with_audio(self, audio: bool) -> Self {
        Self { audio, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivatedConfiguration, Pmr446Channel, PMR446_CHANNELS};

    #[test]
    fn common_channel_identity_is_exact_and_bounded() {
        assert_eq!(Pmr446Channel::FIRST.frequency_hz(), 446_006_250);
        let last = Pmr446Channel::new(PMR446_CHANNELS).expect("last");
        assert_eq!(last.frequency_hz(), 446_193_750);
        assert_eq!(last.next(), Pmr446Channel::FIRST);
        assert_eq!(Pmr446Channel::FIRST.previous(), last);
        assert_eq!(
            Pmr446Channel::from_frequency_hz(446_093_750),
            Pmr446Channel::new(8)
        );
        assert_eq!(Pmr446Channel::from_frequency_hz(446_093_751), None);
        assert_eq!(Pmr446Channel::new(0), None);
        assert_eq!(Pmr446Channel::new(PMR446_CHANNELS + 1), None);
    }

    #[test]
    fn k1_and_k5_activation_fixtures_share_effect_inputs_without_sharing_storage() {
        // K1 supplies its retained generation; K5 uses zero because this
        // boundary deliberately contains no persistence implementation.
        let k1 = ActivatedConfiguration::from_channel(7, 8, true).expect("K1");
        let k5 = ActivatedConfiguration::from_channel(0, 8, true).expect("K5");
        assert_ne!(k1.generation(), k5.generation());
        assert_eq!(k1.frequency_hz(), k5.frequency_hz());
        assert_eq!(k1.channel_number(), k5.channel_number());
        assert_eq!(k1.audio(), k5.audio());
        assert_eq!(k1.channel().next(), k5.channel().next());
    }

    #[test]
    fn replacing_snapshot_keeps_generation_explicit() {
        let old = ActivatedConfiguration::from_channel(4, 1, true).expect("old");
        let selected = old.select(Pmr446Channel::new(3).expect("channel"));
        assert_eq!(selected.generation(), 4);
        let replaced = ActivatedConfiguration::from_channel(5, 3, true).expect("new");
        assert_ne!(selected.generation(), replaced.generation());
        assert_eq!(selected.channel(), replaced.channel());
    }
}
