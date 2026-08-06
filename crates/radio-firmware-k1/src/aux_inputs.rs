//! Bounded, untrusted observations for the K1 auxiliary input surface.
//!
//! The pinned board source identifies PTT on PB10 but does not provide AFIK
//! with an independently verified side-key mapping. This module therefore
//! retains raw GPIOB evidence only; it does not decode side keys or infer
//! polarity.

/// GPIOB pin used by the pinned source for PTT.
pub const PTT_PIN: u8 = 10;

/// Origin of a raw auxiliary-input observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationSource {
    /// A sample read from the target GPIOB input data register.
    TargetGpioB,
    /// A bounded host or simulation fixture, never physical evidence.
    TestFixture,
}

/// One bounded raw GPIOB observation supplied by an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawAuxiliarySample {
    /// Monotonic adapter sequence; zero is reserved as invalid.
    pub sequence: u32,
    /// GPIOB input data register, retained without side-key interpretation.
    pub gpio_b_idr: u16,
    /// Whether the adapter observed a stable sample at its stated instant.
    pub stable: bool,
    /// Provenance of the sample.
    pub source: ObservationSource,
}

/// A validated raw observation with no semantic key meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawAuxiliaryObservation {
    sequence: u32,
    gpio_b_idr: u16,
    source: ObservationSource,
}

impl RawAuxiliaryObservation {
    /// Returns the adapter sequence number.
    #[must_use]
    pub const fn sequence(self) -> u32 {
        self.sequence
    }

    /// Returns the untouched GPIOB input data register value.
    #[must_use]
    pub const fn gpio_b_idr(self) -> u16 {
        self.gpio_b_idr
    }

    /// Returns the observation provenance.
    #[must_use]
    pub const fn source(self) -> ObservationSource {
        self.source
    }

    /// Returns the raw PB10 level without assigning active-high or active-low
    /// meaning.
    #[must_use]
    pub const fn raw_ptt_level(self) -> bool {
        self.gpio_b_idr & (1 << PTT_PIN) != 0
    }
}

/// Why a raw auxiliary observation cannot be accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationError {
    /// Sequence zero is reserved and cannot establish provenance.
    InvalidSequence,
    /// The adapter marked this sample unstable or ambiguous.
    Unstable,
    /// The sequence does not advance beyond the last accepted sample.
    Stale,
}

/// Bounded acceptance state for raw auxiliary observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationGate {
    last_sequence: Option<u32>,
}

impl ObservationGate {
    /// Creates an empty acceptance gate.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_sequence: None,
        }
    }

    /// Accepts one stable, strictly newer raw observation.
    pub fn accept(
        &mut self,
        sample: RawAuxiliarySample,
    ) -> Result<RawAuxiliaryObservation, ObservationError> {
        if sample.sequence == 0 {
            return Err(ObservationError::InvalidSequence);
        }
        if !sample.stable {
            return Err(ObservationError::Unstable);
        }
        if self
            .last_sequence
            .is_some_and(|last| sample.sequence <= last)
        {
            return Err(ObservationError::Stale);
        }
        self.last_sequence = Some(sample.sequence);
        Ok(RawAuxiliaryObservation {
            sequence: sample.sequence,
            gpio_b_idr: sample.gpio_b_idr,
            source: sample.source,
        })
    }
}

impl Default for ObservationGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ObservationError, ObservationGate, ObservationSource, RawAuxiliarySample, PTT_PIN,
    };

    fn sample(sequence: u32, gpio_b_idr: u16, stable: bool) -> RawAuxiliarySample {
        RawAuxiliarySample {
            sequence,
            gpio_b_idr,
            stable,
            source: ObservationSource::TestFixture,
        }
    }

    #[test]
    fn accepts_raw_sample_and_exposes_only_uninterpreted_ptt_bit() {
        let mut gate = ObservationGate::new();
        let observation = gate.accept(sample(1, 1 << PTT_PIN, true)).unwrap();
        assert_eq!(observation.sequence(), 1);
        assert_eq!(observation.gpio_b_idr(), 1 << PTT_PIN);
        assert_eq!(observation.source(), ObservationSource::TestFixture);
        assert!(observation.raw_ptt_level());
    }

    #[test]
    fn rejects_invalid_unstable_and_stale_samples_without_advancing() {
        let mut gate = ObservationGate::new();
        assert_eq!(
            gate.accept(sample(0, 0, true)),
            Err(ObservationError::InvalidSequence)
        );
        assert_eq!(
            gate.accept(sample(1, 0, false)),
            Err(ObservationError::Unstable)
        );
        assert!(gate.accept(sample(2, 0, true)).is_ok());
        assert_eq!(
            gate.accept(sample(2, 1, true)),
            Err(ObservationError::Stale)
        );
        assert_eq!(
            gate.accept(sample(1, 1, true)),
            Err(ObservationError::Stale)
        );
        assert!(gate.accept(sample(3, 1, true)).is_ok());
    }

    #[test]
    fn preserves_zero_and_non_ptt_bits_for_later_evidence_review() {
        let mut gate = ObservationGate::new();
        let raw = 0xA5A5;
        let observation = gate.accept(sample(1, raw, true)).unwrap();
        assert_eq!(observation.gpio_b_idr(), raw);
        assert_eq!(observation.raw_ptt_level(), raw & (1 << PTT_PIN) != 0);
    }
}
