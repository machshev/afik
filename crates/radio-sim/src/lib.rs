//! Deterministic functional device and in-memory programmer transport.

#![forbid(unsafe_code)]

use core::{cmp, convert::Infallible};
use radio_bk4819::{
    Bk4819, DriverError as RfDriverError, DriverState as RfDriverState, FrequencyWord,
    ReceiveStatus, RegisterAddress, RegisterBus,
};
use radio_channel_control::{
    ChannelController, ChannelTxError, ControlError as ChannelControlError, ControlState,
    ControlUpdate as ChannelControlUpdate, ScanConfig, TimerDirective, TimerToken,
};
use radio_channel_plan::{GeneratedBank, PlanEncoding};
use radio_domain::{ActiveChannel, Frequency, SignalMeasurement, TxClass};
use radio_programmer::ProtocolTransport;
use radio_protocol::{
    decode_list_objects_request, encode_frame, Command, DeviceCapabilities, DeviceErrorCode, Frame,
    ObjectDescriptor, ObjectListPage, PayloadReader, PayloadWriter, ProtocolError, Service,
    StreamDecoder, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_LIST_OBJECTS_PER_PAGE,
    MAX_PAYLOAD, PROTOCOL_VERSION,
};
use radio_storage::{
    validate_object, ObjectKey, ObjectKind, StorageError, StorageObject, TransactionalStore,
    MAX_OBJECT_DATA, STORAGE_FORMAT_VERSION,
};
use radio_tx_policy::{LoadStatus, TxAuthorisation, TxPolicy, PERMISSION_RECORD_LEN};
use radio_ui::{BootUi, KeyEvent, KeySet, UiAction, UiView};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

/// Maximum configuration objects in the first simulated device profile.
pub const SIM_MAX_OBJECTS: usize = 8;

/// Explicitly advanced deterministic virtual clock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SimClock {
    now_ms: u64,
}

impl SimClock {
    /// Constructs a clock at virtual time zero.
    pub const fn new() -> Self {
        Self { now_ms: 0 }
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(self) -> u64 {
        self.now_ms
    }

    /// Advances virtual time by an exact duration.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(duration_ms);
    }
}

/// One deterministic boot-UI simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTraceEvent {
    /// Explicit virtual timestamp.
    pub at_ms: u64,
    /// Observable boot-UI event details.
    pub kind: UiTraceKind,
}

/// Observable boot-UI, persistence, and reboot events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiTraceKind {
    /// A controller boot loaded persisted state and selected an initial view.
    Booted {
        /// Logical keys held at the instant of boot.
        initial_keys: KeySet,
        /// Whether persisted permissions were valid or defaulted denied.
        load_status: LoadStatus,
        /// First semantic display view.
        view: UiView,
    },
    /// One logical key edge was handled.
    KeyHandled {
        /// Input edge delivered to the UI controller.
        event: KeyEvent,
        /// External side effect requested by the controller.
        action: UiAction,
        /// Semantic display view after handling the edge.
        view: UiView,
    },
    /// A complete permission record replaced simulated persisted bytes.
    PermissionsPersisted {
        /// Monotonic generation contained in the new record.
        generation: u32,
    },
}

/// Deterministic virtual-time harness for the boot-only permission UI.
///
/// Persisting a record does not alter `active_policy`. Only [`Self::reboot`]
/// validates the current persisted bytes and replaces the active policy.
pub struct UiSimulator {
    clock: SimClock,
    controller: BootUi,
    persisted_permissions: [u8; PERMISSION_RECORD_LEN],
    active_policy: TxPolicy,
    trace: Vec<UiTraceEvent>,
}

impl UiSimulator {
    /// Boots a simulator from one exact persisted permission record buffer.
    pub fn boot(persisted_permissions: [u8; PERMISSION_RECORD_LEN], initial_keys: KeySet) -> Self {
        let (active_policy, policy_status) = TxPolicy::load(&persisted_permissions);
        let (controller, ui_status) = BootUi::boot(&persisted_permissions, initial_keys);
        debug_assert_eq!(policy_status, ui_status);
        let first_event = UiTraceEvent {
            at_ms: 0,
            kind: UiTraceKind::Booted {
                initial_keys,
                load_status: ui_status,
                view: controller.view(),
            },
        };
        Self {
            clock: SimClock::new(),
            controller,
            persisted_permissions,
            active_policy,
            trace: vec![first_event],
        }
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Advances virtual time by an exact duration.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
    }

    /// Returns the current bounded semantic display view.
    pub fn view(&self) -> UiView {
        self.controller.view()
    }

    /// Returns the active policy loaded at the most recent simulated boot.
    pub const fn active_policy(&self) -> TxPolicy {
        self.active_policy
    }

    /// Returns the current simulated persisted permission bytes.
    pub const fn persisted_permissions(&self) -> &[u8; PERMISSION_RECORD_LEN] {
        &self.persisted_permissions
    }

    /// Returns the ordered deterministic boot-UI trace.
    pub fn trace(&self) -> &[UiTraceEvent] {
        &self.trace
    }

    /// Handles one logical key edge at the current virtual time.
    pub fn handle(&mut self, event: KeyEvent) -> UiAction {
        let action = self.controller.handle(event);
        self.record(UiTraceKind::KeyHandled {
            event,
            action,
            view: self.controller.view(),
        });
        if let UiAction::PersistPermissions(record) = action {
            self.persisted_permissions = record;
            let generation = u32::from_le_bytes([record[3], record[4], record[5], record[6]]);
            self.record(UiTraceKind::PermissionsPersisted { generation });
        }
        action
    }

    /// Reboots from current persisted bytes and replaces the active policy only
    /// after its redundant record validation succeeds or defaults denied.
    pub fn reboot(&mut self, initial_keys: KeySet) -> LoadStatus {
        let (active_policy, policy_status) = TxPolicy::load(&self.persisted_permissions);
        let (controller, ui_status) = BootUi::boot(&self.persisted_permissions, initial_keys);
        debug_assert_eq!(policy_status, ui_status);
        self.active_policy = active_policy;
        self.controller = controller;
        self.record(UiTraceKind::Booted {
            initial_keys,
            load_status: ui_status,
            view: self.controller.view(),
        });
        ui_status
    }

    fn record(&mut self, kind: UiTraceKind) {
        self.trace.push(UiTraceEvent {
            at_ms: self.clock.now_ms(),
            kind,
        });
    }
}

/// One deterministic BK4819 simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfTraceEvent {
    /// Explicit virtual timestamp.
    pub at_ms: u64,
    /// Observable command or logical register operation.
    pub kind: RfTraceKind,
}

/// Observable post-initialization RF command events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfTraceKind {
    /// A logical register write completed.
    RegisterWritten {
        /// Validated seven-bit register address.
        address: RegisterAddress,
        /// Complete 16-bit register value.
        value: u16,
    },
    /// A logical register read completed.
    RegisterRead {
        /// Validated seven-bit register address.
        address: RegisterAddress,
        /// Complete 16-bit register value.
        value: u16,
    },
    /// One logical operation was deliberately failed before completion.
    RegisterOperationFailed(RfBusOperation),
    /// A one-shot failure was armed after this many successful operations.
    FailureArmed {
        /// Successful operations permitted before the injected failure.
        successful_operations: usize,
    },
    /// Neutral mode was successfully established.
    StandbyRecovered,
    /// The receive command plan completed.
    ReceiveStarted {
        /// Requested receive frequency.
        frequency: Frequency,
    },
    /// One receive status sample completed.
    ReceiveStatusSampled(ReceiveStatus),
    /// The token-gated transmit command plan completed.
    TransmitStarted {
        /// Requested transmit frequency.
        frequency: Frequency,
        /// Policy class carried by the matching capability token.
        class: TxClass,
    },
    /// A known transmit session returned to neutral mode.
    TransmitStopped,
}

/// Logical bus operation selected for deterministic failure injection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RfBusOperation {
    /// Register write that did not complete.
    Write {
        /// Validated seven-bit register address.
        address: RegisterAddress,
        /// Requested complete 16-bit register value.
        value: u16,
    },
    /// Register read that did not complete.
    Read {
        /// Validated seven-bit register address.
        address: RegisterAddress,
    },
}

/// Deliberate deterministic failure from the simulated logical register bus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RfSimBusError;

impl core::fmt::Display for RfSimBusError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("injected simulated BK4819 bus failure")
    }
}

struct RfShared {
    now_ms: Cell<u64>,
    registers: RefCell<[u16; 128]>,
    trace: RefCell<Vec<RfTraceEvent>>,
    failure_after: Cell<Option<usize>>,
}

impl RfShared {
    fn record(&self, kind: RfTraceKind) {
        self.trace.borrow_mut().push(RfTraceEvent {
            at_ms: self.now_ms.get(),
            kind,
        });
    }

