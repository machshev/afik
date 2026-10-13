//! Complete authorization behavior using synthetic evidence only.

mod support;

use radio_domain::{Bandwidth, Frequency, Modulation, TxClass};
use radio_tx_policy::regional::{Attestation, GrantProfile, Region, RegionalDenial, RuleRange};
use radio_tx_policy::{PermissionSet, StoredPermissions, TxDenied, TxPolicy, TxRequest, TxState};

fn policy() -> TxPolicy {
    let mut permissions = PermissionSet::none();
    for class in [
        TxClass::LicenceFreePlan,
        TxClass::Amateur,
        TxClass::Marine,
        TxClass::Aeronautical,
        TxClass::Business,
        TxClass::Experimental,
    ] {
        permissions = permissions.with(class, true);
    }
    TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0
}

fn request() -> TxRequest {
    support::request(support::channel(TxClass::Amateur))
}

fn denied(request: &TxRequest, reason: TxDenied) {
    assert_eq!(policy().authorise(request).unwrap_err(), reason);
}

#[test]
fn complete_request_is_approved_with_exact_facts_and_snapshot_provenance() {
    let policy = policy();
    let request = request();
    let token = policy.authorise(&request).unwrap();
    assert!(token.matches(&request));
    assert_eq!(token.request(), &request);
    assert_eq!(token.provenance().version, 1);
    assert_eq!(token.provenance().checked_on, 20_260_812);
    assert_eq!(request.configuration.unwrap().generation, 0);
    assert!(core::mem::size_of_val(&token) <= 64);
    assert!(core::mem::size_of::<TxRequest>() <= 512);
}

#[test]
fn every_permission_class_and_corrupted_permission_bit_fails_closed() {
    for class in [
        TxClass::Never,
        TxClass::LicenceFreePlan,
        TxClass::Amateur,
        TxClass::Marine,
        TxClass::Aeronautical,
        TxClass::Business,
        TxClass::Experimental,
    ] {
        let request = support::request(support::channel(class));
        assert_eq!(
            TxPolicy::deny_all().authorise(&request).unwrap_err(),
            TxDenied::Permission
        );
        if class == TxClass::Never {
            denied(&request, TxDenied::Permission);
        } else {
            assert!(policy().authorise(&request).is_ok());
        }
    }
    let bytes =
        StoredPermissions::new(PermissionSet::none().with(TxClass::Amateur, true), 7).encode();
    for bit in 0..bytes.len() * 8 {
        let mut corrupt = bytes;
        corrupt[bit / 8] ^= 1 << (bit % 8);
        assert_eq!(
            TxPolicy::load(&corrupt)
                .0
                .authorise(&request())
                .unwrap_err(),
            TxDenied::Permission
        );
    }
}

#[test]
fn unsettled_state_missing_identity_timeout_and_zero_power_deny() {
    let original = request();
    for state in [
        TxState::Unknown,
        TxState::Scanning,
        TxState::Faulted,
        TxState::Changing,
    ] {
        denied(&TxRequest { state, ..original }, TxDenied::State);
    }
    denied(
        &TxRequest {
            configuration: None,
            ..original
        },
        TxDenied::Configuration,
    );
    for instance in [0, 1] {
        let mut candidate = original;
        let id = candidate.configuration.as_mut().unwrap();
        if instance == 0 {
            id.instance = 0;
        } else {
            id.revision = 0;
        }
        denied(&candidate, TxDenied::Configuration);
    }
    for timeout_ms in [0, 60_001, u32::MAX] {
        denied(
            &TxRequest {
                timeout_ms,
                ..original
            },
            TxDenied::Timeout,
        );
    }
    assert!(policy()
        .authorise(&TxRequest {
            timeout_ms: 60_000,
            ..original
        })
        .is_ok());
    denied(
        &TxRequest {
            conducted_mw: 0,
            ..original
        },
        TxDenied::Power,
    );
}

#[test]
fn every_required_attestation_and_unsupported_modulation_deny() {
    for field in 0..4 {
        let mut candidate = request();
        let facts = &mut candidate.attestations;
        match field {
            0 => facts.location = Attestation::Unattested,
            1 => facts.licence_valid = Attestation::Unattested,
            2 => facts.operator_authorised = Attestation::Unattested,
            _ => facts.equipment_eligible = Attestation::Unattested,
        }
        denied(
            &candidate,
            TxDenied::Regional(if field == 3 {
                RegionalDenial::EquipmentIneligible
            } else {
                RegionalDenial::MissingAttestation
            }),
        );
    }
    for modulation in [Modulation::Am, Modulation::Usb] {
        denied(
            &TxRequest {
                modulation,
                ..request()
            },
            TxDenied::Regional(RegionalDenial::Modulation),
        );
    }
    let mut candidate = request();
    candidate.channel.transmit = Frequency::from_hz(160_000_000).unwrap();
    denied(&candidate, TxDenied::Regional(RegionalDenial::Frequency));
}

