//! Bounded banked-memory and VFO receive control.
//!
//! This module owns receive-side channel selection, bank filtering, manual
//! tuning, monitoring, dual watch, and scanning over explicit channel records.
//! It performs no input or output, allocates nothing, and never mints transmit
//! authority.

use core::fmt;

use radio_channel_plan::{generated_channel_parts, ChannelRecord, GeneratedBank};
use radio_domain::{
    Bandwidth, BankId, ChannelId, DomainError, Frequency, FrequencyStep, Modulation, RadioConfig,
    ScanResume, SquelchLevel, Tone,
};

use crate::{TimerDirective, TimerToken};

/// A bounded, ordered source of explicit channel records.
pub trait ChannelSource {
    /// Returns the number of stored channels.
    fn len(&self) -> u16;

    /// Returns the channel at one zero-based storage index.
    fn get(&self, index: u16) -> Option<ChannelRecord>;

    /// Reports whether the source stores no channels.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reports whether the channel at one index belongs to a bank.
    ///
    /// Bank filtering asks this of every index and the answer decides only
    /// whether a channel is counted, so a source which can answer without
    /// building the record should. The default builds it.
    fn member_at(&self, index: u16, bank: BankId) -> bool {
        self.get(index)
            .is_some_and(|channel| channel.is_member_of(bank))
    }
}

/// A fixed-capacity channel store ordered by stable channel identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelMemory<const CHANNELS: usize> {
    channels: [Option<ChannelRecord>; CHANNELS],
    count: u16,
}

impl<const CHANNELS: usize> Default for ChannelMemory<CHANNELS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CHANNELS: usize> ChannelMemory<CHANNELS> {
    /// Constructs an empty store.
    pub const fn new() -> Self {
        Self {
            channels: [None; CHANNELS],
            count: 0,
        }
    }

    /// Inserts or replaces one channel, keeping identifier order.
    pub fn insert(&mut self, channel: ChannelRecord) -> Result<(), MemoryFull> {
        let mut position = usize::from(self.count);
        for index in 0..usize::from(self.count) {
            let stored = self.channels[index].ok_or(MemoryFull)?;
            if stored.id() == channel.id() {
                self.channels[index] = Some(channel);
                return Ok(());
            }
            if stored.id() > channel.id() {
                position = index;
                break;
            }
        }
        if usize::from(self.count) >= CHANNELS {
            return Err(MemoryFull);
        }
        let mut index = usize::from(self.count);
        while index > position {
            self.channels[index] = self.channels[index - 1];
            index -= 1;
        }
        self.channels[position] = Some(channel);
        self.count += 1;
        Ok(())
    }

    /// Returns the stored channel with one identifier.
    pub fn find(&self, id: ChannelId) -> Option<ChannelRecord> {
        self.channels
            .iter()
            .flatten()
            .copied()
            .find(|channel| channel.id() == id)
    }
}

/// The fixed channel store has no free slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryFull;

impl fmt::Display for MemoryFull {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("channel memory is full")
    }
}

impl<const CHANNELS: usize> ChannelSource for ChannelMemory<CHANNELS> {
    fn len(&self) -> u16 {
        self.count
    }

    fn get(&self, index: u16) -> Option<ChannelRecord> {
        if index >= self.count {
            return None;
        }
        self.channels[usize::from(index)]
    }
}

/// A channel source holding stored channels and the plans a radio expands.
///
/// This is the channelised space-saving model as the radio sees it. An explicit
/// channel costs one stored object; a generated plan costs one stored object
/// however many channels it contains, and its channels are expanded here, on
/// demand, into ordinary records. Selection, bank filtering, dual watch, and
/// scanning cannot tell the two apart.
///
/// Stored channels come first in identifier order, then each plan's channels in
/// bank order. Nothing is materialised: expansion happens per lookup, so a plan
/// of a thousand channels occupies no more memory than a plan of ten.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgrammedMemory<const CHANNELS: usize, const PLANS: usize> {
    stored: ChannelMemory<CHANNELS>,
    plans: [Option<GeneratedBank>; PLANS],
    installed: u16,
    expanded: u16,
}

impl<const CHANNELS: usize, const PLANS: usize> Default for ProgrammedMemory<CHANNELS, PLANS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CHANNELS: usize, const PLANS: usize> ProgrammedMemory<CHANNELS, PLANS> {
    /// Constructs a store holding neither channels nor plans.
    pub const fn new() -> Self {
        Self {
            stored: ChannelMemory::new(),
            plans: [None; PLANS],
            installed: 0,
            expanded: 0,
        }
    }

    /// Constructs a store holding stored channels only.
    pub const fn from_stored(stored: ChannelMemory<CHANNELS>) -> Self {
        Self {
            stored,
            plans: [None; PLANS],
            installed: 0,
            expanded: 0,
        }
    }

    /// Inserts or replaces one stored channel, keeping identifier order.
    pub fn insert(&mut self, channel: ChannelRecord) -> Result<(), MemoryFull> {
        self.stored.insert(channel)
    }

