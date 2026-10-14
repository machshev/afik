//! Deterministic PTT lifecycle and partial-failure integration tests.

use radio_bk4819::DriverState;
use radio_domain::{Frequency, TxClass};
use radio_platform::tx_control::{Action, Error, Event, State, StopReason};
use radio_sim::tx_control::{TraceKind, TxSimulator};
use radio_sim::RfTraceKind;
use radio_tx_policy::{TxDenied, TxPolicy, TxRequest};

#[path = "../../radio-tx-policy/tests/support/mod.rs"]
mod fixture;

fn policy() -> TxPolicy {
    fixture::policy(true)
}

fn request() -> TxRequest {
    fixture::request(fixture::channel(TxClass::Amateur))
}

fn drain(sim: &mut TxSimulator, policy: TxPolicy, request: &TxRequest) {
    for _ in 0..12 {
        if !sim.step(&policy, request).unwrap() {
            return;
        }
    }
    panic!("controller did not settle within its bounded effect sequence");
}

fn ready() -> TxSimulator {
    let mut sim = TxSimulator::new(1, false).unwrap();
    drain(&mut sim, policy(), &request());
    assert_eq!(sim.state(), State::Ready);
    sim
}

fn actions(sim: &TxSimulator) -> Vec<(Action, bool)> {
    sim.trace()
        .iter()
        .filter_map(|event| match event.kind {
            TraceKind::Completed { action, applied } => Some((action, applied)),
            _ => None,
        })
        .collect()
}

fn transmitting(sim: &mut TxSimulator, policy: TxPolicy, request: &TxRequest) {
    sim.deliver(Event::Pressed, &policy, request).unwrap();
    drain(sim, policy, request);
    assert_eq!(sim.state(), State::Transmitting);
    assert!(sim.pa_enabled());
}

#[test]
fn repeated_timed_scripts_have_identical_control_and_rf_traces() {
    fn run() -> (
        Vec<radio_sim::tx_control::TraceEvent>,
        Vec<radio_sim::RfTraceEvent>,
    ) {
        let policy = policy();
        let request = request();
        let mut sim = ready();
        sim.advance_ms(10);
        transmitting(&mut sim, policy, &request);
        sim.advance_ms(5);
        sim.deliver(Event::Released, &policy, &request).unwrap();
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Ready);
        assert!(!sim.pa_enabled());
        assert_eq!(
            sim.rf_state(),
            DriverState::Receiving {
                frequency: request.channel.receive
            }
        );
        sim.advance_ms(5);
        transmitting(&mut sim, policy, &request);
        sim.advance_ms(u64::from(request.timeout_ms));
        // Advancing virtual time performs no implicit work.
        assert!(sim.pa_enabled());
        drain(&mut sim, policy, &request);
        assert_eq!(sim.controller().stop_reason(), Some(StopReason::Timeout));
        assert_eq!(sim.state(), State::ReleaseRequired);
        assert!(!sim.pa_enabled());
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        assert!(!sim.step(&policy, &request).unwrap());
        (sim.trace().to_vec(), sim.rf_trace())
    }
    assert_eq!(run(), run());
}

#[test]
fn held_boot_and_complete_policy_denials_emit_no_tx_write_or_pa_enable() {
    let allowed = policy();
    let request = request();
    let mut held = TxSimulator::new(1, true).unwrap();
    drain(&mut held, allowed, &request);
    held.deliver(Event::Pressed, &allowed, &request).unwrap();
    assert!(!held.step(&allowed, &request).unwrap());
    assert_eq!(held.state(), State::ReleaseRequired);

    let mut sim = ready();
    let before = sim.rf_trace();
    assert_eq!(
        sim.deliver(Event::Pressed, &TxPolicy::deny_all(), &request),
        Err(Error::Policy(TxDenied::Range))
    );
    for field in 0..5 {
        let mut candidate = request;
        match field {
            0 => candidate.hardware = None,
            1 => candidate.calibration = None,
            2 => candidate.configuration = None,
            3 => candidate.timeout_ms = 0,
            _ => candidate.conducted_mw = 501,
        }
        sim.deliver(Event::Released, &allowed, &candidate).unwrap();
        assert!(sim.deliver(Event::Pressed, &allowed, &candidate).is_err());
        assert!(!sim.step(&allowed, &candidate).unwrap());
    }
    assert_eq!(sim.rf_trace(), before);
    assert!(!actions(&sim)
        .iter()
        .any(|(action, _)| *action == Action::EnablePa));
    assert!(!sim.pa_enabled());
}

