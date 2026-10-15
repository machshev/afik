//! Complete caller-supplied TX facts; no target limits are inferred here.

use radio_domain::{ActiveChannel, Bandwidth, Modulation, TxClass};

use crate::{TxDenied, TxPolicy};

/// Maximum number of ranges in a hardware or calibration envelope.
pub const MAX_ENVELOPE_RANGES: usize = 4;
/// Application timeout ceiling, in milliseconds: five minutes, the longest
/// TX timeout an operator can choose on the handset.
pub const MAX_TX_TIMEOUT_MS: u32 = 300_000;

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

/// One inclusive frequency range with its conducted-power and FM limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleRange {
    /// Lowest permitted carrier frequency, inclusive.
    pub first_hz: u32,
    /// Highest permitted carrier frequency, inclusive.
    pub last_hz: u32,
    /// Maximum conducted power, in milliwatts.
    pub max_conducted_mw: u32,
    /// Whether wide FM is permitted.
    pub wide_fm: bool,
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

/// Complete immutable input to central TX authorization.
///
/// Adapters must rebuild this from current authoritative state immediately
/// before TX entry. Identity values and evidence must not be fabricated or
/// reused. A successful host fixture does not establish physical eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxRequest {
    /// Non-zero opaque target/unit identity, shared by both evidence envelopes.
    pub target_id: u64,
    /// Exact selected RX/TX frequencies; the class is not consulted.
    pub channel: ActiveChannel,
    /// Exact requested modulation.
    pub modulation: Modulation,
    /// The selected channel only receives, such as a receive-only duplex
    /// channel; it never transmits, whatever range covers it.
    pub receive_only: bool,
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
    /// Target hardware limits; absence denies.
    pub hardware: Option<TxEnvelope>,
    /// Independently validated per-unit calibration limits; absence denies.
    pub calibration: Option<TxEnvelope>,
}

impl TxRequest {
    /// Validates every fact and returns the class of the permitting range.
    pub(crate) fn validate(&self, policy: &TxPolicy) -> Result<TxClass, TxDenied> {
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
        if self.modulation != Modulation::Fm {
            return Err(TxDenied::Modulation);
        }
        if self.receive_only {
            return Err(TxDenied::ReceiveOnly);
        }
        let class = policy.permitting(self)?;
        if !self.hardware.is_some_and(|limits| limits.permits(self)) {
            return Err(TxDenied::Hardware);
        }
        if !self.calibration.is_some_and(|limits| limits.permits(self)) {
            return Err(TxDenied::Calibration);
        }
        Ok(class)
    }
}
