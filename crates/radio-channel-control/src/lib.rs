//! Bounded hardware-independent channel activation and scanning.

#![no_std]
#![forbid(unsafe_code)]

mod banked;

pub use banked::{
    BankedReceiveController, ChannelActivation as BankedChannelActivation, ChannelMemory,
    ChannelReceiveSetup, ChannelSelection, ChannelSource, MemoryFull, ReceiveError, ReceiveMode,
    ReceiveObservation, ReceiveState, ReceiveUpdate, ScanPhase as BankedScanPhase,
};

use core::fmt;
use radio_channel_plan::{GeneratedBank, PlanError};
use radio_domain::{ActiveChannel, SignalMeasurement};
use radio_tx_policy::{TxAuthorisation, TxPolicy};

/// Explicit logical scan durations in integer milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanConfig {
    dwell_ms: u32,
    hold_ms: u32,
}

impl ScanConfig {
    /// Validates non-zero dwell and hold durations.
    pub const fn new(dwell_ms: u32, hold_ms: u32) -> Result<Self, ScanConfigError> {
        if dwell_ms == 0 || hold_ms == 0 {
            Err(ScanConfigError)
        } else {
            Ok(Self { dwell_ms, hold_ms })
        }
    }

    /// Returns the configured no-signal dwell duration.
    pub const fn dwell_ms(self) -> u32 {
        self.dwell_ms
    }

    /// Returns the configured open-squelch hold duration.
    pub const fn hold_ms(self) -> u32 {
        self.hold_ms
    }
}

/// A scan duration was zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanConfigError;

impl fmt::Display for ScanConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("scan dwell and hold durations must be non-zero")
    }
}

/// Opaque identity of one currently armed logical timer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TimerToken(u32);

impl TimerToken {
    /// Returns the bounded token value for traces and adapters.
    pub const fn get(self) -> u32 {
        self.0
    }

    pub(crate) const fn new(value: u32) -> Self {
        Self(value)
    }
}

/// Required adapter change after one controller input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlUpdate {
    /// Exact channel to activate, if this input changes selection.
    pub activation: Option<ChannelActivation>,
    /// Logical timer operation required by the new state.
    pub timer: TimerDirective,
}

impl ControlUpdate {
    const fn unchanged() -> Self {
        Self {
            activation: None,
            timer: TimerDirective::Unchanged,
        }
    }
}

/// One checked generated-bank channel activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelActivation {
    /// Zero-based generated-bank index.
    pub index: u16,
    /// Fully expanded receive/transmit frequencies and policy class.
    pub channel: ActiveChannel,
}

/// Timer operation emitted for an external logical scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerDirective {
    /// Preserve the currently armed timer, if any.
    Unchanged,
    /// Cancel any currently armed timer.
    Cancel,
    /// Arm one duration under a fresh opaque token.
    Arm {
        /// Token which must accompany the later expiry input.
        token: TimerToken,
        /// Explicit configured duration in integer milliseconds.
        after_ms: u32,
    },
}

/// Current hardware-independent channel-control mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlState {
    /// One channel is selected and controller-level TX may be requested.
    Selected,
    /// A logical scan timer is active.
    Scanning(ScanPhase),
}

/// Current deterministic scan phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    /// Waiting for the no-signal dwell deadline.
    Dwell,
    /// Holding after an open-squelch observation.
    Hold {
        /// Most recently observed logical squelch state.
        squelch_open: bool,
    },
}

/// Channel-control input failed without a partial state change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    /// The generated-bank index was invalid or expansion failed.
    Plan(PlanError),
    /// The operation is not valid in the current controller state.
    InvalidState(ControlState),
    /// No fresh bounded timer token remained.
    TimerTokenExhausted,
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => write!(formatter, "channel activation failed: {error}"),
            Self::InvalidState(state) => {
                write!(formatter, "invalid channel-control state: {state:?}")
            }
            Self::TimerTokenExhausted => formatter.write_str("scan timer token exhausted"),
        }
    }
}

impl From<PlanError> for ControlError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

/// Selected-state TX request failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelTxError {
    /// Scanning state never exposes transmit authority.
    Scanning,
    /// Central policy denied the selected channel's class.
    PolicyDenied,
}

impl fmt::Display for ChannelTxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scanning => formatter.write_str("transmit denied while scanning"),
            Self::PolicyDenied => formatter.write_str("selected channel denied by TX policy"),
        }
    }
}

/// Exact selected channel paired with central-policy capability authority.
#[derive(Debug)]
pub struct AuthorisedTransmission {
    channel: ActiveChannel,
    authorisation: TxAuthorisation,
}

