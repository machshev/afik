//! Fail-closed central transmit authorisation.
//!
//! The operator programs the frequency ranges this radio may transmit in from
//! the studio (`ADR-101`). Each range names its service class and carries its
//! own power ceiling, so every TX path, whether a programmed channel or the
//! VFO, is checked against the frequency it would transmit on. With no
//! programmed range nothing is permitted.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;
use radio_domain::{Bandwidth, Frequency, TxClass};

mod request;

pub use request::{
    ConfigurationIdentity, RuleRange, TxEnvelope, TxRequest, TxState, MAX_ENVELOPE_RANGES,
    MAX_TX_TIMEOUT_MS,
};

/// Most programmed TX ranges one radio holds.
pub const MAX_TX_RANGES: usize = 16;

/// One programmed frequency range in which TX is permitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TxRange {
    /// Service this range belongs to.
    pub class: TxClass,
    /// Lowest permitted carrier frequency, inclusive.
    pub first_hz: u32,
    /// Highest permitted carrier frequency, inclusive.
    pub last_hz: u32,
    /// Highest permitted conducted power, in milliwatts.
    pub max_mw: u16,
    /// Whether wide FM is permitted.
    pub wide_fm: bool,
    /// Channel raster relative to `first_hz`; zero permits any frequency.
    pub raster_hz: u16,
}

impl TxRange {
    /// Checks the range is well formed.
    ///
    /// # Errors
    ///
    /// Returns the first defect found.
    pub const fn validate(&self) -> Result<(), RangeError> {
        if self.first_hz == 0 || self.first_hz > self.last_hz {
            return Err(RangeError::Span);
        }
        if self.max_mw == 0 {
            return Err(RangeError::Power);
        }
        Ok(())
    }

    const fn contains(&self, frequency: Frequency) -> bool {
        frequency.as_hz() >= self.first_hz && frequency.as_hz() <= self.last_hz
    }

    const fn permits(&self, request: &TxRequest) -> bool {
        let hz = request.channel.transmit.as_hz();
        (self.raster_hz == 0 || (hz - self.first_hz) % self.raster_hz as u32 == 0)
            && request.conducted_mw <= self.max_mw as u32
            && (self.wide_fm || !matches!(request.bandwidth, Bandwidth::Wide))
    }
}

/// A set of TX classes, one bit per [`TxClass`] discriminant.
///
/// The handset uses it to switch programmed classes off and on again; it can
/// only narrow what the studio programmed (`ADR-101`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClassSet(u8);

impl ClassSet {
    /// Every class.
    pub const ALL: Self = Self(u8::MAX);
    /// No class.
    pub const NONE: Self = Self(0);

    /// Builds a set from stored bits; bits beyond the known classes are kept,
    /// so a later class is not switched on by an older record.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// The stored bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `class` is in the set.
    pub const fn contains(self, class: TxClass) -> bool {
        self.0 & (1 << class as u8) != 0
    }

    /// The set with `class` added or removed.
    #[must_use]
    pub const fn with(self, class: TxClass, on: bool) -> Self {
        let bit = 1 << class as u8;
        Self(if on { self.0 | bit } else { self.0 & !bit })
    }
}

/// Why a programmed TX range is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeError {
    /// The first frequency is zero or above the last.
    Span,
    /// The power ceiling is zero.
    Power,
    /// More ranges than [`MAX_TX_RANGES`].
    TooMany,
}

impl fmt::Display for RangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Span => "a TX range must start above zero and not end below its start",
            Self::Power => "a TX range needs a power ceiling above zero",
            Self::TooMany => "too many TX ranges",
        })
    }
}

/// Central policy authority used by every transmit request path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TxPolicy {
    ranges: [Option<TxRange>; MAX_TX_RANGES],
}

impl TxPolicy {
    /// Constructs the default policy, which permits nothing.
    pub const fn deny_all() -> Self {
        Self {
            ranges: [None; MAX_TX_RANGES],
        }
    }

    /// Builds a policy from programmed ranges, refusing any malformed one.
    ///
    /// # Errors
    ///
    /// Returns the first defect; the caller then keeps [`Self::deny_all`].
    pub fn new(ranges: &[TxRange]) -> Result<Self, RangeError> {
        if ranges.len() > MAX_TX_RANGES {
            return Err(RangeError::TooMany);
        }
        let mut policy = Self::deny_all();
        for (slot, range) in policy.ranges.iter_mut().zip(ranges) {
            range.validate()?;
            *slot = Some(*range);
        }
        Ok(policy)
    }

    /// Returns the programmed ranges in order.
    pub fn ranges(&self) -> impl Iterator<Item = &TxRange> {
        self.ranges.iter().flatten()
    }