#[test]
fn changed_request_or_policy_before_pa_enable_orders_shutdown() {
    let policy = policy();
    let request = request();
    for field in 0..5 {
        let mut sim = ready();
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        for _ in 0..3 {
            assert!(sim.step(&policy, &request).unwrap());
        }
        assert!(matches!(sim.rf_state(), DriverState::Transmitting { .. }));
        assert!(!sim.pa_enabled());
        let mut changed = request;
        let mut current_policy = policy;
        match field {
            0 => changed.configuration.as_mut().unwrap().generation += 1,
            1 => changed.conducted_mw += 1,
            2 => changed.hardware.as_mut().unwrap().evidence_id += 1,
            3 => changed.channel.receive = Frequency::from_hz(145_525_000).unwrap(),
            _ => current_policy = TxPolicy::deny_all(),
        }
        drain(&mut sim, current_policy, &changed);
        assert_eq!(
            sim.controller().stop_reason(),
            Some(StopReason::Invalidated)
        );
        assert!(!sim.pa_enabled());
        assert!(!actions(&sim)
            .iter()
            .any(|(action, _)| *action == Action::EnablePa));
        assert_eq!(
            sim.rf_state(),
            DriverState::Receiving {
                frequency: changed.channel.receive
            }
        );
        assert_eq!(
            &actions(&sim)[6..],
            &[
                (Action::DisablePa, true),
                (Action::DisableTransmit, true),
                (Action::RestoreReceive, true)
            ]
        );
    }
}

#[test]
fn timeout_during_entry_prevents_pa_enable_and_does_not_renew_on_press() {
    let policy = policy();
    let request = request();
    let mut sim = ready();
    sim.deliver(Event::Pressed, &policy, &request).unwrap();
    for _ in 0..3 {
        sim.step(&policy, &request).unwrap();
    }
    sim.advance_ms(u64::from(request.timeout_ms));
    sim.deliver(Event::Pressed, &policy, &request).unwrap();
    drain(&mut sim, policy, &request);
    assert_eq!(sim.controller().stop_reason(), Some(StopReason::Timeout));
    assert_eq!(sim.state(), State::ReleaseRequired);
    assert!(!actions(&sim)
        .iter()
        .any(|(action, _)| *action == Action::EnablePa));
    assert!(!sim.pa_enabled());
}

#[test]
fn every_entry_effect_failure_cleans_up_partial_application_before_receive() {
    let policy = policy();
    let request = request();
    for boundary in 0..4 {
        let mut sim = ready();
        sim.inject_effect_failure_after(boundary);
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        assert_eq!(
            sim.rf_state(),
            DriverState::Receiving {
                frequency: request.channel.receive
            }
        );
        let completed = actions(&sim);
        let failed = completed.iter().position(|(_, applied)| !applied).unwrap();
        assert_eq!(
            &completed[failed + 1..],
            &[
                (Action::DisablePa, true),
                (Action::DisableTransmit, true),
                (Action::RestoreReceive, true)
            ]
        );
        sim.deliver(Event::Released, &policy, &request).unwrap();
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        assert!(!sim.step(&policy, &request).unwrap());
    }
}

#[test]
fn every_startup_failure_latches_fault_and_attempts_both_shutdown_operations() {
    let policy = policy();
    let request = request();
    for boundary in 0..3 {
        let mut sim = TxSimulator::new(1, true).unwrap();
        sim.inject_effect_failure_after(boundary);
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        let completed = actions(&sim);
        assert_eq!(completed[0].0, Action::DisablePa);
        assert_eq!(completed[1].0, Action::DisableTransmit);
        if boundary < 2 {
            assert_eq!(completed.len(), 2);
        } else {
            assert_eq!(
                &completed[3..],
                &[(Action::DisablePa, true), (Action::DisableTransmit, true)]
            );
        }
    }
}