impl AuthorisedTransmission {
    /// Returns the selected active channel approved for the request.
    pub const fn channel(&self) -> ActiveChannel {
        self.channel
    }

    /// Borrows the shortest-scope capability required by a TX driver.
    pub const fn authorisation(&self) -> &TxAuthorisation {
        &self.authorisation
    }
}

/// One-bank activation and scanning controller with no clock or allocator.
pub struct ChannelController {
    bank: GeneratedBank,
    config: ScanConfig,
    index: u16,
    channel: ActiveChannel,
    state: ControlState,
    last_signal: Option<SignalMeasurement>,
    active_timer: Option<TimerToken>,
    next_timer_token: u32,
}

impl ChannelController {
    /// Checks and activates one initial generated-bank channel.
    pub fn activate(
        bank: GeneratedBank,
        index: u16,
        config: ScanConfig,
    ) -> Result<(Self, ControlUpdate), ControlError> {
        let channel = bank.channel(index)?;
        Ok((
            Self {
                bank,
                config,
                index,
                channel,
                state: ControlState::Selected,
                last_signal: None,
                active_timer: None,
                next_timer_token: 1,
            },
            ControlUpdate {
                activation: Some(ChannelActivation { index, channel }),
                timer: TimerDirective::Unchanged,
            },
        ))
    }

    /// Returns the current controller mode.
    pub const fn state(&self) -> ControlState {
        self.state
    }

    /// Returns the selected generated-bank index.
    pub const fn current_index(&self) -> u16 {
        self.index
    }

    /// Returns the currently active channel.
    pub const fn current_channel(&self) -> ActiveChannel {
        self.channel
    }

    /// Returns the most recent signal sample since activation, if any.
    pub const fn last_signal(&self) -> Option<SignalMeasurement> {
        self.last_signal
    }

    /// Selects one exact channel and stops any scan.
    pub fn select(&mut self, index: u16) -> Result<ControlUpdate, ControlError> {
        let channel = self.bank.channel(index)?;
        self.index = index;
        self.channel = channel;
        self.state = ControlState::Selected;
        self.last_signal = None;
        self.active_timer = None;
        Ok(ControlUpdate {
            activation: Some(ChannelActivation { index, channel }),
            timer: TimerDirective::Cancel,
        })
    }

    /// Selects the next channel with exact bank wraparound.
    pub fn select_next(&mut self) -> Result<ControlUpdate, ControlError> {
        self.select(self.next_index())
    }

    /// Selects the previous channel with exact bank wraparound.
    pub fn select_previous(&mut self) -> Result<ControlUpdate, ControlError> {
        let index = if self.index == 0 {
            self.bank.channel_count() - 1
        } else {
            self.index - 1
        };
        self.select(index)
    }

    /// Starts scanning the current channel and arms its dwell timer.
    pub fn start_scanning(&mut self) -> Result<ControlUpdate, ControlError> {
        if !matches!(self.state, ControlState::Selected) {
            return Err(ControlError::InvalidState(self.state));
        }
        let timer = self.fresh_timer(self.config.dwell_ms())?;
        self.state = ControlState::Scanning(ScanPhase::Dwell);
        self.last_signal = None;
        Ok(ControlUpdate {
            activation: None,
            timer,
        })
    }

    /// Stops scanning on the current channel and cancels its timer.
    pub fn stop_scanning(&mut self) -> Result<ControlUpdate, ControlError> {
        if !matches!(self.state, ControlState::Scanning(_)) {
            return Err(ControlError::InvalidState(self.state));
        }
        self.state = ControlState::Selected;
        self.active_timer = None;
        Ok(ControlUpdate {
            activation: None,
            timer: TimerDirective::Cancel,
        })
    }

    /// Applies one adapter-provided logical signal observation.
    pub fn observe_signal(
        &mut self,
        signal: SignalMeasurement,
    ) -> Result<ControlUpdate, ControlError> {
        if !matches!(self.state, ControlState::Scanning(_)) {
            self.last_signal = Some(signal);
            return Ok(ControlUpdate::unchanged());
        }

        if signal.squelch_open {
            let timer = self.fresh_timer(self.config.hold_ms())?;
            self.last_signal = Some(signal);
            self.state = ControlState::Scanning(ScanPhase::Hold { squelch_open: true });
            return Ok(ControlUpdate {
                activation: None,
                timer,
            });
        }

        self.last_signal = Some(signal);
        if matches!(self.state, ControlState::Scanning(ScanPhase::Hold { .. })) {
            self.state = ControlState::Scanning(ScanPhase::Hold {
                squelch_open: false,
            });
        }
        Ok(ControlUpdate::unchanged())
    }

