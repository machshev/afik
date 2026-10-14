//! Shared PTT sequencing over explicit time and acknowledged adapter effects.
//!
//! Adapters execute each effect once, synchronously while borrowing it, then
//! deliver its exact acknowledgement before executing another effect. They must
//! deliver current time/facts before acquiring an effect and arrange a tick at
//! the exposed deadline. No clock, register, PA binding, or recovery is inferred.

use radio_tx_policy::{TxAuthorisation, TxDenied, TxPolicy, TxRequest};

/// Semantic lifecycle state; adapter completion establishes each transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// Initial PA/TX shutdown and receive restoration are not complete.
    Starting,
    /// Receiving and ready for a fresh press.
    Ready,
    /// Receiving, but a release must precede another press.
    ReleaseRequired,
    /// Authorized entry effects are still being acknowledged.
    Entering,
    /// Every entry effect, including logical PA enable, was acknowledged.
    Transmitting,
    /// Shutdown effects are still being acknowledged.
    ShuttingDown,
    /// An adapter/time failure is latched; replacement requires safe startup.
    Faulted,
}

/// Abstract adapter operation; these names establish no hardware facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    /// Disable the PA independently of chip state, including after a fault.
    DisablePa,
    /// Disable modulation/TX unconditionally, including unknown chip state.
    DisableTransmit,
    /// Restore the current authoritative receive setup after confirmed shutdown.
    RestoreReceive,
    /// Prepare exact settings/power and silence receive output without enabling TX.
    PrepareTransmit,
    /// Enable chip/modulation TX with complete current policy authority.
    EnableTransmit,
    /// Enable the PA last, with complete current policy authority.
    EnablePa,
}

/// Reason for the most recent shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// Operator release.
    Released,
    /// Deadline measured from the accepted press, including entry time.
    Timeout,
    /// Configuration, policy, or other approved facts changed.
    Invalidated,
    /// Explicit target fault or failed adapter completion.
    Fault,
    /// Time moved backwards.
    Clock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    StartupPa,
    StartupTx,
    StartupReceive,
    Idle,
    EntryPa,
    Prepare,
    TxOn,
    PaOn,
    Transmitting,
    ExitPa,
    ExitTx,
    ExitReceive,
    FaultPa,
    FaultTx,
    Faulted,
}

impl Phase {
    const fn action(self) -> Option<Action> {
        match self {
            Self::StartupPa | Self::EntryPa | Self::ExitPa | Self::FaultPa => {
                Some(Action::DisablePa)
            }
            Self::StartupTx | Self::ExitTx | Self::FaultTx => Some(Action::DisableTransmit),
            Self::StartupReceive | Self::ExitReceive => Some(Action::RestoreReceive),
            Self::Prepare => Some(Action::PrepareTransmit),
            Self::TxOn => Some(Action::EnableTransmit),
            Self::PaOn => Some(Action::EnablePa),
            Self::Idle | Self::Transmitting | Self::Faulted => None,
        }
    }

    const fn active(self) -> bool {
        matches!(
            self,
            Self::EntryPa | Self::Prepare | Self::TxOn | Self::PaOn | Self::Transmitting
        )
    }
}

/// Opaque effect identity, distinct across phases, cycles, and app instances.
///
/// An adapter must never reuse the non-zero instance supplied to `Controller`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectId {
    instance: u32,
    cycle: u32,
    phase: Phase,
}

/// Exact completion reported by a trusted adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acknowledgement {
    id: EffectId,
    applied: bool,
}

impl Acknowledgement {
    /// Reports completion of the entire identified operation.
    pub const fn applied(id: EffectId) -> Self {
        Self { id, applied: true }
    }

    /// Reports failure, including partial application or unknown physical state.
    pub const fn failed(id: EffectId) -> Self {
        Self { id, applied: false }
    }
}

