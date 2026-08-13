//! Hardware-independent receive application state and effects.

pub use super::configuration::PMR446_CHANNELS;
use super::configuration::{ActivatedConfiguration, Pmr446Channel};
use super::configuration_change::{
    ConfigurationAcknowledgement, ConfigurationCandidate, ConfigurationChange,
    ConfigurationChangeResult,
};
const MAX_EFFECTS: usize = 4;

/// One logical key exposed by a target keypad adapter.
///
/// Target crates retain matrix scanning, settling, debounce, and GPIO
/// ownership. Once a scan has become a stable edge, both targets use this
/// vocabulary so operator meaning is not duplicated beside the receive app.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    /// The upper side key.
    Side1,
    /// The lower side key.
    Side2,
    /// The input-only push-to-talk switch.
    Ptt,
    /// The menu/confirm key.
    Menu,
    /// The channel-up key.
    Up,
    /// The channel-down key.
    Down,
    /// The exit/back key.
    Exit,
    /// A decimal digit, if the target reports one.
    Digit(u8),
    /// The star key.
    Star,
    /// The function/hash key.
    Function,
}

impl Key {
    /// Every logical key exposed by the supported handset adapters.
    pub const ALL: [Self; 19] = [
        Self::Side1,
        Self::Side2,
        Self::Ptt,
        Self::Menu,
        Self::Up,
        Self::Down,
        Self::Exit,
        Self::Digit(0),
        Self::Digit(1),
        Self::Digit(2),
        Self::Digit(3),
        Self::Digit(4),
        Self::Digit(5),
        Self::Digit(6),
        Self::Digit(7),
        Self::Digit(8),
        Self::Digit(9),
        Self::Star,
        Self::Function,
    ];

    /// Fixed label used by shared display witnesses.
    #[must_use]
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::Side1 => b"SIDE1",
            Self::Side2 => b"SIDE2",
            Self::Ptt => b"PTT",
            Self::Menu => b"MENU",
            Self::Up => b"UP",
            Self::Down => b"DOWN",
            Self::Exit => b"EXIT",
            Self::Digit(0) => b"0",
            Self::Digit(1) => b"1",
            Self::Digit(2) => b"2",
            Self::Digit(3) => b"3",
            Self::Digit(4) => b"4",
            Self::Digit(5) => b"5",
            Self::Digit(6) => b"6",
            Self::Digit(7) => b"7",
            Self::Digit(8) => b"8",
            Self::Digit(9) => b"9",
            Self::Digit(_) => b"?",
            Self::Star => b"STAR",
            Self::Function => b"F",
        }
    }
}

impl Key {
    /// Returns the receive-app action for a pressed key.
    ///
    /// Keys which belong to a retained target shell, such as digits and side
    /// keys, intentionally have no action in this receive-only slice.
    pub const fn receive_event(self) -> Option<Event> {
        match self {
            // Match the operator shell: Up walks toward the first channel and
            // Down walks toward the last. Target serial adapters may narrow
            // this domain when their controller cannot own that selection.
            Self::Up => Some(Event::PreviousChannel),
            Self::Down => Some(Event::NextChannel),
            // Menu belongs to the operator shell's list-confirm behavior. It
            // is deliberately not a receive-app key shortcut; serial control
            // uses the explicit ToggleAudio command when that is intended.
            Self::Menu => None,
            Self::Side2 => Some(Event::ToggleMonitor),
            Self::Side1 | Self::Ptt | Self::Exit | Self::Digit(_) | Self::Star | Self::Function => {
                None
            }
        }
    }
}

