//! Complete caller-supplied TX facts; no target limits are inferred here.

use radio_domain::{ActiveChannel, Bandwidth, Modulation, TxClass};

use crate::regional::{self, Attestation, Attestations, GrantProfile, Region, RuleRange};
use crate::{TxDenied, TxPolicy};

/// Maximum number of ranges in a hardware or calibration envelope.
pub const MAX_ENVELOPE_RANGES: usize = 4;
/// Conservative application timeout ceiling, in milliseconds.
pub const MAX_TX_TIMEOUT_MS: u32 = 60_000;

/// Adapter-owned identity; instance and revision must never be reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationIdentity {
    /// Non-zero application instance, replaced on reboot/app replacement.
    pub instance: u32,
    /// Opaque retained generation; zero is valid for volatile configuration.
    pub generation: u32,
    /// Non-zero revision, changed for every selection/configuration replacement.
    pub revision: u32,
}

/// Current application state, supplied by its authoritative controller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TxState {
    /// No established active configuration.
    #[default]
    Unknown,
    /// A healthy channel is selected and no change is pending.
    Selected,
    /// Scanning must not obtain authority.
    Scanning,
    /// Target failure must not obtain authority.
    Faulted,
    /// An unacknowledged configuration is pending.
    Changing,
}

/// Explicit evidence-backed hardware or per-unit calibration limits.
///
/// These are trusted adapter inputs, not measured or authenticated by policy.
/// There is deliberately no built-in envelope for either handset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxEnvelope {
    /// Non-zero target/unit identity to which this evidence applies.
    pub target_id: u64,
    /// Non-zero evidence identity, replaced when the supporting facts change.
    pub evidence_id: u32,
    /// Exact bounded FM ranges; unused entries are `None`.
    pub ranges: [Option<RuleRange>; MAX_ENVELOPE_RANGES],
    /// Positive frequency resolution in hertz, relative to each range start.
    pub resolution_hz: u32,
}

impl TxEnvelope {
    fn permits(&self, request: &TxRequest) -> bool {
        if self.target_id == 0
            || self.target_id != request.target_id
            || self.evidence_id == 0
            || self.resolution_hz == 0
        {
            return false;
        }
        let mut matched = false;
        for range in self.ranges.iter().flatten() {
            if range.first_hz == 0 || range.first_hz > range.last_hz || range.max_conducted_mw == 0
            {
                return false;
            }
            let hz = request.channel.transmit.as_hz();
            if hz >= range.first_hz && hz <= range.last_hz {
                // Overlaps must satisfy every applicable limit, independent
                // of input order; a permissive range cannot mask a strict one.
                if (hz - range.first_hz) % self.resolution_hz != 0
                    || request.conducted_mw > range.max_conducted_mw
                    || (request.bandwidth == Bandwidth::Wide && !range.wide_fm)
                {
                    return false;
                }
                matched = true;
            }
        }
        matched
    }
}

/// Equipment/operating facts additionally required for the PMR446 model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperatingConditions {
    /// Hand-portable/mobile operation has been established.
    pub mobile: Attestation,
    /// Operation is not airborne.
    pub non_airborne: Attestation,
    /// The equipment can receive on its selected simplex channel.
    pub receiver_capable: Attestation,
}

/// Complete immutable input to central TX authorization.
///
/// Adapters must rebuild this from current authoritative state immediately
/// before TX entry. Identity values and evidence must not be fabricated or
/// reused. A successful host fixture does not establish physical eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxRequest {
    /// Non-zero opaque target/unit identity, shared by both evidence envelopes.
    pub target_id: u64,
    /// Exact selected RX/TX frequencies and service class.
    pub channel: ActiveChannel,
    /// Exact requested modulation.
    pub modulation: Modulation,
    /// Exact requested bandwidth.
    pub bandwidth: Bandwidth,
    /// Requested conducted power in integer milliwatts; never silently clamped.
    pub conducted_mw: u32,
    /// Positive bounded transmit duration in milliseconds.
    pub timeout_ms: u32,
    /// Current authoritative controller state.
    pub state: TxState,
    /// Application/configuration identity, absent until established.
    pub configuration: Option<ConfigurationIdentity>,
    /// Actual operator-selected region.
    pub region: Region,
    /// Required affirmative external facts.
    pub attestations: Attestations,
    /// Exact grant, required for individualized services and honored if supplied.
    pub grant: Option<GrantProfile>,
    /// Target hardware limits; absence denies.
    pub hardware: Option<TxEnvelope>,
    /// Independently validated per-unit calibration limits; absence denies.
    pub calibration: Option<TxEnvelope>,
    /// Additional equipment/operating facts for licence-free operation.
    pub operating: OperatingConditions,
}

impl TxRequest {
    pub(crate) fn validate(&self, policy: TxPolicy) -> Result<regional::RuleProvenance, TxDenied> {
        if !policy.permissions.allows(self.channel.tx_class) {
            return Err(TxDenied::Permission);
        }
        if self.state != TxState::Selected {
            return Err(TxDenied::State);
        }
        if !matches!(self.configuration, Some(id) if id.instance != 0 && id.revision != 0) {
            return Err(TxDenied::Configuration);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TX_TIMEOUT_MS {
            return Err(TxDenied::Timeout);
        }
        if self.conducted_mw == 0 {
            return Err(TxDenied::Power);
        }
        let provenance = regional::evaluate(regional::RegionalRequest {
            region: self.region,
            class: self.channel.tx_class,
            frequency: self.channel.transmit,
            modulation: self.modulation,
            bandwidth: self.bandwidth,
            conducted_mw: self.conducted_mw,
            attestations: self.attestations,
            grant: self.grant.as_ref(),
        })
        .map_err(TxDenied::Regional)?;
        if self.channel.tx_class == TxClass::LicenceFreePlan {
            let hz = self.channel.transmit.as_hz();
            if self.channel.receive != self.channel.transmit
                || !(446_006_250..=446_193_750).contains(&hz)
                || (hz - 446_006_250) % 12_500 != 0
                || self.operating.mobile != Attestation::Attested
                || self.operating.non_airborne != Attestation::Attested
                || self.operating.receiver_capable != Attestation::Attested
            {
                return Err(TxDenied::OperatingConditions);
            }
        }
        if !self.hardware.is_some_and(|limits| limits.permits(self)) {
            return Err(TxDenied::Hardware);
        }
        if !self.calibration.is_some_and(|limits| limits.permits(self)) {
            return Err(TxDenied::Calibration);
        }
        Ok(provenance)
    }
}