#[test]
fn missing_malformed_and_exceeded_envelopes_deny_independently() {
    for hardware in [true, false] {
        let reason = if hardware {
            TxDenied::Hardware
        } else {
            TxDenied::Calibration
        };
        let mut candidate = request();
        if hardware {
            candidate.hardware = None;
        } else {
            candidate.calibration = None;
        }
        denied(&candidate, reason);
        for invalid in 0..11 {
            candidate = request();
            let envelope = if hardware {
                candidate.hardware.as_mut()
            } else {
                candidate.calibration.as_mut()
            }
            .unwrap();
            match invalid {
                0 => envelope.evidence_id = 0,
                1 => envelope.resolution_hz = 0,
                2 => envelope.ranges = [None; 4],
                3 => envelope.ranges[0].as_mut().unwrap().first_hz = 0,
                4 => envelope.ranges[0].as_mut().unwrap().first_hz += 10,
                5 => envelope.ranges[0].as_mut().unwrap().max_conducted_mw = 0,
                6 => envelope.ranges[0].as_mut().unwrap().max_conducted_mw = 99,
                7 => envelope.ranges[0].as_mut().unwrap().last_hz -= 10,
                8 => envelope.ranges[0].as_mut().unwrap().first_hz -= 1,
                9 => envelope.target_id = 0,
                _ => envelope.target_id += 1,
            }
            denied(&candidate, reason);
        }
        candidate = request();
        candidate.bandwidth = Bandwidth::Wide;
        if !hardware {
            candidate.hardware.as_mut().unwrap().ranges[0]
                .as_mut()
                .unwrap()
                .wide_fm = true;
        }
        denied(&candidate, reason);
    }
}

