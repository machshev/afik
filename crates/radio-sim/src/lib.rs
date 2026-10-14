//! Deterministic functional device and in-memory programmer transport.

#![forbid(unsafe_code)]

pub mod memory_radio;
pub mod tx_control;

use core::{cmp, convert::Infallible};
use radio_aprs::{
    parse_repeater_event, AprsError, DiscoveryEntry, DiscoveryError, DiscoveryTable,
    DiscoveryUpdate, RepeaterEvent,
};
use radio_bk4819::{
    Bk4819, DriverError as RfDriverError, DriverState as RfDriverState, FrequencyWord,
    ReceiveStatus, RegisterAddress, RegisterBus,
};
use radio_channel_control::{
    ChannelController, ChannelTxError, ControlError as ChannelControlError, ControlState,
    ControlUpdate as ChannelControlUpdate, ScanConfig, TimerDirective, TimerToken,
};
use radio_channel_plan::{GeneratedBank, PlanEncoding};
use radio_device::{DeviceEvent, DeviceService};
use radio_domain::{ActiveChannel, Frequency, SignalMeasurement, TxClass};
use radio_programmer::ProtocolTransport;
use radio_protocol::k5_extended::{
    decode_k5_extended, encode_k5_extended, K5_EXTENDED_MAX_BYTES, K5_EXTENDED_SESSION,
};
use radio_protocol::{
    decode_packet, encode_frame, Command, DeviceCapabilities, DeviceErrorCode, Frame,
    ProtocolError, Service, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME,
};
use radio_storage::{
    dual_slot::{self, BlockBackend, BLOCK_BYTES},
    ObjectKey,
};
use radio_tx_policy::{TxAuthorisation, TxPolicy, TxRequest};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

/// Configuration bytes the first simulated device profile stores.
///
/// One number, as on a radio: what the simulated device holds is whatever fits
/// these packed bytes, in whatever mixture of objects a project uses.
pub const SIM_CONFIGURATION_BYTES: usize = 512;

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

/// One deterministic APRS parser/discovery simulator observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AprsTraceEvent {
    /// Explicit virtual receive timestamp.
    pub at_ms: u64,
    /// Observable parser or bounded-table outcome.
    pub kind: AprsTraceKind,
}

/// Observable receive-only APRS simulator events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AprsTraceKind {
    /// A complete supplied frame failed bounded AX.25/APRS parsing.
    FrameRejected(AprsError),
    /// A parsed event failed table admission without eviction.
    DiscoveryRejected {
        /// Parsed event that could not be admitted.
        event: RepeaterEvent,
        /// Fixed-capacity table error.
        error: DiscoveryError,
    },
    /// A parsed event produced one deterministic table outcome.
    FrameApplied {
        /// Parsed receive-only event.
        event: RepeaterEvent,
        /// Exact table result.
        update: DiscoveryUpdate,
    },
    /// An explicit cutoff removed live entries or retained kill freshness.
    Expired {
        /// Caller-supplied absolute virtual cutoff.
        cutoff_ms: u64,
        /// Number of occupied slots removed.
        removed: usize,
    },
}

/// APRS simulator input failed parsing or bounded table admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AprsSimError {
    /// Complete-frame or APRS parsing failed.
    Parse(AprsError),
    /// Fixed-capacity discovery admission failed.
    Discovery(DiscoveryError),
}

impl core::fmt::Display for AprsSimError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "simulated APRS parse failed: {error}"),
            Self::Discovery(error) => {
                write!(formatter, "simulated APRS discovery failed: {error}")
            }
        }
    }
}

/// Deterministic virtual-time harness for receive-only APRS discovery.
pub struct AprsSimulator<const CAPACITY: usize> {
    clock: SimClock,
    table: DiscoveryTable<CAPACITY>,
    trace: Vec<AprsTraceEvent>,
}

impl<const CAPACITY: usize> Default for AprsSimulator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> AprsSimulator<CAPACITY> {
    /// Constructs an empty parser/table harness at virtual time zero.
    pub const fn new() -> Self {
        Self {
            clock: SimClock::new(),
            table: DiscoveryTable::new(),
            trace: Vec::new(),
        }
    }