    fn operation_fails(&self) -> bool {
        match self.failure_after.get() {
            None => false,
            Some(0) => {
                self.failure_after.set(None);
                true
            }
            Some(remaining) => {
                self.failure_after.set(Some(remaining - 1));
                false
            }
        }
    }
}

struct RfSimBus {
    shared: Rc<RfShared>,
}

impl RegisterBus for RfSimBus {
    type Error = RfSimBusError;

    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
        if self.shared.operation_fails() {
            self.shared.record(RfTraceKind::RegisterOperationFailed(
                RfBusOperation::Write { address, value },
            ));
            return Err(RfSimBusError);
        }
        self.shared.registers.borrow_mut()[usize::from(address.get())] = value;
        self.shared
            .record(RfTraceKind::RegisterWritten { address, value });
        Ok(())
    }

    fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error> {
        if self.shared.operation_fails() {
            self.shared
                .record(RfTraceKind::RegisterOperationFailed(RfBusOperation::Read {
                    address,
                }));
            return Err(RfSimBusError);
        }
        let value = self.shared.registers.borrow()[usize::from(address.get())];
        self.shared
            .record(RfTraceKind::RegisterRead { address, value });
        Ok(value)
    }
}

/// Deterministic virtual-time harness around the evidence-bounded RF driver.
///
/// It models logical register completion and injected failures only. It does
/// not model a physical bus, chip initialization, board switching, or RF.
pub struct RfSimulator {
    clock: SimClock,
    driver: Bk4819<RfSimBus>,
    shared: Rc<RfShared>,
}

impl Default for RfSimulator {
    fn default() -> Self {
        Self::new()
    }
}

impl RfSimulator {
    /// Constructs an uninitialized logical driver at virtual time zero.
    pub fn new() -> Self {
        let shared = Rc::new(RfShared {
            now_ms: Cell::new(0),
            registers: RefCell::new([0; 128]),
            trace: RefCell::new(Vec::new()),
            failure_after: Cell::new(None),
        });
        Self {
            clock: SimClock::new(),
            driver: Bk4819::new(RfSimBus {
                shared: Rc::clone(&shared),
            }),
            shared,
        }
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Returns the driver's last fully established state.
    pub const fn state(&self) -> RfDriverState {
        self.driver.state()
    }

    /// Returns a stable snapshot of the ordered RF trace.
    pub fn trace(&self) -> Vec<RfTraceEvent> {
        self.shared.trace.borrow().clone()
    }

    /// Advances virtual time by an exact duration.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
        self.shared.now_ms.set(self.clock.now_ms());
    }

    /// Fails one later operation after exactly `successful_operations` succeed.
    pub fn inject_failure_after(&self, successful_operations: usize) {
        self.shared.failure_after.set(Some(successful_operations));
        self.shared.record(RfTraceKind::FailureArmed {
            successful_operations,
        });
    }

    /// Establishes neutral mode, including from unknown or faulted state.
    pub fn recover_to_standby(&mut self) -> Result<(), RfDriverError<RfSimBusError>> {
        self.driver.recover_to_standby()?;
        self.shared.record(RfTraceKind::StandbyRecovered);
        Ok(())
    }

    /// Runs the bounded receive command plan.
    pub fn start_receive(
        &mut self,
        frequency: Frequency,
    ) -> Result<(), RfDriverError<RfSimBusError>> {
        self.driver.start_receive(frequency)?;
        self.shared
            .record(RfTraceKind::ReceiveStarted { frequency });
        Ok(())
    }

    /// Samples receive status through the logical register model.
    pub fn receive_status(&mut self) -> Result<ReceiveStatus, RfDriverError<RfSimBusError>> {
        let status = self.driver.receive_status()?;
        self.shared
            .record(RfTraceKind::ReceiveStatusSampled(status));
        Ok(status)
    }

    /// Runs the transmit plan only through a matching central-policy token.
    pub fn start_transmit(
        &mut self,
        channel: ActiveChannel,
        authorisation: &TxAuthorisation,
    ) -> Result<(), RfDriverError<RfSimBusError>> {
        self.driver.start_transmit(channel, authorisation)?;
        self.shared.record(RfTraceKind::TransmitStarted {
            frequency: channel.transmit,
            class: channel.tx_class,
        });
        Ok(())
    }

    /// Stops a known transmit session and returns to neutral mode.
    pub fn stop_transmit(&mut self) -> Result<(), RfDriverError<RfSimBusError>> {
        self.driver.stop_transmit()?;
        self.shared.record(RfTraceKind::TransmitStopped);
        Ok(())
    }
}

/// One armed deterministic channel-scan timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelSimTimer {
    /// Opaque token required by the controller expiry input.
    pub token: TimerToken,
    /// Absolute virtual deadline in milliseconds.
    pub due_ms: u64,
}

/// One deterministic channel-control simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelTraceEvent {
    /// Explicit virtual timestamp.
    pub at_ms: u64,
    /// Observable channel-control input or completed action.
    pub kind: ChannelTraceKind,
}

/// Observable channel activation, scheduling, signal, and TX events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelTraceKind {
    /// One exact generated-bank channel completed RF activation.
    ChannelActivated {
        /// Zero-based generated-bank index.
        index: u16,
        /// Fully expanded active channel.
        channel: ActiveChannel,
    },
    /// Scanning started on the current channel.
    ScanStarted,
    /// Scanning stopped on the current channel.
    ScanStopped,
    /// A logical timer was armed.
    TimerArmed(ChannelSimTimer),
    /// The current logical timer was cancelled.
    TimerCancelled,
    /// A timer expiry was delivered to the controller.
    TimerDelivered {
        /// Token carried by the expiry input.
        token: TimerToken,
    },
    /// One normalized adapter signal sample was delivered.
    SignalObserved(SignalMeasurement),
    /// A controller-level TX request was denied before RF operations.
    TransmitDenied(ChannelTxError),
    /// A class-bound TX request completed through the RF simulator.
    TransmitStarted {
        /// Selected generated-bank index.
        index: u16,
        /// Exact centrally approved class.
        class: TxClass,
    },
    /// TX stopped and receive mode resumed on the selected channel.
    TransmitStopped,
}

/// Channel simulator command failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChannelSimError {
    /// The hardware-independent controller rejected an input.
    Control(ChannelControlError),
    /// The logical BK4819 driver rejected or faulted an operation.
    Radio(RfDriverError<RfSimBusError>),
    /// Controller-level TX authority was unavailable.
    Transmit(ChannelTxError),
    /// A current timer expiry arrived before its virtual deadline.
    TimerNotDue {
        /// Current virtual time.
        now_ms: u64,
        /// Armed deadline.
        due_ms: u64,
    },
    /// Channel activation or scanning was requested during simulated TX.
    Transmitting,
}

impl core::fmt::Display for ChannelSimError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Control(error) => write!(formatter, "channel controller failed: {error}"),
            Self::Radio(error) => write!(formatter, "channel RF command failed: {error}"),
            Self::Transmit(error) => write!(formatter, "channel TX request failed: {error}"),
            Self::TimerNotDue { now_ms, due_ms } => {
                write!(formatter, "scan timer due at {due_ms} ms, not {now_ms} ms")
            }
            Self::Transmitting => {
                formatter.write_str("channel control is unavailable during transmit")
            }
        }
    }
}

impl From<ChannelControlError> for ChannelSimError {
    fn from(error: ChannelControlError) -> Self {
        Self::Control(error)
    }
}

impl From<RfDriverError<RfSimBusError>> for ChannelSimError {
    fn from(error: RfDriverError<RfSimBusError>) -> Self {
        Self::Radio(error)
    }
}

/// Virtual-time integration of channel control, policy, and logical RF commands.
pub struct ChannelSimulator {
    clock: SimClock,
    controller: ChannelController,
    rf: RfSimulator,
    timer: Option<ChannelSimTimer>,
    transmitting: bool,
    trace: Vec<ChannelTraceEvent>,
}

impl ChannelSimulator {
    /// Activates one initial generated-bank channel at virtual time zero.
    pub fn activate(
        bank: GeneratedBank,
        index: u16,
        config: ScanConfig,
    ) -> Result<Self, ChannelSimError> {
        for channel_index in 0..bank.channel_count() {
            let channel = bank
                .channel(channel_index)
                .map_err(ChannelControlError::from)?;
            FrequencyWord::from_frequency(channel.receive).map_err(RfDriverError::Frequency)?;
            FrequencyWord::from_frequency(channel.transmit).map_err(RfDriverError::Frequency)?;
        }
        let (controller, update) = ChannelController::activate(bank, index, config)?;
        let mut rf = RfSimulator::new();
        rf.recover_to_standby()?;
        let mut simulator = Self {
            clock: SimClock::new(),
            controller,
            rf,
            timer: None,
            transmitting: false,
            trace: Vec::new(),
        };
        simulator.apply(update)?;
        Ok(simulator)
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Returns current hardware-independent control state.
    pub const fn state(&self) -> ControlState {
        self.controller.state()
    }

    /// Returns the current generated-bank index.
    pub const fn current_index(&self) -> u16 {
        self.controller.current_index()
    }

    /// Returns the current fully expanded channel.
    pub const fn current_channel(&self) -> ActiveChannel {
        self.controller.current_channel()
    }

    /// Returns the currently armed scan timer, if any.
    pub const fn timer(&self) -> Option<ChannelSimTimer> {
        self.timer
    }

    /// Returns the ordered channel-control trace.
    pub fn trace(&self) -> &[ChannelTraceEvent] {
        &self.trace
    }

    /// Returns a stable snapshot of underlying logical RF events.
    pub fn rf_trace(&self) -> Vec<RfTraceEvent> {
        self.rf.trace()
    }

    /// Advances both control and RF virtual time without implicit work.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
        self.rf.advance_ms(duration_ms);
    }

