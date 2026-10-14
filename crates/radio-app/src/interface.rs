//! The interface task: display, keypad, receiver, and transmitter.
//!
//! It owns the radio chip and everything the operator touches. Programmed
//! channels and the VFO both drive one banked receive controller, the shared
//! operator shell turns key edges into intents, and every transmission goes
//! through the shared TX controller and policy.

use embassy_time::{Duration, Instant, Timer};
use radio_bk4819::{
    AfOutput, Bk4819, ReadbackRegister, ReceiveSetup, RegisterBus, SquelchThresholds,
};
use radio_channel_control::{
    BankedReceiveController, ChannelReceiveSetup, ChannelSource, ReceiveObservation, ReceiveState,
    ReceiveUpdate, TimerDirective, TimerToken,
};
use radio_channel_plan::{BankName, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord};
use radio_domain::{
    Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel,
    Tone,
};
use radio_platform::backlight::Backlight;
use radio_platform::battery::Battery;
use radio_platform::configuration_change::{ConfigurationAcknowledgement, ConfigurationCandidate};
use radio_platform::dual_watch::{DualWatch, Listening as Watching};
use radio_platform::operator_display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    ListRow, MemoryState, OperatingView, SelectorRow, SlotChannel, SlotPlace, SlotView,
    FRAME_BYTES, LIST_ROWS,
};
use radio_platform::operator_shell::{
    Context, Intent, Mode, Screen, Setting, Shell, Slot, SlotId, Source, HOLD_MILLISECONDS,
    SQUELCH_LEVELS, TX_POWER_ROWS, TX_TIMEOUT_CHOICES, VFO_STEPS_HZ,
};
use radio_platform::ptt::PttFilter;
use radio_platform::receive_app::{
    Effect as SharedEffect, Effects as SharedEffects, Event as SharedEvent, ReceiveApp,
};
use radio_platform::transmit::{self, Delay, TxCalibration, TxTarget};
use radio_platform::tx_control::{self, Action, Controller as TxController, State as TxPhase};
use radio_tx_policy::{TxRequest, TxState};

use crate::configuration::Programmed;
use crate::drivers::{BatterySense, Display, Keypad, Memory, Speaker, Target};
use crate::keys::{Debouncer, Edge};
use crate::operator_state::{OperatorState, OtherSlot};
use crate::serial::read_image;
use crate::state::{
    bus_available, now_ms, peripheral_clock_khz, reset_cause, serial_counters, Publication,
    TaskCell, BATTERY_CALIBRATION, INDEX, OPERATOR_PLACE, PROGRAMMED, SERIAL_ACTIVITY,
    TX_CALIBRATION, TX_POLICY,
};
use crate::unified_receive::{shared_channel, translate, ReceiverEffect};

/// Interval between receive samples while audio is routed.
///
/// This is the squelch's reaction time as well as the meter's refresh rate: the
/// speaker opens and shuts on these samples, so a half-second interval would
/// clip the start of every transmission and leave noise after the end of it.
/// One sample is a handful of bit-banged register reads, and the serial link
/// still holds the bus off while a host is mid-exchange.
const RF_SAMPLE_MILLISECONDS: u64 = 60;

/// Interval between receive samples while a scan is walking channels.
///
/// This is the scan's resolution. A dwell is judged in whole samples, so a
/// dwell shorter than two of these cannot be judged at all, and the operating
/// interval above would quantise every short dwell to itself.
///
/// It is deliberately far below any plausible receiver settling time. How long
/// a board needs after a retune before a squelch reading means anything is
/// unmeasured — `RISK-008` — and the dwell is the operator's control over that.
/// This must not quietly become a second one.
const SCAN_SAMPLE_MILLISECONDS: u64 = 5;

/// Interval between user interface passes.
const LOOP_MILLISECONDS: u64 = 5;

/// Interval between user interface passes while a scan is running.
///
/// Every deadline the loop has is quantised to its own period, so a scan
/// stepping every twenty milliseconds cannot be paced by a loop which wakes
/// every five. A scan is short-lived and the operator is holding the radio
/// while it runs, so the cost of a faster pass is one they are paying for.
const SCAN_LOOP_MILLISECONDS: u64 = 1;

/// Shortest interval between redraws caused by a changed meter reading.
const METER_REDRAW_MILLISECONDS: u64 = 300;

/// Seconds between battery conversions.
///
/// A pack discharges over hours, so this is about how quickly the operator
/// should see a fresh pack after a battery change rather than about tracking
/// the discharge itself.
const BATTERY_SAMPLE_SECONDS: u64 = 2;

/// Milliseconds a new place must hold still before it is worth writing down.
///
/// An operator walking to a channel passes through every channel in between,
/// and none of those is where they left the radio. Waiting for the selection to
/// settle turns a walk across a bank into one record instead of thirty.
const PLACE_SETTLE_MILLISECONDS: u64 = 3_000;

/// The TX settling waits, blocking because the radio bus already blocks.
struct BlockingDelay;

impl Delay for BlockingDelay {
    fn delay_ms(&mut self, milliseconds: u32) {
        embassy_time::block_for(Duration::from_millis(u64::from(milliseconds)));
    }
}

/// Receive state owned by the user interface task.
#[allow(clippy::struct_excessive_bools)]
struct Receiver<B: RegisterBus, S: Speaker> {
    radio: Bk4819<B>,
    /// The receive audio amplifier.
    ///
    /// It is claimed the first time a channel is tuned rather than at boot,
    /// because the programming cable shares the speaker and microphone jack
    /// and an unprogrammed radio has nothing to play through it.
    speaker: S,
    faulted: bool,
    started: bool,
    audio_routed: bool,
    frequency_hz: u32,
    rssi_raw: u16,
    squelch_open: bool,
    /// The last tuned setup and radio-wide squelch, restored after TX.
    last_tune: Option<(ChannelReceiveSetup, SquelchLevel)>,
}

impl<B: RegisterBus, S: Speaker> Receiver<B, S> {
    fn new(radio: Bk4819<B>, speaker: S) -> Self {
        Self {
            radio,
            speaker,
            faulted: false,
            started: false,
            audio_routed: false,
            frequency_hz: 0,
            rssi_raw: 0,
            squelch_open: false,
            last_tune: None,
        }
    }

    /// Executes one TX controller effect; receive restore re-tunes the last
    /// setup through the ordinary receive path.
    fn transmit_effect(&mut self, effect: &tx_control::Effect<'_>, bias: Option<u8>) -> bool {
        match effect.action() {
            Action::RestoreReceive => {
                let Some((setup, squelch)) = self.last_tune else {
                    return false;
                };
                self.faulted = false;
                self.tune(setup, squelch);
                !self.faulted
            }
            action => {
                if action == Action::PrepareTransmit {
                    self.set_speaker(false);
                    self.audio_routed = false;
                }
                transmit::execute(
                    effect,
                    bias,
                    None,
                    &mut self.radio,
                    || {},
                    &mut BlockingDelay,
                )
            }
        }
    }

    /// Enables the receive audio chain.
    ///
    /// A receiver nobody can hear is not receiving, so this is not an operator
    /// mode: audio follows the tuned channel. This drives the receive audio
    /// chain only, the chip's audio output and the speaker amplifier, and it
    /// cannot key the radio.
    fn route_audio(&mut self) {
        if self.faulted || !self.started {
            return;
        }
        if self
            .radio
            .set_af_output(Modulation::Fm, AfOutput::Demodulated)
            .is_err()
        {
            self.faulted = true;
            return;
        }
        self.speaker.claim();
        self.audio_routed = true;
        // The amplifier is claimed, not opened: the squelch decides what the
        // operator actually hears, and it has not been sampled on this channel
        // yet. A retune therefore lands in silence rather than in noise.
        self.gate_audio();
    }

    fn set_chip_audio(&mut self, enabled: bool) {
        if self.faulted || !self.started {
            return;
        }
        let output = if enabled {
            AfOutput::Demodulated
        } else {
            AfOutput::Mute
        };
        if self.radio.set_af_output(Modulation::Fm, output).is_err() {
            self.faulted = true;
            return;
        }
        self.audio_routed = enabled;
        if enabled {
            self.speaker.claim();
        }
        if !enabled {
            self.set_speaker(false);
        }
    }

    fn set_speaker(&mut self, enabled: bool) {
        self.speaker.set_enabled(enabled);
    }

    /// Drives the speaker amplifier from the squelch link.
    ///
    /// The chip's carrier squelch is the decision and this is the consequence,
    /// so the operator hears exactly what the level they chose lets through. At
    /// level zero, and while monitoring, the link reads permanently open and
    /// the amplifier simply stays on.
    fn gate_audio(&mut self) {
        self.speaker.set_enabled(self.squelch_open);
    }