/// Explicit input; duplicate edges and obsolete acknowledgements are harmless.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// Current input became pressed; no automatic retry while held.
    Pressed,
    /// Current input became released; also clears the held-at-boot interlock.
    Released,
    /// Observe explicit monotonic milliseconds, including a deadline delivery.
    Tick,
    /// Invalidate authority before applying a configuration replacement.
    Invalidated,
    /// Request fault cleanup regardless of the current logical chip state.
    TargetFault,
    /// Complete exactly one adapter operation.
    Completed(Acknowledgement),
}

/// Input or authority rejection before an energizing effect can be obtained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A fresh, non-zero application instance is required.
    Instance,
    /// Central complete-request policy denied entry.
    Policy(TxDenied),
    /// Current facts differ from the accepted press.
    StaleRequest,
    /// Absolute deadline cannot be represented.
    DeadlineOverflow,
    /// Cycle identities cannot wrap; replace the controller with a fresh instance.
    Exhausted,
    /// A backwards timestamp caused fault cleanup.
    Clock,
}

/// One borrowed adapter operation. It cannot survive controller invalidation.
///
/// ```compile_fail
/// use radio_platform::tx_control::{Controller, Event};
/// use radio_tx_policy::{TxPolicy, TxRequest};
/// fn invalidate(controller: &mut Controller, policy: &TxPolicy, request: &TxRequest) {
///     let effect = controller.effect(policy, request).unwrap().unwrap();
///     controller.handle(0, Event::Invalidated, policy, request).unwrap();
///     let _ = effect.id();
/// }
/// ```
pub struct Effect<'a> {
    controller: &'a Controller,
    action: Action,
    current: &'a TxRequest,
    authorisation: Option<TxAuthorisation<'a>>,
}

impl Effect<'_> {
    /// Identity to acknowledge after dropping this borrowed operation.
    pub fn id(&self) -> EffectId {
        self.controller.id()
    }

    /// Abstract operation to execute exactly once.
    pub const fn action(&self) -> Action {
        self.action
    }

    /// Current complete request, also supplying the latest receive selection.
    pub const fn request(&self) -> &TxRequest {
        self.current
    }

    /// Fresh central authority for preparation and both energizing operations.
    pub const fn authorisation(&self) -> Option<&TxAuthorisation<'_>> {
        self.authorisation.as_ref()
    }
}

/// Heap-free PTT controller shared above target adapters.
pub struct Controller {
    instance: u32,
    cycle: u32,
    phase: Phase,
    pressed: bool,
    fault: bool,
    shutdown_ok: bool,
    now_ms: u64,
    deadline_ms: Option<u64>,
    accepted: Option<TxRequest>,
    stop_reason: Option<StopReason>,
}

impl Controller {
    /// Starts with explicit PA/TX shutdown, even when boot samples PTT released.
    pub const fn new(instance: u32, ptt_down: bool, now_ms: u64) -> Result<Self, Error> {
        if instance == 0 {
            return Err(Error::Instance);
        }
        Ok(Self {
            instance,
            cycle: 0,
            phase: Phase::StartupPa,
            pressed: ptt_down,
            fault: false,
            shutdown_ok: true,
            now_ms,
            deadline_ms: None,
            accepted: None,
            stop_reason: None,
        })
    }

    /// Returns the state established by exact adapter acknowledgements.
    pub const fn state(&self) -> State {
        match self.phase {
            Phase::StartupPa | Phase::StartupTx | Phase::StartupReceive => State::Starting,
            Phase::Idle if self.pressed => State::ReleaseRequired,
            Phase::Idle => State::Ready,
            Phase::EntryPa | Phase::Prepare | Phase::TxOn | Phase::PaOn => State::Entering,
            Phase::Transmitting => State::Transmitting,
            Phase::ExitPa
            | Phase::ExitTx
            | Phase::ExitReceive
            | Phase::FaultPa
            | Phase::FaultTx => State::ShuttingDown,
            Phase::Faulted => State::Faulted,
        }
    }

    /// Absolute explicit-time deadline, armed before the first entry effect.
    pub const fn deadline_ms(&self) -> Option<u64> {
        self.deadline_ms
    }