    /// Manually selects one exact channel and cancels scanning.
    pub fn select(&mut self, index: u16) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        let update = self.controller.select(index)?;
        self.apply(update)
    }

    /// Starts scanning the current channel.
    pub fn start_scanning(&mut self) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        let update = self.controller.start_scanning()?;
        self.record(ChannelTraceKind::ScanStarted);
        self.apply(update)
    }

    /// Stops scanning on the current channel.
    pub fn stop_scanning(&mut self) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        let update = self.controller.stop_scanning()?;
        self.record(ChannelTraceKind::ScanStopped);
        self.apply(update)
    }

    /// Delivers one normalized adapter signal sample.
    pub fn observe_signal(&mut self, signal: SignalMeasurement) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        self.record(ChannelTraceKind::SignalObserved(signal));
        let update = self.controller.observe_signal(signal)?;
        self.apply(update)
    }

    /// Delivers a timer token, enforcing the deadline only for the current arm.
    ///
    /// Old or cancelled tokens are still delivered so controller stale-token
    /// behavior remains visible and deterministic.
    pub fn deliver_timer(&mut self, token: TimerToken) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        if let Some(timer) = self.timer {
            if timer.token == token && self.now_ms() < timer.due_ms {
                return Err(ChannelSimError::TimerNotDue {
                    now_ms: self.now_ms(),
                    due_ms: timer.due_ms,
                });
            }
        }
        self.record(ChannelTraceKind::TimerDelivered { token });
        let update = self.controller.timer_elapsed(token)?;
        self.apply(update)
    }

    /// Requests selected-state policy authority and starts logical TX.
    pub fn start_transmit(&mut self, policy: &TxPolicy) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        match self.controller.authorise_transmit(policy) {
            Ok(transmission) => {
                self.rf
                    .start_transmit(transmission.channel(), transmission.authorisation())?;
                self.transmitting = true;
                self.record(ChannelTraceKind::TransmitStarted {
                    index: self.controller.current_index(),
                    class: transmission.authorisation().class(),
                });
                Ok(())
            }
            Err(error) => {
                self.record(ChannelTraceKind::TransmitDenied(error));
                Err(ChannelSimError::Transmit(error))
            }
        }
    }

    /// Stops logical TX and resumes receive on the still-selected channel.
    pub fn stop_transmit(&mut self) -> Result<(), ChannelSimError> {
        self.rf.stop_transmit()?;
        self.rf
            .start_receive(self.controller.current_channel().receive)?;
        self.transmitting = false;
        self.record(ChannelTraceKind::TransmitStopped);
        Ok(())
    }

    fn apply(&mut self, update: ChannelControlUpdate) -> Result<(), ChannelSimError> {
        if let Some(activation) = update.activation {
            self.rf.start_receive(activation.channel.receive)?;
            self.record(ChannelTraceKind::ChannelActivated {
                index: activation.index,
                channel: activation.channel,
            });
        }
        match update.timer {
            TimerDirective::Unchanged => {}
            TimerDirective::Cancel => {
                self.timer = None;
                self.record(ChannelTraceKind::TimerCancelled);
            }
            TimerDirective::Arm { token, after_ms } => {
                let timer = ChannelSimTimer {
                    token,
                    due_ms: self.now_ms().saturating_add(u64::from(after_ms)),
                };
                self.timer = Some(timer);
                self.record(ChannelTraceKind::TimerArmed(timer));
            }
        }
        Ok(())
    }

    fn require_not_transmitting(&self) -> Result<(), ChannelSimError> {
        if self.transmitting {
            Err(ChannelSimError::Transmitting)
        } else {
            Ok(())
        }
    }

    fn record(&mut self, kind: ChannelTraceKind) {
        self.trace.push(ChannelTraceEvent {
            at_ms: self.now_ms(),
            kind,
        });
    }
}

/// One deterministic simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    /// Explicit virtual timestamp.
    pub at_ms: u64,
    /// Observable event details.
    pub kind: TraceKind,
}

/// Observable protocol and storage events emitted by the simulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceKind {
    /// A valid request frame arrived.
    Request {
        /// Request sequence number.
        sequence: u16,
        /// Selected service.
        service: Service,
        /// Selected command.
        command: Command,
    },
    /// A malformed stream packet was discarded at a delimiter.
    PacketDiscarded(ProtocolError),
    /// An identical immediately repeated request reused its cached response.
    DuplicateRequestReplayed {
        /// Repeated request sequence number.
        sequence: u16,
    },
    /// A sequence was immediately reused for different request bytes.
    SequenceConflictRejected {
        /// Conflicting request sequence number.
        sequence: u16,
    },
    /// A candidate transaction began from the active generation.
    TransactionBegan {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// Active generation copied into the candidate.
        generation: u32,
    },
    /// One object was staged in the candidate snapshot.
    ObjectStaged {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// Stable object key.
        key: ObjectKey,
    },
    /// The complete candidate passed validation.
    TransactionValidated {
        /// Host-selected transaction identifier.
        transaction: u32,
    },
    /// A candidate transaction was explicitly discarded.
    TransactionAborted {
        /// Host-selected transaction identifier.
        transaction: u32,
    },
    /// A candidate became the active snapshot.
    TransactionCommitted {
        /// Host-selected transaction identifier.
        transaction: u32,
        /// New active generation.
        generation: u32,
    },
    /// An active configuration object was read.
    ObjectRead(ObjectKey),
    /// One deterministic page of active object descriptors was listed.
    ObjectsListed {
        /// Active storage generation described by the page.
        generation: u32,
        /// Zero-based object offset requested by the host.
        offset: u16,
        /// Number of descriptors returned in the page.
        count: u16,
    },
    /// A response frame was queued for the host.
    Response {
        /// Response sequence number.
        sequence: u16,
        /// Response command, including the error command.
        command: Command,
    },
}

/// Deterministic protocol-level simulated radio.
pub struct SimDevice {
    clock: SimClock,
    decoder: StreamDecoder,
    store: TransactionalStore<SIM_MAX_OBJECTS>,
    active_transaction: Option<u32>,
    last_exchange: Option<(Frame, Frame)>,
    trace: Vec<TraceEvent>,
    capabilities: DeviceCapabilities,
}

impl Default for SimDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl SimDevice {
    /// Constructs an empty, generation-zero device with fixed capabilities.
    pub fn new() -> Self {
        Self {
            clock: SimClock::new(),
            decoder: StreamDecoder::new(),
            store: TransactionalStore::new(),
            active_transaction: None,
            last_exchange: None,
            trace: Vec::new(),
            capabilities: DeviceCapabilities {
                protocol_version: PROTOCOL_VERSION,
                storage_version: STORAGE_FORMAT_VERSION,
                max_frame_payload: u16::try_from(MAX_PAYLOAD).unwrap_or(u16::MAX),
                max_objects: u16::try_from(SIM_MAX_OBJECTS).unwrap_or(u16::MAX),
                max_object_size: u16::try_from(MAX_OBJECT_DATA).unwrap_or(u16::MAX),
                plan_encodings: PlanEncoding::LinearSimplex.capability_bit(),
            },
        }
    }

    /// Returns the deterministic virtual clock.
    pub const fn clock(&self) -> SimClock {
        self.clock
    }

    /// Returns the fixed capability profile used for offline compilation.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }

    /// Advances device virtual time explicitly.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
    }

    /// Returns the ordered deterministic event trace.
    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    /// Returns the active storage generation.
    pub const fn generation(&self) -> u32 {
        self.store.generation()
    }

    /// Consumes stream bytes and returns zero or more complete response frames.
    pub fn ingest(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut responses = Vec::new();
        for byte in bytes {
            let Some(result) = self.decoder.push(*byte) else {
                continue;
            };
            match result {
                Ok(request) => {
                    self.record(TraceKind::Request {
                        sequence: request.sequence(),
                        service: request.service(),
                        command: request.command(),
                    });
                    match self.handle_exchange(&request) {
                        Ok(response) => {
                            let mut encoded = [0_u8; MAX_ENCODED_FRAME];
                            if let Ok(length) = encode_frame(&response, &mut encoded) {
                                responses.extend_from_slice(&encoded[..length]);
                                self.record(TraceKind::Response {
                                    sequence: response.sequence(),
                                    command: response.command(),
                                });
                            }
                        }
                        Err(error) => self.record(TraceKind::PacketDiscarded(error)),
                    }
                }
                Err(error) => self.record(TraceKind::PacketDiscarded(error)),
            }
        }
        responses
    }

    fn handle_exchange(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        if let Some((previous_request, previous_response)) = self.last_exchange {
            if previous_request.sequence() == request.sequence() {
                if previous_request == *request {
                    self.record(TraceKind::DuplicateRequestReplayed {
                        sequence: request.sequence(),
                    });
                    return Ok(previous_response);
                }
                self.record(TraceKind::SequenceConflictRejected {
                    sequence: request.sequence(),
                });
                return Self::error_response(request, DeviceErrorCode::SequenceConflict);
            }
        }
        let response = self.handle_request(request)?;
        self.last_exchange = Some((*request, response));
        Ok(response)
    }

    fn handle_request(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        if request.flags() != 0 {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        }
        match request.service() {
            Service::DeviceInfo => self.handle_device_info(request),
            Service::Configuration => self.handle_configuration(request),
            Service::RuntimeControl | Service::FirmwareUpdate | Service::Diagnostics => {
                Self::error_response(request, DeviceErrorCode::UnsupportedService)
            }
        }
    }

    fn handle_device_info(&self, request: &Frame) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::Hello => {
                if request.payload() == [PROTOCOL_VERSION] {
                    Self::success_response(request, &[PROTOCOL_VERSION])
                } else {
                    Self::error_response(request, DeviceErrorCode::MalformedPayload)
                }
            }
            Command::GetCapabilities => {
                if !request.payload().is_empty() {
                    return Self::error_response(request, DeviceErrorCode::MalformedPayload);
                }
                let mut payload = [0_u8; DeviceCapabilities::ENCODED_LEN];
                let length = self.capabilities.encode(&mut payload)?;
                Self::success_response(request, &payload[..length])
            }
            _ => Self::error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn handle_configuration(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        match request.command() {
            Command::ListObjects => self.list_objects(request),
            Command::ReadObject => self.read_object(request),
            Command::BeginTransaction => self.begin_transaction(request),
            Command::WriteObject => self.write_object(request),
            Command::ValidateTransaction => self.validate_transaction(request),
            Command::CommitTransaction => self.commit_transaction(request),
            Command::AbortTransaction => self.abort_transaction(request),
            _ => Self::error_response(request, DeviceErrorCode::UnsupportedCommand),
        }
    }

    fn list_objects(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(offset) = decode_list_objects_request(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let mut objects = Vec::new();
        for object in self.store.active_objects() {
            objects.push(ObjectDescriptor {
                kind: object.key().kind as u8,
                id: object.key().id,
                encoded_len: u16::try_from(object.len())
                    .map_err(|_| ProtocolError::PayloadTooLarge)?,
            });
        }
        objects.sort_unstable_by_key(|object| (object.kind, object.id));
        let total_objects =
            u16::try_from(objects.len()).map_err(|_| ProtocolError::PayloadTooLarge)?;
        if offset > total_objects {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        }
        let start = usize::from(offset);
        let end = cmp::min(
            start.saturating_add(MAX_LIST_OBJECTS_PER_PAGE),
            objects.len(),
        );
        let page = &objects[start..end];
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = ObjectListPage::encode(
            self.store.generation(),
            total_objects,
            offset,
            page,
            &mut payload,
        )?;
        self.record(TraceKind::ObjectsListed {
            generation: self.store.generation(),
            offset,
            count: u16::try_from(page.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
        });
        Self::success_response(request, &payload[..length])
    }

    fn read_object(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(key) = parse_object_key(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        let object = match self.store.read(key) {
            Ok(object) => *object,
            Err(StorageError::ObjectNotFound) => {
                return Self::error_response(request, DeviceErrorCode::ObjectNotFound);
            }
            Err(error) => return Self::error_response(request, map_storage_error(error)),
        };
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = {
            let mut writer = PayloadWriter::new(&mut payload);
            writer.write_u8(object.key().kind as u8)?;
            writer.write_u16(object.key().id)?;
            writer.write_u16(
                u16::try_from(object.len()).map_err(|_| ProtocolError::PayloadTooLarge)?,
            )?;
            writer.write_bytes(object.data())?;
            writer.len()
        };
        self.record(TraceKind::ObjectRead(key));
        Self::success_response(request, &payload[..length])
    }

    fn begin_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok(transaction) = parse_transaction(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction.is_some() {
            return Self::error_response(request, DeviceErrorCode::TransactionAlreadyOpen);
        }
        if let Err(error) = self.store.begin() {
            return Self::error_response(request, map_storage_error(error));
        }
        self.active_transaction = Some(transaction);
        self.record(TraceKind::TransactionBegan {
            transaction,
            generation: self.store.generation(),
        });
        Self::success_response(request, &self.store.generation().to_le_bytes())
    }

    fn write_object(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let Ok((transaction, object)) = parse_write_object(request.payload()) else {
            return Self::error_response(request, DeviceErrorCode::MalformedPayload);
        };
        if self.active_transaction != Some(transaction) {
            return Self::error_response(request, DeviceErrorCode::NoTransaction);
        }
        if let Err(error) = self.store.write(object) {
            return Self::error_response(request, map_storage_error(error));
        }
        self.record(TraceKind::ObjectStaged {
            transaction,
            key: object.key(),
        });
        Self::success_response(request, &[])
    }

    fn validate_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        if let Err(error) = self.store.validate(validate_object) {
            return Self::error_response(request, map_storage_error(error));
        }
        self.record(TraceKind::TransactionValidated { transaction });
        Self::success_response(request, &[])
    }

    fn commit_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        let generation = match self.store.commit() {
            Ok(generation) => generation,
            Err(error) => return Self::error_response(request, map_storage_error(error)),
        };
        self.active_transaction = None;
        self.record(TraceKind::TransactionCommitted {
            transaction,
            generation,
        });
        Self::success_response(request, &generation.to_le_bytes())
    }

    fn abort_transaction(&mut self, request: &Frame) -> Result<Frame, ProtocolError> {
        let transaction_result = self.require_transaction(request);
        let Ok(transaction) = transaction_result else {
            return Self::error_response(request, transaction_result.unwrap_err());
        };
        if let Err(error) = self.store.abort() {
            return Self::error_response(request, map_storage_error(error));
        }
        self.active_transaction = None;
        self.record(TraceKind::TransactionAborted { transaction });
        Self::success_response(request, &[])
    }

    fn require_transaction(&self, request: &Frame) -> Result<u32, DeviceErrorCode> {
        let transaction =
            parse_transaction(request.payload()).map_err(|_| DeviceErrorCode::MalformedPayload)?;
        if self.active_transaction != Some(transaction) {
            return Err(DeviceErrorCode::NoTransaction);
        }
        Ok(transaction)
    }

    fn success_response(request: &Frame, payload: &[u8]) -> Result<Frame, ProtocolError> {
        Frame::new(
            request.service(),
            FLAG_RESPONSE,
            request.sequence(),
            request.command(),
            payload,
        )
    }

    fn error_response(request: &Frame, code: DeviceErrorCode) -> Result<Frame, ProtocolError> {
        Frame::new(
            request.service(),
            FLAG_RESPONSE | FLAG_ERROR,
            request.sequence(),
            Command::Error,
            &[request.command() as u8, code as u8],
        )
    }

    fn record(&mut self, kind: TraceKind) {
        self.trace.push(TraceEvent {
            at_ms: self.clock.now_ms(),
            kind,
        });
    }
}

/// In-memory byte transport backed by a deterministic simulated device.
pub struct SimTransport {
    device: SimDevice,
    receive_queue: VecDeque<u8>,
    max_read_size: usize,
}

impl SimTransport {
    /// Constructs a transport returning up to one encoded frame per receive call.
    pub fn new(device: SimDevice) -> Self {
        Self {
            device,
            receive_queue: VecDeque::new(),
            max_read_size: MAX_ENCODED_FRAME,
        }
    }

    /// Limits each receive call to exercise fragmented transport reads.
    #[must_use]
    pub fn with_max_read_size(mut self, max_read_size: usize) -> Self {
        self.max_read_size = max_read_size.max(1);
        self
    }

    /// Returns the underlying simulated device.
    pub const fn device(&self) -> &SimDevice {
        &self.device
    }

    /// Returns a mutable reference to the underlying simulated device.
    pub fn device_mut(&mut self) -> &mut SimDevice {
        &mut self.device
    }

    /// Consumes the transport and returns the simulated device.
    pub fn into_device(self) -> SimDevice {
        self.device
    }
}