    /// Applies one logical timer expiry; stale or cancelled tokens do nothing.
    pub fn timer_elapsed(&mut self, token: TimerToken) -> Result<ControlUpdate, ControlError> {
        if self.active_timer != Some(token) {
            return Ok(ControlUpdate::unchanged());
        }

        match self.state {
            ControlState::Selected => {
                self.active_timer = None;
                Ok(ControlUpdate::unchanged())
            }
            ControlState::Scanning(
                ScanPhase::Dwell
                | ScanPhase::Hold {
                    squelch_open: false,
                },
            ) => self.advance_scan(),
            ControlState::Scanning(ScanPhase::Hold { squelch_open: true }) => {
                let timer = self.fresh_timer(self.config.hold_ms())?;
                Ok(ControlUpdate {
                    activation: None,
                    timer,
                })
            }
        }
    }

    /// Requests central-policy authority only while one channel is selected.
    pub fn authorise_transmit(
        &self,
        policy: &TxPolicy,
    ) -> Result<AuthorisedTransmission, ChannelTxError> {
        if !matches!(self.state, ControlState::Selected) {
            return Err(ChannelTxError::Scanning);
        }
        let authorisation = policy
            .authorise(self.channel.tx_class)
            .map_err(|_| ChannelTxError::PolicyDenied)?;
        Ok(AuthorisedTransmission {
            channel: self.channel,
            authorisation,
        })
    }

    fn next_index(&self) -> u16 {
        if self.index + 1 == self.bank.channel_count() {
            0
        } else {
            self.index + 1
        }
    }

    fn advance_scan(&mut self) -> Result<ControlUpdate, ControlError> {
        let index = self.next_index();
        let channel = self.bank.channel(index)?;
        let timer = self.fresh_timer(self.config.dwell_ms())?;
        self.index = index;
        self.channel = channel;
        self.state = ControlState::Scanning(ScanPhase::Dwell);
        self.last_signal = None;
        Ok(ControlUpdate {
            activation: Some(ChannelActivation { index, channel }),
            timer,
        })
    }

    fn fresh_timer(&mut self, after_ms: u32) -> Result<TimerDirective, ControlError> {
        let next = self
            .next_timer_token
            .checked_add(1)
            .ok_or(ControlError::TimerTokenExhausted)?;
        let token = TimerToken(self.next_timer_token);
        self.next_timer_token = next;
        self.active_timer = Some(token);
        Ok(TimerDirective::Arm { token, after_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelController, ChannelTxError, ControlError, ControlState, ScanConfig, ScanConfigError,
        ScanPhase, TimerDirective,
    };
    use radio_channel_plan::{BankName, GeneratedBank, PlanError};
    use radio_domain::{BankId, Frequency, FrequencyStep, SignalMeasurement, TxClass};
    use radio_tx_policy::{PermissionSet, StoredPermissions, TxPolicy};

    fn bank() -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(7),
            BankName::new("THREE").unwrap(),
            Frequency::from_hz(145_500_000).unwrap(),
            FrequencyStep::from_hz(25_000).unwrap(),
            3,
            TxClass::Amateur,
        )
        .unwrap()
    }

    fn config() -> ScanConfig {
        ScanConfig::new(10, 30).unwrap()
    }

    fn controller(index: u16) -> ChannelController {
        ChannelController::activate(bank(), index, config())
            .unwrap()
            .0
    }

    fn armed(update: super::ControlUpdate, duration: u32) -> super::TimerToken {
        match update.timer {
            TimerDirective::Arm { token, after_ms } => {
                assert_eq!(after_ms, duration);
                token
            }
            directive => panic!("expected armed timer, got {directive:?}"),
        }
    }

    fn signal(strength: u8, squelch_open: bool) -> SignalMeasurement {
        SignalMeasurement {
            strength,
            squelch_open,
        }
    }

    fn policy(enabled: bool) -> TxPolicy {
        let permissions = PermissionSet::none().with(TxClass::Amateur, enabled);
        TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0
    }

    #[test]
    fn timing_and_initial_activation_are_checked() {
        assert_eq!(ScanConfig::new(0, 1), Err(ScanConfigError));
        assert_eq!(ScanConfig::new(1, 0), Err(ScanConfigError));
        let (_, update) = ChannelController::activate(bank(), 1, config()).unwrap();
        let activation = update.activation.unwrap();
        assert_eq!(activation.index, 1);
        assert_eq!(activation.channel.receive.as_hz(), 145_525_000);
        assert_eq!(update.timer, TimerDirective::Unchanged);
        assert!(matches!(
            ChannelController::activate(bank(), 3, config()),
            Err(ControlError::Plan(PlanError::ChannelOutOfRange))
        ));
    }