    /// Applies one controller-selected channel to the receiver.
    ///
    /// `radio_wide` is the level the operator set for the whole radio. It
    /// replaces whatever level the channel was programmed with, because a
    /// global control a stored channel could silently veto would not be one.
    /// A level the controller has already forced open is left alone: that is
    /// monitoring, and monitoring outranks both.
    fn tune(&mut self, setup: ChannelReceiveSetup, radio_wide: SquelchLevel) {
        if !self.started {
            self.bring_up();
        }
        if self.faulted {
            return;
        }
        let request = ReceiveSetup {
            frequency: setup.frequency,
            modulation: setup.modulation,
            bandwidth: setup.bandwidth,
            tone: setup.tone,
            squelch: SquelchThresholds::for_level(if setup.squelch.is_open() {
                setup.squelch
            } else {
                radio_wide
            }),
            af: AfOutput::Demodulated,
        };
        if self.radio.configure_receive(&request).is_err() {
            self.faulted = true;
            return;
        }
        self.last_tune = Some((setup, radio_wide));
        self.frequency_hz = setup.frequency.as_hz();
        // The previous channel's link result says nothing about this one.
        self.squelch_open = false;
        let _ = self.radio.read_back(ReadbackRegister::FilterBandwidth);
        self.route_audio();
    }

    /// Samples metrics from the tuned receiver.
    #[allow(clippy::single_match_else)]
    fn observe(&mut self, gate_locally: bool) -> Option<ReceiveObservation> {
        if self.faulted || !self.started {
            return None;
        }
        match self.radio.receive_metrics(Tone::None) {
            Ok(metrics) => {
                self.rssi_raw = u16::try_from(metrics.rssi_dbm_x2 + 320).unwrap_or(0);
                self.squelch_open = metrics.squelch_open;
                if gate_locally {
                    self.gate_audio();
                }
                Some(ReceiveObservation {
                    squelch_open: metrics.squelch_open,
                    tone_matched: None,
                })
            }
            Err(_) => {
                self.faulted = true;
                None
            }
        }
    }

    fn bring_up(&mut self) {
        if self.radio.recover_to_standby().is_err() || self.radio.initialise().is_err() {
            self.faulted = true;
            return;
        }
        self.started = true;
    }
}

/// Applies the common receive application's ordered effects to the receiver.
///
/// The shared redraw effect only marks the shell renderer dirty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SharedApplyOutcome {
    redraw: bool,
    complete: bool,
}

fn apply_shared_effects<B: RegisterBus, S: Speaker>(
    effects: SharedEffects,
    receiver: &mut Receiver<B, S>,
) -> SharedApplyOutcome {
    let mut redraw = false;
    let mut complete = !receiver.faulted;
    for effect in effects.iter() {
        let Ok(effect) = translate(effect) else {
            receiver.faulted = true;
            complete = false;
            continue;
        };
        match effect {
            ReceiverEffect::ConfigurationChange(_)
            | ReceiverEffect::ConfigurationAcknowledged(_) => {}
            ReceiverEffect::Tune { setup, audio } => {
                // The shared effect carries the selected radio-wide level in
                // the complete setup. Do not read the shell's independent
                // setting here: doing so could silently apply a stale value
                // after a shared retune.
                receiver.tune(setup, setup.squelch);
                receiver.set_chip_audio(audio);
            }
            ReceiverEffect::SetChipAudio(enabled) => receiver.set_chip_audio(enabled),
            ReceiverEffect::SetSpeaker(enabled) => receiver.set_speaker(enabled),
            ReceiverEffect::Redraw(_) => redraw = true,
        }
        if receiver.faulted {
            complete = false;
        }
    }
    if !complete {
        // Continue through safe effects above so a failed chip operation does
        // not skip the speaker gate or semantic redraw. This final guard also
        // covers a failure in the translation boundary itself.
        receiver.set_speaker(false);
        redraw = true;
    }
    SharedApplyOutcome { redraw, complete }
}

/// What the operator is listening to: the VFO, or the programmed channels.
///
/// The programmed variant holds only the index. Counting, bank filtering and
/// scan navigation — the walks which touch every channel — are answered from it
/// and the bank masks without reading memory. Only materialising a record,
/// once per channel actually shown or tuned, reads the memory.
enum Listening<M: 'static> {
    Vfo(ChannelRecord),
    Memory(Programmed, &'static TaskCell<Option<M>>),
}

impl<M> Clone for Listening<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for Listening<M> {}

impl<M: Memory> ChannelSource for Listening<M> {
    fn len(&self) -> u16 {
        match self {
            Self::Vfo(_) => 1,
            Self::Memory(programmed, _) => programmed.len(),
        }
    }

    fn get(&self, index: u16) -> Option<ChannelRecord> {
        match self {
            Self::Vfo(record) => (index == 0).then_some(*record),
            Self::Memory(_, memory) => memory.with_mut(|cell| {
                let memory = cell.as_mut()?;
                INDEX.with(|image| {
                    image.channel_at(|offset, buffer| read_image(memory, offset, buffer), index)
                })
            }),
        }
    }

    fn member_at(&self, index: u16, bank: BankId) -> bool {
        match self {
            Self::Vfo(_) => false,
            Self::Memory(..) => INDEX.with(|image| image.member_at(index, bank)),
        }
    }
}

/// The memory channel the operator last selected.
///
/// The index is how selection addresses it and the identifier is how a restore
/// checks the index still names the same channel, so both are kept.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Selected {
    index: u16,
    channel_id: u16,
}

impl Selected {
    /// Returns the memory selection one retained place names, if it named one.
    fn from_place(place: OperatorState) -> Option<Self> {
        place.memory_mode.then_some(Self {
            index: place.index,
            channel_id: place.channel_id,
        })
    }

    /// Returns the memory selection of the slot not selected, if it had one.
    fn from_other(place: OperatorState) -> Option<Self> {
        place
            .other
            .filter(|other| other.memory_mode)
            .map(|other| Self {
                index: other.index,
                channel_id: other.channel_id,
            })
    }
}

/// Applies one controller update: the tuning it asks for, and the timer it arms.
///
/// The controller owns every deadline a scan has and expresses them as
/// directives rather than as waits, so this is the whole of the scan's clock:
/// arm what it asked for, cancel what it cancelled, and leave the rest alone.
fn apply(
    update: ReceiveUpdate,
    pending: &mut Option<ChannelReceiveSetup>,
    timer: &mut Option<(TimerToken, Instant)>,
) {
    if let Some(activation) = update.activation {
        *pending = Some(activation.setup);
    }
    match update.timer {
        TimerDirective::Unchanged => {}
        TimerDirective::Cancel => *timer = None,
        TimerDirective::Arm { token, after_ms } => {
            *timer = Some((
                token,
                Instant::now() + Duration::from_millis(u64::from(after_ms)),
            ));
        }
    }
}

/// Rolls back the adapter-local squelch staging before replacing its receive
/// activation. The shared app is cancelled before its instance is dropped,
/// so no later target operation can acknowledge the wrong generation.
fn cancel_pending_squelch(
    shell: &mut Shell,
    shared_receive: &mut Option<ReceiveApp>,
    pending_shared_squelch: &mut Option<SharedEffects>,
    pending_operator_squelch: &mut Option<SquelchLevel>,
) {
    *pending_shared_squelch = None;
    if pending_operator_squelch.take().is_some() {
        if let Some(app) = shared_receive.as_mut() {
            let _ = app.apply(SharedEvent::CancelSquelch);
        }
        shell.reject_squelch();
    }
}

/// Points the shell at one configuration, and at the place it was left in.
///
/// A radio which has been used before comes back to the source, bank, and
/// frequency the operator chose. One which has not starts on its channels,
/// because a programmed radio which opened in the VFO would look unprogrammed.
fn adopt_settings(shell: &mut Shell, programmed: &Programmed) {
    let (banks, bank_count) = programmed.populated_banks();
    shell.set_banks(banks, bank_count);
    let config = programmed.config();
    shell.set_squelch(config.squelch);
    shell.set_tx(config.tx_power, config.tx_timeout);
}

/// Points the shell at the source the operator was last listening to.
///
/// This runs once, when the radio adopts the configuration it booted with. A
/// later configuration replaces what the radio holds, not what the operator is
/// doing with it: someone changing a squelch level has not asked to be returned
/// to the top of their channel list, or dragged out of the VFO.
fn adopt_place(shell: &mut Shell, programmed: &Programmed, place: Option<OperatorState>) {
    if let Some(place) = place {
        shell.restore_vfo(place.vfo_hz, usize::from(place.step_index));
        shell.set_tx_disabled(place.tx_disabled);
        shell.set_dual_watch(place.dual_watch);
        if let Some(other) = place.other {
            let active = if place.b_selected {
                SlotId::B
            } else {
                SlotId::A
            };
            // With no channels the VFO is all either slot can have.
            let memory = other.memory_mode && !programmed.is_empty();
            shell.restore_other(
                active,
                Slot {
                    mode: if memory { Mode::Memory } else { Mode::Vfo },
                    bank_filter: other.bank,
                    vfo_hz: other.vfo_hz,
                    step_index: usize::from(other.step_index),
                },
            );
        }
    }
    if programmed.is_empty() {
        // The VFO is the only source this radio has, whatever it was left on.
        return;
    }
    match place {
        Some(place) => shell.restore_source(place.memory_mode, place.bank),
        None => shell.select_memory(),
    }
}

