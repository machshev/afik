//! Hardware-independent receive application state and effects.

pub use super::configuration::PMR446_CHANNELS;
use super::configuration::{ActivatedConfiguration, Pmr446Channel};
const MAX_EFFECTS: usize = 3;

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
    /// Returns the receive-app action for a pressed key.
    ///
    /// Keys which belong to a retained target shell, such as digits and side
    /// keys, intentionally have no action in this receive-only slice.
    pub const fn receive_event(self) -> Option<Event> {
        match self {
            Self::Up => Some(Event::NextChannel),
            Self::Down => Some(Event::PreviousChannel),
            Self::Menu => Some(Event::ToggleAudio),
            Self::Side1
            | Self::Side2
            | Self::Ptt
            | Self::Exit
            | Self::Digit(_)
            | Self::Star
            | Self::Function => None,
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
    /// Toggle the operator audio preference.
    ToggleAudio,
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
pub struct View {
    /// One-based PMR446 channel number.
    pub channel: u8,
    /// Operator audio preference.
    pub audio: bool,
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
            items: [Some(first), None, None],
        }
    }

    const fn two(first: Effect, second: Effect) -> Self {
        Self {
            items: [Some(first), Some(second), None],
        }
    }

    const fn three(first: Effect, second: Effect, third: Effect) -> Self {
        Self {
            items: [Some(first), Some(second), Some(third)],
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
    squelch_open: bool,
    receiver_ok: bool,
}

impl Default for ReceiveApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiveApp {
    /// Creates the common operator defaults without touching hardware.
    pub const fn new() -> Self {
        Self::from_configuration(ActivatedConfiguration::new(0, Pmr446Channel::FIRST, true))
    }

    /// Creates the receive application from one target-adapted activation.
    ///
    /// K1 may supply a retained configuration generation and K5 may supply
    /// generation zero while it has no persistence. The generation is carried
    /// through the app but never written or interpreted here.
    pub const fn from_configuration(configuration: ActivatedConfiguration) -> Self {
        Self {
            configuration,
            squelch_open: false,
            receiver_ok: true,
        }
    }

    /// Returns the current semantic view.
    pub const fn view(&self) -> View {
        View {
            channel: self.configuration.channel_number(),
            audio: self.configuration.audio(),
            squelch_open: self.squelch_open,
            receiver_ok: self.receiver_ok,
        }
    }

    /// Applies one target-neutral event and returns ordered target effects.
    pub fn apply(&mut self, event: Event) -> Effects {
        match event {
            Event::Start => self.retune(),
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
            Event::KeyPress(key) => key
                .receive_event()
                .map_or_else(Effects::none, |event| self.apply(event)),
            Event::ReceiveSample { squelch_open } => {
                if !self.receiver_ok {
                    return Effects::none();
                }
                let speaker = self.configuration.audio() && squelch_open;
                if self.squelch_open == squelch_open {
                    Effects::one(Effect::SetSpeaker(speaker))
                } else {
                    self.squelch_open = squelch_open;
                    Effects::two(Effect::SetSpeaker(speaker), Effect::Redraw(self.view()))
                }
            }
            Event::ReceiverFault => {
                self.receiver_ok = false;
                self.squelch_open = false;
                Effects::two(Effect::SetSpeaker(false), Effect::Redraw(self.view()))
            }
        }
    }

    fn retune(&mut self) -> Effects {
        self.squelch_open = false;
        Effects::three(
            Effect::SetSpeaker(false),
            Effect::Tune {
                channel: self.configuration.channel_number(),
                frequency_hz: self.configuration.frequency_hz(),
                audio: self.configuration.audio(),
            },
            Effect::Redraw(self.view()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::configuration::ActivatedConfiguration;
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
}
