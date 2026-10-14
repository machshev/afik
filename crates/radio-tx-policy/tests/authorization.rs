//! Complete authorization behavior using synthetic evidence only.

mod support;

use radio_domain::{Bandwidth, Frequency, Modulation, TxClass};
use radio_tx_policy::{
    ClassSet, RangeError, TxDenied, TxPolicy, TxRange, TxRequest, TxState, MAX_TX_RANGES,
};

fn policy() -> TxPolicy {
    support::policy(true)
}

fn request() -> TxRequest {
    support::request(support::channel(TxClass::Amateur))
}

fn at(hz: u32) -> TxRequest {
    let mut channel = support::channel(TxClass::Amateur);
    channel.transmit = Frequency::from_hz(hz).unwrap();
    channel.receive = channel.transmit;
    support::request(channel)
}

fn denied(request: &TxRequest, reason: TxDenied) {
    assert_eq!(policy().authorise(request).unwrap_err(), reason);
}

#[test]
fn a_programmed_range_permits_its_frequencies_and_names_the_class() {
    let policy = policy();
    let amateur = request();
    assert_eq!(
        policy.authorise(&amateur).unwrap().class(),
        TxClass::Amateur
    );
    let pmr = support::request(support::channel(TxClass::LicenceFreePlan));
    assert_eq!(
        policy.authorise(&pmr).unwrap().class(),
        TxClass::LicenceFreePlan
    );
}

#[test]
fn no_programmed_range_permits_nothing() {
    for request in [
        request(),
        support::request(support::channel(TxClass::LicenceFreePlan)),
    ] {
        assert_eq!(
            TxPolicy::deny_all().authorise(&request).unwrap_err(),
            TxDenied::Range
        );
        assert_eq!(
            TxPolicy::default().authorise(&request).unwrap_err(),
            TxDenied::Range
        );
    }
}

#[test]
fn outside_every_range_off_raster_over_power_and_wide_are_denied() {
    // The envelopes follow the candidate, so only the programmed range decides.
    let widen = |mut request: TxRequest| {
        for envelope in [request.hardware.as_mut(), request.calibration.as_mut()] {
            let range = envelope.unwrap().ranges[0].as_mut().unwrap();
            range.max_conducted_mw = 10_000;
            range.wide_fm = true;
        }
        request
    };
    for hz in [143_999_990, 146_000_010, 446_206_250, 156_800_000] {
        denied(&widen(at(hz)), TxDenied::Range);
    }
    assert!(policy().authorise(&widen(at(144_000_000))).is_ok());
    assert!(policy().authorise(&widen(at(146_000_000))).is_ok());
    // PMR446: on the 12.5 kHz raster, at most 500 mW, narrow only.
    assert!(policy().authorise(&widen(at(446_018_750))).is_ok());
    denied(&widen(at(446_012_500 + 1_250)), TxDenied::Range);
    let mut loud = widen(at(446_006_250));
    loud.conducted_mw = 501;
    denied(&loud, TxDenied::Range);
    loud.conducted_mw = 500;
    assert!(policy().authorise(&loud).is_ok());
    let mut wide = widen(at(446_006_250));
    wide.bandwidth = Bandwidth::Wide;
    denied(&wide, TxDenied::Range);
    let mut wide = widen(at(145_500_000));
    wide.bandwidth = Bandwidth::Wide;
    assert!(policy().authorise(&wide).is_ok());
}

#[test]
fn overlapping_ranges_all_apply_whatever_their_order() {
    let broad = TxRange {
        class: TxClass::Amateur,
        first_hz: 144_000_000,
        last_hz: 146_000_000,
        max_mw: 5_000,
        wide_fm: true,
        raster_hz: 0,
    };
    let strict = TxRange {
        class: TxClass::Experimental,
        first_hz: 145_000_000,
        last_hz: 145_900_000,
        max_mw: 50,
        wide_fm: false,
        raster_hz: 0,
    };
    for ranges in [[broad, strict], [strict, broad]] {
        let policy = TxPolicy::new(&ranges).unwrap();
        assert_eq!(
            policy.authorise(&request()).unwrap_err(),
            TxDenied::Range,
            "a permissive range masked a strict one"
        );
        let mut quiet = request();
        quiet.conducted_mw = 50;
        assert_eq!(policy.authorise(&quiet).unwrap().class(), ranges[0].class);
    }
}