#[test]
fn every_shutdown_failure_blocks_receive_or_neutralizes_failed_restore() {
    let policy = policy();
    let request = request();
    for boundary in 0..3 {
        let mut sim = ready();
        transmitting(&mut sim, policy, &request);
        sim.inject_effect_failure_after(boundary);
        sim.deliver(Event::Released, &policy, &request).unwrap();
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        assert_eq!(sim.rf_state(), DriverState::Standby);
        let completed = actions(&sim);
        let cleanup = &completed[7..];
        if boundary < 2 {
            assert_eq!(cleanup.len(), 2);
            assert!(!cleanup
                .iter()
                .any(|(action, _)| *action == Action::RestoreReceive));
        } else {
            assert_eq!(
                &cleanup[3..],
                &[(Action::DisablePa, true), (Action::DisableTransmit, true)]
            );
        }
    }
}

#[test]
fn each_tx_register_failure_neutralizes_unknown_chip_state() {
    let policy = policy();
    let request = request();
    for register in 0..4 {
        let mut sim = ready();
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        for _ in 0..2 {
            sim.step(&policy, &request).unwrap();
        }
        sim.inject_bus_failure_after(register);
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        assert_eq!(
            sim.rf_state(),
            DriverState::Receiving {
                frequency: request.channel.receive
            }
        );
        assert!(!sim
            .rf_trace()
            .iter()
            .any(|event| matches!(event.kind, RfTraceKind::TransmitStarted { .. })));
        assert!(!actions(&sim)
            .iter()
            .any(|(action, _)| *action == Action::EnablePa));
    }
}

#[test]
fn failed_tx_shutdown_never_restores_receive_and_target_fault_cleans_up() {
    let policy = policy();
    let request = request();
    let mut sim = ready();
    transmitting(&mut sim, policy, &request);
    sim.deliver(Event::TargetFault, &policy, &request).unwrap();
    assert!(sim.step(&policy, &request).unwrap()); // PA off
    sim.inject_bus_failure_after(0);
    drain(&mut sim, policy, &request);
    assert_eq!(sim.state(), State::Faulted);
    assert_eq!(sim.rf_state(), DriverState::Faulted);
    assert!(!sim.pa_enabled());
    assert_eq!(
        &actions(&sim)[7..],
        &[(Action::DisablePa, true), (Action::DisableTransmit, false)]
    );
}

#[test]
fn invalid_receive_state_is_not_restored_after_invalidation() {
    let policy = policy();
    let mut request = request();
    let mut sim = ready();
    transmitting(&mut sim, policy, &request);
    request.state = radio_tx_policy::TxState::Changing;
    drain(&mut sim, policy, &request);
    assert_eq!(sim.state(), State::Faulted);
    assert!(!sim.pa_enabled());
    assert_eq!(sim.rf_state(), DriverState::Standby);
}

#[test]
fn rejected_pa_shutdown_keeps_unknown_pa_state_and_still_disables_chip_tx() {
    let policy = policy();
    let request = request();
    let mut sim = ready();
    transmitting(&mut sim, policy, &request);
    sim.inject_effect_rejection_after(0);
    sim.deliver(Event::Released, &policy, &request).unwrap();
    drain(&mut sim, policy, &request);
    assert_eq!(sim.state(), State::Faulted);
    assert!(sim.pa_enabled()); // No fabricated successful PA-off observation.
    assert_eq!(sim.rf_state(), DriverState::Standby);
    assert_eq!(
        &actions(&sim)[7..],
        &[(Action::DisablePa, false), (Action::DisableTransmit, true)]
    );
}