impl ProtocolTransport for SimTransport {
    type Error = Infallible;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.receive_queue.extend(self.device.ingest(frame));
        Ok(())
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        let count = cmp::min(
            cmp::min(buffer.len(), self.max_read_size),
            self.receive_queue.len(),
        );
        for destination in &mut buffer[..count] {
            if let Some(byte) = self.receive_queue.pop_front() {
                *destination = byte;
            }
        }
        Ok(count)
    }
}

fn parse_object_key(payload: &[u8]) -> Result<ObjectKey, ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let kind =
        ObjectKind::try_from(reader.read_u8()?).map_err(|_| ProtocolError::MalformedPayload)?;
    let id = reader.read_u16()?;
    reader.finish()?;
    Ok(ObjectKey { kind, id })
}

fn parse_transaction(payload: &[u8]) -> Result<u32, ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let transaction = reader.read_u32()?;
    reader.finish()?;
    Ok(transaction)
}

fn parse_write_object(payload: &[u8]) -> Result<(u32, StorageObject), ProtocolError> {
    let mut reader = PayloadReader::new(payload);
    let transaction = reader.read_u32()?;
    let kind =
        ObjectKind::try_from(reader.read_u8()?).map_err(|_| ProtocolError::MalformedPayload)?;
    let id = reader.read_u16()?;
    let length = usize::from(reader.read_u16()?);
    let data = reader.read_bytes(length)?;
    reader.finish()?;
    let object = StorageObject::new(ObjectKey { kind, id }, data)
        .map_err(|_| ProtocolError::MalformedPayload)?;
    Ok((transaction, object))
}