/// Adopts one published configuration, and the retained place it carries.
///
/// The serial task publishes an empty configuration before it has read the
/// memory, then the stored one with the place beside it. The interface may take
/// either first, so the place is restored whenever it arrives. Without one, a
/// radio which had no channels and now has some is being given them and leaves
/// the VFO for them; one which already had them is only being told about a
/// changed setting, and the operator stays exactly where they were.
fn adopt_publication(
    shell: &mut Shell,
    programmed: &Programmed,
    was_empty: bool,
    place: Option<OperatorState>,
    selected: &mut Option<Selected>,
    other_selected: &mut Option<Selected>,
) {
    adopt_settings(shell, programmed);
    if place.is_some() {
        adopt_place(shell, programmed, place);
        *selected = place.and_then(Selected::from_place);
        *other_selected = place.and_then(Selected::from_other);
    } else if was_empty && !programmed.is_empty() {
        shell.select_memory();
        *selected = None;
        *other_selected = None;
    }
}

/// One slot's receive controller, and the tune it asked for when built.
type Activation<M> = (
    BankedReceiveController<Listening<M>>,
    Option<ChannelReceiveSetup>,
);

/// Builds the receive controller for the selected slot.
///
/// `keep` is the memory channel the operator is on, which survives the rebuild
/// if the configuration still has it. Every caller has one except the first: a
/// radio which has just been switched on takes it from the retained place, and
/// everything afterwards takes it from where the operator actually is.
fn activate<M: Memory>(
    programmed: &Programmed,
    shell: &Shell,
    keep: Option<Selected>,
    memory: &'static TaskCell<Option<M>>,
) -> Option<Activation<M>> {
    activate_slot(programmed, shell.slot(), shell.tx_power(), keep, memory)
}

/// Builds the receive controller for the slot not selected.
fn activate_other<M: Memory>(
    programmed: &Programmed,
    shell: &Shell,
    keep: Option<Selected>,
    memory: &'static TaskCell<Option<M>>,
) -> Option<Activation<M>> {
    activate_slot(
        programmed,
        shell.other_slot(),
        shell.tx_power(),
        keep,
        memory,
    )
}

/// Builds the receive controller for one slot; `tx_power` is the VFO's level.
fn activate_slot<M: Memory>(
    programmed: &Programmed,
    slot: Slot,
    tx_power: u8,
    keep: Option<Selected>,
    memory: &'static TaskCell<Option<M>>,
) -> Option<Activation<M>> {
    let (memory, bank) = match slot.mode {
        Mode::Vfo => (vfo_source(slot, tx_power)?, None),
        Mode::Memory => {
            if programmed.is_empty() {
                return None;
            }
            (Listening::Memory(*programmed, memory), slot.bank_filter)
        }
    };
    let keep = keep.map(|kept| (kept.index, ChannelId::new(kept.channel_id)));
    let (controller, update) =
        BankedReceiveController::activate_at(memory, programmed.config(), bank, keep).ok()?;
    Some((
        controller,
        update.activation.map(|activation| activation.setup),
    ))
}

/// Builds the one-channel source holding the VFO frequency.
///
/// The VFO is expressed as an ordinary receive-only channel record so it reuses
/// the whole programmed receive path unchanged. It is `TxClass::Never` like
/// everything else this image constructs.
fn vfo_source<M>(slot: Slot, tx_power: u8) -> Option<Listening<M>> {
    let receive = Frequency::from_hz(slot.vfo_hz).ok()?;
    let step = FrequencyStep::from_hz(slot.vfo_step_hz()).ok()?;
    let record = ChannelRecord::new(ChannelDefinition {
        id: ChannelId::new(1),
        name: ChannelName::new("VFO").ok()?,
        receive,
        // Receive-only: the transmit frequency mirrors receive and the class
        // denies transmission outright.
        transmit: receive,
        rx_tone: Tone::None,
        tx_tone: Tone::None,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        // The VFO has no programmed power of its own, so it takes the default.
        power: PowerLevel::try_from(tx_power).unwrap_or(PowerLevel::Low1),
        step,
        flags: ChannelFlags::default(),
        bank: None,
    })
    .ok()?;
    Some(Listening::Vfo(record))
}

