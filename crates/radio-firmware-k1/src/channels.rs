//! The bounded channel set this witness image ships with.
//!
//! AFIK does not yet read channels from the radio, so the image carries a small
//! receive-only set. Every entry is classified `TxClass::Never`: the image has
//! no transmit path, and nothing here may be mistaken for transmit permission.

use radio_channel_plan::{
    BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord, PlanError,
};
use radio_domain::{
    Bandwidth, ChannelId, DomainError, Frequency, FrequencyStep, Modulation, PowerLevel,
    SquelchLevel, Tone, TxClass,
};

/// Number of channels the built-in set defines.
pub const BUILT_IN_CHANNELS: usize = 5;

/// One built-in entry before validation.
struct Entry {
    name: &'static str,
    receive_hz: u32,
    step_hz: u32,
    bandwidth: Bandwidth,
}

const ENTRIES: [Entry; BUILT_IN_CHANNELS] = [
    Entry {
        name: "2M CALL",
        receive_hz: 145_500_000,
        step_hz: 12_500,
        bandwidth: Bandwidth::Narrow,
    },
    Entry {
        name: "2M FM",
        receive_hz: 145_725_000,
        step_hz: 12_500,
        bandwidth: Bandwidth::Narrow,
    },
    Entry {
        name: "70CM",
        receive_hz: 433_500_000,
        step_hz: 12_500,
        bandwidth: Bandwidth::Narrow,
    },
    Entry {
        name: "PMR 1",
        receive_hz: 446_006_250,
        step_hz: 12_500,
        bandwidth: Bandwidth::Narrow,
    },
    Entry {
        name: "PMR 8",
        receive_hz: 446_093_750,
        step_hz: 12_500,
        bandwidth: Bandwidth::Narrow,
    },
];

/// A built-in channel could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltInError {
    /// A frequency or step was outside its domain.
    Domain(DomainError),
    /// A name or channel definition was rejected.
    Plan(PlanError),
}

/// Builds one built-in channel by index.
pub fn built_in(index: usize) -> Result<ChannelRecord, BuiltInError> {
    let entry = ENTRIES
        .get(index)
        .ok_or(BuiltInError::Plan(PlanError::ChannelOutOfRange))?;
    let receive = Frequency::from_hz(entry.receive_hz).map_err(BuiltInError::Domain)?;
    let step = FrequencyStep::from_hz(entry.step_hz).map_err(BuiltInError::Domain)?;
    let name = ChannelName::new(entry.name).map_err(BuiltInError::Plan)?;
    let identifier = u16::try_from(index).unwrap_or(u16::MAX);
    ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(identifier + 1),
        name,
        receive,
        // Receive-only: the transmit frequency mirrors receive and the class
        // denies transmission outright.
        transmit: receive,
        rx_tone: Tone::None,
        tx_tone: Tone::None,
        modulation: Modulation::Fm,
        bandwidth: entry.bandwidth,
        power: PowerLevel::Low,
        step,
        squelch: SquelchLevel::new(3).map_err(BuiltInError::Domain)?,
        flags: ChannelFlags::default(),
        banks: BankMask::default(),
        tx_class: TxClass::Never,
    })
    .map_err(BuiltInError::Plan)
}

#[cfg(test)]
mod tests {
    use super::{built_in, BUILT_IN_CHANNELS};
    use radio_domain::TxClass;

    #[test]
    fn every_built_in_channel_is_valid_and_denies_transmission() {
        for index in 0..BUILT_IN_CHANNELS {
            let channel = built_in(index).expect("built-in channel");
            assert_eq!(channel.tx_class(), TxClass::Never);
            assert_eq!(channel.active().receive, channel.active().transmit);
            assert!(!channel.name().as_str().is_empty());
        }
        assert!(built_in(BUILT_IN_CHANNELS).is_err());
    }

    #[test]
    fn identifiers_are_stable_and_ordered() {
        let first = built_in(0).unwrap();
        let last = built_in(BUILT_IN_CHANNELS - 1).unwrap();
        assert_eq!(first.id().get(), 1);
        assert_eq!(last.id().get(), u16::try_from(BUILT_IN_CHANNELS).unwrap());
    }
}
