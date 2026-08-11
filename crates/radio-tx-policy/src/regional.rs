//! Versioned, fixed-capacity regional transmit rules.
//!
//! These rules are a machine-checkable regulatory snapshot, not a claim of
//! legal compliance. Facts the radio cannot establish are explicit
//! attestations, and service-specific assignments are exact bounded grants.

use radio_domain::{Bandwidth, Frequency, Modulation, TxClass};

/// Version of the built-in regulatory snapshot.
pub const RULE_SNAPSHOT_VERSION: u16 = 1;
/// Maximum number of assignment ranges in one individualized grant.
pub const MAX_GRANT_RANGES: usize = 8;

/// Regulatory jurisdiction selected by the operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
    /// United Kingdom, under Ofcom licensing.
    UnitedKingdom,
    /// United States, under FCC rules.
    UnitedStates,
}

/// Provenance attached to every built-in rule snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleProvenance {
    /// Dataset version.
    pub version: u16,
    /// Date on which the primary sources were checked, as `YYYYMMDD`.
    pub checked_on: u32,
    /// Short stable source identifier suitable for a bounded display/log.
    pub source: &'static str,
}

/// State of one fact which the K1 cannot establish itself.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Attestation {
    /// The fact was not affirmatively supplied; policy must deny.
    #[default]
    Unattested,
    /// The operator affirmatively supplied the fact.
    Attested,
}

/// Facts outside the K1 which an operator must attest for each authorization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Attestations {
    /// The selected region matches the station's actual location.
    pub location: Attestation,
    /// The relevant licence or authorization is current.
    pub licence_valid: Attestation,
    /// The operator is entitled to operate this service and assignment.
    pub operator_authorised: Attestation,
    /// Service-specific equipment eligibility has been established.
    pub equipment_eligible: Attestation,
}

/// One inclusive frequency range with its conducted-power and FM limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleRange {
    /// Lowest permitted carrier frequency, inclusive.
    pub first_hz: u32,
    /// Highest permitted carrier frequency, inclusive.
    pub last_hz: u32,
    /// Maximum conducted power represented by this snapshot, in milliwatts.
    pub max_conducted_mw: u32,
    /// Whether wide FM is permitted by this conservative rule.
    pub wide_fm: bool,
}

impl RuleRange {
    const fn contains(self, frequency: Frequency) -> bool {
        frequency.as_hz() >= self.first_hz && frequency.as_hz() <= self.last_hz
    }
}

/// An individualized assignment supplied from a licence or experimental grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrantProfile {
    /// Region which issued the grant.
    pub region: Region,
    /// Service authorized by the grant.
    pub class: TxClass,
    /// Operator-defined stable grant identifier.
    pub grant_id: u32,
    /// Exact authorized ranges; unused entries are `None`.
    pub ranges: [Option<RuleRange>; MAX_GRANT_RANGES],
    /// Whether the profile contains every fact required by the grant.
    pub complete: bool,
}

/// Complete input to the regional rule decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionalRequest<'a> {
    /// Selected jurisdiction.
    pub region: Region,
    /// Independently permissioned service class.
    pub class: TxClass,
    /// Exact requested carrier frequency.
    pub frequency: Frequency,
    /// Requested modulation.
    pub modulation: Modulation,
    /// Requested channel bandwidth.
    pub bandwidth: Bandwidth,
    /// Requested conducted power in milliwatts.
    pub conducted_mw: u32,
    /// Required external attestations.
    pub attestations: Attestations,
    /// Individual assignment for grant-controlled services.
    pub grant: Option<&'a GrantProfile>,
}

/// Stable reason a regional-rule decision denied transmission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionalDenial {
    /// `Never` is not a transmit service.
    Never,
    /// One or more required external facts were not attested.
    MissingAttestation,
    /// The K1 has not been established as eligible equipment for the service.
    EquipmentIneligible,
    /// The selected service requires an individualized assignment.
    MissingGrant,
    /// The supplied grant is incomplete or belongs to another region/service.
    InvalidGrant,
    /// The frequency is outside every applicable range.
    Frequency,
    /// Only analog FM is represented by this package.
    Modulation,
    /// The requested FM bandwidth exceeds the applicable limit.
    Bandwidth,
    /// The requested conducted power exceeds the applicable ceiling.
    Power,
}