/// Runs the interface task until the display fails.
///
/// A failed display ends the task. That can only happen between passes with
/// the transmitter idle, so the radio is left receiving and the serial task
/// keeps answering the host.
///
/// `radio` is the target's BK4819 with its register bus and profile, and
/// `battery` carries the target's discharge curve.
///
/// # Panics
///
/// Only on a broken internal invariant: a staged squelch level or a validated
/// shared channel that has gone missing.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub async fn run<D, K, B, S, V, M>(
    mut display: D,
    mut keypad: K,
    radio: Bk4819<B>,
    speaker: S,
    mut battery_sense: V,
    mut battery: Battery,
    target: Target,
    memory: &'static TaskCell<Option<M>>,
) where
    D: Display,
    K: Keypad,
    B: RegisterBus,
    S: Speaker,
    V: BatterySense,
    M: Memory,
{
    let mut frame = [0_u8; FRAME_BYTES];
    render_info_screen(
        &mut frame,
        target.identity,
        0,
        0,
        false,
        MemoryState::Unknown,
        serial_counters(),
        peripheral_clock_khz(),
        reset_cause(),
    );
    if !display.initialise().await || !display.frame(&frame).await {
        return;
    }

    // The operator interface does not wait for the serial task. Waiting for its
    // first publication tied the whole radio to that task starting: when it
    // died, the display kept showing this boot frame and no key did anything,
    // which is how `AFIK-K1-2.6` to `2.9` reached the operator. An empty
    // configuration is the correct starting state, and the loop below adopts a
    // publication the moment one arrives.
    let publication = PROGRAMMED.try_take();
    let mut generation = publication.as_ref().map_or(0, |value| value.generation);
    let mut retained = publication.as_ref().is_some_and(|value| value.retained);
    let mut memory_state = publication
        .as_ref()
        .map_or(MemoryState::Unknown, |value| value.memory);
    let place = publication.as_ref().and_then(|value| value.place);
    let mut programmed = publication.map_or_else(Programmed::empty, |value| value.programmed);

    let mut shell = Shell::new();
    // The retained place is where the operator left this radio, and it is the
    // only thing which knows on the pass that adopts it.
    let mut selected = None;
    // The slot not selected has a controller of its own, which keeps its
    // channel and bank while the operator uses the other.
    let mut other_selected = None;
    adopt_publication(
        &mut shell,
        &programmed,
        true,
        place,
        &mut selected,
        &mut other_selected,
    );
    let mut activation = activate(&programmed, &shell, selected, memory);
    let mut pending = activation.as_mut().and_then(|(_, setup)| setup.take());
    let mut other_activation = activate_other(&programmed, &shell, other_selected, memory);

    let mut receiver = Receiver::new(radio, speaker);
    // A reset can follow a panic in the middle of a transmission, so the PA
    // and chip TX are switched off before anything else uses the radio.
    let _ = receiver.radio.disable_pa();
    let _ = receiver.radio.disable_transmit();
    // The programmed TX ranges decide every transmission, in channel and VFO
    // mode alike, and are taken again with each configuration (`ADR-101`).
    let mut tx_policy = TX_POLICY.with(|policy| *policy);
    shell.set_tx_classes(tx_policy.classes());
    let mut tx_calibration: Option<TxCalibration> = None;
    // Created once the receiver has a channel, so its startup restore has
    // something to restore.
    let mut tx: Option<TxController> = None;
    let mut ptt = PttFilter::new(keypad.ptt_pressed(), Instant::now().as_millis());
    let mut tx_tuned_hz = 0_u32;
    let mut tx_revision = 1_u32;
    // Present only while the selected complete setup is one of the common
    // PMR446 examples. Arbitrary programmed channels and the VFO use the
    // banked controller alone.
    let mut shared_receive: Option<ReceiveApp> = None;
    // A handset squelch choice is staged until the target tune succeeds. The
    // effects are retained here because the bit-banged bus runs below the UI
    // task's scheduling boundary.
    let mut pending_shared_squelch: Option<SharedEffects> = None;
    let mut pending_operator_squelch: Option<SquelchLevel> = None;
    // Fresh token for each shared-app replacement, so queued adapter effects
    // cannot cross a publication boundary even when storage generation wraps.
    let mut shared_instance_id = 1_u16;
    // The shared app owns this semantic preference. Keep it across a
    // configuration publication while replacing the activation generation;
    // receiver hardware state is not a valid source because a first tune
    // deliberately starts with the chip audio route closed.
    let mut shared_audio = true;
    let mut debounce = Debouncer::new();
    // When the key currently held down went down, so a hold can be told from a
    // press. Cleared once the hold has acted, so one press acts once.
    let mut held_since: Option<u32> = None;
    // The scan timer the controller asked for, and when it expires.
    let mut scan_timer: Option<(TimerToken, Instant)> = None;
    // Which slot dual watch has the receiver on, whether the last settled
    // reading there was busy, and whether the last pass was transmitting.
    let mut watch = DualWatch::new();
    let mut watched_busy = false;
    let mut transmitted = false;
    let mut quiet_retune = false;
    let mut next_sample = Instant::now();
    // Whether the receiver has produced a reading on the channel it is now on.
    let mut settled = false;
    let mut next_meter_redraw = Instant::now();
    let mut next_battery_sample = Instant::now();
    let mut redraw = true;
    // The place already written down, and the one waiting to settle into it.
    let mut saved_place = place;
    let mut settling: Option<(OperatorState, Instant)> = None;
    let mut backlight = Backlight::new(programmed.config().backlight_seconds, now_ms());
    if let Some(on) = backlight.poll(now_ms()) {
        display.set_backlight(on);
    }

    loop {
        let context = activation
            .as_ref()
            .map_or(Context::default(), |(controller, _)| Context {
                visible_channels: controller.visible_channels(),
                active_index: controller.visible_position(),
                scanning: matches!(controller.state(), ReceiveState::Scanning(_)),
            });

        // The scan's own clock. The controller decides what a dwell or a hold
        // expiring means; this only tells it that one has.
        if let Some((token, deadline)) = scan_timer {
            if Instant::now() >= deadline {
                if pending_operator_squelch.is_none() {
                    scan_timer = None;
                    if let Some((controller, _)) = activation.as_mut() {
                        if let Ok(update) = controller.timer_elapsed(token) {
                            apply(update, &mut pending, &mut scan_timer);
                        }
                    }
                }
                redraw = true;
            }
        }

        if let Some(calibration) = BATTERY_CALIBRATION.try_take() {
            battery.calibrate(calibration);
        }

        // The converter is on no shared bus, so this needs no quiet link and
        // costs one short blocking conversion. A pack discharges over hours;
        // sampling it every couple of seconds is already generous, and the
        // first full set of samples is what the indicator waits for.
        if Instant::now() >= next_battery_sample {
            let previous = battery.percent();
            if let Some(raw) = battery_sense.read() {
                battery.sample(raw);
            }
            next_battery_sample = Instant::now() + Duration::from_secs(BATTERY_SAMPLE_SECONDS);
            if battery.percent() != previous {
                redraw = true;
            }
        }

        if let Some(publication) = PROGRAMMED.try_take() {
            generation = publication.generation;
            retained = publication.retained;
            memory_state = publication.memory;
            let was_empty = programmed.is_empty();
            programmed = publication.programmed;
            // A new active snapshot gets a fresh shared activation. The
            // retained object/index machinery stays with the banked path, and
            // the shared app cannot accidentally carry semantic state from a
            // prior generation into the replacement.
            shared_audio = shared_receive.as_ref().is_none_or(|app| app.view().audio);
            shared_instance_id = shared_instance_id.wrapping_add(1);
            if shared_instance_id == 0 {
                shared_instance_id = 1;
            }
            cancel_pending_squelch(
                &mut shell,
                &mut shared_receive,
                &mut pending_shared_squelch,
                &mut pending_operator_squelch,
            );
            shared_receive = None;
            adopt_publication(
                &mut shell,
                &programmed,
                was_empty,
                publication.place,
                &mut selected,
                &mut other_selected,
            );
            if let Some(on) = backlight.set_seconds(programmed.config().backlight_seconds, now_ms())
            {
                display.set_backlight(on);
            }
            if publication.place.is_some() {
                saved_place = publication.place;
                settling = None;
            }
            activation = activate(&programmed, &shell, selected, memory);
            pending = activation.as_mut().and_then(|(_, setup)| setup.take());
            other_activation = activate_other(&programmed, &shell, other_selected, memory);
            tx_policy = TX_POLICY.with(|policy| *policy);
            shell.set_tx_classes(tx_policy.classes());
            // A configuration arriving replaces what a scan was walking.
            scan_timer = None;
            redraw = true;
        }

        if let Some(calibration) = TX_CALIBRATION.try_take() {
            tx_calibration = calibration;
        }
        // PTT transmits on the selected slot, whichever one dual watch has
        // the receiver on.
        let tx_frequency_hz = activation
            .as_ref()
            .map_or(receiver.frequency_hz, |(controller, _)| {
                controller.setup().frequency.as_hz()
            });
        if tx_frequency_hz != tx_tuned_hz {
            tx_tuned_hz = tx_frequency_hz;
            tx_revision = tx_revision.saturating_add(1);
        }
        // The handset's class switches narrow the programmed ranges, and
        // apply from the next pass, mid-transmission included.
        let tx_policy_now = tx_policy.restricted(shell.tx_enabled());
        // The power chosen for this channel, lowered to what the range being
        // used allows.
        let tx_level = transmit::level_within(
            chosen_power(
                &shell,
                activation
                    .as_ref()
                    .map(|(controller, _)| controller.channel()),
            ),
            Frequency::from_hz(tx_frequency_hz)
                .ok()
                .and_then(|frequency| tx_policy_now.max_mw_at(frequency)),
        );
        let tx_request = tx_request(
            &receiver,
            tx_frequency_hz,
            context.scanning,
            pending.is_some() || pending_operator_squelch.is_some(),
            tx_revision,
            tx_calibration,
            target.tx,
            tx_level,
            shell.tx_timeout_ms(),
        );
        let tx_now = Instant::now().as_millis();
        if SERIAL_ACTIVITY.try_take().is_some() {
            ptt.serial_activity(tx_now);
        }
        let event = ptt.sample(tx_now, keypad.ptt_pressed());
        // A key or PTT press lights the display; it still does what it does.
        if matches!(event, tx_control::Event::Pressed) {
            if let Some(on) = backlight.wake(now_ms()) {
                display.set_backlight(on);
            }
        }
        if tx.is_none() && receiver.started && !receiver.faulted {
            tx = TxController::new(1, ptt.pressed(), tx_now).ok();
        }
        if let Some(controller) = tx.as_mut() {
            let tx_policy = tx_policy_now;
            let _ = controller.handle(tx_now, event, &tx_policy, &tx_request.0);
            let bias = tx_request.1;
            transmit::run_effects(controller, &tx_policy, &tx_request.0, tx_now, |effect| {
                receiver.transmit_effect(effect, bias)
            });
            // Keys, host requests and sampling all touch the radio, so they
            // wait until the controller has restored receive.
            if !matches!(
                controller.state(),
                TxPhase::Ready | TxPhase::ReleaseRequired
            ) {
                transmitted = true;
                Timer::after(Duration::from_millis(LOOP_MILLISECONDS)).await;
                continue;
            }
        }
        // Receive was restored to whichever slot dual watch was on; the
        // selected one is where a reply to that transmission comes.
        if core::mem::take(&mut transmitted) && watch.reset(now_ms()) {
            pending = activation
                .as_ref()
                .map(|(controller, _)| controller.setup());
        }

        let sample = keypad.scan().await;
        let now = now_ms();

        let edge = debounce.update(now, sample);
        // A key press or a received signal lights the display; the press
        // still does what it does.
        let woken = backlight.keep_lit_while(
            matches!(edge, Edge::Pressed(_)) || receiver.squelch_open,
            now,
        );
        if let Some(on) = woken {
            display.set_backlight(on);
        }
        let intent = match edge {
            Edge::Pressed(key) => {
                held_since = Some(now);
                shell.press(key, now, context)
            }
            Edge::Released(key) => {
                held_since = None;
                shell.release(key)
            }
            // A key still down past the hold interval is a second, different
            // input from the press which started it, and it acts once.
            Edge::None => match (debounce.held_key(), held_since) {
                (Some(key), Some(since)) if now.wrapping_sub(since) >= HOLD_MILLISECONDS => {
                    held_since = None;
                    shell.hold(key, context)
                }
                _ => shell.tick(now, context),
            },
        };
        match intent {
            Intent::Idle => {}
            Intent::Redraw => redraw = true,
            // Changing source or retuning the VFO rebuilds the receive source
            // from the shell, so both paths share one activation rule. They come
            // before the guard below because selecting the VFO is exactly how an
            // operator leaves an empty memory.
            Intent::SetSource(_) | Intent::TuneVfo => {
                cancel_pending_squelch(
                    &mut shell,
                    &mut shared_receive,
                    &mut pending_shared_squelch,
                    &mut pending_operator_squelch,
                );
                // Choosing a source is the one rebuild the operator did ask
                // for, so a bank change lands on that bank's first channel
                // rather than dragging the old selection into it.
                selected = None;
                let _ = pending.take();
                activation = activate(&programmed, &shell, None, memory);
                pending = activation.as_mut().and_then(|(_, setup)| setup.take());
                redraw = true;
            }
            // VFO/MR rebuilds the same way but keeps the memory selection, so
            // switching back lands on the channel the operator left.
            Intent::ToggleVfo => {
                cancel_pending_squelch(
                    &mut shell,
                    &mut shared_receive,
                    &mut pending_shared_squelch,
                    &mut pending_operator_squelch,
                );
                let _ = pending.take();
                activation = activate(&programmed, &shell, selected, memory);
                pending = activation.as_mut().and_then(|(_, setup)| setup.take());
                scan_timer = None;
                redraw = true;
            }
            // A/B: the two controllers change places and the receiver tunes the
            // newly selected one.
            Intent::SwapSlot => {
                cancel_pending_squelch(
                    &mut shell,
                    &mut shared_receive,
                    &mut pending_shared_squelch,
                    &mut pending_operator_squelch,
                );
                core::mem::swap(&mut activation, &mut other_activation);
                core::mem::swap(&mut selected, &mut other_selected);
                pending = activation
                    .as_ref()
                    .map(|(controller, _)| controller.setup());
                scan_timer = None;
                redraw = true;
            }
            // Every remaining intent acts on a channel. An empty memory has
            // none, so there is nothing to hold open or select, and the
            // interface only redraws.
            // Squelch is a radio-wide setting, so it applies with or without a
            // channel to hear it on and is handled before the guard below.
            Intent::SetSquelch(level) => {
                // Stage only when the target is available and no earlier bus
                // operation is pending. In particular, do not ask the shared
                // app to stage a change that this adapter cannot immediately
                // acknowledge: that would leave its pending state behind a
                // rejected handset choice.
                let mut staged = pending.is_none()
                    && pending_operator_squelch.is_none()
                    && pending_shared_squelch.is_none()
                    && activation.is_some()
                    && shared_receive.is_some();
                if staged {
                    if let Some(app) = shared_receive.as_mut() {
                        let effects = app.apply(SharedEvent::SetSquelch(level));
                        if effects.iter().any(|effect| {
                            matches!(
                                effect,
                                SharedEffect::Tune { squelch, .. } if squelch == level
                            )
                        }) {
                            pending_shared_squelch = Some(effects);
                        } else {
                            staged = false;
                        }
                    } else {
                        staged = false;
                    }
                }
                if staged {
                    pending_operator_squelch = Some(level);
                    if let Some((controller, _)) = activation.as_ref() {
                        let mut setup = controller.setup();
                        setup.squelch = level;
                        pending = Some(setup);
                    } else {
                        pending_operator_squelch = None;
                        pending_shared_squelch = None;
                        if let Some(app) = shared_receive.as_mut() {
                            let _ = app.apply(SharedEvent::CancelSquelch);
                        }
                        staged = false;
                    }
                } else {
                    shell.reject_squelch();
                }
                redraw = true;
            }
            _ if pending_operator_squelch.is_some() => redraw = true,
            _ if activation.is_none() => redraw = true,
            Intent::ToggleMonitor => {
                if let Some((controller, _)) = activation.as_mut() {
                    let monitoring = !controller.is_monitoring();
                    // The banked controller owns target mechanics, while the
                    // shared app owns the semantic monitor state whenever
                    // this activation is on the common PMR path. Keep both
                    // at one explicit desired value rather than maintaining
                    // two independently toggled booleans.
                    if let Some(app) = shared_receive.as_mut() {
                        let _ = apply_shared_effects(
                            app.apply(SharedEvent::SetMonitor(monitoring)),
                            &mut receiver,
                        )
                        .redraw;
                    }
                    let update = controller.set_monitor(monitoring);
                    apply(update, &mut pending, &mut scan_timer);
                }
                redraw = true;
            }
            Intent::StartScan | Intent::StopScan => {
                if let Some((controller, _)) = activation.as_mut() {
                    let update = if matches!(intent, Intent::StartScan) {
                        controller.start_scanning()
                    } else {
                        controller.stop_scanning()
                    };
                    // A scan of one channel, or of none, is refused by the
                    // controller and leaves the radio exactly where it was.
                    if let Ok(update) = update {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                redraw = true;
            }
            Intent::SelectNext | Intent::SelectPrevious | Intent::SelectIndex(_) => {
                if let Some((controller, _)) = activation.as_mut() {
                    let update = match intent {
                        Intent::SelectNext => controller.select_next(),
                        Intent::SelectPrevious => controller.select_previous(),
                        Intent::SelectIndex(position) => controller.select_visible(position),
                        _ => unreachable!(),
                    };
                    if let Ok(update) = update {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                redraw = true;
            }
        }

        // Dual watch. Every action the operator takes is on the selected slot,
        // so any intent beyond a redraw brings the receiver back to it. While
        // dual watch runs, the receiver moves between the slots whenever the
        // one it is on has been quiet for its dwell; scanning, a squelch being
        // staged, or a slot with nothing to tune stops it on the selected one.
        let watching = shell.dual_watch()
            && pending_operator_squelch.is_none()
            && other_activation.is_some()
            && activation.as_ref().is_some_and(|(controller, _)| {
                !matches!(controller.state(), ReceiveState::Scanning(_))
            });
        let acted = !matches!(intent, Intent::Idle | Intent::Redraw);
        if (acted || !watching) && watch.reset(now) && pending.is_none() {
            pending = activation
                .as_ref()
                .map(|(controller, _)| controller.setup());
        }
        if watching && pending.is_none() {
            let dwell = programmed.config().scan_dwell_ms;
            pending = match watch.poll(now, dwell, watched_busy) {
                Some(Watching::Selected) => activation
                    .as_ref()
                    .map(|(controller, _)| controller.setup()),
                Some(Watching::Other) => other_activation
                    .as_ref()
                    .map(|(controller, _)| controller.setup()),
                None => None,
            };
            // A move between quiet slots changes nothing on screen.
            quiet_retune = pending.is_some();
        }

        // What the operator is listening to, in the form the radio would have to
        // restore it from. A memory selection is remembered even while the VFO
        // is in use, so leaving the VFO returns to the channel rather than to
        // the top of the list.
        if let Some((controller, _)) = activation.as_ref() {
            if matches!(shell.mode(), Mode::Memory) {
                selected = Some(Selected {
                    index: controller.index(),
                    channel_id: controller.channel().id().get(),
                });
            }
        }
        let other_slot = shell.other_slot();
        if let Some((controller, _)) = other_activation.as_ref() {
            if matches!(other_slot.mode, Mode::Memory) {
                other_selected = Some(Selected {
                    index: controller.index(),
                    channel_id: controller.channel().id().get(),
                });
            }
        }
        let kept = selected.unwrap_or_default();
        let other_kept = other_selected.unwrap_or_default();
        let place = OperatorState {
            memory_mode: matches!(shell.mode(), Mode::Memory),
            bank: shell.bank_filter(),
            index: kept.index,
            channel_id: kept.channel_id,
            vfo_hz: shell.vfo_hz(),
            step_index: u8::try_from(shell.step_index()).unwrap_or(0),
            squelch: Some(shell.squelch()),
            tx_disabled: shell.tx_disabled(),
            other: Some(OtherSlot {
                memory_mode: matches!(other_slot.mode, Mode::Memory),
                bank: other_slot.bank_filter,
                index: other_kept.index,
                channel_id: other_kept.channel_id,
                vfo_hz: other_slot.vfo_hz,
                step_index: u8::try_from(other_slot.step_index).unwrap_or(0),
            }),
            b_selected: shell.active_slot() == SlotId::B,
            dual_watch: shell.dual_watch(),
        };
        if context.scanning {
            // A scan moves the selection several times a second and none of
            // those is a place the operator chose. Where it stops is.
            settling = None;
        } else if saved_place == Some(place) {
            settling = None;
        } else {
            match settling {
                Some((candidate, deadline)) if candidate == place => {
                    if Instant::now() >= deadline {
                        OPERATOR_PLACE.signal(place);
                        saved_place = Some(place);
                        settling = None;
                    }
                }
                _ => {
                    settling = Some((
                        place,
                        Instant::now() + Duration::from_millis(PLACE_SETTLE_MILLISECONDS),
                    ));
                }
            }
        }

        // The bit-banged radio bus blocks the executor, so it only runs while
        // the serial link is quiet. Bus work is deferred, never dropped.
        if receiver.faulted {
            pending = None;
            cancel_pending_squelch(
                &mut shell,
                &mut shared_receive,
                &mut pending_shared_squelch,
                &mut pending_operator_squelch,
            );
        }
        if let Some(setup) = pending.filter(|_| bus_available()) {
            if let Some(channel) = shared_channel(setup) {
                let had_shared = shared_receive.is_some();
                let initial_squelch = pending_operator_squelch.unwrap_or(setup.squelch);
                let app = shared_receive.get_or_insert_with(|| {
                    ReceiveApp::new_with_identity_and_squelch(shared_instance_id, initial_squelch)
                });
                if !had_shared {
                    let monitoring = activation
                        .as_ref()
                        .is_some_and(|(controller, _)| controller.is_monitoring());
                    let _ = apply_shared_effects(
                        app.apply(SharedEvent::SetMonitor(monitoring)),
                        &mut receiver,
                    );
                }
                let operator_squelch = pending_operator_squelch.take();
                if let Some(squelch_effects) = pending_shared_squelch.take() {
                    let outcome = apply_shared_effects(squelch_effects, &mut receiver);
                    let level = operator_squelch.expect("staged squelch level");
                    let acknowledgement = SharedEvent::SquelchAcknowledged {
                        level,
                        applied: outcome.complete,
                    };
                    let _ = apply_shared_effects(app.apply(acknowledgement), &mut receiver);
                    if outcome.complete && app.squelch() == level {
                        let _ = shell.accept_squelch(level);
                        adopt_squelch(&programmed, generation, retained, memory_state, level);
                    } else {
                        shell.reject_squelch();
                    }
                } else {
                    let candidate = ConfigurationCandidate::from_channel(
                        generation,
                        channel,
                        if had_shared {
                            app.view().audio
                        } else {
                            shared_audio
                        },
                    )
                    .expect("shared channel was validated");
                    let candidate_effects =
                        app.apply(SharedEvent::ConfigurationCandidate(candidate));
                    let change = candidate_effects.iter().find_map(|effect| match effect {
                        SharedEffect::ConfigurationChange(change) => Some(change),
                        _ => None,
                    });
                    let outcome = apply_shared_effects(candidate_effects, &mut receiver);
                    if let Some(change) = change {
                        let acknowledgement = if outcome.complete {
                            ConfigurationAcknowledgement::applied(change)
                        } else {
                            ConfigurationAcknowledgement::rejected(change)
                        };
                        let _ = apply_shared_effects(
                            app.apply(SharedEvent::ConfigurationAcknowledged(acknowledgement)),
                            &mut receiver,
                        );
                        // The banked controller owns the physical monitor intent;
                        // after an applied tune/recovery, make the shared view
                        // agree with that owner. A failed adapter path never
                        // reaches this synchronization and remains fail-silent.
                        if outcome.complete {
                            if let Some((controller, _)) = activation.as_ref() {
                                let _ = apply_shared_effects(
                                    app.apply(SharedEvent::SetMonitor(controller.is_monitoring())),
                                    &mut receiver,
                                );
                            }
                        }
                    }
                    if let Some(level) = operator_squelch {
                        if outcome.complete && app.squelch() == level {
                            let _ = shell.accept_squelch(level);
                            adopt_squelch(&programmed, generation, retained, memory_state, level);
                        } else {
                            shell.reject_squelch();
                        }
                    }
                }
            } else {
                shared_receive = None;
                receiver.tune(setup, setup.squelch);
                if let Some(level) = pending_operator_squelch.take() {
                    if receiver.faulted {
                        shell.reject_squelch();
                    } else {
                        let _ = shell.accept_squelch(level);
                        adopt_squelch(&programmed, generation, retained, memory_state, level);
                    }
                }
            }
            pending = None;
            // Sample this channel from now rather than on whatever grid the
            // last one left behind. A free-running interval meant the first
            // reading of a scanned channel landed anywhere inside the dwell,
            // so how many readings a channel got was a matter of phase.
            next_sample = Instant::now();
            settled = false;
            watched_busy = false;
            redraw |= !core::mem::take(&mut quiet_retune);
        } else if receiver.audio_routed && Instant::now() >= next_sample && bus_available() {
            let was_open = receiver.squelch_open;
            let scanning = activation.as_ref().is_some_and(|(controller, _)| {
                matches!(controller.state(), ReceiveState::Scanning(_))
            });
            // The reading belongs to whichever slot the receiver is on.
            let tuned = match watch.listening() {
                Watching::Selected => activation.as_mut(),
                Watching::Other => other_activation.as_mut(),
            };
            if let (Some(observation), Some((controller, _))) =
                (receiver.observe(shared_receive.is_none()), tuned)
            {
                // Dual watch holds on a busy slot, judged on settled readings
                // only, as a scan is.
                watched_busy = settled && observation.squelch_open;
                if let Some(app) = shared_receive.as_mut() {
                    redraw |= apply_shared_effects(
                        app.apply(SharedEvent::ReceiveSample {
                            squelch_open: observation.squelch_open,
                        }),
                        &mut receiver,
                    )
                    .redraw;
                }
                // While scanning this is what decides whether the channel is
                // busy, so the hold it asks for has to reach the scan's clock.
                //
                // The first reading after a retune is taken while the
                // synthesiser is still settling on the new frequency, and how
                // long that takes on this board is unmeasured — `RISK-008`. It
                // still updates the meter, which is honest about what was read,
                // but it does not get to tell a scan a channel is busy: a false
                // stop costs the whole hold, which is the one failure an
                // operator would notice and could not explain.
                if pending_operator_squelch.is_none() && (settled || !scanning) {
                    if let Ok(update) = controller.observe(observation) {
                        apply(update, &mut pending, &mut scan_timer);
                    }
                }
                settled = true;
            } else if receiver.faulted {
                cancel_pending_squelch(
                    &mut shell,
                    &mut shared_receive,
                    &mut pending_shared_squelch,
                    &mut pending_operator_squelch,
                );
                if let Some(app) = shared_receive.as_mut() {
                    redraw |=
                        apply_shared_effects(app.apply(SharedEvent::ReceiverFault), &mut receiver)
                            .redraw;
                }
            }
            next_sample = Instant::now()
                + Duration::from_millis(if scanning || watching {
                    SCAN_SAMPLE_MILLISECONDS
                } else {
                    RF_SAMPLE_MILLISECONDS
                });
            // The squelch is sampled far faster than a screen needs redrawing.
            // The link opening or shutting is worth showing at once; a moving
            // meter reading is not, and repainting every sample would spend the
            // display bus on nothing an operator can read.
            if was_open != receiver.squelch_open || Instant::now() >= next_meter_redraw {
                next_meter_redraw =
                    Instant::now() + Duration::from_millis(METER_REDRAW_MILLISECONDS);
                redraw = true;
            }
        }

        if redraw {
            render(
                &mut frame,
                &shell,
                activation.as_ref().map(|(controller, _)| controller),
                other_activation.as_ref().map(|(controller, _)| controller),
                watch.listening(),
                &receiver,
                battery.percent(),
                generation,
                retained,
                memory_state,
                target.identity,
                memory,
            );
            if !display.frame(&frame).await {
                return;
            }
            redraw = false;
        }

        Timer::after(Duration::from_millis(if context.scanning {
            SCAN_LOOP_MILLISECONDS
        } else {
            LOOP_MILLISECONDS
        }))
        .await;
    }
}

/// Returns the TX power level for `channel`: the handset override if the
/// operator chose one, otherwise the channel's own. The VFO's record carries
/// the programmed default as its own, which is also the level with nothing
/// tuned.
fn chosen_power(shell: &Shell, channel: Option<ChannelRecord>) -> u8 {
    shell.tx_level(channel.map_or(shell.tx_power(), |channel| channel.power() as u8))
}

/// Adopts a squelch level the operator accepted on the handset.
///
/// It is the operator's own setting, so it is kept in the place record rather
/// than written into the host's image. Republishing the configuration with it
/// rebuilds the controller exactly as a new configuration would, so every later
/// tune uses it.
fn adopt_squelch(
    programmed: &Programmed,
    generation: u32,
    retained: bool,
    memory: MemoryState,
    level: SquelchLevel,
) {
    PROGRAMMED.signal(Publication {
        programmed: programmed.with_squelch(level),
        generation,
        retained,
        memory,
        place: None,
    });
}

/// Builds the TX request for the selected slot's frequency, and the bias it
/// would use. Off-raster, wide, or non-PMR446 tuning is denied by policy.
#[allow(clippy::too_many_arguments)]
fn tx_request<B: RegisterBus, S: Speaker>(
    receiver: &Receiver<B, S>,
    frequency_hz: u32,
    scanning: bool,
    changing: bool,
    revision: u32,
    calibration: Option<TxCalibration>,
    tx: TxTarget,
    level: u8,
    timeout_ms: u32,
) -> (TxRequest, Option<u8>) {
    let frequency = Frequency::from_hz(frequency_hz).ok();
    let state = if receiver.faulted || !receiver.started || frequency.is_none() {
        TxState::Faulted
    } else if scanning {
        TxState::Scanning
    } else if changing {
        TxState::Changing
    } else {
        TxState::Selected
    };
    let bias = calibration
        .zip(frequency)
        .and_then(|(calibration, frequency)| tx.bias(calibration, frequency.as_hz(), level));
    let frequency = frequency.unwrap_or_else(|| {
        Frequency::from_hz(446_006_250).expect("PMR446 channel 1 is a frequency")
    });
    (
        tx.request(frequency, state, revision, bias, level, timeout_ms),
        bias,
    )
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_range_loop
)]
fn render<B: RegisterBus, S: Speaker, M: Memory>(
    frame: &mut [u8; FRAME_BYTES],
    shell: &Shell,
    controller: Option<&BankedReceiveController<Listening<M>>>,
    other: Option<&BankedReceiveController<Listening<M>>>,
    listening: Watching,
    receiver: &Receiver<B, S>,
    battery_percent: Option<u8>,
    generation: u32,
    retained: bool,
    memory_state: MemoryState,
    identity: &[u8],
    memory: &'static TaskCell<Option<M>>,
) {
    // Only an empty memory has no controller: the VFO always has one.
    let visible = controller.map_or(0, BankedReceiveController::visible_channels);
    match shell.screen() {
        Screen::Operating => {
            // Names are fixed-capacity values copied out of the records, so
            // they are held here for as long as the view borrows their bytes.
            let selected_name = controller.map(|controller| controller.channel().name());
            let other_name = other.map(|controller| controller.channel().name());
            let selected = slot_channel(
                shell,
                shell.slot(),
                controller,
                selected_name.as_ref(),
                (listening == Watching::Selected).then_some(receiver),
            );
            let other = slot_channel(
                shell,
                shell.other_slot(),
                other,
                other_name.as_ref(),
                (listening == Watching::Other).then_some(receiver),
            );
            let (a, b) = match shell.active_slot() {
                SlotId::A => (selected, other),
                SlotId::B => (other, selected),
            };
            render_operating_screen(
                frame,
                &OperatingView {
                    a: SlotView {
                        letter: b'A',
                        selected: shell.active_slot() == SlotId::A,
                        channel: a,
                    },
                    b: SlotView {
                        letter: b'B',
                        selected: shell.active_slot() == SlotId::B,
                        channel: b,
                    },
                    receiver_ok: !receiver.faulted,
                    battery_percent,
                    monitoring: controller.is_some_and(BankedReceiveController::is_monitoring),
                    scanning: controller.is_some_and(|controller| {
                        matches!(controller.state(), ReceiveState::Scanning(_))
                    }),
                    dual_watch: shell.dual_watch(),
                    function: shell.function_pending(),
                    entry: shell.entry(),
                    entry_vfo: matches!(shell.mode(), Mode::Vfo),
                },
            );
        }
        Screen::SourceList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            // The rows the shell offers are paged directly, so the cursor it
            // reports needs no translation into a second numbering scheme.
            let first = shell.source_cursor() / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let row = first + offset;
                let Some(source) = shell.source_at(row) else {
                    break;
                };
                let active = shell.is_active_source(row);
                rows[offset] = match source {
                    Source::Vfo => SelectorRow::text(b"VFO", active),
                    Source::AllChannels => SelectorRow::text(b"ALL CHANNELS", active),
                    Source::Bank(bank) => {
                        let name = bank_name(bank, memory);
                        SelectorRow::bank(
                            bank.get(),
                            name.as_ref()
                                .map_or(&[][..], |name| name.as_str().as_bytes()),
                            active,
                        )
                    }
                };
                count += 1;
            }
            render_selector_list(
                frame,
                b"SOURCE",
                &rows[..count],
                shell.source_cursor() - first,
            );
        }
        Screen::Settings => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let mut count = 0;
            for (offset, setting) in shell.settings().iter().take(LIST_ROWS).enumerate() {
                rows[offset] = match setting {
                    Setting::Squelch => SelectorRow::squelch_setting(shell.squelch().get()),
                    Setting::TxPower => SelectorRow::tx_power_setting(shell.tx_override()),
                    Setting::TxTimeout => SelectorRow::tx_timeout_setting(shell.tx_timeout()),
                    Setting::TxClasses => {
                        let (list, programmed) = shell.tx_class_list();
                        let on = list
                            .iter()
                            .flatten()
                            .filter(|class| shell.tx_enabled().contains(**class))
                            .count();
                        SelectorRow::tx_classes_setting(on, programmed)
                    }
                    Setting::DualWatch => SelectorRow::dual_watch_setting(shell.dual_watch()),
                };
                count += 1;
            }
            render_selector_list(frame, b"SETTINGS", &rows[..count], shell.settings_cursor());
        }
        Screen::TxPowerList => {
            // CHANNEL and six levels in five rows: page by whole screens, as
            // the squelch list does.
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let cursor = usize::from(shell.tx_power_cursor());
            let first = cursor / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for (list_row, row) in (0..TX_POWER_ROWS).skip(first).zip(rows.iter_mut()) {
                *row = match list_row.checked_sub(1) {
                    None => SelectorRow::tx_power_channel(shell.tx_override().is_none()),
                    Some(level) => {
                        SelectorRow::tx_power_level(level, shell.tx_override() == Some(level))
                    }
                };
                count += 1;
            }
            render_selector_list(frame, b"TX POWER", &rows[..count], cursor - first);
        }
        Screen::TxTimeoutList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let mut count = 0;
            for (choice, row) in (0..TX_TIMEOUT_CHOICES).zip(rows.iter_mut()) {
                *row = SelectorRow::tx_timeout_choice(choice, choice == shell.tx_timeout());
                count += 1;
            }
            render_selector_list(
                frame,
                b"TX TIMEOUT",
                &rows[..count],
                usize::from(shell.tx_timeout_cursor()),
            );
        }
        Screen::TxClassList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let (list, programmed) = shell.tx_class_list();
            let cursor = usize::from(shell.tx_class_cursor());
            let first = cursor / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for (row, class) in rows
                .iter_mut()
                .zip(list[first..programmed.max(first)].iter().flatten())
            {
                *row = SelectorRow::tx_class(*class, shell.tx_enabled().contains(*class));
                count += 1;
            }
            render_selector_list(frame, b"TX CLASSES", &rows[..count], cursor - first);
        }
        Screen::SquelchList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let cursor = usize::from(shell.squelch_cursor());
            let first = cursor / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let Ok(level) = u8::try_from(first + offset) else {
                    break;
                };
                if level >= SQUELCH_LEVELS {
                    break;
                }
                rows[offset] = SelectorRow::squelch_level(level, level == shell.squelch().get());
                count += 1;
            }
            render_selector_list(frame, b"SQUELCH", &rows[..count], cursor - first);
        }
        Screen::StepList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let first = shell.step_cursor() / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for offset in 0..LIST_ROWS {
                let row = first + offset;
                let Some(step) = VFO_STEPS_HZ.get(row) else {
                    break;
                };
                rows[offset] = SelectorRow::step(*step, row == shell.step_index());
                count += 1;
            }
            render_selector_list(frame, b"STEP", &rows[..count], shell.step_cursor() - first);
        }
        Screen::ChannelList => {
            let mut rows = [ListRow::default(); LIST_ROWS];
            let visible_rows = u16::try_from(LIST_ROWS).unwrap_or(u16::MAX);
            // Scroll by whole pages so the cursor is always on screen without
            // the list jittering under a single key press.
            let first = shell.cursor() / visible_rows * visible_rows;
            let mut count = 0;
            for offset in 0..visible_rows {
                let position = first + offset;
                let Some((channel, active)) = controller.and_then(|controller| {
                    controller
                        .visible_channel(position)
                        .map(|channel| (channel, position == controller.visible_position()))
                }) else {
                    break;
                };
                rows[usize::from(offset)] = ListRow::new(
                    position.saturating_add(1),
                    channel.name().as_str().as_bytes(),
                    active,
                );
                count += 1;
            }
            render_channel_list(
                frame,
                &rows[..count],
                usize::from(shell.cursor() - first),
                visible,
            );
        }
        Screen::Info => render_info_screen(
            frame,
            identity,
            generation,
            visible,
            retained,
            memory_state,
            serial_counters(),
            peripheral_clock_khz(),
            reset_cause(),
        ),
    }
}

