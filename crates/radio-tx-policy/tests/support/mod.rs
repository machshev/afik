//! Synthetic host fixtures, never included in target firmware.

use radio_domain::{ActiveChannel, Bandwidth, Frequency, Modulation, TxClass};
use radio_tx_policy::{
    ConfigurationIdentity, RuleRange, TxEnvelope, TxPolicy, TxRange, TxRequest, TxState,
};

/// Fixture PMR446 channel 1.
pub const PMR446_HZ: u32 = 446_006_250;
/// Fixture 2 m amateur simplex frequency.
pub const AMATEUR_HZ: u32 = 145_500_000;

#[allow(dead_code)] // Some consumers already own their channel fixture.
pub fn channel(class: TxClass) -> ActiveChannel {
    let hz = if class == TxClass::LicenceFreePlan {
        PMR446_HZ
    } else {
        AMATEUR_HZ
    };
    ActiveChannel {
        receive: Frequency::from_hz(hz).unwrap(),
        transmit: Frequency::from_hz(hz).unwrap(),
    }
}

/// Programmed PMR446 (500 mW, 12.5 kHz raster) and UK 2 m amateur ranges.
#[allow(dead_code)]
pub const RANGES: [TxRange; 2] = [
    TxRange {
        class: TxClass::LicenceFreePlan,
        first_hz: PMR446_HZ,
        last_hz: 446_193_750,
        max_mw: 500,
        wide_fm: false,
        raster_hz: 12_500,
    },
    TxRange {
        class: TxClass::Amateur,
        first_hz: 144_000_000,
        last_hz: 146_000_000,
        max_mw: 5_000,
        wide_fm: true,
        raster_hz: 0,
    },
];

/// A policy permitting the fixture ranges, or nothing.
#[allow(dead_code)]
pub fn policy(enabled: bool) -> TxPolicy {
    if enabled {
        TxPolicy::new(&RANGES).unwrap()
    } else {
        TxPolicy::deny_all()
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
        receive_only: false,
        bandwidth: Bandwidth::Narrow,
        conducted_mw: 100,
        timeout_ms: 1_000,
        state: TxState::Selected,
        configuration: Some(ConfigurationIdentity {
            instance: 1,
            generation: 0,
            revision: 1,
        }),
        hardware: Some(envelope),
        calibration: Some(TxEnvelope {
            evidence_id: 2,
            ..envelope
        }),
    }
}
