//! Bounded configuration-change hand-off for the shared receive application.
//!
//! A target owns how a candidate is obtained and, on K1, how it is validated
//! and retained.  The shared application owns only the final hand-off: it
//! asks an adapter to tune a bounded candidate, then commits that candidate
//! only after the adapter acknowledges success.  This keeps a failed tune
//! from becoming a semantic configuration change and does not imply that a
//! target has persistent configuration storage.

use super::configuration::{ActivatedConfiguration, Pmr446Channel};

/// A bounded candidate which an adapter may ask the shared app to activate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationCandidate {
    configuration: ActivatedConfiguration,
}

impl ConfigurationCandidate {
    /// Builds a candidate from a validated common channel.
    pub const fn new(configuration: ActivatedConfiguration) -> Self {
        Self { configuration }
    }

    /// Builds a candidate from a one-based common channel number.
    pub const fn from_channel(generation: u32, channel: u8, audio: bool) -> Option<Self> {
        match ActivatedConfiguration::from_channel(generation, channel, audio) {
            Some(configuration) => Some(Self::new(configuration)),
            None => None,
        }
    }

    /// Returns the validated activation carried by this candidate.
    pub const fn configuration(self) -> ActivatedConfiguration {
        self.configuration
    }

    /// Returns the opaque target/storage generation.
    pub const fn generation(self) -> u32 {
        self.configuration.generation()
    }

    /// Returns the selected common channel.
    pub const fn channel(self) -> Pmr446Channel {
        self.configuration.channel()
    }

    /// Returns the selected one-based channel number.
    pub const fn channel_number(self) -> u8 {
        self.configuration.channel_number()
    }

    /// Returns the operator's chip-audio preference.
    pub const fn audio(self) -> bool {
        self.configuration.audio()
    }
}

/// One bounded attempt to apply a candidate.
///
/// The identifier is deliberately separate from the candidate.  A target can
/// receive the same channel/audio value twice, and an acknowledgement from the
/// first attempt must not be accepted for the second attempt.  It is an
/// application-local token, not a storage generation or a serial sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationChange {
    id: u32,
    candidate: ConfigurationCandidate,
}

impl ConfigurationChange {
    /// Creates a change with an application-local identity.
    pub const fn new(id: u32, candidate: ConfigurationCandidate) -> Self {
        Self { id, candidate }
    }

    /// Creates a change with a bounded application-instance identity.
    ///
    /// The high half identifies the application instance and the low half is
    /// its monotonic sequence.  This remains an opaque application token: it
    /// does not contain a target, storage, or transport type.
    pub const fn with_instance(
        instance_id: u16,
        sequence: u16,
        candidate: ConfigurationCandidate,
    ) -> Self {
        Self {
            id: ((instance_id as u32) << 16) | sequence as u32,
            candidate,
        }
    }

    /// Returns the application-local change identity.
    pub const fn id(self) -> u32 {
        self.id
    }

    /// Returns the bounded application-instance portion of this identity.
    pub const fn instance_id(self) -> u16 {
        (self.id >> 16) as u16
    }

    /// Returns the bounded per-instance sequence portion of this identity.
    pub const fn sequence(self) -> u16 {
        let bytes = self.id.to_le_bytes();
        u16::from_le_bytes([bytes[0], bytes[1]])
    }

    /// Returns the candidate requested by this change.
    pub const fn candidate(self) -> ConfigurationCandidate {
        self.candidate
    }

    /// Returns the opaque target/storage generation in the candidate.
    pub const fn generation(self) -> u32 {
        self.candidate.generation()
    }
}

/// The result an adapter reports after attempting one change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationChangeResult {
    /// The target tuned and read back the candidate successfully.
    Applied,
    /// The target could not apply the candidate; its reason remains target
    /// specific and is deliberately not represented in this layer.
    Rejected,
}

/// A bounded acknowledgement tied to the exact candidate attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationAcknowledgement {
    change: ConfigurationChange,
    result: ConfigurationChangeResult,
}

impl ConfigurationAcknowledgement {
    /// Creates a successful acknowledgement.
    pub const fn applied(change: ConfigurationChange) -> Self {
        Self {
            change,
            result: ConfigurationChangeResult::Applied,
        }
    }

    /// Creates a failed acknowledgement.
    pub const fn rejected(change: ConfigurationChange) -> Self {
        Self {
            change,
            result: ConfigurationChangeResult::Rejected,
        }
    }

    /// Returns the exact change this acknowledgement refers to.
    pub const fn change(self) -> ConfigurationChange {
        self.change
    }

    /// Returns the candidate this acknowledgement refers to.
    pub const fn candidate(self) -> ConfigurationCandidate {
        self.change.candidate()
    }

    /// Returns the adapter result.
    pub const fn result(self) -> ConfigurationChangeResult {
        self.result
    }

    /// Returns whether the adapter applied the exact candidate.
    pub const fn is_applied(self) -> bool {
        matches!(self.result, ConfigurationChangeResult::Applied)
    }
}