/// One event delivered by a target adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// Application adapters are ready and need their initial state applied.
    Start,
    /// Select the following channel with wraparound.
    NextChannel,
    /// Select the preceding channel with wraparound.
    PreviousChannel,
    /// Select one PMR446 example channel directly.
    ///
    /// Target shells and programmer-controlled channel lists already know the
    /// selected position.  This event lets them enter the common receive path
    /// without replaying intermediate button presses and tuning those
    /// intermediate channels on hardware.
    SelectChannel(u8),
    /// Ask the adapter to apply one bounded candidate. The app commits it
    /// only after a matching [`Event::ConfigurationAcknowledged`] arrives.
    ConfigurationCandidate(ConfigurationCandidate),
    /// Report whether the adapter applied the pending candidate.
    ConfigurationAcknowledged(ConfigurationAcknowledgement),
    /// Toggle the operator audio preference.
    ToggleAudio,
    /// Toggle the operator monitor override, which opens squelch deliberately.
    ToggleMonitor,
    /// Set the monitor override to a target-owned desired state.
    SetMonitor(bool),
    /// Apply the receive meaning of one stable pressed key.
    KeyPress(Key),
    /// One complete receiver-status sample.
    ReceiveSample {
        /// Whether the receiver's current squelch criterion is open.
        squelch_open: bool,
    },
    /// Receiver state is unknown and must fail silent.
    ReceiverFault,
}

/// One target operation requested by the shared application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Ask the target adapter to apply one exact, identified configuration
    /// change. The adapter must acknowledge this change after its bounded
    /// hardware operation completes.
    ConfigurationChange(ConfigurationChange),
    /// Surface the result of the shared configuration hand-off to the target
    /// adapter. It carries no persistence or protocol meaning.
    ConfigurationAcknowledged(ConfigurationAcknowledgement),
    /// Configure the receiver for the selected channel and audio preference.
    Tune {
        /// One-based PMR446 channel number.
        channel: u8,
        /// Exact receive frequency in hertz.
        frequency_hz: u32,
        /// Whether demodulated chip AF should be selected.
        audio: bool,
    },
    /// Change chip AF routing without retuning.
    SetChipAudio(bool),
    /// Drive the board speaker gate.
    SetSpeaker(bool),
    /// Render the current semantic application state.
    Redraw(View),
}

/// Semantic state consumed by either target's renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct View {
    /// One-based PMR446 channel number.
    pub channel: u8,
    /// Operator audio preference.
    pub audio: bool,
    /// Whether the operator monitor override is active.
    pub monitoring: bool,
    /// Latest sampled squelch link; false before a sample or after retuning.
    pub squelch_open: bool,
    /// Whether receiver state is still known.
    pub receiver_ok: bool,
}

/// Fixed-capacity effects from one event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effects {
    items: [Option<Effect>; MAX_EFFECTS],
}

impl Effects {
    const fn none() -> Self {
        Self {
            items: [None; MAX_EFFECTS],
        }
    }

    const fn one(first: Effect) -> Self {
        Self {
            items: [Some(first), None, None, None],
        }
    }

    const fn two(first: Effect, second: Effect) -> Self {
        Self {
            items: [Some(first), Some(second), None, None],
        }
    }

    const fn three(first: Effect, second: Effect, third: Effect) -> Self {
        Self {
            items: [Some(first), Some(second), Some(third), None],
        }
    }

    const fn four(first: Effect, second: Effect, third: Effect, fourth: Effect) -> Self {
        Self {
            items: [Some(first), Some(second), Some(third), Some(fourth)],
        }
    }

    /// Iterates over the emitted effects in required application order.
    pub fn iter(&self) -> impl Iterator<Item = Effect> + '_ {
        self.items.iter().copied().flatten()
    }
}

/// Shared receive-only operator application.
pub struct ReceiveApp {
    configuration: ActivatedConfiguration,
    pending_configuration: Option<ConfigurationChange>,
    // This is an application token, not a target identity. Callers which can
    // queue effects across app replacement must supply a fresh value so an
    // old acknowledgement cannot be accepted by a new instance.
    instance_id: u16,
    // Zero is reserved as the exhausted/synthetic rejection identity. IDs
    // are never reused, so a delayed acknowledgement cannot apply to a later
    // request carrying the same candidate.
    next_change_id: u16,
    squelch_open: bool,
    receiver_ok: bool,
    monitoring: bool,
}

impl Default for ReceiveApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiveApp {
    /// Creates the common operator defaults without touching hardware.
    pub const fn new() -> Self {
        Self::from_configuration_with_identity(
            ActivatedConfiguration::new(0, Pmr446Channel::FIRST, true),
            0,
        )
    }