/// Returns provenance for the selected built-in snapshot.
pub const fn provenance(region: Region) -> RuleProvenance {
    match region {
        Region::UnitedKingdom => RuleProvenance {
            version: RULE_SNAPSHOT_VERSION,
            checked_on: 20_260_812,
            source: "Ofcom OFW611/IR2028",
        },
        Region::UnitedStates => RuleProvenance {
            version: RULE_SNAPSHOT_VERSION,
            checked_on: 20_260_812,
            source: "47 CFR 97.301/.305/.313",
        },
    }
}

/// Evaluates the conservative built-in regional snapshot and an optional grant.
pub fn evaluate(request: RegionalRequest<'_>) -> Result<RuleProvenance, RegionalDenial> {
    if request.class == TxClass::Never {
        return Err(RegionalDenial::Never);
    }
    if request.attestations.location != Attestation::Attested
        || request.attestations.licence_valid != Attestation::Attested
        || request.attestations.operator_authorised != Attestation::Attested
    {
        return Err(RegionalDenial::MissingAttestation);
    }
    if request.attestations.equipment_eligible != Attestation::Attested {
        return Err(RegionalDenial::EquipmentIneligible);
    }
    if request.modulation != Modulation::Fm {
        return Err(RegionalDenial::Modulation);
    }

    let built_in = built_in_range(request.region, request.class, request.frequency);
    let rule = if requires_grant(request.class) {
        grant_range(request)?
    } else {
        built_in.ok_or(RegionalDenial::Frequency)?
    };
    if request.bandwidth == Bandwidth::Wide && !rule.wide_fm {
        return Err(RegionalDenial::Bandwidth);
    }
    if request.conducted_mw == 0 || request.conducted_mw > rule.max_conducted_mw {
        return Err(RegionalDenial::Power);
    }
    Ok(provenance(request.region))
}

const fn requires_grant(class: TxClass) -> bool {
    matches!(
        class,
        TxClass::Marine | TxClass::Aeronautical | TxClass::Business | TxClass::Experimental
    )
}

fn grant_range(request: RegionalRequest<'_>) -> Result<RuleRange, RegionalDenial> {
    let grant = request.grant.ok_or(RegionalDenial::MissingGrant)?;
    if !grant.complete || grant.region != request.region || grant.class != request.class {
        return Err(RegionalDenial::InvalidGrant);
    }
    grant
        .ranges
        .iter()
        .flatten()
        .copied()
        .find(|range| range.contains(request.frequency))
        .ok_or(RegionalDenial::Frequency)
}

