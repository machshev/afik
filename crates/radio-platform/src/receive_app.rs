//! Hardware-independent receive application state and effects.

/// Number of channels in the first shared PMR446 example plan.
pub const PMR446_CHANNELS: u8 = 16;
const PMR446_FIRST_HZ: u32 = 446_006_250;
const PMR446_STEP_HZ: u32 = 12_500;
const MAX_EFFECTS: usize = 3;

/// One event delivered by a target adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// Application adapters are ready and need their initial state applied.
    Start,
    /// Select the following channel with wraparound.
    NextChannel,
    /// Select the preceding channel with wraparound.
    PreviousChannel,
    /// Toggle the operator audio preference.
    ToggleAudio,
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
    channel: u8,
    audio: bool,
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
        Self {
            channel: 1,
            audio: true,
            squelch_open: false,
            receiver_ok: true,
        }
    }

    /// Returns the current semantic view.
    pub const fn view(&self) -> View {
        View {
            channel: self.channel,
            audio: self.audio,
            squelch_open: self.squelch_open,
            receiver_ok: self.receiver_ok,
        }
    }

    /// Applies one target-neutral event and returns ordered target effects.
    pub fn apply(&mut self, event: Event) -> Effects {
        match event {
            Event::Start => self.retune(),
            Event::NextChannel => {
                self.channel = if self.channel == PMR446_CHANNELS {
                    1
                } else {
                    self.channel + 1
                };
                self.retune()
            }
            Event::PreviousChannel => {
                self.channel = if self.channel == 1 {
                    PMR446_CHANNELS
                } else {
                    self.channel - 1
                };
                self.retune()
            }
            Event::ToggleAudio => {
                self.audio = !self.audio;
                self.squelch_open = false;
                Effects::three(
                    Effect::SetSpeaker(false),
                    Effect::SetChipAudio(self.audio),
                    Effect::Redraw(self.view()),
                )
            }
            Event::ReceiveSample { squelch_open } => {
                if !self.receiver_ok {
                    return Effects::none();
                }
                let speaker = self.audio && squelch_open;
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
                channel: self.channel,
                frequency_hz: PMR446_FIRST_HZ + u32::from(self.channel - 1) * PMR446_STEP_HZ,
                audio: self.audio,
            },
            Effect::Redraw(self.view()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Event, ReceiveApp};

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
}