#[test]
fn power_must_satisfy_regional_grant_hardware_and_calibration_ceilings() {
    let mut candidate = request();
    candidate.hardware.as_mut().unwrap().ranges[0]
        .as_mut()
        .unwrap()
        .max_conducted_mw = 40_000;
    candidate.calibration.as_mut().unwrap().ranges[0]
        .as_mut()
        .unwrap()
        .max_conducted_mw = 40_000;
    candidate.conducted_mw = 25_001;
    denied(&candidate, TxDenied::Regional(RegionalDenial::Power));
    candidate.conducted_mw = 25_000;
    assert!(policy().authorise(&candidate).is_ok());
    candidate.grant = Some(GrantProfile {
        region: candidate.region,
        class: candidate.channel.tx_class,
        grant_id: 2,
        ranges: [
            Some(RuleRange {
                first_hz: 145_000_000,
                last_hz: 146_000_000,
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
    });
    denied(&candidate, TxDenied::Regional(RegionalDenial::Power));
    candidate.conducted_mw = 1_000;
    assert!(policy().authorise(&candidate).is_ok());
    candidate.hardware.as_mut().unwrap().ranges[0]
        .as_mut()
        .unwrap()
        .max_conducted_mw = 999;
    denied(&candidate, TxDenied::Hardware);
    candidate.hardware.as_mut().unwrap().ranges[0]
        .as_mut()
        .unwrap()
        .max_conducted_mw = 1_000;
    candidate.calibration.as_mut().unwrap().ranges[0]
        .as_mut()
        .unwrap()
        .max_conducted_mw = 999;
    denied(&candidate, TxDenied::Calibration);
}

#[test]
fn grants_are_complete_exact_and_all_overlapping_limits_apply() {
    let original = support::request(support::channel(TxClass::Business));
    denied(
        &TxRequest {
            grant: None,
            ..original
        },
        TxDenied::Regional(RegionalDenial::MissingGrant),
    );
    for field in 0..6 {
        let mut candidate = original;
        let grant = candidate.grant.as_mut().unwrap();
        match field {
            0 => grant.complete = false,
            1 => grant.grant_id = 0,
            2 => grant.region = Region::UnitedStates,
            3 => grant.class = TxClass::Marine,
            4 => grant.ranges[0].as_mut().unwrap().first_hz = 0,
            _ => grant.ranges[0].as_mut().unwrap().max_conducted_mw = 0,
        }
        denied(&candidate, TxDenied::Regional(RegionalDenial::InvalidGrant));
    }
    let mut candidate = original;
    let mut strict = candidate.grant.unwrap().ranges[0].unwrap();
    strict.max_conducted_mw = 99;
    candidate.grant.as_mut().unwrap().ranges[1] = Some(strict);
    denied(&candidate, TxDenied::Regional(RegionalDenial::Power));
    candidate.grant.as_mut().unwrap().ranges.swap(0, 1);
    denied(&candidate, TxDenied::Regional(RegionalDenial::Power));
    for hardware in [true, false] {
        candidate = request();
        let envelope = if hardware {
            candidate.hardware.as_mut()
        } else {
            candidate.calibration.as_mut()
        }
        .unwrap();
        envelope.ranges[1] = Some(strict);
        denied(
            &candidate,
            if hardware {
                TxDenied::Hardware
            } else {
                TxDenied::Calibration
            },
        );
    }
}

#[test]
fn licence_free_requires_exact_raster_simplex_operating_facts_and_limits() {
    for index in 0..16 {
        let mut channel = support::channel(TxClass::LicenceFreePlan);
        channel.receive = Frequency::from_hz(446_006_250 + index * 12_500).unwrap();
        channel.transmit = channel.receive;
        assert!(policy().authorise(&support::request(channel)).is_ok());
    }
    let original = support::request(support::channel(TxClass::LicenceFreePlan));
    for field in 0..4 {
        let mut candidate = original;
        match field {
            0 => candidate.operating.mobile = Attestation::Unattested,
            1 => candidate.operating.non_airborne = Attestation::Unattested,
            2 => candidate.operating.receiver_capable = Attestation::Unattested,
            _ => candidate.channel.receive = Frequency::from_hz(446_018_750).unwrap(),
        }
        denied(&candidate, TxDenied::OperatingConditions);
    }
    for hz in [446_000_000, 446_006_251, 446_012_500, 446_200_000] {
        let mut candidate = original;
        candidate.channel.transmit = Frequency::from_hz(hz).unwrap();
        candidate.channel.receive = candidate.channel.transmit;
        denied(&candidate, TxDenied::OperatingConditions);
    }
    denied(
        &TxRequest {
            bandwidth: Bandwidth::Wide,
            ..original
        },
        TxDenied::Regional(RegionalDenial::Bandwidth),
    );
    denied(
        &TxRequest {
            conducted_mw: 501,
            ..original
        },
        TxDenied::Regional(RegionalDenial::Power),
    );
}

#[test]
fn every_changed_request_fact_invalidates_the_token() {
    let policy = policy();
    let original = request();
    let token = policy.authorise(&original).unwrap();
    for field in 0..19 {
        let mut current = original;
        match field {
            0 => current.channel.transmit = Frequency::from_hz(145_525_000).unwrap(),
            1 => current.channel.receive = Frequency::from_hz(145_525_000).unwrap(),
            2 => current.channel.tx_class = TxClass::Business,
            3 => current.modulation = Modulation::Am,
            4 => current.bandwidth = Bandwidth::Wide,
            5 => current.conducted_mw += 1,
            6 => current.timeout_ms += 1,
            7 => current.configuration.as_mut().unwrap().generation += 1,
            8 => current.configuration.as_mut().unwrap().instance += 1,
            9 => current.configuration.as_mut().unwrap().revision += 1,
            10 => current.state = TxState::Changing,
            11 => current.region = Region::UnitedStates,
            12 => current.attestations.location = Attestation::Unattested,
            13 => current.hardware.as_mut().unwrap().evidence_id += 1,
            14 => current.calibration.as_mut().unwrap().evidence_id += 1,
            15 => {
                current.hardware.as_mut().unwrap().ranges[0]
                    .as_mut()
                    .unwrap()
                    .max_conducted_mw += 1;
            }
            16 => {
                current.calibration.as_mut().unwrap().ranges[0]
                    .as_mut()
                    .unwrap()
                    .max_conducted_mw += 1;
            }
            17 => current.operating.mobile = Attestation::Unattested,
            _ => current.target_id += 1,
        }
        assert!(
            !token.matches(&current),
            "changed field {field} reused authority"
        );
    }
    let business = support::request(support::channel(TxClass::Business));
    let token = policy.authorise(&business).unwrap();
    let mut current = business;
    current.grant.as_mut().unwrap().grant_id += 1;
    assert!(!token.matches(&current));
}