    /// Returns the most recent shutdown reason, if any.
    pub const fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    /// Delivers current time/facts before any subsequent adapter operation.
    pub fn handle(
        &mut self,
        now_ms: u64,
        event: Event,
        policy: &TxPolicy,
        current: &TxRequest,
    ) -> Result<(), Error> {
        if now_ms < self.now_ms {
            self.fault = true;
            self.shutdown(StopReason::Clock);
            return Err(Error::Clock);
        }
        self.now_ms = now_ms;
        if self.phase.active() {
            if self.deadline_ms.is_some_and(|deadline| now_ms >= deadline) {
                self.shutdown(StopReason::Timeout);
            } else if self.accepted.as_ref() != Some(current) || policy.authorise(current).is_err()
            {
                self.shutdown(StopReason::Invalidated);
            }
        }
        match event {
            Event::Pressed => {
                let ready = self.phase == Phase::Idle && !self.pressed;
                self.pressed = true;
                if ready {
                    policy.authorise(current).map_err(Error::Policy)?;
                    let deadline = now_ms
                        .checked_add(u64::from(current.timeout_ms))
                        .ok_or(Error::DeadlineOverflow)?;
                    self.next_cycle()?;
                    self.accepted = Some(*current);
                    self.deadline_ms = Some(deadline);
                    self.phase = Phase::EntryPa;
                    self.shutdown_ok = true;
                    self.stop_reason = None;
                }
            }
            Event::Released => {
                self.pressed = false;
                if self.phase.active() {
                    self.shutdown(StopReason::Released);
                }
            }
            Event::Invalidated => self.shutdown(StopReason::Invalidated),
            Event::TargetFault => {
                self.fault = true;
                self.shutdown(StopReason::Fault);
            }
            Event::Completed(ack) => {
                if self.phase.action().is_some() && ack.id == self.id() {
                    self.complete(ack.applied);
                }
            }
            Event::Tick => {}
        }
        Ok(())
    }

    /// Borrows the sole pending operation after observing current time/facts.
    ///
    /// The borrow prevents a safe adapter from retaining an operation across
    /// release/invalidation. Fresh policy/request checks also guard each entry
    /// effect. A rejection requires delivering `Invalidated` before continuing.
    pub fn effect<'a>(
        &'a self,
        policy: &'a TxPolicy,
        current: &'a TxRequest,
    ) -> Result<Option<Effect<'a>>, Error> {
        let Some(action) = self.phase.action() else {
            return Ok(None);
        };
        let authorisation = if self.phase.active() {
            if self.accepted.as_ref() != Some(current) {
                return Err(Error::StaleRequest);
            }
            let token = policy.authorise(current).map_err(Error::Policy)?;
            matches!(
                action,
                Action::PrepareTransmit | Action::EnableTransmit | Action::EnablePa
            )
            .then_some(token)
        } else {
            None
        };
        Ok(Some(Effect {
            controller: self,
            action,
            current,
            authorisation,
        }))
    }

    fn id(&self) -> EffectId {
        EffectId {
            instance: self.instance,
            cycle: self.cycle,
            phase: self.phase,
        }
    }

    fn next_cycle(&mut self) -> Result<(), Error> {
        self.cycle = self.cycle.checked_add(1).ok_or(Error::Exhausted)?;
        Ok(())
    }

    fn shutdown(&mut self, reason: StopReason) {
        if self.phase.active() || self.phase == Phase::Idle {
            if self.phase == Phase::Idle && self.next_cycle().is_err() {
                self.phase = Phase::Faulted;
                return;
            }
            self.phase = Phase::ExitPa;
            self.shutdown_ok = true;
            self.deadline_ms = None;
            self.accepted = None;
            self.stop_reason = Some(reason);
        }
    }

    fn finish(&mut self) {
        self.phase = if self.fault {
            Phase::Faulted
        } else {
            Phase::Idle
        };
    }

    fn complete(&mut self, applied: bool) {
        if !applied {
            self.fault = true;
            self.stop_reason = Some(StopReason::Fault);
        }
        match self.phase {
            Phase::StartupPa | Phase::ExitPa => {
                self.shutdown_ok &= applied;
                self.phase = if self.phase == Phase::StartupPa {
                    Phase::StartupTx
                } else {
                    Phase::ExitTx
                };
            }
            Phase::StartupTx | Phase::ExitTx => {
                self.shutdown_ok &= applied;
                self.phase = if !self.shutdown_ok {
                    Phase::Faulted
                } else if self.phase == Phase::StartupTx {
                    Phase::StartupReceive
                } else {
                    Phase::ExitReceive
                };
            }
            Phase::StartupReceive | Phase::ExitReceive => {
                if applied {
                    self.finish();
                } else {
                    self.phase = Phase::FaultPa;
                }
            }
            Phase::EntryPa | Phase::Prepare | Phase::TxOn | Phase::PaOn => {
                if applied {
                    self.phase = match self.phase {
                        Phase::EntryPa => Phase::Prepare,
                        Phase::Prepare => Phase::TxOn,
                        Phase::TxOn => Phase::PaOn,
                        _ => Phase::Transmitting,
                    };
                } else {
                    self.shutdown(StopReason::Fault);
                }
            }
            Phase::FaultPa => self.phase = Phase::FaultTx,
            Phase::FaultTx => self.phase = Phase::Faulted,
            Phase::Idle | Phase::Transmitting | Phase::Faulted => {}
        }
    }
}

