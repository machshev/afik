//! Simulator-only PTT composition; PA and power are logical observations.

use radio_bk4819::DriverState;
use radio_platform::tx_control::{Acknowledgement, Action, Controller, Error, Event, State};
use radio_tx_policy::{TxPolicy, TxRequest, TxState};

use crate::{RfSimulator, RfTraceEvent};

/// Timestamped PTT input or adapter completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    /// Explicit virtual milliseconds.
    pub at_ms: u64,
    /// Input, rejection, or attempted effect.
    pub kind: TraceKind,
}

/// Deterministic PTT observations; no physical PA or RF is modeled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceKind {
    /// One event delivered to the shared controller.
    Input(Event),
    /// A controller input or fresh-effect request was rejected.
    Rejected(Error),
    /// One whole adapter operation completed or failed.
    Completed {
        /// Abstract operation attempted.
        action: Action,
        /// Success of the entire operation.
        applied: bool,
    },
}

/// Shared PTT core composed with the existing logical RF driver.
///
/// Callers supply all policy/evidence facts. Advancing time alone does no work;
/// `deliver` or `step` observes deadlines before another effect can execute.
pub struct TxSimulator {
    controller: Controller,
    rf: RfSimulator,
    trace: Vec<TraceEvent>,
    pa_enabled: bool,
    prepared: Option<TxRequest>,
    effect_failure_after: Option<(usize, bool)>,
}

impl TxSimulator {
    /// Constructs a safe-startup controller without performing any operation.
    pub fn new(instance: u32, ptt_down: bool) -> Result<Self, Error> {
        Ok(Self {
            controller: Controller::new(instance, ptt_down, 0)?,
            rf: RfSimulator::new(),
            trace: Vec::new(),
            pa_enabled: false,
            prepared: None,
            effect_failure_after: None,
        })
    }

    /// Returns shared lifecycle state.
    pub const fn state(&self) -> State {
        self.controller.state()
    }

    /// Observes the shared contract, including deadline and shutdown reason.
    pub const fn controller(&self) -> &Controller {
        &self.controller
    }

    /// Returns the logical chip state.
    pub const fn rf_state(&self) -> DriverState {
        self.rf.state()
    }

    /// Returns the logical PA observation, with no GPIO or RF meaning.
    pub const fn pa_enabled(&self) -> bool {
        self.pa_enabled
    }

    /// Returns deterministic input/effect observations.
    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    /// Returns the existing logical register trace.
    pub fn rf_trace(&self) -> Vec<RfTraceEvent> {
        self.rf.trace()
    }

    /// Advances explicit virtual time without executing effects or inputs.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.rf.advance_ms(duration_ms);
    }

    /// Fails one completion after this many effect attempts.
    ///
    /// Failure is injected after applying the operation to exercise uncertain
    /// partial completion, including an enabled PA whose completion was lost.
    pub fn inject_effect_failure_after(&mut self, successful_effects: usize) {
        self.effect_failure_after = Some((successful_effects, true));
    }

    /// Rejects one effect before applying it, after this many effect attempts.
    pub fn inject_effect_rejection_after(&mut self, successful_effects: usize) {
        self.effect_failure_after = Some((successful_effects, false));
    }

    /// Fails one logical register operation at an exact command boundary.
    pub fn inject_bus_failure_after(&self, successful_operations: usize) {
        self.rf.inject_failure_after(successful_operations);
    }

    /// Delivers an event with current authoritative facts at explicit time.
    pub fn deliver(
        &mut self,
        event: Event,
        policy: &TxPolicy,
        current: &TxRequest,
    ) -> Result<(), Error> {
        self.record(TraceKind::Input(event));
        let result = self
            .controller
            .handle(self.rf.now_ms(), event, policy, current);
        if let Err(error) = result {
            self.record(TraceKind::Rejected(error));
        }
        result
    }

    /// Applies and exactly acknowledges one pending operation, after a time tick.
    pub fn step(&mut self, policy: &TxPolicy, current: &TxRequest) -> Result<bool, Error> {
        self.deliver(Event::Tick, policy, current)?;
        let effect = match self.controller.effect(policy, current) {
            Ok(Some(effect)) => effect,
            Ok(None) => return Ok(false),
            Err(error) => {
                self.record(TraceKind::Rejected(error));
                self.deliver(Event::Invalidated, policy, current)?;
                return Err(error);
            }
        };
        let action = effect.action();
        let id = effect.id();
        let failure = match self.effect_failure_after {
            Some((0, after_apply)) => {
                self.effect_failure_after = None;
                Some(after_apply)
            }
            Some((remaining, after_apply)) => {
                self.effect_failure_after = Some((remaining - 1, after_apply));
                None
            }
            None => None,
        };
        let applied = failure != Some(false)
            && match action {
                Action::DisablePa => {
                    self.pa_enabled = false;
                    true
                }
                Action::DisableTransmit => {
                    self.prepared = None;
                    self.rf.recover_to_standby().is_ok()
                }
                Action::RestoreReceive => {
                    current.state == TxState::Selected
                        && !self.pa_enabled
                        && self.rf.start_receive(current.channel.receive).is_ok()
                }
                Action::PrepareTransmit => {
                    let valid = effect
                        .authorisation()
                        .is_some_and(|token| token.matches(current));
                    if valid && !self.pa_enabled && self.rf.recover_to_standby().is_ok() {
                        self.prepared = Some(*current);
                        true
                    } else {
                        false
                    }
                }
                Action::EnableTransmit => {
                    self.prepared.as_ref() == Some(current)
                        && effect
                            .authorisation()
                            .is_some_and(|token| self.rf.start_transmit(current, token).is_ok())
                }
                Action::EnablePa => {
                    let valid = self.prepared.as_ref() == Some(current)
                        && effect.authorisation().is_some_and(|token| {
                            token.matches(current)
                                && self.rf.state()
                                    == DriverState::Transmitting {
                                        frequency: current.channel.transmit,
                                        class: token.class(),
                                    }
                        });
                    if valid {
                        self.pa_enabled = true;
                    }
                    valid
                }
            };
        let applied = applied && failure.is_none();
        self.record(TraceKind::Completed { action, applied });
        let ack = if applied {
            Acknowledgement::applied(id)
        } else {
            Acknowledgement::failed(id)
        };
        self.deliver(Event::Completed(ack), policy, current)?;
        Ok(true)
    }

    fn record(&mut self, kind: TraceKind) {
        self.trace.push(TraceEvent {
            at_ms: self.rf.now_ms(),
            kind,
        });
    }
}