    #[test]
    fn manual_navigation_wraps_and_invalid_selection_is_non_mutating() {
        let mut control = controller(0);
        let previous = control.select_previous().unwrap().activation.unwrap();
        assert_eq!(previous.index, 2);
        assert_eq!(previous.channel.receive.as_hz(), 145_550_000);
        assert_eq!(control.select_next().unwrap().activation.unwrap().index, 0);

        let before = (
            control.state(),
            control.current_index(),
            control.current_channel(),
        );
        assert_eq!(
            control.select(3),
            Err(ControlError::Plan(PlanError::ChannelOutOfRange))
        );
        assert_eq!(
            (
                control.state(),
                control.current_index(),
                control.current_channel()
            ),
            before
        );
    }

    #[test]
    fn dwell_expiry_advances_once_wraps_and_cancelled_tokens_are_stale() {
        let mut control = controller(2);
        let first = armed(control.start_scanning().unwrap(), 10);
        assert_eq!(control.state(), ControlState::Scanning(ScanPhase::Dwell));

        let update = control.timer_elapsed(first).unwrap();
        assert_eq!(update.activation.unwrap().index, 0);
        let second = armed(update, 10);
        assert_eq!(control.current_index(), 0);
        assert_eq!(
            control.stop_scanning().unwrap().timer,
            TimerDirective::Cancel
        );
        assert_eq!(
            control.timer_elapsed(second).unwrap(),
            super::ControlUpdate::unchanged()
        );
        assert_eq!(control.current_index(), 0);
    }

    #[test]
    fn open_squelch_restarts_and_rearms_hold_until_closed() {
        let mut control = controller(0);
        let dwell = armed(control.start_scanning().unwrap(), 10);
        let first_hold = armed(control.observe_signal(signal(80, true)).unwrap(), 30);
        assert_eq!(control.last_signal(), Some(signal(80, true)));
        assert_eq!(
            control.timer_elapsed(dwell).unwrap(),
            super::ControlUpdate::unchanged()
        );

        let restarted_hold = armed(control.observe_signal(signal(90, true)).unwrap(), 30);
        assert_eq!(
            control.timer_elapsed(first_hold).unwrap(),
            super::ControlUpdate::unchanged()
        );
        let still_open = control.timer_elapsed(restarted_hold).unwrap();
        assert!(still_open.activation.is_none());
        let final_hold = armed(still_open, 30);

        assert_eq!(
            control.observe_signal(signal(20, false)).unwrap().timer,
            TimerDirective::Unchanged
        );
        assert_eq!(
            control.state(),
            ControlState::Scanning(ScanPhase::Hold {
                squelch_open: false
            })
        );
        let advanced = control.timer_elapsed(final_hold).unwrap();
        assert_eq!(advanced.activation.unwrap().index, 1);
        armed(advanced, 10);
        assert_eq!(control.last_signal(), None);
    }

    #[test]
    fn invalid_start_stop_and_exhausted_timer_are_non_mutating() {
        let mut control = controller(1);
        assert_eq!(
            control.stop_scanning(),
            Err(ControlError::InvalidState(ControlState::Selected))
        );
        control.next_timer_token = u32::MAX;
        assert_eq!(
            control.start_scanning(),
            Err(ControlError::TimerTokenExhausted)
        );
        assert_eq!(control.state(), ControlState::Selected);
        assert_eq!(control.last_signal(), None);
    }

    #[test]
    fn scan_denies_tx_and_selected_state_uses_exact_policy_class() {
        let mut control = controller(1);
        assert_eq!(
            control.authorise_transmit(&policy(false)).unwrap_err(),
            ChannelTxError::PolicyDenied
        );
        let transmission = control.authorise_transmit(&policy(true)).unwrap();
        assert_eq!(transmission.channel(), control.current_channel());
        assert_eq!(transmission.authorisation().class(), TxClass::Amateur);

        control.start_scanning().unwrap();
        assert_eq!(
            control.authorise_transmit(&policy(true)).unwrap_err(),
            ChannelTxError::Scanning
        );
    }

    #[test]
    fn invalid_manual_selection_preserves_an_active_scan_timer() {
        let mut control = controller(0);
        let timer = armed(control.start_scanning().unwrap(), 10);
        assert_eq!(
            control.select(4),
            Err(ControlError::Plan(PlanError::ChannelOutOfRange))
        );
        assert_eq!(control.state(), ControlState::Scanning(ScanPhase::Dwell));
        assert_eq!(
            control
                .timer_elapsed(timer)
                .unwrap()
                .activation
                .unwrap()
                .index,
            1
        );
    }
}