#[test]
fn malformed_or_too_many_ranges_are_refused() {
    let good = support::RANGES[1];
    for (range, error) in [
        (
            TxRange {
                first_hz: 0,
                ..good
            },
            RangeError::Span,
        ),
        (
            TxRange {
                last_hz: good.first_hz - 1,
                ..good
            },
            RangeError::Span,
        ),
        (TxRange { max_mw: 0, ..good }, RangeError::Power),
    ] {
        assert_eq!(range.validate(), Err(error));
        assert_eq!(TxPolicy::new(&[good, range]), Err(error));
    }
    assert!(TxPolicy::new(&[good; MAX_TX_RANGES]).is_ok());
    assert_eq!(
        TxPolicy::new(&[good; MAX_TX_RANGES + 1]),
        Err(RangeError::TooMany)
    );
    assert_eq!(
        TxPolicy::new(&support::RANGES).unwrap().ranges().count(),
        support::RANGES.len()
    );
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
    for timeout_ms in [0, 300_001, u32::MAX] {
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
            timeout_ms: 300_000,
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
fn switching_a_class_off_only_ever_narrows_the_programmed_ranges() {
    let policy = policy();
    let programmed = ClassSet::NONE
        .with(TxClass::LicenceFreePlan, true)
        .with(TxClass::Amateur, true);
    assert_eq!(policy.classes(), programmed);
    let pmr = support::request(support::channel(TxClass::LicenceFreePlan));

    let no_amateur = policy.restricted(ClassSet::ALL.with(TxClass::Amateur, false));
    assert_eq!(
        no_amateur.authorise(&request()).unwrap_err(),
        TxDenied::Range
    );
    assert!(no_amateur.authorise(&pmr).is_ok());
    assert_eq!(
        no_amateur.classes(),
        ClassSet::NONE.with(TxClass::LicenceFreePlan, true)
    );

    // Switching on a class nothing programmed adds nothing.
    let pmr_only = TxPolicy::new(&support::RANGES[..1])
        .unwrap()
        .restricted(ClassSet::ALL);
    assert_eq!(pmr_only.authorise(&request()).unwrap_err(), TxDenied::Range);
    assert_eq!(policy.restricted(ClassSet::ALL), policy);
    assert_eq!(policy.restricted(ClassSet::NONE), TxPolicy::deny_all());
    assert_eq!(
        policy.restricted(ClassSet::NONE).restricted(ClassSet::ALL),
        TxPolicy::deny_all()
    );
}

#[test]
fn analog_fm_only() {
    let mut candidate = request();
    candidate.modulation = Modulation::Am;
    denied(&candidate, TxDenied::Modulation);
}

#[test]
fn every_changed_request_fact_invalidates_the_token() {
    let policy = policy();
    let original = request();
    let token = policy.authorise(&original).unwrap();
    for field in 0..15 {
        let mut current = original;
        match field {
            0 => current.channel.transmit = Frequency::from_hz(145_525_000).unwrap(),
            1 => current.channel.receive = Frequency::from_hz(145_525_000).unwrap(),
            2 => current.calibration = None,
            3 => current.modulation = Modulation::Am,
            4 => current.bandwidth = Bandwidth::Wide,
            5 => current.conducted_mw += 1,
            6 => current.timeout_ms += 1,
            7 => current.configuration.as_mut().unwrap().generation += 1,
            8 => current.configuration.as_mut().unwrap().instance += 1,
            9 => current.configuration.as_mut().unwrap().revision += 1,
            10 => current.state = TxState::Changing,
            11 => current.hardware.as_mut().unwrap().evidence_id += 1,
            12 => current.calibration.as_mut().unwrap().evidence_id += 1,
            13 => {
                current.hardware.as_mut().unwrap().ranges[0]
                    .as_mut()
                    .unwrap()
                    .max_conducted_mw += 1;
            }
            _ => current.target_id += 1,
        }
        assert!(
            !token.matches(&current),
            "changed field {field} reused authority"
        );
    }
}

#[test]
fn the_policy_stays_small_enough_for_the_k5() {
    assert!(core::mem::size_of::<TxPolicy>() <= 256);
}
