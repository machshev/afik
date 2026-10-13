//! Synthetic host fixtures, never included in target firmware.

use radio_domain::{ActiveChannel, Bandwidth, Frequency, Modulation, TxClass};
use radio_tx_policy::regional::{Attestation, Attestations, GrantProfile, Region, RuleRange};
use radio_tx_policy::{ConfigurationIdentity, OperatingConditions, TxEnvelope, TxRequest, TxState};

#[allow(dead_code)] // Some consumers already own their channel fixture.
pub fn channel(class: TxClass) -> ActiveChannel {
    let hz = if class == TxClass::LicenceFreePlan {
        446_006_250
    } else {
        145_500_000
    };
    ActiveChannel {
        receive: Frequency::from_hz(hz).unwrap(),
        transmit: Frequency::from_hz(hz).unwrap(),
        tx_class: class,
    }
}

pub fn request(channel: ActiveChannel) -> TxRequest {
    let range = RuleRange {
        first_hz: channel.transmit.as_hz(),
        last_hz: channel.transmit.as_hz(),
        max_conducted_mw: 500,
        wide_fm: false,
    };
    let envelope = TxEnvelope {
        target_id: 1,
        evidence_id: 1,
        ranges: [Some(range), None, None, None],
        resolution_hz: 10,
    };
    TxRequest {
        target_id: 1,
        channel,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        conducted_mw: 100,
        timeout_ms: 1_000,
        state: TxState::Selected,
        configuration: Some(ConfigurationIdentity {
            instance: 1,
            generation: 0,
            revision: 1,
        }),
        region: Region::UnitedKingdom,
        attestations: Attestations {
            location: Attestation::Attested,
            licence_valid: Attestation::Attested,
            operator_authorised: Attestation::Attested,
            equipment_eligible: Attestation::Attested,
        },
        grant: matches!(
            channel.tx_class,
            TxClass::Marine | TxClass::Aeronautical | TxClass::Business | TxClass::Experimental
        )
        .then_some(GrantProfile {
            region: Region::UnitedKingdom,
            class: channel.tx_class,
            grant_id: 1,
            ranges: [Some(range), None, None, None, None, None, None, None],
            complete: true,
        }),
        hardware: Some(envelope),
        calibration: Some(TxEnvelope {
            evidence_id: 2,
            ..envelope
        }),
        operating: OperatingConditions {
            mobile: Attestation::Attested,
            non_airborne: Attestation::Attested,
            receiver_capable: Attestation::Attested,
        },
    }
}