/// Returns the host-programmed name of one bank, if the host named it.
/// Builds what one slot shows from its controller. `receiver` is given only
/// while the receiver is on this slot, and its meter only while the squelch is
/// open.
fn slot_channel<'a, B: RegisterBus, S: Speaker, M: Memory>(
    shell: &Shell,
    slot: Slot,
    controller: Option<&BankedReceiveController<Listening<M>>>,
    name: Option<&'a ChannelName>,
    receiver: Option<&Receiver<B, S>>,
) -> Option<SlotChannel<'a>> {
    let controller = controller?;
    let channel = controller.channel();
    Some(SlotChannel {
        name: name.map_or(&[][..], |name| name.as_str().as_bytes()),
        frequency_hz: controller.setup().frequency.as_hz(),
        place: match slot.mode {
            Mode::Vfo => SlotPlace::Vfo {
                step_hz: slot.vfo_step_hz(),
            },
            Mode::Memory => SlotPlace::Memory {
                bank: channel.bank().map(BankId::get),
                number: controller.visible_position().saturating_add(1),
            },
        },
        modulation: channel.modulation(),
        bandwidth: channel.bandwidth(),
        power: shell.tx_level(channel.power() as u8),
        meter: receiver
            .filter(|receiver| receiver.squelch_open)
            .map(|receiver| receiver.rssi_raw),
    })
}

