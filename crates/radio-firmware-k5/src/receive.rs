//! Receive-only PMR446 examples for the K5 BK4819 validation image.

use radio_bk4819::{AfOutput, ReceiveSetup, SquelchThresholds};
use radio_domain::{Bandwidth, Frequency, Modulation, Tone};

/// Number of analogue PMR446 channels in the example raster.
pub const PMR446_CHANNELS: u8 = 16;
const PMR446_FIRST_HZ: u32 = 446_006_250;
const PMR446_STEP_HZ: u32 = 12_500;

/// One bounded one-based PMR446 channel number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pmr446Channel(u8);

impl Pmr446Channel {
    /// First PMR446 example channel.
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

    /// Returns the channel centre frequency.
    ///
    /// # Panics
    ///
    /// Cannot panic: every validated channel maps to a fixed non-zero `u32`
    /// frequency. The assertion keeps that invariant local to this module.
    #[must_use]
    pub fn frequency(self) -> Frequency {
        let offset = u32::from(self.0 - 1) * PMR446_STEP_HZ;
        Frequency::from_hz(PMR446_FIRST_HZ + offset).expect("PMR446 frequencies are non-zero")
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
    pub fn setup(self) -> ReceiveSetup {
        ReceiveSetup {
            frequency: self.frequency(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            tone: Tone::None,
            squelch: SquelchThresholds::squelch_off(),
            af: AfOutput::Mute,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pmr446Channel, PMR446_CHANNELS};
    use radio_bk4819::{Bk4819, DriverState, RegisterAddress, RegisterBus};

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
    fn pmr_receive_is_narrow_muted_and_never_writes_the_tx_word() {
        let channel = Pmr446Channel::new(8).unwrap();
        let mut radio = Bk4819::new(Bus::default());
        radio.initialise().unwrap();
        radio.configure_receive(&channel.setup()).unwrap();

        assert_eq!(
            radio.state(),
            DriverState::Receiving {
                frequency: channel.frequency()
            }
        );
        assert!(radio.bus().writes.contains(&(0x43, 0x3648)));
        assert!(!radio.bus().writes.contains(&(0x30, 0x80FE)));
    }
}