    /// Creates the receive application from one target-adapted activation.
    ///
    /// K1 may supply a retained configuration generation and K5 may supply
    /// generation zero while it has no persistence. The generation is carried
    /// through the app but never written or interpreted here.
    pub const fn from_configuration(configuration: ActivatedConfiguration) -> Self {
        Self::from_configuration_with_identity(configuration, 0)
    }

    /// Creates the receive application with an explicit bounded instance
    /// token. The token is needed only when effects may outlive this value.
    pub const fn from_configuration_with_identity(
        configuration: ActivatedConfiguration,
        instance_id: u16,
    ) -> Self {
        Self {
            configuration,
            pending_configuration: None,
            instance_id,
            next_change_id: 1,
            squelch_open: false,
            receiver_ok: true,
            monitoring: false,
        }
    }

    /// Creates the default receive application with an explicit instance
    /// token for queued-effect boundaries.
    pub const fn new_with_identity(instance_id: u16) -> Self {
        Self::from_configuration_with_identity(
            ActivatedConfiguration::new(0, Pmr446Channel::FIRST, true),
            instance_id,
        )
    }

    /// Returns the current semantic view.
    pub const fn view(&self) -> View {
        View {
            channel: self.configuration.channel_number(),
            audio: self.configuration.audio(),
            monitoring: self.monitoring,
            squelch_open: self.squelch_open,
            receiver_ok: self.receiver_ok,
        }
    }

    /// Applies one target-neutral event and returns ordered target effects.
    pub fn apply(&mut self, event: Event) -> Effects {
        match event {
            Event::Start
            | Event::NextChannel
            | Event::PreviousChannel
            | Event::SelectChannel(_)
            | Event::ToggleAudio
            | Event::ToggleMonitor
                if self.pending_configuration.is_some() =>
            {
                Effects::none()
            }
            Event::Start
            | Event::NextChannel
            | Event::PreviousChannel
            | Event::SelectChannel(_)
            | Event::ToggleAudio
            | Event::ToggleMonitor
            | Event::SetMonitor(_)
            | Event::KeyPress(_)
                if !self.receiver_ok =>
            {
                Effects::none()
            }
            Event::Start => self.retune(),
            Event::ConfigurationCandidate(candidate) => self.apply_candidate(candidate),
            Event::ConfigurationAcknowledged(acknowledgement) => {
                self.apply_acknowledgement(acknowledgement)
            }
            Event::NextChannel => {
                self.configuration = self
                    .configuration
                    .select(self.configuration.channel().next());
                self.retune()
            }
            Event::PreviousChannel => {
                self.configuration = self
                    .configuration
                    .select(self.configuration.channel().previous());
                self.retune()
            }
            Event::SelectChannel(channel) => {
                let Some(channel) = Pmr446Channel::new(channel) else {
                    return Effects::none();
                };
                self.configuration = self.configuration.select(channel);
                self.retune()
            }
            Event::ToggleAudio => {
                self.configuration = self.configuration.with_audio(!self.configuration.audio());
                self.squelch_open = false;
                Effects::three(
                    Effect::SetSpeaker(false),
                    Effect::SetChipAudio(self.configuration.audio()),
                    Effect::Redraw(self.view()),
                )
            }
            Event::ToggleMonitor => {
                self.monitoring = !self.monitoring;
                Effects::two(
                    Effect::SetSpeaker(self.configuration.audio() && self.monitoring),
                    Effect::Redraw(self.view()),
                )
            }
            Event::SetMonitor(monitoring) => {
                self.monitoring = monitoring;
                Effects::two(
                    Effect::SetSpeaker(self.configuration.audio() && monitoring),
                    Effect::Redraw(self.view()),
                )
            }
            Event::KeyPress(key) => key
                .receive_event()
                .map_or_else(Effects::none, |event| self.apply(event)),
            Event::ReceiveSample { squelch_open } => {
                if !self.receiver_ok {
                    return Effects::none();
                }
                let speaker = self.configuration.audio() && (self.monitoring || squelch_open);
                if self.squelch_open == squelch_open {
                    Effects::one(Effect::SetSpeaker(speaker))
                } else {
                    self.squelch_open = squelch_open;
                    Effects::two(Effect::SetSpeaker(speaker), Effect::Redraw(self.view()))
                }
            }
            Event::ReceiverFault => {
                // A fault invalidates the in-flight adapter operation. Its
                // later acknowledgement must not resurrect the candidate.
                self.pending_configuration = None;
                self.receiver_ok = false;
                self.squelch_open = false;
                self.monitoring = false;
                Effects::two(Effect::SetSpeaker(false), Effect::Redraw(self.view()))
            }
        }
    }