    /// Installs one generated plan, keeping bank-identifier order.
    ///
    /// Plans are held packed rather than one slot per addressable bank, because
    /// a radio sizes this by the plans it will accept and every unused slot
    /// costs it RAM in every copy of the configuration. A plan replaces one
    /// already held for the same bank. The whole selection space is addressed by
    /// a `u16` index, so a plan which would push the total past that bound is
    /// refused rather than silently truncated.
    pub fn install(&mut self, plan: GeneratedBank) -> Result<(), MemoryFull> {
        let mut position = usize::from(self.installed);
        for index in 0..usize::from(self.installed) {
            let held = self.plans[index].ok_or(MemoryFull)?;
            if held.id().get() == plan.id().get() {
                self.expanded =
                    self.checked_total(self.expanded - held.channel_count(), plan.channel_count())?;
                self.plans[index] = Some(plan);
                return Ok(());
            }
            if held.id().get() > plan.id().get() {
                position = index;
                break;
            }
        }
        if usize::from(self.installed) >= PLANS {
            return Err(MemoryFull);
        }
        let expanded = self.checked_total(self.expanded, plan.channel_count())?;
        let mut index = usize::from(self.installed);
        while index > position {
            self.plans[index] = self.plans[index - 1];
            index -= 1;
        }
        self.plans[position] = Some(plan);
        self.installed += 1;
        self.expanded = expanded;
        Ok(())
    }

    /// Returns the expanded total, refusing one the index space cannot address.
    fn checked_total(&self, expanded: u16, added: u16) -> Result<u16, MemoryFull> {
        let total = expanded.checked_add(added).ok_or(MemoryFull)?;
        if u32::from(self.stored.len()) + u32::from(total) > u32::from(u16::MAX) {
            return Err(MemoryFull);
        }
        Ok(total)
    }

    /// Returns the plan installed for one bank.
    pub fn plan(&self, bank: BankId) -> Option<GeneratedBank> {
        self.plans
            .iter()
            .flatten()
            .copied()
            .find(|plan| plan.id().get() == bank.get())
    }

    /// Returns the number of channels which cost one stored object each.
    pub fn stored_len(&self) -> u16 {
        self.stored.len()
    }

    /// Returns the number of channels expanded from stored plans.
    pub const fn expanded_len(&self) -> u16 {
        self.expanded
    }

    /// Returns the channel with one identifier, stored or expanded.
    ///
    /// An expanded identifier packs the bank and index that minted it, so the
    /// channel is reconstructed directly from the owning plan rather than found
    /// by expanding the space until one matches.
    pub fn find(&self, id: ChannelId) -> Option<ChannelRecord> {
        let Some((bank, index)) = generated_channel_parts(id) else {
            return self.stored.find(id);
        };
        self.plan(bank)?.channel_record(index).ok()
    }

    /// Returns the bank owning the expanded channel at one plan-space offset.
    ///
    /// Every channel a plan expands belongs to that plan's bank and no other,
    /// so membership is decided by locating the plan alone.
    fn expanded_bank(&self, offset: u16) -> Option<BankId> {
        let mut remaining = offset;
        for plan in self.plans.iter().flatten() {
            if remaining < plan.channel_count() {
                return Some(plan.id());
            }
            remaining -= plan.channel_count();
        }
        None
    }

    /// Expands the channel at one zero-based offset into the plan space.
    fn expanded_channel(&self, offset: u16) -> Option<ChannelRecord> {
        let mut remaining = offset;
        for plan in self.plans.iter().flatten() {
            if remaining < plan.channel_count() {
                return plan.channel_record(remaining).ok();
            }
            remaining -= plan.channel_count();
        }
        None
    }
}

impl<const CHANNELS: usize, const PLANS: usize> ChannelSource
    for ProgrammedMemory<CHANNELS, PLANS>
{
    fn len(&self) -> u16 {
        self.stored.len().saturating_add(self.expanded)
    }

    fn get(&self, index: u16) -> Option<ChannelRecord> {
        match index.checked_sub(self.stored.len()) {
            None => self.stored.get(index),
            Some(offset) => self.expanded_channel(offset),
        }
    }

    fn member_at(&self, index: u16, bank: BankId) -> bool {
        match index.checked_sub(self.stored.len()) {
            None => self
                .stored
                .get(index)
                .is_some_and(|channel| channel.is_member_of(bank)),
            // An expanded channel needs no record built to answer this, which
            // is what keeps a filtered view over a band-sized plan cheap.
            Some(offset) => self.expanded_bank(offset) == Some(bank),
        }
    }
}

/// Current receive tuning source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveMode {
    /// The active channel comes from banked memory.
    Memory,
    /// The active frequency is tuned manually from the last active channel.
    Vfo,
}

/// Current receive-control activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveState {
    /// One channel or frequency is selected.
    Idle,
    /// A bank scan is running.
    Scanning(ScanPhase),
}

/// Current banked-scan phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    /// Waiting for the no-signal dwell deadline.
    Dwell,
    /// Holding on a channel which was found busy.
    Hold,
}

/// One receive-side observation supplied by an RF adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveObservation {
    /// Whether the carrier squelch criterion is open.
    pub squelch_open: bool,
    /// Tone-squelch result, absent when the channel requires no tone.
    pub tone_matched: Option<bool>,
}

/// Complete hardware-independent receive settings for the active selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelReceiveSetup {
    /// Exact receive frequency.
    pub frequency: Frequency,
    /// Requested demodulator family.
    pub modulation: Modulation,
    /// Requested channel bandwidth.
    pub bandwidth: Bandwidth,
    /// Receive-side tone squelch requirement.
    pub tone: Tone,
    /// Effective squelch level; monitoring forces the open level.
    pub squelch: SquelchLevel,
    /// Manual tuning step.
    pub step: FrequencyStep,
}

/// Identity of the current selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelSelection {
    /// One banked memory channel at a storage index.
    Memory {
        /// Zero-based storage index.
        index: u16,
        /// Stable channel identifier.
        id: ChannelId,
    },
    /// A manually tuned frequency derived from the last active channel.
    Vfo,
}

/// Required adapter change after one control input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveUpdate {
    /// New tuning request, when the selection or frequency changed.
    pub activation: Option<ChannelActivation>,
    /// Whether demodulated audio should currently reach the output.
    pub audio_open: bool,
    /// Logical timer operation required by the new state.
    pub timer: TimerDirective,
}