const fn built_in_range(region: Region, class: TxClass, frequency: Frequency) -> Option<RuleRange> {
    let hz = frequency.as_hz();
    match (region, class) {
        // PMR446 is represented, but equipment eligibility remains a separate
        // mandatory fact; this dataset does not establish the K1 as compliant.
        (Region::UnitedKingdom, TxClass::LicenceFreePlan)
            if hz >= 446_000_000 && hz <= 446_200_000 =>
        {
            Some(RuleRange {
                first_hz: 446_000_000,
                last_hz: 446_200_000,
                max_conducted_mw: 500,
                wide_fm: false,
            })
        }
        (Region::UnitedKingdom, TxClass::Amateur)
            if (hz >= 144_000_000 && hz <= 146_000_000)
                || (hz >= 430_000_000 && hz <= 440_000_000) =>
        {
            Some(RuleRange {
                first_hz: if hz < 200_000_000 {
                    144_000_000
                } else {
                    430_000_000
                },
                last_hz: if hz < 200_000_000 {
                    146_000_000
                } else {
                    440_000_000
                },
                // Conservative Foundation ceiling; band-specific and licence-
                // tier reductions still require a later exact profile.
                max_conducted_mw: 25_000,
                wide_fm: true,
            })
        }
        (Region::UnitedStates, TxClass::Amateur)
            if (hz >= 144_000_000 && hz <= 148_000_000)
                || (hz >= 420_000_000 && hz <= 450_000_000) =>
        {
            Some(RuleRange {
                first_hz: if hz < 200_000_000 {
                    144_000_000
                } else {
                    420_000_000
                },
                last_hz: if hz < 200_000_000 {
                    148_000_000
                } else {
                    450_000_000
                },
                // This is a policy ceiling, not the broad legal maximum.
                max_conducted_mw: 25_000,
                wide_fm: true,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTESTED: Attestations = Attestations {
        location: Attestation::Attested,
        licence_valid: Attestation::Attested,
        operator_authorised: Attestation::Attested,
        equipment_eligible: Attestation::Attested,
    };

    fn request(region: Region, class: TxClass, hz: u32) -> RegionalRequest<'static> {
        RegionalRequest {
            region,
            class,
            frequency: Frequency::from_hz(hz).unwrap(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            conducted_mw: 500,
            attestations: ATTESTED,
            grant: None,
        }
    }

    #[test]
    fn amateur_boundaries_are_inclusive_and_region_specific() {
        for hz in [144_000_000, 146_000_000, 430_000_000, 440_000_000] {
            assert!(evaluate(request(Region::UnitedKingdom, TxClass::Amateur, hz)).is_ok());
        }
        assert_eq!(
            evaluate(request(
                Region::UnitedKingdom,
                TxClass::Amateur,
                146_000_001
            )),
            Err(RegionalDenial::Frequency)
        );
        assert!(evaluate(request(Region::UnitedStates, TxClass::Amateur, 147_000_000)).is_ok());
        assert_eq!(
            evaluate(request(
                Region::UnitedKingdom,
                TxClass::Amateur,
                147_000_000
            )),
            Err(RegionalDenial::Frequency)
        );
    }

    #[test]
    fn every_non_amateur_service_is_independently_denied_without_its_facts() {
        assert_eq!(
            evaluate(request(Region::UnitedKingdom, TxClass::Never, 145_000_000)),
            Err(RegionalDenial::Never)
        );
        for class in [
            TxClass::Marine,
            TxClass::Aeronautical,
            TxClass::Business,
            TxClass::Experimental,
        ] {
            assert_eq!(
                evaluate(request(Region::UnitedKingdom, class, 160_000_000)),
                Err(RegionalDenial::MissingGrant)
            );
        }
    }

    #[test]
    fn a_complete_exact_grant_permits_only_its_tuple() {
        let grant = GrantProfile {
            region: Region::UnitedKingdom,
            class: TxClass::Business,
            grant_id: 7,
            ranges: [
                Some(RuleRange {
                    first_hz: 165_000_000,
                    last_hz: 165_000_000,
                    max_conducted_mw: 1_000,
                    wide_fm: false,
                }),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            complete: true,
        };
        let mut candidate = request(Region::UnitedKingdom, TxClass::Business, 165_000_000);
        candidate.grant = Some(&grant);
        assert!(evaluate(candidate).is_ok());
        candidate.frequency = Frequency::from_hz(165_000_001).unwrap();
        assert_eq!(evaluate(candidate), Err(RegionalDenial::Frequency));
        candidate.frequency = Frequency::from_hz(165_000_000).unwrap();
        candidate.conducted_mw = 1_001;
        assert_eq!(evaluate(candidate), Err(RegionalDenial::Power));
    }

    #[test]
    fn mode_bandwidth_power_and_attestations_fail_closed() {
        let mut candidate = request(Region::UnitedKingdom, TxClass::Amateur, 145_500_000);
        candidate.modulation = Modulation::Am;
        assert_eq!(evaluate(candidate), Err(RegionalDenial::Modulation));
        candidate.modulation = Modulation::Fm;
        candidate.conducted_mw = 25_001;
        assert_eq!(evaluate(candidate), Err(RegionalDenial::Power));
        candidate.conducted_mw = 500;
        candidate.attestations.location = Attestation::Unattested;
        assert_eq!(evaluate(candidate), Err(RegionalDenial::MissingAttestation));
        candidate.attestations = Attestations {
            equipment_eligible: Attestation::Unattested,
            ..ATTESTED
        };
        assert_eq!(
            evaluate(candidate),
            Err(RegionalDenial::EquipmentIneligible)
        );
    }

    #[test]
    fn licence_free_representation_does_not_bypass_equipment_eligibility() {
        let mut candidate = request(Region::UnitedKingdom, TxClass::LicenceFreePlan, 446_006_250);
        candidate.attestations.equipment_eligible = Attestation::Unattested;
        assert_eq!(
            evaluate(candidate),
            Err(RegionalDenial::EquipmentIneligible)
        );
        candidate.attestations.equipment_eligible = Attestation::Attested;
        candidate.bandwidth = Bandwidth::Wide;
        assert_eq!(evaluate(candidate), Err(RegionalDenial::Bandwidth));
    }
}