#[test]
fn repeated_before_and_after_failure_scripts_match_at_every_effect_boundary() {
    fn run(
        boundary: usize,
        after: bool,
    ) -> (
        Vec<radio_sim::tx_control::TraceEvent>,
        Vec<radio_sim::RfTraceEvent>,
        State,
        bool,
    ) {
        let policy = policy();
        let request = request();
        let mut sim = TxSimulator::new(1, false).unwrap();
        if after {
            sim.inject_effect_failure_after(boundary);
        } else {
            sim.inject_effect_rejection_after(boundary);
        }
        drain(&mut sim, policy, &request);
        sim.advance_ms(10);
        sim.deliver(Event::Pressed, &policy, &request).unwrap();
        drain(&mut sim, policy, &request);
        sim.advance_ms(10);
        sim.deliver(Event::Released, &policy, &request).unwrap();
        drain(&mut sim, policy, &request);
        (
            sim.trace().to_vec(),
            sim.rf_trace(),
            sim.state(),
            sim.pa_enabled(),
        )
    }
    // Three startup, four entry, and three ordinary shutdown effects.
    for boundary in 0..10 {
        for after in [false, true] {
            let first = run(boundary, after);
            assert_eq!(first, run(boundary, after));
            assert_eq!(first.2, State::Faulted);
            if boundary != 7 || after {
                assert!(!first.3);
            }
        }
    }
}

#[test]
fn invalidation_while_pa_is_enabled_restores_the_latest_receive_selection() {
    let policy = policy();
    let mut request = request();
    let mut sim = ready();
    transmitting(&mut sim, policy, &request);
    request.configuration.as_mut().unwrap().revision += 1;
    request.channel.receive = Frequency::from_hz(145_550_000).unwrap();
    sim.deliver(Event::Invalidated, &policy, &request).unwrap();
    assert_eq!(sim.state(), State::ShuttingDown);
    assert!(sim.pa_enabled());
    sim.step(&policy, &request).unwrap();
    assert!(!sim.pa_enabled());
    drain(&mut sim, policy, &request);
    assert_eq!(sim.state(), State::ReleaseRequired);
    assert_eq!(
        sim.rf_state(),
        DriverState::Receiving {
            frequency: request.channel.receive
        }
    );
    assert_eq!(
        &actions(&sim)[7..],
        &[
            (Action::DisablePa, true),
            (Action::DisableTransmit, true),
            (Action::RestoreReceive, true)
        ]
    );
}

#[test]
fn each_partial_receive_restore_failure_runs_a_second_shutdown_without_retry() {
    let policy = policy();
    let request = request();
    for register in 0..4 {
        let mut sim = ready();
        transmitting(&mut sim, policy, &request);
        sim.deliver(Event::Released, &policy, &request).unwrap();
        for _ in 0..2 {
            sim.step(&policy, &request).unwrap();
        }
        sim.inject_bus_failure_after(register);
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        assert_eq!(sim.rf_state(), DriverState::Standby);
        assert_eq!(
            &actions(&sim)[9..],
            &[
                (Action::RestoreReceive, false),
                (Action::DisablePa, true),
                (Action::DisableTransmit, true)
            ]
        );
    }
}

#[test]
fn failures_in_second_shutdown_still_attempt_both_disables_and_latch_fault() {
    let policy = policy();
    let request = request();
    for boundary in 0..2 {
        let mut sim = ready();
        transmitting(&mut sim, policy, &request);
        sim.inject_effect_failure_after(2); // Fail receive restoration after applying it.
        sim.deliver(Event::Released, &policy, &request).unwrap();
        for _ in 0..3 {
            sim.step(&policy, &request).unwrap();
        }
        sim.inject_effect_rejection_after(boundary);
        drain(&mut sim, policy, &request);
        assert_eq!(sim.state(), State::Faulted);
        assert!(!sim.pa_enabled());
        assert_eq!(actions(&sim)[10].0, Action::DisablePa);
        assert_eq!(actions(&sim)[11].0, Action::DisableTransmit);
        assert_eq!(actions(&sim).len(), 12);
        assert!(!actions(&sim)[10 + boundary].1);
    }
}
