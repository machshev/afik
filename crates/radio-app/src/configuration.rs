//! The programmed configuration this radio runs.
//!
//! The host compiles a project into a bank-grouped image (storage format 7,
//! `radio-storage::radio_image`) and writes it into the radio's memory
//! (`ADR-100`). The radio validates it there and keeps only its
//! [`ImageIndex`] — six bytes per bank — in RAM; a channel is read
//! from memory when it is shown or tuned. [`Programmed`] is the small summary
//! the tasks pass around.
//!
//! Nothing here mints transmit authority: every transmission still goes
//! through the TX policy.

use radio_channel_plan::BankSet;
use radio_domain::{RadioConfig, SquelchLevel};
use radio_platform::memory_link::IMAGE_REGION_BYTES;
use radio_storage::radio_image::HEADER_LEN;

pub use radio_storage::radio_image::Index as ImageIndex;

/// Bytes of image payload the memory view holds after the image header.
pub const CONFIGURATION_STORE_BYTES: usize = IMAGE_REGION_BYTES as usize - HEADER_LEN;

/// What the tasks need to know about the running configuration.
///
/// Channel lookups go through the shared [`ImageIndex`]; this carries the
/// count, the populated banks and the global configuration, small enough to
/// copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Programmed {
    len: u16,
    populated: BankSet,
    config: RadioConfig,
}

impl Default for Programmed {
    fn default() -> Self {
        Self::empty()
    }
}

impl Programmed {
    /// No channels and the conservative defaults.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            len: 0,
            populated: BankSet::EMPTY,
            config: RadioConfig::conservative(),
        }
    }

    /// Summarises one validated image index.
    #[must_use]
    pub const fn of(index: &ImageIndex) -> Self {
        Self {
            len: index.len(),
            populated: index.populated(),
            config: index.config(),
        }
    }

    /// A summary of `len` channels stored in the banks `populated` sets,
    /// bank `n` as bit `n`.
    #[cfg(test)]
    pub(crate) fn with_channels(len: u16, populated: u16) -> Self {
        let mut banks = BankSet::EMPTY;
        for bank in (0..16).filter(|bank| populated & (1 << bank) != 0) {
            banks.insert(radio_domain::BankId::new(bank));
        }
        Self {
            len,
            populated: banks,
            config: RadioConfig::conservative(),
        }
    }

    /// Returns this summary with a different programmed TX power and timeout.
    #[cfg(test)]
    pub(crate) const fn with_tx(mut self, power: u8, timeout: u8) -> Self {
        self.config.tx_power = power;
        self.config.tx_timeout = timeout;
        self
    }

    /// Returns this summary with dual watch programmed on or off.
    #[cfg(test)]
    pub(crate) const fn with_dual_watch(mut self, on: bool) -> Self {
        self.config.dual_watch = on;
        self
    }

    /// Returns this summary with the radio-wide squelch replaced.
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
    pub const fn len(&self) -> u16 {
        self.len
    }

    /// Returns the number of programmed channels.
    #[must_use]
    pub const fn channel_count(&self) -> u16 {
        self.len
    }

    /// Reports whether any channel was programmed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the banks at least one channel is stored in, so a bank
    /// filter can never select an empty view.
    #[must_use]
    pub const fn populated_banks(&self) -> BankSet {
        self.populated
    }
}

#[cfg(test)]
mod tests {
    use super::{ImageIndex, Programmed, CONFIGURATION_STORE_BYTES};
    use radio_channel_plan::BankSet;
    use radio_domain::{BankId, RadioConfig, SquelchLevel};

    #[test]
    fn an_empty_index_summarises_as_an_unprogrammed_radio() {
        let programmed = Programmed::of(&ImageIndex::empty());
        assert!(programmed.is_empty());
        assert_eq!(programmed, Programmed::empty());
        assert_eq!(programmed.config(), RadioConfig::conservative());
        assert!(programmed.populated_banks().is_empty());
    }

    #[test]
    fn the_handset_squelch_replaces_the_image_default() {
        let level = SquelchLevel::new(7).expect("level");
        assert_eq!(
            Programmed::empty().with_squelch(level).config().squelch,
            level
        );
    }

    /// A radio programmed with more banks than a 16-bit mask holds lists
    /// every one of them in its source menu.
    #[test]
    fn an_image_of_forty_banks_lists_forty_banks() {
        use radio_channel_plan::{ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord};
        use radio_domain::{
            Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, Tone,
        };
        use radio_platform::operator_shell::{Shell, Source};
        use radio_storage::radio_image::{compile, HEADER_LEN};

        let channels: std::vec::Vec<ChannelRecord> = (0..40_u16)
            .map(|bank| {
                let hz = 430_000_000 + u32::from(bank) * 100_000;
                ChannelRecord::new(ChannelDefinition {
                    id: ChannelId::new(bank + 1),
                    name: ChannelName::new("CH").unwrap(),
                    receive: Frequency::from_hz(hz).unwrap(),
                    transmit: Frequency::from_hz(hz).unwrap(),
                    rx_tone: Tone::None,
                    tx_tone: Tone::None,
                    modulation: Modulation::Fm,
                    bandwidth: Bandwidth::Narrow,
                    power: PowerLevel::Low1,
                    step: FrequencyStep::from_hz(12_500).unwrap(),
                    flags: ChannelFlags::default(),
                    bank: Some(BankId::new(bank * 6)),
                })
                .unwrap()
            })
            .collect();
        let mut image = [0_u8; 4096];
        let len = compile(
            RadioConfig::conservative(),
            &[],
            &[],
            &channels,
            &[],
            &mut image,
        )
        .expect("compiles");
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&image[..HEADER_LEN]);
        let payload = &image[HEADER_LEN..len];
        let index = ImageIndex::build(&header, |offset, buffer| {
            payload
                .get(offset..offset + buffer.len())
                .is_some_and(|bytes| {
                    buffer.copy_from_slice(bytes);
                    true
                })
        })
        .expect("the radio accepts it");

        let programmed = Programmed::of(&index);
        assert_eq!(programmed.populated_banks().len(), 40);
        let mut shell = Shell::new();
        shell.set_banks(programmed.populated_banks());
        // The VFO, every channel, then each bank.
        assert_eq!(shell.source_rows(), 42);
        assert_eq!(shell.source_at(41), Some(Source::Bank(BankId::new(234))));
    }

    #[test]
    fn populated_banks_are_listed_in_order() {
        let programmed = Programmed {
            len: 3,
            populated: BankSet::EMPTY
                .with(BankId::new(200))
                .with(BankId::new(0))
                .with(BankId::new(7)),
            config: RadioConfig::conservative(),
        };
        let banks = programmed.populated_banks();
        assert_eq!(banks.nth(0), Some(BankId::new(0)));
        assert_eq!(banks.nth(1), Some(BankId::new(7)));
        assert_eq!(banks.nth(2), Some(BankId::new(200)));
        assert_eq!(banks.len(), 3);
        assert_eq!(CONFIGURATION_STORE_BYTES, 0x1D00 - 16);
    }
}