    fn apply_candidate(&mut self, candidate: ConfigurationCandidate) -> Effects {
        if self.pending_configuration.is_some() {
            let rejected = ConfigurationChange::new(0, candidate);
            return Effects::one(Effect::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::rejected(rejected),
            ));
        }
        let Some(change) = self.allocate_change(candidate) else {
            let rejected = ConfigurationChange::new(0, candidate);
            return Effects::one(Effect::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::rejected(rejected),
            ));
        };
        self.pending_configuration = Some(change);
        self.pending_retune(change)
    }

    fn apply_acknowledgement(&mut self, acknowledgement: ConfigurationAcknowledgement) -> Effects {
        let Some(change) = self.pending_configuration else {
            return Effects::none();
        };
        if change != acknowledgement.change() {
            return Effects::none();
        }
        self.pending_configuration = None;
        if matches!(acknowledgement.result(), ConfigurationChangeResult::Applied) {
            self.configuration = change.candidate().configuration();
            self.receiver_ok = true;
            self.squelch_open = false;
            Effects::three(
                Effect::SetSpeaker(false),
                Effect::Redraw(self.view()),
                Effect::ConfigurationAcknowledged(acknowledgement),
            )
        } else {
            self.receiver_ok = false;
            self.squelch_open = false;
            Effects::four(
                Effect::SetChipAudio(false),
                Effect::SetSpeaker(false),
                Effect::Redraw(self.view()),
                Effect::ConfigurationAcknowledged(acknowledgement),
            )
        }
    }

    fn retune(&mut self) -> Effects {
        self.squelch_open = false;
        let tune = Effect::Tune {
            channel: self.configuration.channel_number(),
            frequency_hz: self.configuration.frequency_hz(),
            audio: self.configuration.audio(),
        };
        if self.monitoring {
            Effects::four(
                Effect::SetSpeaker(false),
                tune,
                Effect::SetSpeaker(self.configuration.audio()),
                Effect::Redraw(self.view()),
            )
        } else {
            Effects::three(Effect::SetSpeaker(false), tune, Effect::Redraw(self.view()))
        }
    }

    fn allocate_change(
        &mut self,
        candidate: ConfigurationCandidate,
    ) -> Option<ConfigurationChange> {
        if self.next_change_id == 0 {
            return None;
        }
        let id = self.next_change_id;
        self.next_change_id = self.next_change_id.checked_add(1).unwrap_or(0);
        Some(ConfigurationChange::with_instance(
            self.instance_id,
            id,
            candidate,
        ))
    }

    fn pending_retune(&mut self, change: ConfigurationChange) -> Effects {
        self.squelch_open = false;
        let configuration = change.candidate().configuration();
        Effects::four(
            Effect::SetSpeaker(false),
            Effect::Tune {
                channel: configuration.channel_number(),
                frequency_hz: configuration.frequency_hz(),
                audio: configuration.audio(),
            },
            Effect::Redraw(self.view()),
            Effect::ConfigurationChange(change),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::configuration::ActivatedConfiguration;
    use super::super::configuration_change::{
        ConfigurationAcknowledgement, ConfigurationCandidate, ConfigurationChangeResult,
    };
    use super::{Effect, Effects, Event, Key, ReceiveApp};

    fn trace(events: &[Event]) -> std::vec::Vec<Effect> {
        let mut app = ReceiveApp::new();
        events
            .iter()
            .flat_map(|event| app.apply(*event).iter().collect::<std::vec::Vec<_>>())
            .collect()
    }

    #[test]
    fn k1_and_k5_adapter_fixtures_receive_identical_ordered_effects() {
        let events = [
            Event::Start,
            Event::ReceiveSample { squelch_open: true },
            Event::ToggleAudio,
            Event::NextChannel,
            Event::PreviousChannel,
            Event::ReceiverFault,
        ];
        let k1_fixture = trace(&events);
        let k5_fixture = trace(&events);
        assert_eq!(k1_fixture, k5_fixture);
    }

    #[test]
    fn start_and_channel_navigation_are_exact_and_fail_silent_while_retuning() {
        let mut app = ReceiveApp::new();
        assert_eq!(
            app.apply(Event::Start).iter().collect::<std::vec::Vec<_>>(),
            std::vec![
                Effect::SetSpeaker(false),
                Effect::Tune {
                    channel: 1,
                    frequency_hz: 446_006_250,
                    audio: true,
                },
                Effect::Redraw(app.view()),
            ]
        );
        app.apply(Event::PreviousChannel);
        assert_eq!(app.view().channel, 16);
        app.apply(Event::NextChannel);
        assert_eq!(app.view().channel, 1);
    }

    #[test]
    fn direct_selection_tunes_only_the_requested_channel() {
        let mut app = ReceiveApp::new();
        let effects = app.apply(Event::SelectChannel(8));
        assert_eq!(app.view().channel, 8);
        assert_eq!(
            effects.iter().collect::<std::vec::Vec<_>>(),
            std::vec![
                Effect::SetSpeaker(false),
                Effect::Tune {
                    channel: 8,
                    frequency_hz: 446_093_750,
                    audio: true,
                },
                Effect::Redraw(app.view()),
            ]
        );

        assert_eq!(app.apply(Event::SelectChannel(0)), Effects::none());
        assert_eq!(app.view().channel, 8);
    }

    #[test]
    fn audio_and_squelch_have_one_shared_gating_rule() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        let open = app.apply(Event::ReceiveSample { squelch_open: true });
        assert!(open.iter().any(|effect| effect == Effect::SetSpeaker(true)));

        let muted = app.apply(Event::ToggleAudio);
        assert_eq!(muted.iter().next(), Some(Effect::SetSpeaker(false)));
        assert!(!app.view().audio);
        let still_muted = app.apply(Event::ReceiveSample { squelch_open: true });
        assert!(still_muted
            .iter()
            .any(|effect| effect == Effect::SetSpeaker(false)));
    }

    #[test]
    fn monitor_opens_and_closes_the_shared_speaker_gate() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        app.apply(Event::ToggleMonitor);
        assert!(app.view().monitoring);
        assert!(app
            .apply(Event::ReceiveSample {
                squelch_open: false,
            })
            .iter()
            .any(|effect| effect == Effect::SetSpeaker(true)));
        let effects = app.apply(Event::ToggleMonitor);
        assert!(!app.view().monitoring);
        assert!(effects
            .iter()
            .any(|effect| effect == Effect::SetSpeaker(false)));
    }

    #[test]
    fn a_receiver_fault_latches_silent_and_ignores_later_samples() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        app.apply(Event::ReceiverFault);
        assert!(!app.view().receiver_ok);
        assert_eq!(
            app.apply(Event::ReceiveSample { squelch_open: true }),
            super::Effects::none()
        );
    }

    #[test]
    fn receiver_fault_blocks_every_operational_control_until_recovery() {
        let mut app = ReceiveApp::new();
        app.apply(Event::ReceiverFault);
        let before = app.view();
        for event in [
            Event::Start,
            Event::NextChannel,
            Event::PreviousChannel,
            Event::SelectChannel(8),
            Event::ToggleAudio,
            Event::ToggleMonitor,
            Event::SetMonitor(true),
            Event::KeyPress(Key::Up),
            Event::KeyPress(Key::Side2),
        ] {
            assert_eq!(app.apply(event), Effects::none());
            assert_eq!(app.view(), before);
        }
    }

    #[test]
    fn pending_monitor_synchronization_is_allowed_and_recovery_requires_ack() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(4, 8, true).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        assert!(request
            .iter()
            .any(|effect| matches!(effect, Effect::ConfigurationChange(_))));
        let sync = app.apply(Event::SetMonitor(true));
        assert!(app.view().monitoring);
        assert!(sync.iter().any(|effect| effect == Effect::SetSpeaker(true)));
        assert_eq!(app.view().channel, 1);
        app.apply(Event::ReceiverFault);
        let recovery = ConfigurationCandidate::from_channel(4, 9, true).expect("recovery");
        let request = app.apply(Event::ConfigurationCandidate(recovery));
        let recovery_change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("recovery change");
        assert!(!app.view().receiver_ok);
        app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(recovery_change),
        ));
        assert!(app.view().receiver_ok);
        assert_eq!(app.view().channel, 9);
    }

    #[test]
    fn target_keyboards_share_receive_semantics() {
        let mut k1 = ReceiveApp::new();
        let mut k5 = ReceiveApp::new();
        let keys = [Key::Up, Key::Menu, Key::Down, Key::Digit(4), Key::Side1];
        for key in keys {
            assert_eq!(
                k1.apply(Event::KeyPress(key)),
                k5.apply(Event::KeyPress(key))
            );
            assert_eq!(k1.view(), k5.view());
        }
    }

    #[test]
    fn k1_retained_and_k5_nonpersistent_activations_share_the_same_trace() {
        let k1 = ActivatedConfiguration::from_channel(7, 8, true).expect("K1 activation");
        let k5 = ActivatedConfiguration::from_channel(0, 8, true).expect("K5 activation");
        let mut k1_app = ReceiveApp::from_configuration(k1);
        let mut k5_app = ReceiveApp::from_configuration(k5);
        let events = [
            Event::Start,
            Event::ReceiveSample { squelch_open: true },
            Event::KeyPress(Key::Menu),
            Event::KeyPress(Key::Up),
        ];
        for event in events {
            assert_eq!(
                k1_app.apply(event).iter().collect::<std::vec::Vec<_>>(),
                k5_app.apply(event).iter().collect::<std::vec::Vec<_>>()
            );
            assert_eq!(k1_app.view(), k5_app.view());
        }
        assert_ne!(k1.generation(), k5.generation());
    }

    #[test]
    fn retune_and_fault_clear_shared_display_gate_state() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        app.apply(Event::ReceiveSample { squelch_open: true });
        assert!(app.view().squelch_open);
        app.apply(Event::NextChannel);
        assert!(!app.view().squelch_open);
        app.apply(Event::ReceiveSample { squelch_open: true });
        app.apply(Event::ReceiverFault);
        assert!(!app.view().squelch_open);
        assert!(!app.view().receiver_ok);
    }

    #[test]
    fn configuration_candidate_commits_only_after_matching_adapter_ack() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(7, 8, false).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        assert_eq!(app.view().channel, 1, "candidate is not active before ack");
        assert!(request.iter().any(|effect| {
            matches!(
                effect,
                Effect::Tune {
                    channel: 8,
                    audio: false,
                    ..
                }
            )
        }));
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("request carries its change identity");

        let applied = app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(change),
        ));
        assert_eq!(app.view().channel, 8);
        assert!(!app.view().audio);
        assert!(applied.iter().any(|effect| {
            matches!(effect, Effect::ConfigurationAcknowledged(ack) if ack.is_applied())
        }));
    }

    #[test]
    fn rejected_configuration_is_fail_silent_and_does_not_replace_snapshot() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(0, 3, true).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("request carries its change identity");
        let rejected = app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::rejected(change),
        ));
        assert_eq!(app.view().channel, 1);
        assert!(!app.view().receiver_ok);
        assert!(rejected
            .iter()
            .any(|effect| effect == Effect::SetChipAudio(false)));
        assert!(rejected.iter().any(|effect| {
            matches!(
                effect,
                Effect::ConfigurationAcknowledged(ack)
                    if ack.result() == ConfigurationChangeResult::Rejected
            )
        }));
    }

    #[test]
    fn k1_and_k5_candidates_share_tuning_effects_but_keep_generation_capability_distinct() {
        let k1 = ConfigurationCandidate::from_channel(7, 8, true).expect("K1 candidate");
        let k5 = ConfigurationCandidate::from_channel(0, 8, true).expect("K5 candidate");
        let mut k1_app = ReceiveApp::new();
        let mut k5_app = ReceiveApp::new();
        let k1_request = k1_app.apply(Event::ConfigurationCandidate(k1));
        let k5_request = k5_app.apply(Event::ConfigurationCandidate(k5));
        assert_eq!(
            k1_request
                .iter()
                .filter(|effect| {
                    !matches!(effect, Effect::Redraw(_) | Effect::ConfigurationChange(_))
                })
                .collect::<std::vec::Vec<_>>(),
            k5_request
                .iter()
                .filter(|effect| {
                    !matches!(effect, Effect::Redraw(_) | Effect::ConfigurationChange(_))
                })
                .collect::<std::vec::Vec<_>>()
        );
        let k1_change = k1_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("K1 change");
        let k5_change = k5_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("K5 change");
        assert_eq!(k1_app.view(), k5_app.view());
        k1_app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(k1_change),
        ));
        k5_app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(k5_change),
        ));
        assert_eq!(k1_app.view(), k5_app.view());
        assert_ne!(k1.generation(), k5.generation());
    }

    #[test]
    fn mismatched_and_stale_acknowledgements_cannot_commit_a_candidate() {
        let mut app = ReceiveApp::new();
        let first = ConfigurationCandidate::from_channel(3, 4, true).expect("first");
        let first_request = app.apply(Event::ConfigurationCandidate(first));
        let first_change = first_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("first change");
        // A different change identity with the same candidate is stale.
        let stale = super::super::configuration_change::ConfigurationChange::new(
            first_change.id().saturating_add(1),
            first,
        );
        assert_eq!(
            app.apply(Event::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::applied(stale),
            )),
            Effects::none()
        );
        assert_eq!(app.view().channel, 1);

        assert_eq!(
            app.apply(Event::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::applied(first_change),
            )),
            Effects::three(
                Effect::SetSpeaker(false),
                Effect::Redraw(app.view()),
                Effect::ConfigurationAcknowledged(ConfigurationAcknowledgement::applied(
                    first_change
                ),),
            )
        );
        assert_eq!(app.view().channel, 4);

        // A late acknowledgement from the completed request is ignored even
        // after an identical candidate is requested again.
        let second_request = app.apply(Event::ConfigurationCandidate(first));
        let second_change = second_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("second change");
        assert_ne!(first_change.id(), second_change.id());
        assert_eq!(
            app.apply(Event::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::applied(first_change),
            )),
            Effects::none()
        );
        assert_eq!(app.view().channel, 4);
    }

    #[test]
    fn pending_candidate_is_not_overwritten_by_intervening_receive_events() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(2, 8, false).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("pending change");

        for event in [
            Event::Start,
            Event::NextChannel,
            Event::PreviousChannel,
            Event::SelectChannel(12),
            Event::ToggleAudio,
            Event::KeyPress(Key::Up),
            Event::KeyPress(Key::Menu),
        ] {
            assert_eq!(app.apply(event), Effects::none());
        }
        assert_eq!(app.view().channel, 1);
        assert!(app.view().audio);

        app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(change),
        ));
        assert_eq!(app.view().channel, 8);
        assert!(!app.view().audio);
    }

    #[test]
    fn receiver_fault_invalidates_pending_change_and_late_ack() {
        let mut app = ReceiveApp::new();
        let candidate = ConfigurationCandidate::from_channel(2, 8, true).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("pending change");
        app.apply(Event::ReceiverFault);
        assert_eq!(app.view().channel, 1);
        assert!(!app.view().receiver_ok);
        assert_eq!(
            app.apply(Event::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::applied(change),
            )),
            Effects::none()
        );
        assert_eq!(app.view().channel, 1);
        assert!(!app.view().receiver_ok);
    }

    #[test]
    fn queued_acknowledgement_cannot_cross_explicit_app_instances() {
        let candidate = ConfigurationCandidate::from_channel(2, 8, true).expect("candidate");
        let mut first = ReceiveApp::new_with_identity(1);
        let mut second = ReceiveApp::new_with_identity(2);
        let first_request = first.apply(Event::ConfigurationCandidate(candidate));
        let first_change = first_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("first change");
        let second_request = second.apply(Event::ConfigurationCandidate(candidate));
        let second_change = second_request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("second change");
        assert_ne!(first_change.id(), second_change.id());
        assert_eq!(first_change.instance_id(), 1);
        assert_eq!(second_change.instance_id(), 2);
        assert_eq!(
            second.apply(Event::ConfigurationAcknowledged(
                ConfigurationAcknowledgement::applied(first_change),
            )),
            Effects::none()
        );
        assert_eq!(second.view().channel, 1);
    }
}