/// One resolved receive activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelActivation {
    /// The selection which produced this activation.
    pub selection: ChannelSelection,
    /// Complete receive settings to apply.
    pub setup: ChannelReceiveSetup,
}

/// Banked receive-control failure with no partial state change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveError {
    /// The current bank filter selects no channel.
    NoEligibleChannel,
    /// The requested storage index does not exist or is filtered out.
    IndexOutOfRange,
    /// The operation is invalid in the current state.
    InvalidState(ReceiveState),
    /// The operation requires the other tuning mode.
    InvalidMode(ReceiveMode),
    /// Manual tuning left the representable frequency range.
    TuningLimit,
    /// No fresh bounded timer token remained.
    TimerTokenExhausted,
    /// The stored radio configuration is invalid.
    InvalidConfig(DomainError),
}

impl fmt::Display for ReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEligibleChannel => formatter.write_str("no channel matches the active bank"),
            Self::IndexOutOfRange => formatter.write_str("channel index is outside the selection"),
            Self::InvalidState(state) => write!(formatter, "invalid receive state: {state:?}"),
            Self::InvalidMode(mode) => write!(formatter, "invalid receive mode: {mode:?}"),
            Self::TuningLimit => formatter.write_str("manual tuning left the frequency range"),
            Self::TimerTokenExhausted => formatter.write_str("receive timer token exhausted"),
            Self::InvalidConfig(error) => write!(formatter, "invalid radio configuration: {error}"),
        }
    }
}

/// Banked memory, VFO, monitor, dual-watch, and scanning receive controller.
pub struct BankedReceiveController<C: ChannelSource> {
    source: C,
    config: RadioConfig,
    bank: Option<BankId>,
    mode: ReceiveMode,
    index: u16,
    channel: ChannelRecord,
    vfo: ChannelReceiveSetup,
    monitor: bool,
    audio_open: bool,
    state: ReceiveState,
    dual_watch_partner: Option<u16>,
    active_timer: Option<TimerToken>,
    next_timer_token: u32,
}

impl<C: ChannelSource> BankedReceiveController<C> {
    /// Activates the first channel matching the requested bank filter.
    pub fn activate(
        source: C,
        config: RadioConfig,
        bank: Option<BankId>,
    ) -> Result<(Self, ReceiveUpdate), ReceiveError> {
        let config = config.validate().map_err(ReceiveError::InvalidConfig)?;
        let index = (0..source.len())
            .find(|index| {
                source
                    .get(*index)
                    .is_some_and(|channel| is_member(&channel, bank))
            })
            .ok_or(ReceiveError::NoEligibleChannel)?;
        let channel = source.get(index).ok_or(ReceiveError::NoEligibleChannel)?;
        let vfo = channel_setup(&channel, false);
        let mut controller = Self {
            source,
            config,
            bank,
            mode: ReceiveMode::Memory,
            index,
            channel,
            vfo,
            monitor: false,
            audio_open: false,
            state: ReceiveState::Idle,
            dual_watch_partner: None,
            active_timer: None,
            next_timer_token: 1,
        };
        let update = ReceiveUpdate {
            activation: Some(controller.activation()),
            audio_open: false,
            timer: TimerDirective::Cancel,
        };
        controller.audio_open = false;
        Ok((controller, update))
    }

    /// Returns the current tuning mode.
    pub const fn mode(&self) -> ReceiveMode {
        self.mode
    }

    /// Returns the current control state.
    pub const fn state(&self) -> ReceiveState {
        self.state
    }

    /// Returns the active bank filter.
    pub const fn bank(&self) -> Option<BankId> {
        self.bank
    }

    /// Returns the current storage index of the selected memory channel.
    pub const fn index(&self) -> u16 {
        self.index
    }

    /// Returns the currently selected memory channel.
    pub const fn channel(&self) -> ChannelRecord {
        self.channel
    }

    /// Returns the global radio configuration in use.
    pub const fn config(&self) -> RadioConfig {
        self.config
    }

    /// Reports whether monitoring forces the squelch open.
    pub const fn is_monitoring(&self) -> bool {
        self.monitor
    }

    /// Reports whether audio should currently reach the output.
    pub const fn is_audio_open(&self) -> bool {
        self.audio_open
    }

    /// Returns the current selection identity.
    pub const fn selection(&self) -> ChannelSelection {
        match self.mode {
            ReceiveMode::Memory => ChannelSelection::Memory {
                index: self.index,
                id: self.channel.id(),
            },
            ReceiveMode::Vfo => ChannelSelection::Vfo,
        }
    }

    /// Returns the complete receive settings for the current selection.
    pub fn setup(&self) -> ChannelReceiveSetup {
        match self.mode {
            ReceiveMode::Memory => channel_setup(&self.channel, self.monitor),
            ReceiveMode::Vfo => ChannelReceiveSetup {
                squelch: effective_squelch(self.vfo.squelch, self.monitor),
                ..self.vfo
            },
        }
    }

    /// Replaces the bank filter and selects its first eligible channel.
    pub fn set_bank(&mut self, bank: Option<BankId>) -> Result<ReceiveUpdate, ReceiveError> {
        let index = (0..self.source.len())
            .find(|index| {
                self.source
                    .get(*index)
                    .is_some_and(|channel| is_member(&channel, bank))
            })
            .ok_or(ReceiveError::NoEligibleChannel)?;
        self.bank = bank;
        self.dual_watch_partner = None;
        self.select(index)
    }