    /// Returns current virtual milliseconds.
    pub const fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Advances virtual receive time without implicit parsing or expiry.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
    }

    /// Returns current visible live discovery entries in stable slot order.
    pub fn entries(&self) -> impl Iterator<Item = &DiscoveryEntry> {
        self.table.entries()
    }

    /// Returns the ordered deterministic APRS trace.
    pub fn trace(&self) -> &[AprsTraceEvent] {
        &self.trace
    }

    /// Parses and applies one complete de-stuffed AX.25 frame at current time.
    pub fn ingest(&mut self, frame: &[u8]) -> Result<DiscoveryUpdate, AprsSimError> {
        let event = match parse_repeater_event(frame) {
            Ok(event) => event,
            Err(error) => {
                self.record(AprsTraceKind::FrameRejected(error));
                return Err(AprsSimError::Parse(error));
            }
        };
        match self.table.apply(event, self.now_ms()) {
            Ok(update) => {
                self.record(AprsTraceKind::FrameApplied { event, update });
                Ok(update)
            }
            Err(error) => {
                self.record(AprsTraceKind::DiscoveryRejected { event, error });
                Err(AprsSimError::Discovery(error))
            }
        }
    }

    /// Explicitly expires occupied slots strictly older than `cutoff_ms`.
    pub fn expire_before(&mut self, cutoff_ms: u64) -> usize {
        let removed = self.table.expire_before(cutoff_ms);
        self.record(AprsTraceKind::Expired { cutoff_ms, removed });
        removed
    }

    fn record(&mut self, kind: AprsTraceKind) {
        self.trace.push(AprsTraceEvent {
            at_ms: self.now_ms(),
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
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
    ) -> Result<(), RfDriverError<RfSimBusError>> {
        self.driver.start_transmit(request, authorisation)?;
        let channel = request.channel;
        self.shared.record(RfTraceKind::TransmitStarted {
            frequency: channel.transmit,
            class: authorisation.class(),
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
    pub fn start_transmit(
        &mut self,
        policy: &TxPolicy,
        request: &TxRequest,
    ) -> Result<(), ChannelSimError> {
        self.require_not_transmitting()?;
        match self.controller.authorise_transmit(policy, request) {
            Ok(transmission) => {
                self.rf
                    .start_transmit(request, transmission.authorisation())?;
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

impl From<DeviceEvent> for TraceKind {
    fn from(event: DeviceEvent) -> Self {
        match event {
            DeviceEvent::Request {
                sequence,
                service,
                command,
            } => Self::Request {
                sequence,
                service,
                command,
            },
            DeviceEvent::PacketDiscarded(error) => Self::PacketDiscarded(error),
            DeviceEvent::DuplicateRequestReplayed { sequence } => {
                Self::DuplicateRequestReplayed { sequence }
            }
            DeviceEvent::SequenceConflictRejected { sequence } => {
                Self::SequenceConflictRejected { sequence }
            }
            DeviceEvent::TransactionBegan {
                transaction,
                generation,
            } => Self::TransactionBegan {
                transaction,
                generation,
            },
            DeviceEvent::ObjectStaged { transaction, key } => {
                Self::ObjectStaged { transaction, key }
            }
            DeviceEvent::TransactionValidated { transaction } => {
                Self::TransactionValidated { transaction }
            }
            DeviceEvent::TransactionAborted { transaction } => {
                Self::TransactionAborted { transaction }
            }
            DeviceEvent::TransactionCommitted {
                transaction,
                generation,
            } => Self::TransactionCommitted {
                transaction,
                generation,
            },
            DeviceEvent::ObjectRead(key) => Self::ObjectRead(key),
            DeviceEvent::ObjectsListed {
                generation,
                offset,
                count,
            } => Self::ObjectsListed {
                generation,
                offset,
                count,
            },
            DeviceEvent::Response { sequence, command } => Self::Response { sequence, command },
        }
    }
}

/// Deterministic protocol-level simulated radio.
///
/// The device half of the protocol lives in `radio-device` and is shared with
/// the target firmware, so this simulator adds only virtual time and the
/// observable trace.
pub struct SimDevice {
    clock: SimClock,
    service: DeviceService<SIM_CONFIGURATION_BYTES>,
    trace: Vec<TraceEvent>,
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
            service: DeviceService::with_plan_encodings(
                PlanEncoding::LinearSimplex.capability_bit(),
            ),
            trace: Vec::new(),
        }
    }

    /// Returns the deterministic virtual clock.
    pub const fn clock(&self) -> SimClock {
        self.clock
    }

    /// Returns the fixed capability profile used for offline compilation.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.service.capabilities()
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
        self.service.generation()
    }

    /// Consumes stream bytes and returns zero or more complete response frames.
    pub fn ingest(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut responses = Vec::new();
        let at_ms = self.clock.now_ms();
        let trace = &mut self.trace;
        let mut encoded = [0_u8; MAX_ENCODED_FRAME];
        for byte in bytes {
            let mut record = |event: DeviceEvent| {
                trace.push(TraceEvent {
                    at_ms,
                    kind: TraceKind::from(event),
                });
            };
            if let Some(length) = self.service.push(*byte, &mut encoded, &mut record) {
                responses.extend_from_slice(&encoded[..length]);
            }
        }
        responses
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

/// K1-parity object-store capacity used by the explicit K5 simulator adapter.
pub const K5_SIM_CONFIGURATION_BYTES: usize = 1_264;

/// Faults exposed by the deterministic simulator EEPROM model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum K5SimStorageError {
    /// The injected operation interruption occurred.
    Fault,
    /// The atomic expected bytes did not match.
    Stale,
    /// A requested simulator byte was outside the relative model.
    OutOfRange,
}

/// Failure returned by the explicit K5 object transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum K5TransportError {
    /// The bounded envelope or inner protocol frame was invalid.
    Protocol(ProtocolError),
}

impl core::fmt::Display for K5TransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "K5 protocol error: {error}"),
        }
    }
}

/// Relative eight-byte backend retained by the K5 simulator.
#[derive(Clone)]
pub struct K5SimStorage {
    bytes: [u8; dual_slot::TOTAL_BYTES],
    operation: usize,
    fault_at: Option<usize>,
    writes: usize,
}

impl Default for K5SimStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl K5SimStorage {
    /// Constructs an erased relative persistence region.
    pub const fn new() -> Self {
        Self {
            bytes: [0xFF; dual_slot::TOTAL_BYTES],
            operation: 0,
            fault_at: None,
            writes: 0,
        }
    }

    /// Arms one deterministic backend operation interruption.
    pub fn set_fault_at(&mut self, operation: Option<usize>) {
        self.fault_at = operation;
        self.operation = 0;
    }

    /// Returns the number of successful atomic writes.
    pub const fn writes(&self) -> usize {
        self.writes
    }

    /// Corrupts one simulator byte for reboot/fallback tests.
    pub fn corrupt_byte(&mut self, offset: usize) -> Result<(), K5SimStorageError> {
        let byte = self
            .bytes
            .get_mut(offset)
            .ok_or(K5SimStorageError::OutOfRange)?;
        *byte ^= 0x01;
        Ok(())
    }
}

impl BlockBackend for K5SimStorage {
    type Error = K5SimStorageError;

    fn read_block(
        &mut self,
        block: u16,
        output: &mut [u8; BLOCK_BYTES],
    ) -> Result<(), Self::Error> {
        let operation = self.operation;
        self.operation += 1;
        if self.fault_at == Some(operation) {
            return Err(K5SimStorageError::Fault);
        }
        let start = usize::from(block)
            .checked_mul(BLOCK_BYTES)
            .ok_or(K5SimStorageError::OutOfRange)?;
        let end = start
            .checked_add(BLOCK_BYTES)
            .ok_or(K5SimStorageError::OutOfRange)?;
        let source = self
            .bytes
            .get(start..end)
            .ok_or(K5SimStorageError::OutOfRange)?;
        output.copy_from_slice(source);
        Ok(())
    }

    fn write_verified(
        &mut self,
        block: u16,
        expected: &[u8; BLOCK_BYTES],
        replacement: &[u8; BLOCK_BYTES],
    ) -> Result<(), Self::Error> {
        let operation = self.operation;
        self.operation += 1;
        let start = usize::from(block)
            .checked_mul(BLOCK_BYTES)
            .ok_or(K5SimStorageError::OutOfRange)?;
        let end = start
            .checked_add(BLOCK_BYTES)
            .ok_or(K5SimStorageError::OutOfRange)?;
        if self.bytes.get(start..end).is_none() {
            return Err(K5SimStorageError::OutOfRange);
        }
        if self.fault_at == Some(operation) {
            self.bytes[start..start + 3].copy_from_slice(&replacement[..3]);
            self.bytes[start + 3..end].fill(0xFF);
            return Err(K5SimStorageError::Fault);
        }
        if self.bytes[start..end] != expected[..] {
            return Err(K5SimStorageError::Stale);
        }
        self.bytes[start..end].copy_from_slice(replacement);
        self.writes += 1;
        Ok(())
    }
}

/// Explicit K5 extended-envelope simulator device.
pub struct K5ObjectDevice {
    clock: SimClock,
    service: DeviceService<K5_SIM_CONFIGURATION_BYTES>,
    storage: K5SimStorage,
    durability_error: Option<(Frame, Frame)>,
}

impl Default for K5ObjectDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl K5ObjectDevice {
    /// Constructs an empty generation-zero K5 object device.
    pub fn new() -> Self {
        Self {
            clock: SimClock::new(),
            service: DeviceService::with_plan_encodings(
                PlanEncoding::LinearSimplex.capability_bit(),
            ),
            storage: K5SimStorage::new(),
            durability_error: None,
        }
    }

    /// Returns the active service generation.
    pub const fn generation(&self) -> u32 {
        self.service.generation()
    }

    /// Returns the explicit simulator capability profile used by the host
    /// compiler; this is not a target capability advertisement.
    pub const fn capabilities(&self) -> DeviceCapabilities {
        self.service.capabilities()
    }

    /// Returns the deterministic virtual clock.
    pub const fn clock(&self) -> SimClock {
        self.clock
    }

    /// Advances virtual time without implicit device work.
    pub fn advance_ms(&mut self, duration_ms: u64) {
        self.clock.advance_ms(duration_ms);
    }

    /// Gives tests access to the simulator-only persistence fault injector.
    pub fn storage_mut(&mut self) -> &mut K5SimStorage {
        &mut self.storage
    }

    /// Reconstructs the service from the newest valid dual-slot image.
    pub fn reboot(&mut self) -> Result<bool, dual_slot::PersistenceError<K5SimStorageError>> {
        let mut slot = dual_slot::empty_slot();
        let mut image = [0; dual_slot::MAX_CANONICAL_IMAGE_BYTES];
        let loaded = dual_slot::load(&mut self.storage, &mut slot, &mut image)?;
        let mut replacement =
            DeviceService::with_plan_encodings(PlanEncoding::LinearSimplex.capability_bit());
        if let Some(snapshot) = loaded {
            replacement
                .load_image_at_generation(snapshot.bytes, snapshot.generation)
                .map_err(|_| dual_slot::PersistenceError::InvalidImage)?;
            self.service = replacement;
            self.durability_error = None;
            Ok(true)
        } else {
            self.service = replacement;
            self.durability_error = None;
            Ok(false)
        }
    }

    fn error_frame(request: &Frame, code: DeviceErrorCode) -> Frame {
        Frame::new(
            request.service(),
            FLAG_RESPONSE | FLAG_ERROR,
            request.sequence(),
            Command::Error,
            &[request.command() as u8, code as u8],
        )
        .expect("bounded protocol error frame")
    }

    fn wrap_frame(frame: Frame) -> Vec<u8> {
        let mut encoded = [0; MAX_ENCODED_FRAME];
        let Ok(length) = encode_frame(&frame, &mut encoded) else {
            return Vec::new();
        };
        Self::wrap_encoded(&encoded[..length])
    }

    fn wrap_encoded(encoded: &[u8]) -> Vec<u8> {
        let mut wrapped = [0; K5_EXTENDED_MAX_BYTES];
        let Ok(length) = encode_k5_extended(K5_EXTENDED_SESSION, encoded, &mut wrapped) else {
            return Vec::new();
        };
        wrapped[..length].to_vec()
    }

    /// Processes one strict K5 envelope and returns one strict response
    /// envelope. Malformed input is discarded without reaching the service.
    pub fn ingest(&mut self, envelope: &[u8]) -> Vec<u8> {
        let Ok(decoded) = decode_k5_extended(envelope, K5_EXTENDED_SESSION) else {
            return Vec::new();
        };
        let Some(request_bytes) = decoded.payload.strip_suffix(&[0]) else {
            return Vec::new();
        };
        let Ok(request) = decode_packet(request_bytes) else {
            return Vec::new();
        };
        if let Some((cached_request, cached_response)) = self.durability_error {
            if cached_request.sequence() == request.sequence() {
                if cached_request == request {
                    return Self::wrap_frame(cached_response);
                }
                return Self::wrap_frame(Self::error_frame(
                    &request,
                    DeviceErrorCode::SequenceConflict,
                ));
            }
        }
        self.durability_error = None;
        let prior_service = if request.command() == Command::CommitTransaction {
            Some(self.service.clone())
        } else {
            None
        };
        let mut encoded = [0; MAX_ENCODED_FRAME];
        let mut response_len = None;
        let mut fresh_commit = false;
        let mut observer = |event: DeviceEvent| {
            if matches!(event, DeviceEvent::TransactionCommitted { .. }) {
                fresh_commit = true;
            }
        };
        for byte in decoded.payload {
            if let Some(length) = self.service.push(*byte, &mut encoded, &mut observer) {
                response_len = Some(length);
            }
        }
        let Some(mut response_len) = response_len else {
            return Vec::new();
        };
        if request.command() == Command::CommitTransaction && fresh_commit {
            // The service has already committed in RAM. The simulator's
            // authoritative durable boundary is the dual-slot update; a
            // failed update must not be reported as a successful commit.
            if let Some(response_bytes) = encoded[..response_len].strip_suffix(&[0]) {
                if let Ok(response) = decode_packet(response_bytes) {
                    if response.flags() & radio_protocol::FLAG_ERROR == 0 {
                        let mut image = [0; dual_slot::MAX_CANONICAL_IMAGE_BYTES];
                        if let Ok(length) = self.service.encode_active_image(&mut image) {
                            let mut slot = dual_slot::empty_slot();
                            let mut image_scratch = [0; dual_slot::MAX_CANONICAL_IMAGE_BYTES];
                            let persisted = dual_slot::update(
                                &mut self.storage,
                                &image[..length],
                                &mut slot,
                                &mut image_scratch,
                            )
                            .and_then(|generation| {
                                if generation == self.service.generation() {
                                    Ok(generation)
                                } else {
                                    Err(dual_slot::PersistenceError::InvalidImage)
                                }
                            });
                            if persisted.is_err() {
                                let Some(restored) = prior_service else {
                                    return Vec::new();
                                };
                                self.service = restored;
                                let error = Self::error_frame(&request, DeviceErrorCode::Internal);
                                self.durability_error = Some((request, error));
                                let Some(error_length) = encode_frame(&error, &mut encoded).ok()
                                else {
                                    return Vec::new();
                                };
                                response_len = error_length;
                            }
                        }
                    }
                }
            }
        }
        Self::wrap_encoded(&encoded[..response_len])
    }
}

/// Explicit transport selecting the K5 extended object envelope.
pub struct K5ObjectTransport {
    device: K5ObjectDevice,
    receive_queue: VecDeque<u8>,
    max_read_size: usize,
}

impl K5ObjectTransport {
    /// Constructs the K5 object transport; no target auto-detection exists.
    pub fn new(device: K5ObjectDevice) -> Self {
        Self {
            device,
            receive_queue: VecDeque::new(),
            max_read_size: MAX_ENCODED_FRAME,
        }
    }

    /// Limits each receive call to exercise deterministic fragmentation.
    #[must_use]
    pub fn with_max_read_size(mut self, max_read_size: usize) -> Self {
        self.max_read_size = max_read_size.max(1);
        self
    }

    /// Returns the simulated K5 device.
    pub const fn device(&self) -> &K5ObjectDevice {
        &self.device
    }

    /// Returns the mutable simulated K5 device.
    pub fn device_mut(&mut self) -> &mut K5ObjectDevice {
        &mut self.device
    }
}

impl ProtocolTransport for K5ObjectTransport {
    type Error = K5TransportError;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        let mut wrapped = [0; K5_EXTENDED_MAX_BYTES];
        let length = encode_k5_extended(K5_EXTENDED_SESSION, frame, &mut wrapped)
            .map_err(K5TransportError::Protocol)?;
        let response = self.device.ingest(&wrapped[..length]);
        let decoded = decode_k5_extended(&response, K5_EXTENDED_SESSION)
            .map_err(K5TransportError::Protocol)?;
        self.receive_queue.extend(decoded.payload);
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

#[cfg(test)]
#[path = "../../radio-tx-policy/tests/support/mod.rs"]
mod tx_fixture;

#[cfg(test)]
mod tests {
    use super::tx_fixture::{self, request as tx_request};
    use super::{
        AprsSimError, AprsSimulator, AprsTraceEvent, AprsTraceKind, ChannelSimError,
        ChannelSimulator, ChannelTraceEvent, ChannelTraceKind, K5ObjectDevice, K5ObjectTransport,
        RfBusOperation, RfSimulator, RfTraceEvent, RfTraceKind, SimDevice, SimTransport, TraceKind,
    };
    use radio_aprs::{AprsError, DiscoveryError, DiscoveryUpdate};
    use radio_channel_control::{ChannelTxError, ControlState, ScanConfig};
    use radio_channel_plan::{
        BankFlags, BankName, ChannelBank, ChannelDefinition, ChannelName, ChannelRecord,
        GeneratedBank, PlanEncoding,
    };
    use radio_device::map_storage_error;
    use radio_domain::{
        ActiveChannel, Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation,
        PowerLevel, RadioConfig, SquelchLevel, Tone, TxClass,
    };
    use radio_platform::receive_app::{Event as SharedEvent, Key as SharedKey, ReceiveApp};
    use radio_platform::serial_control::{
        dispatch as dispatch_shared, Capabilities as SharedCapabilities,
        Capability as SharedCapability, Command as SharedCommand, Request as SharedRequest,
        Result as SharedResult,
    };
    use radio_programmer::{
        ListedObject, Programmer, ProgrammerError, ProtocolTransport, RadioProject,
    };
    use radio_protocol::{
        decode_packet, encode_frame, Command, DeviceErrorCode, Frame, PayloadWriter, ProtocolError,
        Service, FLAG_ERROR, FLAG_RESPONSE, MAX_ENCODED_FRAME, MAX_PAYLOAD,
    };
    use radio_storage::dual_slot::{self, BlockBackend};
    use radio_storage::{
        decode_configuration_image, decode_generated_bank, encode_generated_bank, ObjectKey,
        ObjectKind, StorageError, StorageObject, GENERATED_BANK_CORE_LEN, OBJECT_ENTRY_HEADER_LEN,
    };
    use radio_tx_policy::TxPolicy;

    #[test]
    fn deterministic_simulator_shared_serial_trace_matches_keypad() {
        let capabilities = SharedCapabilities::shared_receive();
        let mut serial = ReceiveApp::new();
        let mut keypad = ReceiveApp::new();
        let serial_result = dispatch_shared(
            &mut serial,
            capabilities,
            SharedRequest::Command(SharedCommand::Key(SharedKey::Up)),
        );
        let keypad_effects = keypad.apply(SharedEvent::KeyPress(SharedKey::Up));
        assert_eq!(serial_result, SharedResult::Effects(keypad_effects));
        assert_eq!(serial.view(), keypad.view());
        assert!(capabilities.contains(SharedCapability::SharedReceive));
        assert!(!capabilities.contains(SharedCapability::PersistentConfiguration));

        let serial_result = dispatch_shared(
            &mut serial,
            capabilities,
            SharedRequest::Command(SharedCommand::Key(SharedKey::Side2)),
        );
        let keypad_effects = keypad.apply(SharedEvent::KeyPress(SharedKey::Side2));
        assert_eq!(serial_result, SharedResult::Effects(keypad_effects));
        assert_eq!(serial.view(), keypad.view());
        assert!(serial.view().monitoring);
    }

    #[test]
    fn deterministic_simulator_carries_squelch_across_retune_and_monitor() {
        let mut app = ReceiveApp::new();
        let level = radio_domain::SquelchLevel::new(6).expect("bounded level");
        assert_eq!(app.squelch(), radio_domain::SquelchLevel::CONSERVATIVE);

        let applied = app.apply(SharedEvent::SetSquelch(level));
        assert!(applied.iter().any(|effect| {
            matches!(
                effect,
                radio_platform::receive_app::Effect::Tune { squelch, .. }
                    if squelch == level
            )
        }));
        assert_eq!(app.squelch(), radio_domain::SquelchLevel::CONSERVATIVE);
        app.apply(SharedEvent::SquelchAcknowledged {
            level,
            applied: true,
        });
        assert_eq!(app.squelch(), level);

        let monitored = app.apply(SharedEvent::ToggleMonitor);
        assert!(monitored.iter().any(|effect| {
            matches!(
                effect,
                radio_platform::receive_app::Effect::SetSpeaker(true)
            )
        }));
        let retuned = app.apply(SharedEvent::NextChannel);
        assert!(retuned.iter().any(|effect| {
            matches!(
                effect,
                radio_platform::receive_app::Effect::Tune { squelch, .. }
                    if squelch == level
            )
        }));
    }

    #[test]
    fn deterministic_simulator_decodes_common_wire_and_returns_shared_view() {
        use radio_platform::serial_control::wire::{
            decode_request, decode_response, encode_key_response, KeyCode, Request as WireRequest,
            Response as WireResponse,
        };

        let request = decode_request(&[0x03, 0x04]).expect("wire key");
        assert_eq!(request, WireRequest::Key(KeyCode::Up));
        let mut app = ReceiveApp::new();
        let result = dispatch_shared(
            &mut app,
            SharedCapabilities::shared_receive(),
            radio_platform::serial_control::wire::into_request(request),
        );
        assert!(matches!(result, SharedResult::Effects(_)));
        let mut response = [0_u8; 4];
        let length = encode_key_response(app.view(), &mut response).expect("wire response");
        assert_eq!(
            decode_response(&response[..length]),
            Ok(WireResponse::View(app.view()))
        );
    }

    fn bank(id: u16, name: &str) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new(name).unwrap(),
            Frequency::from_hz(446_006_250).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            16,
        )
        .unwrap()
    }

    fn k5_project() -> RadioProject {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(6, "PMR446"));
        project.add_bank(
            ChannelBank::new(
                BankId::new(1),
                BankName::new("Named").unwrap(),
                BankFlags::default(),
            )
            .unwrap(),
        );
        project.add_channel(
            ChannelRecord::new(ChannelDefinition {
                id: ChannelId::new(42),
                name: ChannelName::new("Explicit").unwrap(),
                receive: Frequency::from_hz(145_725_000).unwrap(),
                transmit: Frequency::from_hz(145_125_000).unwrap(),
                rx_tone: Tone::Ctcss(1_000),
                tx_tone: Tone::Ctcss(1_000),
                modulation: Modulation::Fm,
                bandwidth: Bandwidth::Wide,
                power: PowerLevel::Medium,
                step: FrequencyStep::from_hz(12_500).unwrap(),
                squelch: SquelchLevel::new(6).unwrap(),
                flags: radio_channel_plan::ChannelFlags::default(),
                banks: radio_channel_plan::BankMask::from_bits(1 << 1),
            })
            .unwrap(),
        );
        project.set_config(RadioConfig {
            squelch: SquelchLevel::new(7).unwrap(),
            ..RadioConfig::conservative()
        });
        project
    }

    #[test]
    fn k5_extended_programmer_round_trip_reboot_and_generation_are_exact() {
        let device = K5ObjectDevice::new();
        let mut programmer =
            Programmer::connect(K5ObjectTransport::new(device).with_max_read_size(1)).unwrap();
        let compiled = programmer.compiler().compile(&k5_project()).unwrap();
        assert_eq!(compiled.report().generated_channels, 16);
        assert_eq!(compiled.report().explicit_channels, 1);
        assert_eq!(compiled.report().banks, 1);
        assert!(compiled.report().has_radio_config);

        let mut expected_image = vec![0; compiled.image_len().unwrap()];
        let expected_len = compiled.encode_image(&mut expected_image).unwrap();
        let first = programmer.write_configuration_verified(&compiled).unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(
            programmer.backup_configuration().unwrap().image,
            expected_image
        );
        programmer.transport_mut().device_mut().advance_ms(37);
        assert_eq!(programmer.transport().device().clock().now_ms(), 37);

        let mut transport = programmer.into_transport();
        let mut slot = dual_slot::empty_slot();
        let mut persisted = [0; dual_slot::MAX_CANONICAL_IMAGE_BYTES];
        let loaded = dual_slot::load(
            transport.device_mut().storage_mut(),
            &mut slot,
            &mut persisted,
        )
        .unwrap();
        assert_eq!(loaded.map(|image| image.generation), Some(1));
        assert!(transport.device_mut().reboot().unwrap());
        let mut rebooted = Programmer::connect(transport).unwrap();
        let restored = rebooted.backup_configuration().unwrap();
        assert_eq!(restored.generation, 1);
        assert_eq!(restored.image, expected_image[..expected_len]);
        let second = rebooted.write_configuration_verified(&compiled).unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(
            rebooted.backup_configuration().unwrap().image,
            expected_image
        );
    }

    #[test]
    fn k5_interrupted_or_corrupt_newest_persistence_keeps_previous_generation() {
        let mut programmer =
            Programmer::connect(K5ObjectTransport::new(K5ObjectDevice::new())).unwrap();
        let compiled = programmer.compiler().compile(&k5_project()).unwrap();
        assert_eq!(
            programmer
                .write_configuration_verified(&compiled)
                .unwrap()
                .generation,
            1
        );

        programmer
            .transport_mut()
            .device_mut()
            .storage_mut()
            .set_fault_at(Some(0));
        assert!(programmer.transport_mut().device_mut().reboot().is_err());
        assert_eq!(programmer.transport().device().generation(), 1);
        programmer
            .transport_mut()
            .device_mut()
            .storage_mut()
            .set_fault_at(None);

        programmer
            .transport_mut()
            .device_mut()
            .storage_mut()
            .set_fault_at(Some(0));
        assert!(matches!(
            programmer.write_configuration_verified(&compiled),
            Err(ProgrammerError::Device {
                code: DeviceErrorCode::Internal,
                ..
            })
        ));
        assert_eq!(programmer.backup_configuration().unwrap().generation, 1);
        programmer
            .transport_mut()
            .device_mut()
            .storage_mut()
            .set_fault_at(None);
        let mut transport = programmer.into_transport();
        assert!(transport.device_mut().reboot().unwrap());
        let mut rebooted = Programmer::connect(transport).unwrap();
        assert_eq!(rebooted.backup_configuration().unwrap().generation, 1);

        assert_eq!(
            rebooted
                .write_configuration_verified(&compiled)
                .unwrap()
                .generation,
            2
        );
        let mut transport = rebooted.into_transport();
        // The corruption is in the newest slot's header; the prior slot must
        // remain loadable.
        transport
            .device_mut()
            .storage_mut()
            .corrupt_byte(dual_slot::SLOT_BYTES)
            .unwrap();
        assert!(transport.device_mut().reboot().unwrap());
        let rebooted = Programmer::connect(transport).unwrap();
        assert_eq!(rebooted.transport().device().generation(), 1);
    }

    #[test]
    fn k5_replayed_commit_does_not_repeat_persistence_or_change_generation() {
        let mut device = K5ObjectDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&k5_project())
            .unwrap();
        let transaction = 21_u32;
        let packet = |sequence: u16, command: Command, payload: &[u8]| {
            let frame = Frame::new(Service::Configuration, 0, sequence, command, payload).unwrap();
            let mut inner = [0; MAX_ENCODED_FRAME];
            let inner_len = encode_frame(&frame, &mut inner).unwrap();
            let mut outer = [0; radio_protocol::k5_extended::K5_EXTENDED_MAX_BYTES];
            let outer_len = radio_protocol::k5_extended::encode_k5_extended(
                radio_protocol::k5_extended::K5_EXTENDED_SESSION,
                &inner[..inner_len],
                &mut outer,
            )
            .unwrap();
            outer[..outer_len].to_vec()
        };
        let transaction_bytes = transaction.to_le_bytes();
        let _ = device.ingest(&packet(1, Command::BeginTransaction, &transaction_bytes));
        for (index, object) in compiled.objects().iter().enumerate() {
            let mut payload = [0; MAX_PAYLOAD];
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
            let sequence = u16::try_from(index + 2).unwrap();
            let _ = device.ingest(&packet(sequence, Command::WriteObject, &payload[..length]));
        }
        let _ = device.ingest(&packet(
            100,
            Command::ValidateTransaction,
            &transaction_bytes,
        ));
        let commit = packet(101, Command::CommitTransaction, &transaction_bytes);
        let first_response = device.ingest(&commit);
        let writes = device.storage.writes();
        assert_eq!(device.generation(), 1);
        let replay_response = device.ingest(&commit);
        assert_eq!(replay_response, first_response);
        assert_eq!(device.storage.writes(), writes);
        assert_eq!(device.generation(), 1);
        assert!(device.reboot().unwrap());
        assert_eq!(device.generation(), 1);
    }

    #[test]
    fn k5_storage_rejects_out_of_range_relative_blocks_without_panicking() {
        let mut storage = super::K5SimStorage::new();
        let mut block = [0; radio_storage::dual_slot::BLOCK_BYTES];
        let expected = block;
        assert_eq!(
            storage.read_block(u16::MAX, &mut block),
            Err(super::K5SimStorageError::OutOfRange)
        );
        assert_eq!(
            storage.write_verified(u16::MAX, &expected, &block),
            Err(super::K5SimStorageError::OutOfRange)
        );
        assert_eq!(
            storage.corrupt_byte(radio_storage::dual_slot::TOTAL_BYTES),
            Err(super::K5SimStorageError::OutOfRange)
        );
    }

    #[test]
    fn k5_transport_rejects_malformed_or_oversized_frames_without_mutation() {
        let mut transport = K5ObjectTransport::new(K5ObjectDevice::new());
        assert!(matches!(
            transport.send(&[1, 2, 3]),
            Err(super::K5TransportError::Protocol(_))
        ));
        let oversized = [0; MAX_ENCODED_FRAME + 1];
        assert!(matches!(
            transport.send(&oversized),
            Err(super::K5TransportError::Protocol(
                ProtocolError::PayloadTooLarge
            ))
        ));
        assert_eq!(transport.device().generation(), 0);
    }

    #[test]
    fn k5_envelope_rejects_session_command_crc_and_trailing_input_without_mutation() {
        let mut device = K5ObjectDevice::new();
        let mut inner = [0; MAX_ENCODED_FRAME];
        let frame = Frame::new(Service::DeviceInfo, 0, 1, Command::Hello, &[1]).unwrap();
        let inner_len = encode_frame(&frame, &mut inner).unwrap();
        let mut outer = [0; radio_protocol::k5_extended::K5_EXTENDED_MAX_BYTES];
        let length = radio_protocol::k5_extended::encode_k5_extended(
            radio_protocol::k5_extended::K5_EXTENDED_SESSION,
            &inner[..inner_len],
            &mut outer,
        )
        .unwrap();
        let valid = outer[..length].to_vec();
        assert!(!device.ingest(&valid).is_empty());
        assert_eq!(device.generation(), 0);

        let mut wrong_session = valid.clone();
        wrong_session[5] ^= 1;
        assert!(device.ingest(&wrong_session).is_empty());
        let mut wrong_command = valid.clone();
        wrong_command[3] ^= 1;
        assert!(device.ingest(&wrong_command).is_empty());
        let mut wrong_crc = valid.clone();
        let last = wrong_crc.len() - 1;
        wrong_crc[last] ^= 1;
        assert!(device.ingest(&wrong_crc).is_empty());
        let mut trailing = valid;
        trailing.push(0);
        assert!(device.ingest(&trailing).is_empty());
        assert_eq!(device.generation(), 0);
    }

    #[test]
    fn k5_aborted_and_validation_failed_transactions_never_persist() {
        fn send(
            device: &mut K5ObjectDevice,
            sequence: u16,
            command: Command,
            payload: &[u8],
        ) -> Vec<u8> {
            let frame = Frame::new(Service::Configuration, 0, sequence, command, payload).unwrap();
            let mut inner = [0; MAX_ENCODED_FRAME];
            let inner_len = encode_frame(&frame, &mut inner).unwrap();
            let mut outer = [0; radio_protocol::k5_extended::K5_EXTENDED_MAX_BYTES];
            let outer_len = radio_protocol::k5_extended::encode_k5_extended(
                radio_protocol::k5_extended::K5_EXTENDED_SESSION,
                &inner[..inner_len],
                &mut outer,
            )
            .unwrap();
            device.ingest(&outer[..outer_len])
        }

        let mut device = K5ObjectDevice::new();
        let transaction = 11_u32.to_le_bytes();
        assert!(!send(&mut device, 1, Command::BeginTransaction, &transaction).is_empty());
        assert!(!send(&mut device, 2, Command::AbortTransaction, &transaction).is_empty());
        assert_eq!(device.generation(), 0);
        assert!(!device.reboot().unwrap());

        let transaction = 12_u32.to_le_bytes();
        assert!(!send(&mut device, 3, Command::BeginTransaction, &transaction).is_empty());
        let mut malformed = [0; MAX_PAYLOAD];
        let malformed_len = {
            let mut writer = PayloadWriter::new(&mut malformed);
            writer.write_u32(12).unwrap();
            writer.write_u8(ObjectKind::GeneratedBank as u8).unwrap();
            writer.write_u16(1).unwrap();
            writer.write_u16(1).unwrap();
            writer.write_u8(0).unwrap();
            writer.len()
        };
        assert!(!send(
            &mut device,
            4,
            Command::WriteObject,
            &malformed[..malformed_len],
        )
        .is_empty());
        assert!(!send(&mut device, 5, Command::ValidateTransaction, &transaction).is_empty());
        assert!(!send(&mut device, 6, Command::AbortTransaction, &transaction).is_empty());
        assert_eq!(device.generation(), 0);
        assert!(!device.reboot().unwrap());
    }

    fn aprs_fcs_accumulator(bytes: &[u8]) -> u16 {
        let mut fcs = 0xffff_u16;
        for byte in bytes {
            fcs ^= u16::from(*byte);
            for _ in 0..8 {
                fcs = if fcs & 1 == 0 {
                    fcs >> 1
                } else {
                    (fcs >> 1) ^ 0x8408
                };
            }
        }
        fcs
    }

    fn aprs_address(callsign: &[u8], ssid: u8, final_address: bool) -> [u8; 7] {
        let mut encoded = [b' ' << 1; 7];
        for (destination, source) in encoded.iter_mut().zip(callsign.iter().copied()) {
            *destination = source << 1;
        }
        encoded[6] = 0x60 | (ssid << 1) | u8::from(final_address);
        encoded
    }

    fn aprs_frame(source: &[u8], ssid: u8, information: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&aprs_address(b"APRS", 0, false));
        bytes.extend_from_slice(&aprs_address(source, ssid, true));
        bytes.extend_from_slice(&[0x03, 0xf0]);
        bytes.extend_from_slice(information);
        let fcs = !aprs_fcs_accumulator(&bytes);
        bytes.extend_from_slice(&fcs.to_le_bytes());
        bytes
    }

    fn run_aprs_script() -> (Vec<AprsTraceEvent>, Vec<radio_aprs::DiscoveryEntry>) {
        let live = aprs_frame(
            b"DIGI",
            1,
            b";146.940-A*111111z4903.50N/07201.75WrT107 R25m",
        );
        let changed = aprs_frame(
            b"DIGI",
            1,
            b";146.940-A*111111z4903.50N/07201.75WrT088 R25m",
        );
        let killed = aprs_frame(b"DIGI", 1, b";146.940-A_111111z4903.50N/07201.75Wrretired");
        let mut corrupt = live.clone();
        corrupt[20] ^= 1;

        let mut simulator = AprsSimulator::<2>::new();
        assert_eq!(simulator.ingest(&live), Ok(DiscoveryUpdate::Inserted));
        simulator.advance_ms(5);
        assert_eq!(simulator.ingest(&changed), Ok(DiscoveryUpdate::Updated));
        assert!(matches!(
            simulator.ingest(&corrupt),
            Err(AprsSimError::Parse(AprsError::Ax25(_)))
        ));
        simulator.advance_ms(1);
        assert_eq!(simulator.ingest(&killed), Ok(DiscoveryUpdate::Removed));
        assert_eq!(simulator.expire_before(7), 1);

        (
            simulator.trace().to_vec(),
            simulator.entries().copied().collect(),
        )
    }

    #[test]
    fn identical_timed_aprs_scripts_have_identical_receive_only_traces() {
        let first = run_aprs_script();
        let second = run_aprs_script();
        assert_eq!(first, second);
        assert!(first.1.is_empty());
        assert_eq!(first.0.len(), 5);
        assert!(matches!(
            first.0[2],
            AprsTraceEvent {
                at_ms: 5,
                kind: AprsTraceKind::FrameRejected(AprsError::Ax25(_)),
            }
        ));
        assert_eq!(
            first.0.last().copied(),
            Some(AprsTraceEvent {
                at_ms: 6,
                kind: AprsTraceKind::Expired {
                    cutoff_ms: 7,
                    removed: 1,
                },
            })
        );
    }

    #[test]
    fn aprs_simulator_full_capacity_rejects_without_eviction_or_tx_side_effects() {
        let first = aprs_frame(b"DIGI", 0, b";146.940-A*111111z4903.50N/07201.75Wr");
        let second = aprs_frame(b"DIGI", 0, b";147.000-A*111111z4903.50N/07201.75Wr");
        let mut simulator = AprsSimulator::<1>::new();
        assert_eq!(simulator.ingest(&first), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(
            simulator.ingest(&second),
            Err(AprsSimError::Discovery(DiscoveryError::Full))
        );
        let entries = simulator.entries().collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].advertisement.output_frequency.as_hz(),
            146_940_000
        );
        assert!(matches!(
            simulator.trace().last(),
            Some(AprsTraceEvent {
                kind: AprsTraceKind::DiscoveryRejected {
                    error: DiscoveryError::Full,
                    ..
                },
                ..
            })
        ));
    }

    fn expected_bank() -> GeneratedBank {
        bank(6, "PMR446")
    }

    struct TamperReadTransport {
        inner: SimTransport,
        response: std::collections::VecDeque<u8>,
    }

    impl TamperReadTransport {
        fn new(device: SimDevice) -> Self {
            Self {
                inner: SimTransport::new(device),
                response: std::collections::VecDeque::new(),
            }
        }
    }

    impl ProtocolTransport for TamperReadTransport {
        type Error = core::convert::Infallible;

        fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
            let request = decode_packet(&frame[..frame.len() - 1]).unwrap();
            self.inner.send(frame)?;
            if request.command() == Command::ReadObject {
                let mut encoded = [0_u8; MAX_ENCODED_FRAME];
                let length = self.inner.receive(&mut encoded)?;
                let response = decode_packet(&encoded[..length - 1]).unwrap();
                let mut payload = response.payload().to_vec();
                *payload.last_mut().unwrap() ^= 1;
                let altered = Frame::new(
                    response.service(),
                    response.flags(),
                    response.sequence(),
                    response.command(),
                    &payload,
                )
                .unwrap();
                let altered_length = encode_frame(&altered, &mut encoded).unwrap();
                self.response.extend(&encoded[..altered_length]);
            }
            Ok(())
        }

        fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
            if self.response.is_empty() {
                return self.inner.receive(buffer);
            }
            let count = buffer.len().min(self.response.len());
            for destination in &mut buffer[..count] {
                *destination = self.response.pop_front().unwrap();
            }
            Ok(count)
        }
    }

    /// A request at another frequency, so its token cannot transmit `rf_channel`.
    fn other_rf_request() -> radio_tx_policy::TxRequest {
        let mut channel = rf_channel();
        channel.transmit = Frequency::from_hz(145_525_000).unwrap();
        tx_request(channel)
    }

    fn rf_channel() -> ActiveChannel {
        ActiveChannel {
            receive: Frequency::from_hz(145_500_000).unwrap(),
            transmit: Frequency::from_hz(145_500_000).unwrap(),
        }
    }

    fn run_rf_script() -> (Vec<RfTraceEvent>, super::RfDriverState) {
        let policy = tx_fixture::policy(true);
        let request = tx_request(rf_channel());
        let amateur = policy.authorise(&request).unwrap();
        let other_request = other_rf_request();
        let other = policy.authorise(&other_request).unwrap();
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
        assert!(simulator.start_transmit(&request, &other).is_err());

        simulator.advance_ms(2);
        simulator.inject_failure_after(3);
        assert!(simulator.start_transmit(&request, &amateur).is_err());
        assert_eq!(simulator.state(), super::RfDriverState::Faulted);

        simulator.advance_ms(3);
        simulator.recover_to_standby().unwrap();
        simulator.advance_ms(5);
        simulator.start_transmit(&request, &amateur).unwrap();
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
        let policy = tx_fixture::policy(true);
        let other_request = other_rf_request();
        let other = policy.authorise(&other_request).unwrap();
        let mut simulator = RfSimulator::new();
        simulator.recover_to_standby().unwrap();
        let before = simulator.trace();

        assert!(simulator
            .start_transmit(&tx_request(rf_channel()), &other)
            .is_err());
        assert_eq!(simulator.trace(), before);
        assert!(!simulator.trace().iter().any(|event| matches!(
            event.kind,
            RfTraceKind::TransmitStarted { .. }
                | RfTraceKind::RegisterWritten { value: 0x80FE, .. }
        )));
    }

    fn scan_policy(enabled: bool) -> TxPolicy {
        tx_fixture::policy(enabled)
    }

    #[test]
    fn complete_tx_denials_emit_no_rf_writes_and_repeat_the_same_trace() {
        fn run() -> (Vec<ChannelTraceEvent>, Vec<RfTraceEvent>) {
            let mut simulator =
                ChannelSimulator::activate(bank(8, "DENY"), 0, ScanConfig::new(10, 30).unwrap())
                    .unwrap();
            let policy = scan_policy(true);
            let original = tx_request(simulator.current_channel());
            let rf_before = simulator.rf_trace();
            for field in 0..8 {
                let mut request = original;
                match field {
                    0 => request.hardware = None,
                    1 => request.calibration = None,
                    2 => request.configuration = None,
                    3 => request.state = radio_tx_policy::TxState::Faulted,
                    4 => request.timeout_ms = 0,
                    5 => request.conducted_mw = 501,
                    6 => request.modulation = Modulation::Am,
                    _ => request.target_id += 1,
                }
                simulator.advance_ms(7);
                assert_eq!(
                    simulator.start_transmit(&policy, &request),
                    Err(ChannelSimError::Transmit(ChannelTxError::PolicyDenied))
                );
                assert_eq!(simulator.rf_trace(), rf_before);
            }
            (simulator.trace().to_vec(), simulator.rf_trace())
        }
        assert_eq!(run(), run());
    }

    #[test]
    fn rf_simulator_refuses_stale_generation_and_power_without_tx_events() {
        let policy = scan_policy(true);
        let request = tx_request(tx_fixture::channel(TxClass::LicenceFreePlan));
        let token = policy.authorise(&request).unwrap();
        let mut simulator = RfSimulator::new();
        simulator.recover_to_standby().unwrap();
        let before = simulator.trace();
        for power in [false, true] {
            let mut current = request;
            if power {
                current.conducted_mw += 1;
            } else {
                current.configuration.as_mut().unwrap().generation += 1;
            }
            assert!(simulator.start_transmit(&current, &token).is_err());
            assert_eq!(simulator.trace(), before);
        }
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
        simulator
            .start_transmit(&scan_policy(true), &tx_request(simulator.current_channel()))
            .unwrap();
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
            simulator.start_transmit(&scan_policy(true), &tx_request(simulator.current_channel())),
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
            simulator.start_transmit(
                &scan_policy(false),
                &tx_request(simulator.current_channel())
            ),
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
        assert_eq!(
            compiled.report().storage_bytes,
            u32::try_from(OBJECT_ENTRY_HEADER_LEN + GENERATED_BANK_CORE_LEN).unwrap()
        );
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
        assert!(tx_policy
            .authorise(&tx_request(tx_fixture::channel(TxClass::LicenceFreePlan)))
            .is_err());
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
        let encoded_len = u16::try_from(GENERATED_BANK_CORE_LEN).unwrap();
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
    fn shared_verified_write_backup_and_restore_workflows_round_trip() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(2, "two"));
        project.add_generated_bank(bank(1, "one"));

        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        let mut programmer = Programmer::connect(SimTransport::new(device)).unwrap();
        let receipt = programmer.write_configuration_verified(&compiled).unwrap();
        assert_eq!(receipt.generation, 1);
        assert_eq!(receipt.report, compiled.report());

        let backup = programmer.backup_configuration().unwrap();
        assert_eq!(backup.generation, 1);
        assert_eq!(backup.report, compiled.report());
        assert_eq!(
            decode_configuration_image(&backup.image)
                .unwrap()
                .objects()
                .collect::<Vec<_>>(),
            compiled.objects()
        );

        let mut restored = Programmer::connect(SimTransport::new(SimDevice::new())).unwrap();
        let restore_receipt = restored.restore_configuration_image(&backup.image).unwrap();
        assert_eq!(restore_receipt.generation, 1);
        assert_eq!(restore_receipt.report, compiled.report());
        assert_eq!(restored.backup_configuration().unwrap().image, backup.image);
    }

    #[test]
    fn verified_write_rejects_tampered_read_back() {
        let mut project = RadioProject::new();
        project.add_generated_bank(bank(1, "one"));
        let device = SimDevice::new();
        let compiled = radio_programmer::ConfigurationCompiler::new(device.capabilities())
            .compile(&project)
            .unwrap();
        let mut programmer = Programmer::connect(TamperReadTransport::new(device)).unwrap();
        assert!(matches!(
            programmer.write_configuration_verified(&compiled),
            Err(ProgrammerError::VerificationMismatch)
        ));
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
            &[0_u8; GENERATED_BANK_CORE_LEN],
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