fn bank_name<M: Memory>(bank: BankId, memory: &'static TaskCell<Option<M>>) -> Option<BankName> {
    memory.with_mut(|cell| {
        let memory = cell.as_mut()?;
        INDEX.with(|image| {
            image.bank_name(|offset, buffer| read_image(memory, offset, buffer), bank)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{adopt_publication, chosen_power, vfo_source, Listening, Selected};
    use crate::configuration::Programmed;
    use crate::operator_state::OperatorState;
    use radio_domain::BankId;
    use radio_domain::PowerLevel;
    use radio_platform::operator_shell::{Mode, Shell};
    use radio_platform::receive_app::Key;

    const VFO_HZ: u32 = 145_500_000;

    fn place(memory_mode: bool) -> OperatorState {
        OperatorState {
            memory_mode,
            bank: Some(BankId::new(1)),
            index: 3,
            channel_id: 9,
            vfo_hz: VFO_HZ,
            step_index: 2,
            squelch: None,
            tx_disabled: radio_tx_policy::ClassSet::NONE,
            other: None,
            b_selected: false,
            dual_watch: false,
        }
    }

    /// Boots the way the radio does: the serial task's empty publication
    /// first, then the stored configuration with the place beside it.
    fn boot(place: Option<OperatorState>) -> (Shell, Option<Selected>) {
        let mut shell = Shell::new();
        let mut selected = None;
        adopt_publication(
            &mut shell,
            &Programmed::empty(),
            true,
            None,
            &mut selected,
            &mut None,
        );
        adopt_publication(
            &mut shell,
            &Programmed::with_channels(4, 0b11),
            true,
            place,
            &mut selected,
            &mut None,
        );
        (shell, selected)
    }

    #[test]
    fn a_place_published_after_the_empty_configuration_is_restored() {
        let (shell, selected) = boot(Some(place(true)));
        assert!(matches!(shell.mode(), Mode::Memory));
        assert_eq!(shell.bank_filter(), Some(BankId::new(1)));
        assert_eq!(shell.vfo_hz(), VFO_HZ);
        assert_eq!(shell.step_index(), 2);
        assert_eq!(
            selected,
            Some(Selected {
                index: 3,
                channel_id: 9
            })
        );
    }

    #[test]
    fn both_slots_and_dual_watch_come_back_after_a_power_cycle() {
        use crate::operator_state::OtherSlot;
        use radio_platform::operator_shell::SlotId;

        let two_slots = OperatorState {
            other: Some(OtherSlot {
                memory_mode: true,
                bank: Some(BankId::new(1)),
                index: 2,
                channel_id: 7,
                vfo_hz: 433_500_000,
                step_index: 0,
            }),
            b_selected: true,
            dual_watch: true,
            ..place(false)
        };
        let mut shell = Shell::new();
        let mut selected = None;
        let mut other_selected = None;
        adopt_publication(
            &mut shell,
            &Programmed::with_channels(4, 0b11),
            true,
            Some(two_slots),
            &mut selected,
            &mut other_selected,
        );
        assert_eq!(shell.active_slot(), SlotId::B);
        assert!(shell.dual_watch());
        assert!(matches!(shell.mode(), Mode::Vfo), "B was on its VFO");
        let a = shell.other_slot();
        assert!(matches!(a.mode, Mode::Memory));
        assert_eq!(a.bank_filter, Some(BankId::new(1)));
        assert_eq!(a.vfo_hz, 433_500_000);
        assert_eq!(
            other_selected,
            Some(Selected {
                index: 2,
                channel_id: 7
            })
        );
    }

    #[test]
    fn a_radio_left_on_the_vfo_comes_back_to_it() {
        let (shell, selected) = boot(Some(place(false)));
        assert!(matches!(shell.mode(), Mode::Vfo));
        assert_eq!(shell.vfo_hz(), VFO_HZ);
        assert_eq!(selected, None);
    }

    #[test]
    fn classes_switched_off_stay_off_after_a_power_cycle() {
        use radio_domain::TxClass;
        use radio_tx_policy::ClassSet;

        let lent = OperatorState {
            tx_disabled: ClassSet::NONE.with(TxClass::Amateur, true),
            ..place(false)
        };
        let (shell, _) = boot(Some(lent));
        assert_eq!(shell.tx_disabled(), lent.tx_disabled);
        assert!(!shell.tx_enabled().contains(TxClass::Amateur));

        // A VFO-only radio, with no channels, keeps them too.
        let mut shell = Shell::new();
        let mut selected = None;
        adopt_publication(
            &mut shell,
            &Programmed::empty(),
            true,
            Some(lent),
            &mut selected,
            &mut None,
        );
        assert_eq!(shell.tx_disabled(), lent.tx_disabled);
    }

    #[test]
    fn a_radio_with_no_place_starts_on_its_channels() {
        let (shell, selected) = boot(None);
        assert!(matches!(shell.mode(), Mode::Memory));
        assert_eq!(shell.bank_filter(), None);
        assert_eq!(selected, None);
    }

    #[test]
    fn a_later_configuration_without_a_place_keeps_the_operator_where_they_are() {
        let (mut shell, mut selected) = boot(Some(place(true)));
        adopt_publication(
            &mut shell,
            &Programmed::with_channels(4, 0b11),
            false,
            None,
            &mut selected,
            &mut None,
        );
        assert_eq!(shell.bank_filter(), Some(BankId::new(1)));
        assert_eq!(selected.map(|kept| kept.index), Some(3));
    }
    #[test]
    fn a_programmed_tx_power_and_timeout_reach_the_handset_menu() {
        let mut shell = Shell::new();
        let mut selected = None;
        adopt_publication(
            &mut shell,
            &Programmed::with_channels(4, 0b11).with_tx(3, 4),
            true,
            None,
            &mut selected,
            &mut None,
        );
        assert_eq!(shell.tx_power(), 3);
        assert_eq!(shell.tx_timeout_ms(), 300_000);
    }

    #[test]
    fn a_channel_transmits_at_its_own_power_unless_the_handset_overrides_it() {
        let mut shell = Shell::new();
        shell.set_capabilities(radio_platform::operator_shell::Capabilities::K5);
        shell.set_tx(2, 0);
        // The VFO's record carries the programmed default as its own power.
        let Some(Listening::Vfo(vfo)) = vfo_source::<()>(shell.slot(), shell.tx_power()) else {
            panic!("a VFO source");
        };
        assert_eq!(vfo.power(), PowerLevel::Low3);
        // A channel's own power wins over the default.
        shell.set_tx(4, 0);
        assert_eq!(chosen_power(&shell, Some(vfo)), 2);
        assert_eq!(chosen_power(&shell, None), 4);

        // Choose 5 W on the handset: every source transmits at it.
        shell.press(Key::Menu, 0, context());
        shell.press(Key::Down, 1, context());
        shell.press(Key::Menu, 2, context());
        shell.press(Key::Digit(6), 3, context());
        assert_eq!(chosen_power(&shell, Some(vfo)), 5);
        assert_eq!(chosen_power(&shell, None), 5);
    }

    fn context() -> radio_platform::operator_shell::Context {
        radio_platform::operator_shell::Context {
            visible_channels: 0,
            active_index: 0,
            scanning: false,
        }
    }
}