    /// Selects one eligible memory channel and leaves any scan.
    pub fn select(&mut self, index: u16) -> Result<ReceiveUpdate, ReceiveError> {
        let channel = self
            .source
            .get(index)
            .filter(|channel| is_member(channel, self.bank))
            .ok_or(ReceiveError::IndexOutOfRange)?;
        self.index = index;
        self.channel = channel;
        self.mode = ReceiveMode::Memory;
        self.state = ReceiveState::Idle;
        self.active_timer = None;
        self.audio_open = self.monitor;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer: TimerDirective::Cancel,
        })
    }

    /// Reports whether the channel at one index passes the active filter.
    ///
    /// This asks the source rather than building the record, so a filtered view
    /// over a band-sized plan costs arithmetic per channel instead of a full
    /// expansion per channel.
    fn is_member_at(&self, index: u16) -> bool {
        self.bank
            .is_none_or(|bank| self.source.member_at(index, bank))
    }

    /// Returns the number of channels eligible under the active bank filter.
    ///
    /// A user interface numbers channels as the operator sees them, which is
    /// the filtered view rather than the storage table, so these positions are
    /// defined here beside the filter itself.
    pub fn visible_channels(&self) -> u16 {
        (0..self.source.len())
            .filter(|index| self.is_member_at(*index))
            .count()
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// Returns the zero-based position of the selection in the filtered view.
    pub fn visible_position(&self) -> u16 {
        (0..self.index)
            .filter(|index| self.is_member_at(*index))
            .count()
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// Returns the channel at one zero-based position in the filtered view.
    pub fn visible_channel(&self, position: u16) -> Option<ChannelRecord> {
        self.visible_index(position)
            .and_then(|index| self.source.get(index))
    }

    /// Returns the storage index of one zero-based position in the view.
    pub fn visible_index(&self, position: u16) -> Option<u16> {
        (0..self.source.len())
            .filter(|index| self.is_member_at(*index))
            .nth(usize::from(position))
    }

    /// Selects one zero-based position within the filtered view.
    pub fn select_visible(&mut self, position: u16) -> Result<ReceiveUpdate, ReceiveError> {
        let index = self
            .visible_index(position)
            .ok_or(ReceiveError::IndexOutOfRange)?;
        self.select(index)
    }

    /// Selects the next eligible channel, wrapping around the bank.
    pub fn select_next(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        let index = self.neighbour(self.index, true, false)?;
        self.select(index)
    }

    /// Selects the previous eligible channel, wrapping around the bank.
    pub fn select_previous(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        let index = self.neighbour(self.index, false, false)?;
        self.select(index)
    }

    /// Switches to manual tuning, seeded from the selected channel.
    pub fn enter_vfo(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        if matches!(self.state, ReceiveState::Scanning(_)) {
            return Err(ReceiveError::InvalidState(self.state));
        }
        self.vfo = channel_setup(&self.channel, false);
        self.mode = ReceiveMode::Vfo;
        self.audio_open = self.monitor;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer: TimerDirective::Cancel,
        })
    }

    /// Returns to the selected banked memory channel.
    pub fn enter_memory(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        if matches!(self.state, ReceiveState::Scanning(_)) {
            return Err(ReceiveError::InvalidState(self.state));
        }
        self.mode = ReceiveMode::Memory;
        self.audio_open = self.monitor;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer: TimerDirective::Cancel,
        })
    }

    /// Tunes the manual frequency one step up.
    pub fn tune_up(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        self.tune(true)
    }

    /// Tunes the manual frequency one step down.
    pub fn tune_down(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        self.tune(false)
    }

    /// Tunes the manual frequency to one exact value.
    pub fn tune_to(&mut self, frequency: Frequency) -> Result<ReceiveUpdate, ReceiveError> {
        if !matches!(self.mode, ReceiveMode::Vfo) {
            return Err(ReceiveError::InvalidMode(self.mode));
        }
        self.vfo.frequency = frequency;
        self.audio_open = self.monitor;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer: TimerDirective::Cancel,
        })
    }

    /// Forces the squelch open, or restores squelch control.
    pub fn set_monitor(&mut self, monitor: bool) -> ReceiveUpdate {
        self.monitor = monitor;
        if monitor {
            self.audio_open = true;
        }
        ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer: TimerDirective::Unchanged,
        }
    }

    /// Arms dual watch against one other eligible memory channel.
    pub fn set_dual_watch(&mut self, partner: Option<u16>) -> Result<ReceiveUpdate, ReceiveError> {
        if !self.config.dual_watch {
            return Err(ReceiveError::InvalidMode(self.mode));
        }
        if let Some(index) = partner {
            let eligible = self
                .source
                .get(index)
                .is_some_and(|channel| is_member(&channel, self.bank));
            if !eligible || index == self.index {
                return Err(ReceiveError::IndexOutOfRange);
            }
        }
        self.dual_watch_partner = partner;
        let timer = if partner.is_some() {
            self.fresh_timer(self.config.scan_dwell_ms)?
        } else {
            self.active_timer = None;
            TimerDirective::Cancel
        };
        Ok(ReceiveUpdate {
            activation: None,
            audio_open: self.audio_open,
            timer,
        })
    }

    /// Starts scanning the eligible, non-skipped channels of the active bank.
    pub fn start_scanning(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        if !matches!(self.state, ReceiveState::Idle) {
            return Err(ReceiveError::InvalidState(self.state));
        }
        if !matches!(self.mode, ReceiveMode::Memory) {
            return Err(ReceiveError::InvalidMode(self.mode));
        }
        // A scan must have at least one non-skipped eligible channel.
        self.neighbour(self.index, true, true)?;
        let timer = self.fresh_timer(self.config.scan_dwell_ms)?;
        self.state = ReceiveState::Scanning(ScanPhase::Dwell);
        self.dual_watch_partner = None;
        Ok(ReceiveUpdate {
            activation: None,
            audio_open: self.audio_open,
            timer,
        })
    }

    /// Stops scanning and stays on the current channel.
    pub fn stop_scanning(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        if !matches!(self.state, ReceiveState::Scanning(_)) {
            return Err(ReceiveError::InvalidState(self.state));
        }
        self.state = ReceiveState::Idle;
        self.active_timer = None;
        Ok(ReceiveUpdate {
            activation: None,
            audio_open: self.audio_open,
            timer: TimerDirective::Cancel,
        })
    }

    /// Applies one adapter observation, updating audio gating and scanning.
    pub fn observe(
        &mut self,
        observation: ReceiveObservation,
    ) -> Result<ReceiveUpdate, ReceiveError> {
        let busy = observation.squelch_open && observation.tone_matched.unwrap_or(true);
        self.audio_open = self.monitor || busy;

        if !matches!(self.state, ReceiveState::Scanning(_)) {
            return Ok(ReceiveUpdate {
                activation: None,
                audio_open: self.audio_open,
                timer: TimerDirective::Unchanged,
            });
        }

        if busy {
            if matches!(self.config.scan_resume, ScanResume::Stop) {
                self.state = ReceiveState::Idle;
                self.active_timer = None;
                return Ok(ReceiveUpdate {
                    activation: None,
                    audio_open: self.audio_open,
                    timer: TimerDirective::Cancel,
                });
            }
            let timer = self.fresh_timer(self.config.scan_hold_ms)?;
            self.state = ReceiveState::Scanning(ScanPhase::Hold);
            return Ok(ReceiveUpdate {
                activation: None,
                audio_open: self.audio_open,
                timer,
            });
        }

        if matches!(self.state, ReceiveState::Scanning(ScanPhase::Hold))
            && matches!(self.config.scan_resume, ScanResume::Carrier)
        {
            return self.advance_scan();
        }
        Ok(ReceiveUpdate {
            activation: None,
            audio_open: self.audio_open,
            timer: TimerDirective::Unchanged,
        })
    }

    /// Applies one logical timer expiry; stale tokens change nothing.
    pub fn timer_elapsed(&mut self, token: TimerToken) -> Result<ReceiveUpdate, ReceiveError> {
        if self.active_timer != Some(token) {
            return Ok(ReceiveUpdate {
                activation: None,
                audio_open: self.audio_open,
                timer: TimerDirective::Unchanged,
            });
        }
        match self.state {
            ReceiveState::Idle => self.dual_watch_elapsed(),
            ReceiveState::Scanning(ScanPhase::Dwell) => self.advance_scan(),
            ReceiveState::Scanning(ScanPhase::Hold) => match self.config.scan_resume {
                ScanResume::TimeOut => self.advance_scan(),
                ScanResume::Carrier | ScanResume::Stop => {
                    let timer = self.fresh_timer(self.config.scan_hold_ms)?;
                    Ok(ReceiveUpdate {
                        activation: None,
                        audio_open: self.audio_open,
                        timer,
                    })
                }
            },
        }
    }

    fn dual_watch_elapsed(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        let Some(partner) = self.dual_watch_partner else {
            self.active_timer = None;
            return Ok(ReceiveUpdate {
                activation: None,
                audio_open: self.audio_open,
                timer: TimerDirective::Unchanged,
            });
        };
        if self.audio_open {
            let timer = self.fresh_timer(self.config.scan_dwell_ms)?;
            return Ok(ReceiveUpdate {
                activation: None,
                audio_open: self.audio_open,
                timer,
            });
        }
        let previous = self.index;
        let channel = self
            .source
            .get(partner)
            .filter(|channel| is_member(channel, self.bank))
            .ok_or(ReceiveError::IndexOutOfRange)?;
        self.index = partner;
        self.channel = channel;
        self.dual_watch_partner = Some(previous);
        self.mode = ReceiveMode::Memory;
        let timer = self.fresh_timer(self.config.scan_dwell_ms)?;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer,
        })
    }

    fn advance_scan(&mut self) -> Result<ReceiveUpdate, ReceiveError> {
        let index = self.neighbour(self.index, true, true)?;
        let channel = self
            .source
            .get(index)
            .ok_or(ReceiveError::IndexOutOfRange)?;
        self.index = index;
        self.channel = channel;
        self.state = ReceiveState::Scanning(ScanPhase::Dwell);
        self.audio_open = self.monitor;
        let timer = self.fresh_timer(self.config.scan_dwell_ms)?;
        Ok(ReceiveUpdate {
            activation: Some(self.activation()),
            audio_open: self.audio_open,
            timer,
        })
    }

    fn activation(&self) -> ChannelActivation {
        ChannelActivation {
            selection: self.selection(),
            setup: self.setup(),
        }
    }

    fn tune(&mut self, up: bool) -> Result<ReceiveUpdate, ReceiveError> {
        if !matches!(self.mode, ReceiveMode::Vfo) {
            return Err(ReceiveError::InvalidMode(self.mode));
        }
        let step = self.vfo.step.as_hz();
        let hertz = if up {
            self.vfo
                .frequency
                .as_hz()
                .checked_add(step)
                .ok_or(ReceiveError::TuningLimit)?
        } else {
            self.vfo
                .frequency
                .as_hz()
                .checked_sub(step)
                .ok_or(ReceiveError::TuningLimit)?
        };
        let frequency = Frequency::from_hz(hertz).map_err(|_| ReceiveError::TuningLimit)?;
        self.tune_to(frequency)
    }

    fn neighbour(&self, from: u16, forward: bool, scanning: bool) -> Result<u16, ReceiveError> {
        let count = self.source.len();
        if count == 0 {
            return Err(ReceiveError::NoEligibleChannel);
        }
        let mut index = from;
        for _ in 0..count {
            index = if forward {
                if index + 1 >= count {
                    0
                } else {
                    index + 1
                }
            } else if index == 0 {
                count - 1
            } else {
                index - 1
            };
            let Some(channel) = self.source.get(index) else {
                continue;
            };
            if is_member(&channel, self.bank) && !(scanning && channel.is_scan_skipped()) {
                return Ok(index);
            }
        }
        Err(ReceiveError::NoEligibleChannel)
    }

    fn fresh_timer(&mut self, after_ms: u32) -> Result<TimerDirective, ReceiveError> {
        let next = self
            .next_timer_token
            .checked_add(1)
            .ok_or(ReceiveError::TimerTokenExhausted)?;
        let token = TimerToken::new(self.next_timer_token);
        self.next_timer_token = next;
        self.active_timer = Some(token);
        Ok(TimerDirective::Arm { token, after_ms })
    }
}

