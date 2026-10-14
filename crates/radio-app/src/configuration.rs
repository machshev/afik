//! The programmed configuration this radio runs.
//!
//! The host compiles a project into a bank-grouped image (storage format 5,
//! `radio-storage::radio_image`) and writes it into the radio's memory
//! (`ADR-100`). The radio validates it there and keeps only its
//! [`ImageIndex`] — a few bytes per bank block — in RAM; a channel is read
//! from memory when it is shown or tuned. [`Programmed`] is the small summary
//! the tasks pass around.
//!
//! Nothing here mints transmit authority. Channels keep the transmit class the
//! host programmed, and every transmission still goes through the TX policy.

use radio_channel_plan::MAX_BANKS;
use radio_domain::{BankId, RadioConfig, SquelchLevel};
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
    populated: u16,
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
            populated: 0,
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

    /// A summary of `len` channels stored in the banks `populated` sets.
    #[cfg(test)]
    pub(crate) const fn with_channels(len: u16, populated: u16) -> Self {
        Self {
            len,
            populated,
            config: RadioConfig::conservative(),
        }
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

    /// Returns the banks at least one channel is stored in, in identifier
    /// order, so a bank filter can never select an empty view.
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

#[cfg(test)]
mod tests {
    use super::{ImageIndex, Programmed, CONFIGURATION_STORE_BYTES};
    use radio_domain::{BankId, RadioConfig, SquelchLevel};

    #[test]
    fn an_empty_index_summarises_as_an_unprogrammed_radio() {
        let programmed = Programmed::of(&ImageIndex::empty());
        assert!(programmed.is_empty());
        assert_eq!(programmed, Programmed::empty());
        assert_eq!(programmed.config(), RadioConfig::conservative());
        assert_eq!(programmed.populated_banks().1, 0);
    }

    #[test]
    fn the_handset_squelch_replaces_the_image_default() {
        let level = SquelchLevel::new(7).expect("level");
        assert_eq!(
            Programmed::empty().with_squelch(level).config().squelch,
            level
        );
    }

    #[test]
    fn populated_banks_are_listed_in_order() {
        let programmed = Programmed {
            len: 3,
            populated: 0b1010_0001,
            config: RadioConfig::conservative(),
        };
        let (banks, count) = programmed.populated_banks();
        assert_eq!(
            &banks[..count],
            &[
                Some(BankId::new(0)),
                Some(BankId::new(5)),
                Some(BankId::new(7))
            ]
        );
        assert_eq!(CONFIGURATION_STORE_BYTES, 0x1D00 - 16);
    }
}