    /// The classes of the programmed ranges.
    pub fn classes(&self) -> ClassSet {
        self.ranges()
            .fold(ClassSet::NONE, |set, range| set.with(range.class, true))
    }

    /// The policy with only the ranges whose class is in `enabled`.
    ///
    /// This is the handset's class switch: it can drop ranges but never add
    /// one, so the result is never more permissive than `self`.
    #[must_use]
    pub fn restricted(&self, enabled: ClassSet) -> Self {
        let mut policy = *self;
        for slot in &mut policy.ranges {
            if slot.is_some_and(|range| !enabled.contains(range.class)) {
                *slot = None;
            }
        }
        policy
    }

    /// The most power the ranges containing `frequency` allow, if any does.
    ///
    /// Every containing range applies, so this is the lowest of their
    /// ceilings. A caller uses it to choose a power before asking; the policy
    /// itself still refuses anything above it rather than clamping.
    #[must_use]
    pub fn max_mw_at(&self, frequency: Frequency) -> Option<u16> {
        self.ranges()
            .filter(|range| range.contains(frequency))
            .map(|range| range.max_mw)
            .min()
    }

    /// Finds the class of the range permitting `request`.
    ///
    /// Every range containing the frequency must permit it, so an overlapping
    /// permissive range cannot mask a strict one; the first one names the class.
    fn permitting(&self, request: &TxRequest) -> Result<TxClass, TxDenied> {
        let mut class = None;
        for range in self.ranges() {
            if range.contains(request.channel.transmit) {
                if !range.permits(request) {
                    return Err(TxDenied::Range);
                }
                class.get_or_insert(range.class);
            }
        }
        class.ok_or(TxDenied::Range)
    }

    /// Authorizes one complete request, borrowing its immutable policy/facts.
    ///
    /// Class-only token construction is unavailable:
    /// ```compile_fail
    /// use radio_domain::TxClass;
    /// use radio_tx_policy::TxPolicy;
    /// TxPolicy::deny_all().authorise(TxClass::Amateur);
    /// ```
    /// Approved facts cannot be changed while authority is still used:
    /// ```compile_fail
    /// use radio_tx_policy::{TxPolicy, TxRequest};
    /// fn change(policy: &TxPolicy, request: &mut TxRequest) {
    ///     let token = policy.authorise(request).unwrap();
    ///     request.conducted_mw += 1;
    ///     assert!(token.matches(request));
    /// }
    /// ```
    /// Policy replacement also ends the borrow before new authority is possible:
    /// ```compile_fail
    /// use radio_tx_policy::{TxPolicy, TxRequest};
    /// fn change(policy: &mut TxPolicy, request: &TxRequest) {
    ///     let token = policy.authorise(request).unwrap();
    ///     *policy = TxPolicy::deny_all();
    ///     assert!(token.matches(request));
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first failed check.
    pub fn authorise<'a>(
        &'a self,
        request: &'a TxRequest,
    ) -> Result<TxAuthorisation<'a>, TxDenied> {
        let class = request.validate(self)?;
        Ok(TxAuthorisation {
            request,
            policy: self,
            class,
        })
    }
}

/// Capability token required to enter hardware transmit state.
///
/// Its constructor is private to this crate. Drivers should accept a borrowed
/// token for the shortest possible scope.
#[derive(Debug)]
pub struct TxAuthorisation<'a> {
    request: &'a TxRequest,
    policy: &'a TxPolicy,
    class: TxClass,
}

impl TxAuthorisation<'_> {
    /// Returns the class of the programmed range that permitted this request.
    pub const fn class(&self) -> TxClass {
        self.class
    }

    /// Checks the entire freshly supplied request, including opaque identities.
    pub fn matches(&self, current: &TxRequest) -> bool {
        self.request == current && current.validate(self.policy).is_ok()
    }

    /// Returns the exact approved request; this is not proof of current state.
    pub const fn request(&self) -> &TxRequest {
        self.request
    }
}

/// Transmit authorisation was denied by central policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxDenied {
    /// No programmed range permits this frequency, power and bandwidth.
    Range,
    /// No healthy, settled selected channel is established.
    State,
    /// Configuration identity is missing or invalid.
    Configuration,
    /// Timeout is zero or exceeds the bounded application ceiling.
    Timeout,
    /// Requested power is zero.
    Power,
    /// Only analog FM is transmitted.
    Modulation,
    /// Hardware limits are absent, malformed, or exceeded.
    Hardware,
    /// Calibration limits are absent, malformed, or exceeded.
    Calibration,
}

impl fmt::Display for TxDenied {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "transmit denied by policy: {self:?}")
    }
}