fn is_member(channel: &ChannelRecord, bank: Option<BankId>) -> bool {
    bank.is_none_or(|bank| channel.is_member_of(bank))
}

fn effective_squelch(squelch: SquelchLevel, monitor: bool) -> SquelchLevel {
    if monitor {
        SquelchLevel::default()
    } else {
        squelch
    }
}

fn channel_setup(channel: &ChannelRecord, monitor: bool) -> ChannelReceiveSetup {
    ChannelReceiveSetup {
        frequency: channel.active().receive,
        modulation: channel.modulation(),
        bandwidth: channel.bandwidth(),
        tone: channel.rx_tone(),
        squelch: effective_squelch(channel.squelch(), monitor),
        step: channel.step(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BankedReceiveController, ChannelMemory, ChannelSelection, ChannelSource, MemoryFull,
        ProgrammedMemory, ReceiveError, ReceiveMode, ReceiveObservation, ReceiveState, ScanPhase,
    };
    use crate::{TimerDirective, TimerToken};
    use radio_channel_plan::{
        BankMask, BankName, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
        GeneratedBank,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel,
        RadioConfig, ScanResume, SquelchLevel, Tone, TxClass,
    };

    fn channel(id: u16, hertz: u32, banks: u16, flags: u8) -> ChannelRecord {
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("CH").unwrap(),
            receive: Frequency::from_hz(hertz).unwrap(),
            transmit: Frequency::from_hz(hertz).unwrap(),
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            squelch: SquelchLevel::new(4).unwrap(),
            flags: ChannelFlags::from_bits(flags).unwrap(),
            banks: BankMask::from_bits(banks),
            tx_class: TxClass::Amateur,
        })
        .unwrap()
    }

    fn plan(id: u16, name: &str, base_hz: u32, count: u16) -> GeneratedBank {
        GeneratedBank::linear_simplex(
            BankId::new(id),
            BankName::new(name).unwrap(),
            Frequency::from_hz(base_hz).unwrap(),
            FrequencyStep::from_hz(12_500).unwrap(),
            count,
            TxClass::LicenceFreePlan,
        )
        .unwrap()
    }

    #[test]
    fn stored_channels_and_expanded_plans_share_one_selection_space() {
        let mut memory = ProgrammedMemory::<8, 2>::new();
        memory.insert(channel(2, 145_200_000, 0b0010, 0)).unwrap();
        memory.insert(channel(1, 145_100_000, 0b0010, 0)).unwrap();
        memory.install(plan(3, "PMR446", 446_006_250, 16)).unwrap();
        memory.install(plan(1, "Marine", 156_050_000, 4)).unwrap();

        assert_eq!(memory.stored_len(), 2);
        assert_eq!(memory.expanded_len(), 20);
        assert_eq!(memory.len(), 22);

        // Stored channels first in identifier order, then each plan in bank
        // order, which is the order the operator steps through them.
        assert_eq!(memory.get(0).unwrap().id().get(), 1);
        assert_eq!(memory.get(1).unwrap().id().get(), 2);
        let first_plan = memory.get(2).unwrap();
        assert_eq!(first_plan.name().as_str(), "Mar 1");
        assert_eq!(first_plan.receive().as_hz(), 156_050_000);
        assert_eq!(memory.get(6).unwrap().name().as_str(), "PMR 1");
        assert_eq!(memory.get(21).unwrap().name().as_str(), "PMR 16");
        assert_eq!(memory.get(22), None);

        let expanded = memory.get(21).unwrap();
        assert_eq!(memory.find(expanded.id()), Some(expanded));
        assert_eq!(memory.find(ChannelId::new(2)), memory.get(1));

        // Replacing a plan resizes the space instead of appending to it.
        memory.install(plan(3, "PMR446", 446_006_250, 8)).unwrap();
        assert_eq!(memory.len(), 14);
        assert_eq!(memory.plan(BankId::new(3)).unwrap().channel_count(), 8);
        assert_eq!(memory.plan(BankId::new(0)), None);

        // The store is sized by the plans it accepts, not by the addressable
        // banks, so a further plan is refused rather than silently dropped.
        assert_eq!(
            memory.install(plan(5, "Extra", 433_000_000, 4)),
            Err(MemoryFull)
        );
    }

    #[test]
    fn a_bank_filter_selects_and_scans_expanded_channels() {
        let mut memory = ProgrammedMemory::<8, 4>::new();
        memory.insert(channel(1, 145_100_000, 0b0010, 0)).unwrap();
        memory.install(plan(3, "PMR446", 446_006_250, 4)).unwrap();

        let (mut controller, _) = BankedReceiveController::activate(
            memory,
            RadioConfig::conservative(),
            Some(BankId::new(3)),
        )
        .unwrap();
        assert_eq!(controller.visible_channels(), 4);
        let update = controller.select_visible(1).unwrap();
        let activation = update.activation.unwrap();
        assert_eq!(activation.setup.frequency.as_hz(), 446_018_750);
        assert_eq!(
            controller.visible_channel(1).unwrap().name().as_str(),
            "PMR 2",
            "an expanded channel names its designator and number"
        );

        // The stored channel is outside the filtered view, and scanning walks
        // the expanded channels exactly as it walks stored ones.
        assert!(controller.select_visible(4).is_err());
        controller.start_scanning().unwrap();
        assert_eq!(controller.state(), ReceiveState::Scanning(ScanPhase::Dwell));
    }

    fn memory() -> ChannelMemory<8> {
        let mut memory = ChannelMemory::new();
        memory.insert(channel(3, 145_300_000, 0b0010, 0)).unwrap();
        memory.insert(channel(1, 145_100_000, 0b0011, 0)).unwrap();
        memory
            .insert(channel(2, 145_200_000, 0b0010, ChannelFlags::SCAN_SKIP))
            .unwrap();
        memory
    }

    fn controller(
        config: RadioConfig,
        bank: Option<BankId>,
    ) -> BankedReceiveController<ChannelMemory<8>> {
        BankedReceiveController::activate(memory(), config, bank)
            .unwrap()
            .0
    }

    fn armed(timer: TimerDirective) -> TimerToken {
        match timer {
            TimerDirective::Arm { token, .. } => token,
            other => panic!("expected an armed timer, got {other:?}"),
        }
    }

    #[test]
    fn memory_keeps_identifier_order_and_replaces_in_place() {
        let memory = memory();
        assert_eq!(memory.len(), 3);
        assert_eq!(memory.get(0).unwrap().id(), ChannelId::new(1));
        assert_eq!(memory.get(1).unwrap().id(), ChannelId::new(2));
        assert_eq!(memory.get(2).unwrap().id(), ChannelId::new(3));
        assert_eq!(memory.get(3), None);

        let mut replaced = memory;
        replaced.insert(channel(2, 145_250_000, 0b0010, 0)).unwrap();
        assert_eq!(replaced.len(), 3);
        assert_eq!(
            replaced.find(ChannelId::new(2)).unwrap().receive().as_hz(),
            145_250_000
        );

        let mut full = ChannelMemory::<1>::new();
        full.insert(channel(1, 145_100_000, 0, 0)).unwrap();
        assert!(full.insert(channel(2, 145_200_000, 0, 0)).is_err());
    }

    #[test]
    fn bank_filters_selection_and_wraps_around() {
        let mut controller = controller(RadioConfig::conservative(), Some(BankId::new(0)));
        assert_eq!(
            controller.selection(),
            ChannelSelection::Memory {
                index: 0,
                id: ChannelId::new(1)
            }
        );
        // Bank 0 contains only channel 1, so navigation stays in place.
        assert!(controller.select_next().unwrap().activation.is_some());
        assert_eq!(controller.index(), 0);

        let update = controller.set_bank(Some(BankId::new(1))).unwrap();
        assert_eq!(
            update.activation.unwrap().setup.frequency.as_hz(),
            145_100_000
        );
        controller.select_next().unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(2));
        controller.select_previous().unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(1));

        assert_eq!(
            controller.set_bank(Some(BankId::new(5))),
            Err(ReceiveError::NoEligibleChannel)
        );
        assert_eq!(controller.select(9), Err(ReceiveError::IndexOutOfRange));
    }

    #[test]
    fn vfo_tuning_uses_the_channel_step_and_is_mode_gated() {
        let mut controller = controller(RadioConfig::conservative(), None);
        assert_eq!(
            controller.tune_up(),
            Err(ReceiveError::InvalidMode(ReceiveMode::Memory))
        );
        controller.enter_vfo().unwrap();
        assert_eq!(controller.selection(), ChannelSelection::Vfo);
        let update = controller.tune_up().unwrap();
        assert_eq!(
            update.activation.unwrap().setup.frequency.as_hz(),
            145_112_500
        );
        controller.tune_down().unwrap();
        controller.tune_down().unwrap();
        assert_eq!(controller.setup().frequency.as_hz(), 145_087_500);
        controller
            .tune_to(Frequency::from_hz(433_500_000).unwrap())
            .unwrap();
        assert_eq!(controller.setup().frequency.as_hz(), 433_500_000);
        controller.enter_memory().unwrap();
        assert_eq!(controller.setup().frequency.as_hz(), 145_100_000);
    }

    #[test]
    fn monitor_opens_audio_and_forces_the_open_squelch_level() {
        let mut controller = controller(RadioConfig::conservative(), None);
        assert!(!controller.is_audio_open());
        assert_eq!(controller.setup().squelch, SquelchLevel::new(4).unwrap());

        let update = controller.set_monitor(true);
        assert!(update.audio_open);
        assert!(controller.setup().squelch.is_open());

        controller.set_monitor(false);
        let closed = controller
            .observe(ReceiveObservation {
                squelch_open: false,
                tone_matched: None,
            })
            .unwrap();
        assert!(!closed.audio_open);
    }

    #[test]
    fn tone_squelch_gates_audio_independently_of_the_carrier() {
        let mut controller = controller(RadioConfig::conservative(), None);
        let carrier_only = controller
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: Some(false),
            })
            .unwrap();
        assert!(!carrier_only.audio_open);
        let matched = controller
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: Some(true),
            })
            .unwrap();
        assert!(matched.audio_open);
    }

    #[test]
    fn scanning_skips_marked_channels_and_resumes_after_the_hold() {
        let mut controller = controller(RadioConfig::conservative(), Some(BankId::new(1)));
        let start = controller.start_scanning().unwrap();
        assert_eq!(controller.state(), ReceiveState::Scanning(ScanPhase::Dwell));
        let token = armed(start.timer);

        // Channel 2 is scan-skipped, so the dwell expiry lands on channel 3.
        let advanced = controller.timer_elapsed(token).unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(3));
        let token = armed(advanced.timer);

        let busy = controller
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: None,
            })
            .unwrap();
        assert_eq!(controller.state(), ReceiveState::Scanning(ScanPhase::Hold));
        assert!(busy.audio_open);
        let hold_token = armed(busy.timer);

        // The stale dwell token no longer changes anything.
        assert_eq!(
            controller.timer_elapsed(token).unwrap().timer,
            TimerDirective::Unchanged
        );

        let resumed = controller.timer_elapsed(hold_token).unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(1));
        assert!(!resumed.audio_open);

        controller.stop_scanning().unwrap();
        assert_eq!(controller.state(), ReceiveState::Idle);
        assert_eq!(
            controller.stop_scanning(),
            Err(ReceiveError::InvalidState(ReceiveState::Idle))
        );
    }

    #[test]
    fn carrier_and_stop_resume_modes_change_scan_behaviour() {
        let mut carrier = RadioConfig::conservative();
        carrier.scan_resume = ScanResume::Carrier;
        let mut resuming = controller(carrier, Some(BankId::new(1)));
        resuming.start_scanning().unwrap();
        resuming
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: None,
            })
            .unwrap();
        assert_eq!(resuming.state(), ReceiveState::Scanning(ScanPhase::Hold));
        let resumed = resuming
            .observe(ReceiveObservation {
                squelch_open: false,
                tone_matched: None,
            })
            .unwrap();
        assert!(resumed.activation.is_some());
        assert_eq!(resuming.state(), ReceiveState::Scanning(ScanPhase::Dwell));

        let mut stop = RadioConfig::conservative();
        stop.scan_resume = ScanResume::Stop;
        let mut stopping = controller(stop, Some(BankId::new(1)));
        stopping.start_scanning().unwrap();
        let stopped = stopping
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: None,
            })
            .unwrap();
        assert_eq!(stopping.state(), ReceiveState::Idle);
        assert_eq!(stopped.timer, TimerDirective::Cancel);
    }

    #[test]
    fn scanning_requires_memory_mode_and_an_eligible_channel() {
        let mut controller = controller(RadioConfig::conservative(), Some(BankId::new(0)));
        controller.enter_vfo().unwrap();
        assert_eq!(
            controller.start_scanning(),
            Err(ReceiveError::InvalidMode(ReceiveMode::Vfo))
        );
        controller.enter_memory().unwrap();
        // Bank 0 holds one eligible channel, so the scan stays on it.
        let token = armed(controller.start_scanning().unwrap().timer);
        controller.timer_elapsed(token).unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(1));

        let mut skipped = ChannelMemory::<8>::new();
        skipped
            .insert(channel(1, 145_100_000, 0b0001, ChannelFlags::SCAN_SKIP))
            .unwrap();
        let (mut only_skipped, _) =
            BankedReceiveController::activate(skipped, RadioConfig::conservative(), None).unwrap();
        assert_eq!(
            only_skipped.start_scanning(),
            Err(ReceiveError::NoEligibleChannel)
        );
    }

    #[test]
    fn dual_watch_alternates_only_while_audio_is_closed() {
        let mut config = RadioConfig::conservative();
        config.dual_watch = true;
        let mut controller = controller(config, Some(BankId::new(1)));
        assert_eq!(
            controller.set_dual_watch(Some(0)),
            Err(ReceiveError::IndexOutOfRange)
        );
        let update = controller.set_dual_watch(Some(2)).unwrap();
        let token = armed(update.timer);

        let switched = controller.timer_elapsed(token).unwrap();
        assert_eq!(controller.channel().id(), ChannelId::new(3));
        let token = armed(switched.timer);

        controller
            .observe(ReceiveObservation {
                squelch_open: true,
                tone_matched: None,
            })
            .unwrap();
        let held = controller.timer_elapsed(token).unwrap();
        assert!(held.activation.is_none());
        assert_eq!(controller.channel().id(), ChannelId::new(3));

        let mut without = controller;
        without.set_monitor(false);
        without
            .observe(ReceiveObservation {
                squelch_open: false,
                tone_matched: None,
            })
            .unwrap();
        let token = armed(without.set_dual_watch(Some(0)).unwrap().timer);
        without.timer_elapsed(token).unwrap();
        assert_eq!(without.channel().id(), ChannelId::new(1));
    }

    #[test]
    fn dual_watch_requires_the_configured_option() {
        let mut controller = controller(RadioConfig::conservative(), None);
        assert!(controller.set_dual_watch(Some(1)).is_err());
    }

    #[test]
    fn view_positions_follow_the_bank_filter_and_select_what_they_name() {
        let mut controller = controller(RadioConfig::conservative(), None);
        assert_eq!(controller.visible_channels(), 3);
        assert_eq!(controller.visible_position(), 0);
        assert_eq!(
            controller.visible_channel(2).unwrap().id(),
            ChannelId::new(3)
        );
        assert_eq!(
            controller
                .select_visible(2)
                .unwrap()
                .activation
                .unwrap()
                .setup
                .frequency
                .as_hz(),
            145_300_000
        );
        assert_eq!(controller.visible_position(), 2);

        // Bank 1 holds only channel 1, so the view collapses to one position.
        controller.set_bank(Some(BankId::new(0))).unwrap();
        assert_eq!(controller.visible_channels(), 1);
        assert_eq!(controller.visible_position(), 0);
        assert!(controller.select_visible(1).is_err());
        assert_eq!(controller.visible_channel(1), None);
    }
}