const fn map_storage_error(error: StorageError) -> DeviceErrorCode {
    match error {
        StorageError::ObjectTooLarge | StorageError::StoreFull => DeviceErrorCode::CapacityExceeded,
        StorageError::TransactionAlreadyOpen => DeviceErrorCode::TransactionAlreadyOpen,
        StorageError::NoTransaction => DeviceErrorCode::NoTransaction,
        StorageError::CandidateNotValidated => DeviceErrorCode::NotValidated,
        StorageError::ValidationFailed
        | StorageError::UnsupportedObject
        | StorageError::MalformedObject => DeviceErrorCode::ValidationFailed,
        StorageError::ObjectNotFound => DeviceErrorCode::ObjectNotFound,
        StorageError::GenerationOverflow
        | StorageError::ImageBufferTooSmall
        | StorageError::ImageTooLarge
        | StorageError::MalformedImage
        | StorageError::UnsupportedImageVersion
        | StorageError::ImageIntegrity
        | StorageError::NonCanonicalImage => DeviceErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        map_storage_error, ChannelSimError, ChannelSimulator, ChannelTraceEvent, ChannelTraceKind,
        RfBusOperation, RfSimulator, RfTraceEvent, RfTraceKind, SimDevice, SimTransport, TraceKind,
        UiSimulator, UiTraceKind,
    };
    use radio_channel_control::{ChannelTxError, ControlState, ScanConfig};
    use radio_channel_plan::{BankName, GeneratedBank, PlanEncoding};
    use radio_domain::{ActiveChannel, BankId, Frequency, FrequencyStep, TxClass};
    use radio_programmer::{ListedObject, Programmer, RadioProject};
    use radio_protocol::{
        decode_packet, encode_frame, Command, DeviceErrorCode, Frame, PayloadWriter, ProtocolError,
        Service, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_PAYLOAD,
    };
    use radio_storage::{
        decode_generated_bank, encode_generated_bank, ObjectKey, ObjectKind, StorageError,
        StorageObject, GENERATED_BANK_ENCODED_LEN,
    };
    use radio_tx_policy::{LoadStatus, PermissionSet, StoredPermissions, TxPolicy};
    use radio_ui::{Key, KeyEvent, KeySet, UiAction, UiView};

    fn bank(id: u16, name: &str) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new(name).unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
            TxClass::LicenceFreePlan,
        )
        .unwrap()
    }

    fn expected_bank() -> GeneratedBank {
        bank(6, "PMR446")
    }

    fn rf_channel(class: TxClass) -> ActiveChannel {
        ActiveChannel {
            receive: Frequency::from_hz(145_500_000).unwrap(),
            transmit: Frequency::from_hz(145_500_000).unwrap(),
            tx_class: class,
        }
    }

    fn run_rf_script() -> (Vec<RfTraceEvent>, super::RfDriverState) {
        let permissions = PermissionSet::none()
            .with(TxClass::Amateur, true)
            .with(TxClass::Business, true);
        let policy = TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0;
        let amateur = policy.authorise(TxClass::Amateur).unwrap();
        let business = policy.authorise(TxClass::Business).unwrap();
        let channel = rf_channel(TxClass::Amateur);
        let mut simulator = RfSimulator::new();

        simulator.recover_to_standby().unwrap();
        simulator.advance_ms(5);
        simulator
            .start_receive(Frequency::from_hz(409_750_000).unwrap())
            .unwrap();
        simulator.advance_ms(2);
        assert_eq!(
            simulator.receive_status().unwrap(),
            radio_bk4819::ReceiveStatus {
                rssi_dbm_x2: -320,
                squelch_open: false,
            }
        );
        simulator.advance_ms(3);
        assert!(simulator.start_transmit(channel, &business).is_err());

        simulator.advance_ms(2);
        simulator.inject_failure_after(3);
        assert!(simulator.start_transmit(channel, &amateur).is_err());
        assert_eq!(simulator.state(), super::RfDriverState::Faulted);

        simulator.advance_ms(3);
        simulator.recover_to_standby().unwrap();
        simulator.advance_ms(5);
        simulator.start_transmit(channel, &amateur).unwrap();
        simulator.advance_ms(1);
        simulator.stop_transmit().unwrap();

        (simulator.trace(), simulator.state())
    }

    #[test]
    fn identical_timed_rf_scripts_have_identical_failure_and_recovery_traces() {
        let first = run_rf_script();
        let second = run_rf_script();
        assert_eq!(first, second);
        assert_eq!(first.1, super::RfDriverState::Standby);

        assert!(first.0.iter().any(|event| {
            event.at_ms == 12
                && matches!(
                    event.kind,
                    RfTraceKind::RegisterOperationFailed(RfBusOperation::Write {
                        address,
                        value: 0x80FE,
                    }) if address.get() == 0x30
                )
        }));
        let transmit_events: Vec<_> = first
            .0
            .iter()
            .filter(|event| matches!(event.kind, RfTraceKind::TransmitStarted { .. }))
            .collect();
        assert_eq!(transmit_events.len(), 1);
        assert_eq!(transmit_events[0].at_ms, 20);
    }

    #[test]
    fn rf_simulator_emits_no_tx_event_or_write_for_a_mismatched_token() {
        let permissions = PermissionSet::none()
            .with(TxClass::Amateur, true)
            .with(TxClass::Business, true);
        let policy = TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0;
        let business = policy.authorise(TxClass::Business).unwrap();
        let mut simulator = RfSimulator::new();
        simulator.recover_to_standby().unwrap();
        let before = simulator.trace();

        assert!(simulator
            .start_transmit(rf_channel(TxClass::Amateur), &business)
            .is_err());
        assert_eq!(simulator.trace(), before);
        assert!(!simulator.trace().iter().any(|event| matches!(
            event.kind,
            RfTraceKind::TransmitStarted { .. }
                | RfTraceKind::RegisterWritten { value: 0x80FE, .. }
        )));
    }

    fn scan_policy(enabled: bool) -> TxPolicy {
        let permissions = PermissionSet::none().with(TxClass::LicenceFreePlan, enabled);
        TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0
    }

    fn run_channel_script() -> (Vec<ChannelTraceEvent>, Vec<RfTraceEvent>, u16, ControlState) {
        let mut simulator =
            ChannelSimulator::activate(bank(7, "SCAN"), 0, ScanConfig::new(10, 30).unwrap())
                .unwrap();
        simulator.start_scanning().unwrap();
        let first = simulator.timer().unwrap();
        simulator.advance_ms(10);
        simulator.deliver_timer(first.token).unwrap();
        assert_eq!(simulator.current_index(), 1);

        simulator.advance_ms(2);
        simulator
            .observe_signal(radio_domain::SignalMeasurement {
                strength: 90,
                squelch_open: true,
            })
            .unwrap();
        let open_hold = simulator.timer().unwrap();
        simulator.advance_ms(30);
        simulator.deliver_timer(open_hold.token).unwrap();
        assert_eq!(simulator.current_index(), 1);

        simulator
            .observe_signal(radio_domain::SignalMeasurement {
                strength: 20,
                squelch_open: false,
            })
            .unwrap();
        let closed_hold = simulator.timer().unwrap();
        simulator.advance_ms(30);
        simulator.deliver_timer(closed_hold.token).unwrap();
        assert_eq!(simulator.current_index(), 2);

        simulator.stop_scanning().unwrap();
        simulator.start_transmit(&scan_policy(true)).unwrap();
        simulator.advance_ms(5);
        simulator.stop_transmit().unwrap();

        (
            simulator.trace().to_vec(),
            simulator.rf_trace(),
            simulator.current_index(),
            simulator.state(),
        )
    }

    #[test]
    fn identical_timed_channel_scripts_have_identical_control_and_rf_traces() {
        let first = run_channel_script();
        let second = run_channel_script();
        assert_eq!(first, second);
        assert_eq!(first.2, 2);
        assert_eq!(first.3, ControlState::Selected);
        assert_eq!(
            first
                .0
                .iter()
                .filter(|event| matches!(event.kind, ChannelTraceKind::ChannelActivated { .. }))
                .count(),
            3
        );
        assert_eq!(
            first
                .0
                .iter()
                .filter(|event| matches!(event.kind, ChannelTraceKind::TransmitStarted { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn channel_simulator_rejects_an_rf_incompatible_bank_before_activation() {
        let incompatible = GeneratedBank::linear_simplex(
            BankId::new(9),
            BankName::new("ODD-HZ").unwrap(),
            Frequency::from_hz(145_500_001).unwrap(),
            FrequencyStep::from_hz(10).unwrap(),
            2,
            TxClass::Amateur,
        )
        .unwrap();
        assert!(matches!(
            ChannelSimulator::activate(incompatible, 0, ScanConfig::new(10, 30).unwrap(),),
            Err(ChannelSimError::Radio(
                radio_bk4819::DriverError::Frequency(
                    radio_bk4819::FrequencyError::NotTenHertzAligned
                )
            ))
        ));
    }

    #[test]
    fn channel_simulator_denies_scan_tx_and_ignores_cancelled_timer() {
        let mut simulator =
            ChannelSimulator::activate(bank(8, "DENY"), 0, ScanConfig::new(10, 30).unwrap())
                .unwrap();
        simulator.start_scanning().unwrap();
        let cancelled = simulator.timer().unwrap();
        assert_eq!(
            simulator.deliver_timer(cancelled.token),
            Err(ChannelSimError::TimerNotDue {
                now_ms: 0,
                due_ms: 10,
            })
        );
        assert_eq!(simulator.current_index(), 0);
        let rf_before = simulator.rf_trace();
        assert_eq!(
            simulator.start_transmit(&scan_policy(true)),
            Err(ChannelSimError::Transmit(ChannelTxError::Scanning))
        );
        assert_eq!(simulator.rf_trace(), rf_before);

        simulator.stop_scanning().unwrap();
        simulator.advance_ms(10);
        simulator.deliver_timer(cancelled.token).unwrap();
        assert_eq!(simulator.current_index(), 0);
        assert!(simulator.timer().is_none());

        let rf_before = simulator.rf_trace();
        assert_eq!(
            simulator.start_transmit(&scan_policy(false)),
            Err(ChannelSimError::Transmit(ChannelTxError::PolicyDenied))
        );
        assert_eq!(simulator.rf_trace(), rf_before);
        assert_eq!(
            simulator
                .trace()
                .iter()
                .filter(|event| matches!(event.kind, ChannelTraceKind::TransmitDenied(_)))
                .count(),
            2
        );
    }

    fn run_permission_script() -> UiSimulator {
        let persisted = StoredPermissions::new(PermissionSet::none(), 10).encode();
        let mut simulator = UiSimulator::boot(persisted, KeySet::permission_menu_gesture());
        simulator.advance_ms(10);
        assert_eq!(
            simulator.handle(KeyEvent::released(Key::Menu)),
            UiAction::None
        );
        simulator.advance_ms(5);
        assert_eq!(
            simulator.handle(KeyEvent::released(Key::Back)),
            UiAction::None
        );
        simulator.advance_ms(5);
        assert_eq!(
            simulator.handle(KeyEvent::pressed(Key::Confirm)),
            UiAction::None
        );
        simulator.advance_ms(1);
        assert_eq!(
            simulator.handle(KeyEvent::released(Key::Confirm)),
            UiAction::None
        );
        simulator.advance_ms(9);
        assert!(matches!(
            simulator.handle(KeyEvent::pressed(Key::Menu)),
            UiAction::PersistPermissions(_)
        ));
        simulator
    }

    #[test]
    fn timed_permission_save_is_repeatable_and_activates_only_after_reboot() {
        let mut first = run_permission_script();
        let second = run_permission_script();
        assert_eq!(first.trace(), second.trace());
        assert_eq!(
            first.persisted_permissions(),
            second.persisted_permissions()
        );
        assert_eq!(first.now_ms(), 30);
        assert_eq!(first.view(), UiView::PermissionsSaved { generation: 11 });
        assert!(first
            .active_policy()
            .authorise(TxClass::LicenceFreePlan)
            .is_err());
        assert!(matches!(
            first.trace().last().map(|event| event.kind),
            Some(UiTraceKind::PermissionsPersisted { generation: 11 })
        ));

        first.advance_ms(10);
        assert_eq!(first.reboot(KeySet::none()), LoadStatus::Valid);
        assert_eq!(first.view(), UiView::Normal);
        assert!(first
            .active_policy()
            .authorise(TxClass::LicenceFreePlan)
            .is_ok());
        assert!(first.active_policy().authorise(TxClass::Amateur).is_err());
        assert_eq!(first.trace().last().unwrap().at_ms, 40);
    }

    #[test]
    fn simulated_cancel_preserves_bytes_and_corruption_fails_closed() {
        let permissions = PermissionSet::none().with(TxClass::Amateur, true);
        let mut corrupt = StoredPermissions::new(permissions, 6).encode();
        corrupt[2] ^= 1;
        let original = corrupt;
        let mut simulator = UiSimulator::boot(corrupt, KeySet::permission_menu_gesture());
        assert!(matches!(
            simulator.trace()[0].kind,
            UiTraceKind::Booted {
                load_status: LoadStatus::DefaultedDenied(_),
                ..
            }
        ));
        assert!(simulator
            .active_policy()
            .authorise(TxClass::Amateur)
            .is_err());
        simulator.handle(KeyEvent::released(Key::Menu));
        simulator.handle(KeyEvent::released(Key::Back));
        simulator.handle(KeyEvent::pressed(Key::Confirm));
        simulator.handle(KeyEvent::released(Key::Confirm));
        assert_eq!(
            simulator.handle(KeyEvent::pressed(Key::Back)),
            UiAction::MenuCancelled
        );
        assert_eq!(simulator.persisted_permissions(), &original);
        assert!(!simulator
            .trace()
            .iter()
            .any(|event| matches!(event.kind, UiTraceKind::PermissionsPersisted { .. })));
        assert!(matches!(
            simulator.reboot(KeySet::none()),
            LoadStatus::DefaultedDenied(_)
        ));
        assert!(simulator
            .active_policy()
            .authorise(TxClass::Amateur)
            .is_err());
    }

    #[test]
    fn offline_image_failures_map_only_to_internal_device_errors() {
        for error in [
            StorageError::ImageBufferTooSmall,
            StorageError::ImageTooLarge,
            StorageError::MalformedImage,
            StorageError::UnsupportedImageVersion,
            StorageError::ImageIntegrity,
            StorageError::NonCanonicalImage,
        ] {
            assert_eq!(map_storage_error(error), DeviceErrorCode::Internal);
        }
    }

    fn protocol_exchange(
        device: &mut SimDevice,
        sequence: u16,
        service: Service,
        flags: u8,
        command: Command,
        payload: &[u8],
    ) -> Frame {
        let request = Frame::new(service, flags, sequence, command, payload).unwrap();
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        let request_len = encode_frame(&request, &mut encoded).unwrap();
        let response = device.ingest(&encoded[..request_len]);
        assert_eq!(response.last(), Some(&0));
        decode_packet(&response[..response.len() - 1]).unwrap()
    }

    fn configuration_exchange(
        device: &mut SimDevice,
        sequence: u16,
        command: Command,
        payload: &[u8],
    ) -> Frame {
        protocol_exchange(
            device,
            sequence,
            Service::Configuration,
            0,
            command,
            payload,
        )
    }

    fn write_object_payload(
        transaction: u32,
        object: &StorageObject,
    ) -> ([u8; MAX_PAYLOAD], usize) {
        let mut payload = [0_u8; MAX_PAYLOAD];
        let length = {
            let mut writer = PayloadWriter::new(&mut payload);
            writer.write_u32(transaction).unwrap();
            writer.write_u8(object.key().kind as u8).unwrap();
            writer.write_u16(object.key().id).unwrap();
            writer
                .write_u16(u16::try_from(object.len()).unwrap())
                .unwrap();
            writer.write_bytes(object.data()).unwrap();
            writer.len()
        };
        (payload, length)
    }

    fn assert_device_error(response: &Frame, rejected: Command, code: DeviceErrorCode) {
        assert_eq!(response.flags(), FLAG_RESPONSE | FLAG_ERROR);
        assert_eq!(response.command(), Command::Error);
        assert_eq!(response.payload(), [rejected as u8, code as u8]);
    }

    fn programmed_banks(active: &[GeneratedBank]) -> (SimDevice, u32) {
        let mut project = RadioProject::new();
        for bank in active {
            project.add_generated_bank(*bank);
        }
        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        let generation = programmer
            .write_configuration(&compiled)
            .unwrap()
            .generation;
        (programmer.into_transport().into_device(), generation)
    }

    fn programmed_device(active: GeneratedBank) -> (SimDevice, u32) {
        programmed_banks(&[active])
    }

    fn run_milestone() -> (GeneratedBank, u32, Vec<super::TraceEvent>) {
        let mut project = RadioProject::new();
        project.add_generated_bank(expected_bank());
        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        assert_eq!(compiled.report().storage_bytes, 31);
        assert_eq!(compiled.report().generated_channels, 16);

        let transport = SimTransport::new(device).with_max_read_size(1);
        let mut programmer = Programmer::connect(transport).unwrap();
        assert_eq!(
            programmer.capabilities().plan_encodings,
            PlanEncoding::LinearSimplex.capability_bit()
        );
        let receipt = programmer.write_configuration(&compiled).unwrap();
        let actual = programmer.read_generated_bank(6).unwrap();
        let device = programmer.into_transport().into_device();
        (actual, receipt.generation, device.trace().to_vec())
    }

    #[test]
    fn first_milestone_round_trip_is_deterministic_and_tx_safe() {
        let first = run_milestone();
        let second = run_milestone();
        assert_eq!(first, second);
        assert_eq!(first.0, expected_bank());
        assert_eq!(first.1, 1);
        assert!(first.2.iter().any(|event| matches!(
            event.kind,
            TraceKind::TransactionCommitted { generation: 1, .. }
        )));

        let tx_policy = TxPolicy::default();
        assert!(tx_policy.authorise(TxClass::LicenceFreePlan).is_err());
    }

    #[test]
    fn fragmented_stream_recovers_after_crc_cobs_and_overflow_errors() {
        let request = Frame::new(Service::DeviceInfo, 0, 1, Command::Hello, &[1]).unwrap();
        let mut valid = [0_u8; MAX_ENCODED_FRAME];
        let length = encode_frame(&request, &mut valid).unwrap();
        let mut corrupt = valid;
        corrupt[length - 2] ^= 0x20;
        let mut stream = Vec::from(&corrupt[..length]);
        stream.extend_from_slice(&[2, 0]);
        stream.extend(core::iter::repeat_n(1, MAX_ENCODED_FRAME + 1));
        stream.push(0);
        stream.extend_from_slice(&valid[..length]);

        let mut device = SimDevice::new();
        let mut response = Vec::new();
        for byte in stream {
            response.extend(device.ingest(&[byte]));
        }
        assert!(!response.is_empty());
        let discarded = device
            .trace()
            .iter()
            .filter_map(|event| match event.kind {
                TraceKind::PacketDiscarded(error) => Some(error),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            discarded,
            vec![
                ProtocolError::CrcMismatch,
                ProtocolError::MalformedCobs,
                ProtocolError::StreamOverflow,
            ]
        );
        assert!(device.trace().iter().any(|event| matches!(
            event.kind,
            TraceKind::Request {
                command: Command::Hello,
                ..
            }
        )));
    }

    #[test]
    fn multi_object_configuration_lists_and_reads_back_in_key_order() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(7, "seven"));
        project.add_generated_bank(bank(1, "one"));
        project.add_generated_bank(bank(4, "four"));

        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        let transport = SimTransport::new(device).with_max_read_size(3);
        let mut programmer = Programmer::connect(transport).unwrap();
        let empty = programmer.list_objects().unwrap();
        assert_eq!(empty.generation, 0);
        assert!(empty.objects.is_empty());

        let receipt = programmer.write_configuration(&compiled).unwrap();
        let listing = programmer.list_objects().unwrap();
        let encoded_len = u16::try_from(GENERATED_BANK_ENCODED_LEN).unwrap();
        assert_eq!(listing.generation, receipt.generation);
        assert_eq!(
            listing.objects,
            vec![
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 1,
                    },
                    encoded_len,
                },
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 4,
                    },
                    encoded_len,
                },
                ListedObject {
                    key: ObjectKey {
                        kind: ObjectKind::GeneratedBank,
                        id: 7,
                    },
                    encoded_len,
                },
            ]
        );
        let snapshot = programmer.read_configuration().unwrap();
        assert_eq!(snapshot.generation, receipt.generation);
        assert_eq!(
            snapshot
                .objects
                .iter()
                .map(|object| decode_generated_bank(object).unwrap())
                .collect::<Vec<_>>(),
            vec![bank(1, "one"), bank(4, "four"), bank(7, "seven")]
        );
        let read_keys = programmer
            .transport()
            .device()
            .trace()
            .iter()
            .filter_map(|event| match event.kind {
                TraceKind::ObjectRead(key) => Some(key),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            read_keys,
            vec![
                ObjectKey {
                    kind: ObjectKind::GeneratedBank,
                    id: 1,
                },
                ObjectKey {
                    kind: ObjectKind::GeneratedBank,
                    id: 4,
                },
                ObjectKey {
                    kind: ObjectKind::GeneratedBank,
                    id: 7,
                },
            ]
        );
        assert!(programmer.transport().device().trace().iter().any(|event| {
            matches!(
                event.kind,
                TraceKind::ObjectsListed {
                    generation: 1,
                    offset: 0,
                    count: 3,
                }
            )
        }));
    }

    #[test]
    fn explicit_abort_preserves_active_data_and_allows_a_new_transaction() {
        let original = bank(2, "original");
        let (mut device, generation) = programmed_device(original);

        let transaction = 77_u32;
        let begin = configuration_exchange(
            &mut device,
            100,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(begin.flags(), FLAG_RESPONSE);
        assert_eq!(begin.payload(), generation.to_le_bytes());

        let replacement = encode_generated_bank(bank(2, "replacement")).unwrap();
        let (write_payload, write_len) = write_object_payload(transaction, &replacement);
        let write = configuration_exchange(
            &mut device,
            101,
            Command::WriteObject,
            &write_payload[..write_len],
        );
        assert_eq!(write.flags(), FLAG_RESPONSE);
        assert!(write.payload().is_empty());

        let abort = configuration_exchange(
            &mut device,
            102,
            Command::AbortTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(abort.flags(), FLAG_RESPONSE);
        assert!(abort.payload().is_empty());
        assert_eq!(device.generation(), generation);

        let next_transaction = 78_u32;
        let next_begin = configuration_exchange(
            &mut device,
            103,
            Command::BeginTransaction,
            &next_transaction.to_le_bytes(),
        );
        assert_eq!(next_begin.flags(), FLAG_RESPONSE);
        assert_eq!(next_begin.payload(), generation.to_le_bytes());
        let _ = configuration_exchange(
            &mut device,
            104,
            Command::AbortTransaction,
            &next_transaction.to_le_bytes(),
        );

        assert!(device.trace().iter().any(|event| matches!(
            event.kind,
            TraceKind::TransactionAborted { transaction: 77 }
        )));
        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        assert_eq!(programmer.read_generated_bank(2).unwrap(), original);
        assert_eq!(programmer.transport().device().generation(), generation);
    }

    #[test]
    fn transaction_state_errors_preserve_the_active_snapshot() {
        let original = bank(3, "active");
        let (mut device, generation) = programmed_device(original);
        let transaction = 90_u32;
        let other_transaction = 91_u32;

        let no_transaction = configuration_exchange(
            &mut device,
            200,
            Command::CommitTransaction,
            &transaction.to_le_bytes(),
        );
        assert_device_error(
            &no_transaction,
            Command::CommitTransaction,
            DeviceErrorCode::NoTransaction,
        );

        let begin = configuration_exchange(
            &mut device,
            201,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(begin.flags(), FLAG_RESPONSE);
        let already_open = configuration_exchange(
            &mut device,
            202,
            Command::BeginTransaction,
            &other_transaction.to_le_bytes(),
        );
        assert_device_error(
            &already_open,
            Command::BeginTransaction,
            DeviceErrorCode::TransactionAlreadyOpen,
        );

        let replacement = encode_generated_bank(bank(3, "candidate")).unwrap();
        let (wrong_write_payload, wrong_write_len) =
            write_object_payload(other_transaction, &replacement);
        let wrong_write = configuration_exchange(
            &mut device,
            203,
            Command::WriteObject,
            &wrong_write_payload[..wrong_write_len],
        );
        assert_device_error(
            &wrong_write,
            Command::WriteObject,
            DeviceErrorCode::NoTransaction,
        );
        let wrong_validate = configuration_exchange(
            &mut device,
            204,
            Command::ValidateTransaction,
            &other_transaction.to_le_bytes(),
        );
        assert_device_error(
            &wrong_validate,
            Command::ValidateTransaction,
            DeviceErrorCode::NoTransaction,
        );

        let not_validated = configuration_exchange(
            &mut device,
            205,
            Command::CommitTransaction,
            &transaction.to_le_bytes(),
        );
        assert_device_error(
            &not_validated,
            Command::CommitTransaction,
            DeviceErrorCode::NotValidated,
        );
        let wrong_abort = configuration_exchange(
            &mut device,
            206,
            Command::AbortTransaction,
            &other_transaction.to_le_bytes(),
        );
        assert_device_error(
            &wrong_abort,
            Command::AbortTransaction,
            DeviceErrorCode::NoTransaction,
        );
        let abort = configuration_exchange(
            &mut device,
            207,
            Command::AbortTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(abort.flags(), FLAG_RESPONSE);
        assert_eq!(device.generation(), generation);

        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        assert_eq!(programmer.read_generated_bank(3).unwrap(), original);
        assert_eq!(programmer.transport().device().generation(), generation);
    }

    #[test]
    fn candidate_validation_failure_preserves_the_active_snapshot() {
        let original = bank(4, "active");
        let (mut device, generation) = programmed_device(original);
        let transaction = 300_u32;
        let begin = configuration_exchange(
            &mut device,
            300,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(begin.flags(), FLAG_RESPONSE);

        let malformed = StorageObject::new(
            ObjectKey {
                kind: ObjectKind::GeneratedBank,
                id: 4,
            },
            &[0_u8; GENERATED_BANK_ENCODED_LEN],
        )
        .unwrap();
        let (write_payload, write_len) = write_object_payload(transaction, &malformed);
        let write = configuration_exchange(
            &mut device,
            301,
            Command::WriteObject,
            &write_payload[..write_len],
        );
        assert_eq!(write.flags(), FLAG_RESPONSE);

        let validation = configuration_exchange(
            &mut device,
            302,
            Command::ValidateTransaction,
            &transaction.to_le_bytes(),
        );
        assert_device_error(
            &validation,
            Command::ValidateTransaction,
            DeviceErrorCode::ValidationFailed,
        );
        assert_eq!(device.generation(), generation);
        let abort = configuration_exchange(
            &mut device,
            303,
            Command::AbortTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(abort.flags(), FLAG_RESPONSE);

        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        assert_eq!(programmer.read_generated_bank(4).unwrap(), original);
        assert_eq!(programmer.transport().device().generation(), generation);
    }

    #[test]
    fn candidate_capacity_failure_preserves_a_full_active_snapshot() {
        let active = (1_u16..=8)
            .map(|id| bank(id, &format!("bank{id}")))
            .collect::<Vec<_>>();
        let (mut device, generation) = programmed_banks(&active);
        let transaction = 400_u32;
        let begin = configuration_exchange(
            &mut device,
            400,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(begin.flags(), FLAG_RESPONSE);

        let ninth = encode_generated_bank(bank(9, "ninth")).unwrap();
        let (write_payload, write_len) = write_object_payload(transaction, &ninth);
        let write = configuration_exchange(
            &mut device,
            401,
            Command::WriteObject,
            &write_payload[..write_len],
        );
        assert_device_error(
            &write,
            Command::WriteObject,
            DeviceErrorCode::CapacityExceeded,
        );
        assert_eq!(device.generation(), generation);
        let abort = configuration_exchange(
            &mut device,
            402,
            Command::AbortTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(abort.flags(), FLAG_RESPONSE);

        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        let snapshot = programmer.read_configuration().unwrap();
        assert_eq!(snapshot.generation, generation);
        assert_eq!(
            snapshot
                .objects
                .iter()
                .map(|object| decode_generated_bank(object).unwrap())
                .collect::<Vec<_>>(),
            active
        );
    }

    struct ErrorCase<'a> {
        service: Service,
        flags: u8,
        command: Command,
        payload: &'a [u8],
        code: DeviceErrorCode,
    }

    const COMMAND_ERROR_CASES: [ErrorCase<'static>; 17] = [
        ErrorCase {
            service: Service::RuntimeControl,
            flags: 0,
            command: Command::Hello,
            payload: &[],
            code: DeviceErrorCode::UnsupportedService,
        },
        ErrorCase {
            service: Service::FirmwareUpdate,
            flags: 0,
            command: Command::Hello,
            payload: &[],
            code: DeviceErrorCode::UnsupportedService,
        },
        ErrorCase {
            service: Service::Diagnostics,
            flags: 0,
            command: Command::Hello,
            payload: &[],
            code: DeviceErrorCode::UnsupportedService,
        },
        ErrorCase {
            service: Service::DeviceInfo,
            flags: 0,
            command: Command::ReadObject,
            payload: &[],
            code: DeviceErrorCode::UnsupportedCommand,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::GetCapabilities,
            payload: &[],
            code: DeviceErrorCode::UnsupportedCommand,
        },
        ErrorCase {
            service: Service::DeviceInfo,
            flags: 0,
            command: Command::Hello,
            payload: &[],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::DeviceInfo,
            flags: 0,
            command: Command::GetCapabilities,
            payload: &[0],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::ListObjects,
            payload: &[],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::ListObjects,
            payload: &[2, 0],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::ReadObject,
            payload: &[ObjectKind::GeneratedBank as u8],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::BeginTransaction,
            payload: &[1, 0, 0],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::WriteObject,
            payload: &[1, 0, 0, 0],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::ValidateTransaction,
            payload: &[1, 0, 0, 0, 0],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::CommitTransaction,
            payload: &[],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::AbortTransaction,
            payload: &[],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::DeviceInfo,
            flags: FLAG_RESPONSE,
            command: Command::Hello,
            payload: &[radio_protocol::PROTOCOL_VERSION],
            code: DeviceErrorCode::MalformedPayload,
        },
        ErrorCase {
            service: Service::Configuration,
            flags: 0,
            command: Command::ReadObject,
            payload: &[ObjectKind::GeneratedBank as u8, 0xff, 0xff],
            code: DeviceErrorCode::ObjectNotFound,
        },
    ];

    #[test]
    fn command_and_payload_errors_are_explicit_and_non_mutating() {
        let original = bank(5, "active");
        let (mut device, generation) = programmed_device(original);

        for (index, case) in COMMAND_ERROR_CASES.iter().enumerate() {
            let response = protocol_exchange(
                &mut device,
                500 + u16::try_from(index).unwrap(),
                case.service,
                case.flags,
                case.command,
                case.payload,
            );
            assert_device_error(&response, case.command, case.code);
            assert_eq!(device.generation(), generation);
        }

        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        assert_eq!(programmer.read_generated_bank(5).unwrap(), original);
        assert_eq!(programmer.transport().device().generation(), generation);
    }

    #[test]
    fn duplicate_sequences_replay_exact_responses_without_repeating_mutations() {
        let original = bank(6, "original");
        let replacement_bank = bank(6, "replacement");
        let (mut device, generation) = programmed_device(original);
        let transaction = 600_u32;

        let begin = configuration_exchange(
            &mut device,
            600,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        let begin_replay = configuration_exchange(
            &mut device,
            600,
            Command::BeginTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(begin_replay, begin);

        let replacement = encode_generated_bank(replacement_bank).unwrap();
        let (write_payload, write_len) = write_object_payload(transaction, &replacement);
        let write = configuration_exchange(
            &mut device,
            601,
            Command::WriteObject,
            &write_payload[..write_len],
        );
        assert_eq!(write.flags(), FLAG_RESPONSE);
        let validation = configuration_exchange(
            &mut device,
            602,
            Command::ValidateTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(validation.flags(), FLAG_RESPONSE);

        let commit = configuration_exchange(
            &mut device,
            603,
            Command::CommitTransaction,
            &transaction.to_le_bytes(),
        );
        let commit_replay = configuration_exchange(
            &mut device,
            603,
            Command::CommitTransaction,
            &transaction.to_le_bytes(),
        );
        assert_eq!(commit_replay, commit);
        assert_eq!(commit.payload(), generation.wrapping_add(1).to_le_bytes());
        assert_eq!(device.generation(), generation + 1);

        let conflict = configuration_exchange(
            &mut device,
            603,
            Command::AbortTransaction,
            &transaction.to_le_bytes(),
        );
        assert_device_error(
            &conflict,
            Command::AbortTransaction,
            DeviceErrorCode::SequenceConflict,
        );
        assert_eq!(device.generation(), generation + 1);
        assert!(device.trace().iter().any(|event| matches!(
            event.kind,
            TraceKind::DuplicateRequestReplayed { sequence: 603 }
        )));
        assert!(device.trace().iter().any(|event| matches!(
            event.kind,
            TraceKind::SequenceConflictRejected { sequence: 603 }
        )));

        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        assert_eq!(programmer.read_generated_bank(6).unwrap(), replacement_bank);
        assert_eq!(programmer.transport().device().generation(), generation + 1);
    }
}