#[cfg(test)]
#[path = "../../radio-tx-policy/tests/support/mod.rs"]
mod tx_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use radio_domain::TxClass;

    fn policy() -> TxPolicy {
        tx_fixture::policy(true)
    }

    fn request() -> TxRequest {
        tx_fixture::request(tx_fixture::channel(TxClass::Amateur))
    }

    fn pending(
        controller: &Controller,
        policy: TxPolicy,
        request: &TxRequest,
    ) -> (EffectId, Action) {
        let effect = controller.effect(&policy, request).unwrap().unwrap();
        (effect.id(), effect.action())
    }

    fn complete(
        controller: &mut Controller,
        now: u64,
        policy: TxPolicy,
        request: &TxRequest,
    ) -> Action {
        let (id, action) = pending(controller, policy, request);
        controller
            .handle(
                now,
                Event::Completed(Acknowledgement::applied(id)),
                &policy,
                request,
            )
            .unwrap();
        action
    }

    fn startup(instance: u32, down: bool) -> Controller {
        let mut controller = Controller::new(instance, down, 0).unwrap();
        let policy = policy();
        let request = request();
        for expected in [
            Action::DisablePa,
            Action::DisableTransmit,
            Action::RestoreReceive,
        ] {
            assert_eq!(complete(&mut controller, 0, policy, &request), expected);
        }
        controller
    }

    fn shutdown(controller: &mut Controller, now: u64, policy: TxPolicy, request: &TxRequest) {
        for expected in [
            Action::DisablePa,
            Action::DisableTransmit,
            Action::RestoreReceive,
        ] {
            assert_eq!(complete(controller, now, policy, request), expected);
        }
    }

    #[test]
    fn held_at_boot_and_press_during_startup_require_release() {
        let policy = policy();
        let request = request();
        let mut controller = startup(1, true);
        assert_eq!(controller.state(), State::ReleaseRequired);
        controller
            .handle(1, Event::Pressed, &policy, &request)
            .unwrap();
        assert!(controller.effect(&policy, &request).unwrap().is_none());
        controller
            .handle(2, Event::Released, &policy, &request)
            .unwrap();
        controller
            .handle(3, Event::Pressed, &policy, &request)
            .unwrap();
        assert_eq!(controller.state(), State::Entering);

        let mut controller = Controller::new(2, false, 0).unwrap();
        controller
            .handle(0, Event::Pressed, &policy, &request)
            .unwrap();
        for _ in 0..3 {
            complete(&mut controller, 0, policy, &request);
        }
        assert_eq!(controller.state(), State::ReleaseRequired);
        assert!(controller.effect(&policy, &request).unwrap().is_none());
    }

    #[test]
    fn denied_press_never_issues_an_entry_effect_or_retries_while_held() {
        let allowed = policy();
        let request = request();
        let mut controller = startup(1, false);
        assert_eq!(
            controller.handle(1, Event::Pressed, &TxPolicy::deny_all(), &request),
            Err(Error::Policy(TxDenied::Range))
        );
        assert_eq!(controller.state(), State::ReleaseRequired);
        controller
            .handle(2, Event::Pressed, &allowed, &request)
            .unwrap();
        assert!(controller.effect(&allowed, &request).unwrap().is_none());
        controller
            .handle(3, Event::Released, &allowed, &request)
            .unwrap();
        controller
            .handle(4, Event::Pressed, &allowed, &request)
            .unwrap();
        assert_eq!(controller.deadline_ms(), Some(1004));
        controller
            .handle(5, Event::Pressed, &allowed, &request)
            .unwrap();
        assert_eq!(controller.deadline_ms(), Some(1004));
    }

    #[test]
    fn release_and_invalidation_cancel_every_entry_boundary_and_stale_ack() {
        let policy = policy();
        let request = request();
        for event in [Event::Released, Event::Invalidated, Event::TargetFault] {
            for boundary in 0..=4 {
                let mut controller = startup(1, false);
                controller
                    .handle(1, Event::Pressed, &policy, &request)
                    .unwrap();
                let mut old_id = pending(&controller, policy, &request).0;
                for _ in 0..boundary {
                    old_id = pending(&controller, policy, &request).0;
                    complete(&mut controller, 2, policy, &request);
                }
                controller.handle(3, event, &policy, &request).unwrap();
                let cleanup_id = pending(&controller, policy, &request).0;
                controller
                    .handle(
                        3,
                        Event::Completed(Acknowledgement::applied(old_id)),
                        &policy,
                        &request,
                    )
                    .unwrap();
                assert_eq!(pending(&controller, policy, &request).0, cleanup_id);
                shutdown(&mut controller, 3, policy, &request);
                assert_eq!(
                    controller.state(),
                    match event {
                        Event::Released => State::Ready,
                        Event::Invalidated => State::ReleaseRequired,
                        _ => State::Faulted,
                    }
                );
            }
        }
    }

    #[test]
    fn exact_deadline_cancels_entry_completion_and_requires_release() {
        let policy = policy();
        let request = request();
        let mut controller = startup(1, false);
        controller
            .handle(10, Event::Pressed, &policy, &request)
            .unwrap();
        for _ in 0..3 {
            complete(&mut controller, 11, policy, &request);
        }
        let (id, action) = pending(&controller, policy, &request);
        assert_eq!(action, Action::EnablePa);
        controller
            .handle(
                1010,
                Event::Completed(Acknowledgement::applied(id)),
                &policy,
                &request,
            )
            .unwrap();
        assert_eq!(controller.stop_reason(), Some(StopReason::Timeout));
        assert_eq!(controller.deadline_ms(), None);
        shutdown(&mut controller, 1010, policy, &request);
        assert_eq!(controller.state(), State::ReleaseRequired);
        controller
            .handle(1011, Event::Pressed, &policy, &request)
            .unwrap();
        assert!(controller.effect(&policy, &request).unwrap().is_none());
    }

    #[test]
    fn fresh_request_and_policy_are_checked_before_each_entry_effect() {
        let policy = policy();
        let request = request();
        for boundary in 0..4 {
            let mut controller = startup(1, false);
            controller
                .handle(1, Event::Pressed, &policy, &request)
                .unwrap();
            for _ in 0..boundary {
                complete(&mut controller, 2, policy, &request);
            }
            let mut changed = request;
            changed.configuration.as_mut().unwrap().revision += 1;
            assert!(matches!(
                controller.effect(&policy, &changed),
                Err(Error::StaleRequest)
            ));
            assert!(matches!(
                controller.effect(&TxPolicy::deny_all(), &request),
                Err(Error::Policy(TxDenied::Range))
            ));
            controller
                .handle(3, Event::Tick, &policy, &changed)
                .unwrap();
            assert_eq!(controller.stop_reason(), Some(StopReason::Invalidated));
            assert_eq!(pending(&controller, policy, &changed).1, Action::DisablePa);
        }
    }

    #[test]
    fn acknowledgements_cannot_cross_cycles_phases_or_instances() {
        let policy = policy();
        let request = request();
        let mut first = startup(1, false);
        let mut second = startup(2, false);
        first.handle(1, Event::Pressed, &policy, &request).unwrap();
        second.handle(1, Event::Pressed, &policy, &request).unwrap();
        let old = pending(&first, policy, &request).0;
        second
            .handle(
                1,
                Event::Completed(Acknowledgement::applied(old)),
                &policy,
                &request,
            )
            .unwrap();
        assert_eq!(pending(&second, policy, &request).1, Action::DisablePa);
        complete(&mut first, 1, policy, &request);
        first
            .handle(
                1,
                Event::Completed(Acknowledgement::failed(old)),
                &policy,
                &request,
            )
            .unwrap();
        assert_eq!(pending(&first, policy, &request).1, Action::PrepareTransmit);
        first.handle(2, Event::Released, &policy, &request).unwrap();
        shutdown(&mut first, 2, policy, &request);
        first.handle(3, Event::Pressed, &policy, &request).unwrap();
        let current = pending(&first, policy, &request).0;
        first
            .handle(
                3,
                Event::Completed(Acknowledgement::applied(old)),
                &policy,
                &request,
            )
            .unwrap();
        assert_eq!(pending(&first, policy, &request).0, current);
    }

    #[test]
    fn invalid_identity_deadline_overflow_and_cycle_exhaustion_fail_closed() {
        assert!(matches!(Controller::new(0, false, 0), Err(Error::Instance)));
        let policy = policy();
        let request = request();
        let mut controller = startup(1, false);
        assert_eq!(
            controller.handle(u64::MAX, Event::Pressed, &policy, &request),
            Err(Error::DeadlineOverflow)
        );
        assert_eq!(controller.state(), State::ReleaseRequired);
        assert!(controller.effect(&policy, &request).unwrap().is_none());
        let mut controller = startup(2, false);
        controller.cycle = u32::MAX;
        assert_eq!(
            controller.handle(1, Event::Pressed, &policy, &request),
            Err(Error::Exhausted)
        );
        assert!(controller.effect(&policy, &request).unwrap().is_none());
        // The last accepted cycle still has distinct shutdown IDs; no increment
        // is needed to turn an active transmitter off.
        let mut controller = startup(3, false);
        controller.cycle = u32::MAX - 1;
        controller
            .handle(1, Event::Pressed, &policy, &request)
            .unwrap();
        for _ in 0..4 {
            complete(&mut controller, 1, policy, &request);
        }
        controller
            .handle(2, Event::Released, &policy, &request)
            .unwrap();
        shutdown(&mut controller, 2, policy, &request);
        assert_eq!(controller.state(), State::Ready);
    }

    #[test]
    fn backwards_time_orders_fault_shutdown_before_receive() {
        let policy = policy();
        let request = request();
        let mut controller = startup(1, false);
        controller
            .handle(10, Event::Pressed, &policy, &request)
            .unwrap();
        for _ in 0..4 {
            complete(&mut controller, 10, policy, &request);
        }
        assert_eq!(
            controller.handle(9, Event::Tick, &policy, &request),
            Err(Error::Clock)
        );
        assert_eq!(controller.stop_reason(), Some(StopReason::Clock));
        shutdown(&mut controller, 10, policy, &request);
        assert_eq!(controller.state(), State::Faulted);
    }
}
