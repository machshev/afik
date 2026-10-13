//! Receive-only PMR446 examples for the K5 BK4819 validation image.

use radio_bk4819::{AfOutput, ReceiveSetup, SquelchThresholds};
use radio_domain::{Bandwidth, Frequency, Modulation, SquelchLevel, Tone};
use radio_platform::configuration::Pmr446Channel as SharedPmr446Channel;

/// Number of analogue PMR446 channels in the example raster.
pub use radio_platform::configuration::PMR446_CHANNELS;

/// One bounded one-based PMR446 channel number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pmr446Channel(u8);

impl Pmr446Channel {
    /// First PMR446 example channel.
    pub const FIRST: Self = Self(1);

    /// Validates a one-based channel number.
    pub const fn new(number: u8) -> Option<Self> {
        match SharedPmr446Channel::new(number) {
            Some(_) => Some(Self(number)),
            None => None,
        }
    }

    /// Returns the one-based channel number.
    pub const fn number(self) -> u8 {
        self.0
    }

    /// Returns the channel centre frequency.
    ///
    /// # Panics
    ///
    /// Cannot panic: every validated channel maps to a fixed non-zero `u32`
    /// frequency. The assertion keeps that invariant local to this module.
    #[must_use]
    pub fn frequency(self) -> Frequency {
        Frequency::from_hz(
            SharedPmr446Channel::new(self.0)
                .expect("validated PMR446 channel")
                .frequency_hz(),
        )
        .expect("PMR446 frequencies are non-zero")
    }

    /// Selects the following channel, wrapping after channel 16.
    #[must_use]
    pub const fn next(self) -> Self {
        if self.0 == PMR446_CHANNELS {
            Self::FIRST
        } else {
            Self(self.0 + 1)
        }
    }

    /// Selects the preceding channel, wrapping before channel 1.
    #[must_use]
    pub const fn previous(self) -> Self {
        if self.0 == 1 {
            Self(PMR446_CHANNELS)
        } else {
            Self(self.0 - 1)
        }
    }

    /// Builds the receive-only setup used by the validation image.
    #[must_use]
    pub fn setup(self, audio: bool) -> ReceiveSetup {
        self.setup_with_squelch(audio, SquelchLevel::CONSERVATIVE)
    }

    /// Builds the receive-only setup with the shared volatile squelch level.
    #[must_use]
    pub fn setup_with_squelch(self, audio: bool, squelch: SquelchLevel) -> ReceiveSetup {
        ReceiveSetup {
            frequency: self.frequency(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            tone: Tone::None,
            squelch: SquelchThresholds::for_level(squelch),
            af: if audio {
                AfOutput::Demodulated
            } else {
                AfOutput::Mute
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pmr446Channel, PMR446_CHANNELS};
    use radio_bk4819::{Bk4819, DriverState, RegisterAddress, RegisterBus};
    use radio_domain::SquelchLevel;

    #[derive(Default)]
    struct Bus {
        writes: std::vec::Vec<(u8, u16)>,
    }

    impl RegisterBus for Bus {
        type Error = core::convert::Infallible;

        fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
            self.writes.push((address.get(), value));
            Ok(())
        }

        fn read(&mut self, _: RegisterAddress) -> Result<u16, Self::Error> {
            Ok(0)
        }
    }

    #[test]
    fn the_sixteen_channel_raster_is_exact_and_wraps() {
        assert_eq!(Pmr446Channel::FIRST.frequency().as_hz(), 446_006_250);
        let last = Pmr446Channel::new(PMR446_CHANNELS).unwrap();
        assert_eq!(last.frequency().as_hz(), 446_193_750);
        assert_eq!(last.next(), Pmr446Channel::FIRST);
        assert_eq!(Pmr446Channel::FIRST.previous(), last);
        assert_eq!(Pmr446Channel::new(0), None);
        assert_eq!(Pmr446Channel::new(17), None);
    }

    #[test]
    fn pmr_receive_is_narrow_squelched_and_never_writes_the_tx_word() {
        let channel = Pmr446Channel::new(8).unwrap();
        let mut radio = Bk4819::new(Bus::default());
        radio.initialise().unwrap();
        radio.configure_receive(&channel.setup(true)).unwrap();

        assert_eq!(
            radio.state(),
            DriverState::Receiving {
                frequency: channel.frequency()
            }
        );
        assert!(radio.bus().writes.contains(&(0x43, 0x3648)));
        assert!(radio.bus().writes.contains(&(0x78, 0x5C5A)));
        assert!(radio
            .bus()
            .writes
            .iter()
            .any(|(address, value)| { *address == 0x47 && *value != 0 }));
        assert!(!radio.bus().writes.contains(&(0x30, 0x80FE)));
    }

    #[test]
    fn k5_receive_leaves_the_registers_armel_k5_never_writes() {
        let mut radio = Bk4819::new(Bus::default());
        radio.initialise().unwrap();
        radio
            .configure_receive(&Pmr446Channel::FIRST.setup(true))
            .unwrap();
        for register in [0x2A, 0x2B, 0x2F, 0x42, 0x54, 0x55] {
            assert!(
                !radio
                    .bus()
                    .writes
                    .iter()
                    .any(|(address, _)| *address == register),
                "REG_{register:02X} written"
            );
        }
    }

    #[test]
    fn operator_mute_keeps_the_same_squelch_but_disables_chip_audio() {
        let channel = Pmr446Channel::FIRST;
        let audible = channel.setup_with_squelch(true, SquelchLevel::new(5).unwrap());
        let muted = channel.setup_with_squelch(false, SquelchLevel::new(5).unwrap());
        assert_eq!(audible.squelch, muted.squelch);
        assert_eq!(muted.af, radio_bk4819::AfOutput::Mute);
        assert_eq!(audible.af, radio_bk4819::AfOutput::Demodulated);
    }

    #[test]
    fn selected_squelch_level_changes_the_threshold_registers() {
        let channel = Pmr446Channel::FIRST;
        let mut low = Bk4819::new(Bus::default());
        low.initialise().unwrap();
        low.configure_receive(&channel.setup_with_squelch(true, SquelchLevel::new(2).unwrap()))
            .unwrap();
        let low_threshold = low
            .bus()
            .writes
            .iter()
            .find(|(address, _)| *address == 0x78);

        let mut high = Bk4819::new(Bus::default());
        high.initialise().unwrap();
        high.configure_receive(&channel.setup_with_squelch(true, SquelchLevel::new(6).unwrap()))
            .unwrap();
        let high_threshold = high
            .bus()
            .writes
            .iter()
            .find(|(address, _)| *address == 0x78);

        assert!(low_threshold.is_some());
        assert!(high_threshold.is_some());
        assert_ne!(low_threshold, high_threshold);
    }
}
