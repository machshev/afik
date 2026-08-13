//! K5 application-to-target effect boundary.

use radio_bk4819::{AfOutput, Bk4819, ReadbackRegister, ReceiveMetrics, RegisterBus};
use radio_domain::{Modulation, SquelchLevel, Tone};
use radio_platform::configuration_change::ConfigurationAcknowledgement;
use radio_platform::operator_shell::{Context, Intent, Shell};
use radio_platform::receive_app::{Effect, Event, ReceiveApp};

use crate::audio::SpeakerGate;
use crate::keypad::Key as PhysicalKey;
use crate::receive::Pmr446Channel;

/// The result of one bounded application event at the target boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    /// The application state and every requested target effect completed.
    Applied,
    /// The event had no applicable effect, or the application rejected it
    /// because it was same-level, pending, faulted, or otherwise unavailable.
    Rejected,
    /// A target operation failed and the application was faulted fail-silent.
    Faulted,
}

/// Minimal speaker seam used by the application boundary and host tests.
pub trait SpeakerOutput {
    /// Open or mute the target speaker path.
    fn set_enabled(&mut self, enabled: bool);
}

/// Production K5 edge dispatcher shared by the firmware loop and host tests.
/// Matrix scanning remains target-owned; this seam owns the bounded
/// press/release edge and routes it through the same shell/application path.
pub struct UiDispatcher {
    shown_key: Option<PhysicalKey>,
}

impl UiDispatcher {
    /// Starts with the last physical sample already observed by the target.
    #[must_use]
    pub const fn new(shown_key: Option<PhysicalKey>) -> Self {
        Self { shown_key }
    }

    /// Dispatches one physical scan result, emitting at most one shell edge.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_scan<B: RegisterBus, S: SpeakerOutput>(
        &mut self,
        key: Option<PhysicalKey>,
        now: u32,
        context: Context,
        shell: &mut Shell,
        app: &mut ReceiveApp,
        radio: &mut Bk4819<B>,
        speaker: &mut S,
        configured: &mut Option<u16>,
        metrics: &mut Option<ReceiveMetrics>,
    ) -> Option<ApplyOutcome> {
        if key == self.shown_key {
            return None;
        }
        let previous = self.shown_key;
        self.shown_key = key;
        let intent = if let Some(key) = key {
            shell.press(key.shared(), now, context)
        } else if let Some(previous) = previous {
            shell.release(previous.shared())
        } else {
            Intent::Idle
        };
        Some(apply_ui_intent(
            intent, shell, app, radio, speaker, configured, metrics,
        ))
    }
}

impl SpeakerOutput for SpeakerGate {
    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }
}

/// Applies one operator-shell intent through the shared application and target
/// adapter. Shell state commits only for [`ApplyOutcome::Applied`] squelch.
pub fn apply_ui_intent<B: RegisterBus, S: SpeakerOutput>(
    intent: Intent,
    shell: &mut Shell,
    app: &mut ReceiveApp,
    radio: &mut Bk4819<B>,
    speaker: &mut S,
    configured: &mut Option<u16>,
    metrics: &mut Option<ReceiveMetrics>,
) -> ApplyOutcome {
    let event = match intent {
        Intent::SelectNext => Some(Event::NextChannel),
        Intent::SelectPrevious => Some(Event::PreviousChannel),
        Intent::SelectIndex(index) => Some(Event::SelectChannel(
            u8::try_from(index.saturating_add(1)).unwrap_or(0),
        )),
        Intent::ToggleMonitor => Some(Event::ToggleMonitor),
        Intent::SetSquelch(level) => {
            let before = app.squelch();
            let outcome = apply_event(
                app,
                Event::SetSquelch(level),
                radio,
                speaker,
                configured,
                metrics,
            );
            if outcome == ApplyOutcome::Applied && app.squelch() == level {
                let _ = shell.accept_squelch(level);
            } else {
                shell.reject_squelch();
                debug_assert_eq!(app.squelch(), before);
            }
            return outcome;
        }
        Intent::SetSource(_)
        | Intent::TuneVfo
        | Intent::StartScan
        | Intent::StopScan
        | Intent::Redraw
        | Intent::Idle => None,
    };
    event.map_or(ApplyOutcome::Rejected, |event| {
        apply_event(app, event, radio, speaker, configured, metrics)
    })
}

/// Applies one shared application event and reports whether it was applied,
/// rejected without target work, or faulted during target work.
pub fn apply_event<B: RegisterBus, S: SpeakerOutput>(
    app: &mut ReceiveApp,
    event: Event,
    radio: &mut Bk4819<B>,
    speaker: &mut S,
    configured: &mut Option<u16>,
    metrics: &mut Option<ReceiveMetrics>,
) -> ApplyOutcome {
    let effects = app.apply(event);
    let change = effects.iter().find_map(|effect| match effect {
        Effect::ConfigurationChange(change) => Some(change),
        _ => None,
    });
    let squelch = effects.iter().find_map(|effect| match effect {
        Effect::Tune { squelch, .. } => Some(squelch),
        _ => None,
    });
    let squelch_change = matches!(event, Event::SetSquelch(_));
    let has_effects = effects.iter().next().is_some();
    let target_applied =
        has_effects && apply_effects(effects.iter(), radio, speaker, configured, metrics);

    if !has_effects {
        return ApplyOutcome::Rejected;
    }

    if let Some(change) = change {
        let acknowledgement = if target_applied {
            ConfigurationAcknowledgement::applied(change)
        } else {
            ConfigurationAcknowledgement::rejected(change)
        };
        let _ = apply_effects(
            app.apply(Event::ConfigurationAcknowledged(acknowledgement))
                .iter(),
            radio,
            speaker,
            configured,
            metrics,
        );
        return if target_applied {
            ApplyOutcome::Applied
        } else {
            ApplyOutcome::Faulted
        };
    }

    if squelch_change {
        let Some(level) = squelch else {
            return ApplyOutcome::Rejected;
        };
        let _ = apply_effects(
            app.apply(Event::SquelchAcknowledged {
                level,
                applied: target_applied,
            })
            .iter(),
            radio,
            speaker,
            configured,
            metrics,
        );
        return if target_applied && app.squelch() == level {
            ApplyOutcome::Applied
        } else if target_applied {
            ApplyOutcome::Rejected
        } else {
            ApplyOutcome::Faulted
        };
    }

    if matches!(event, Event::ReceiverFault) {
        return ApplyOutcome::Faulted;
    }
    if target_applied {
        ApplyOutcome::Applied
    } else {
        let _ = apply_effects(
            app.apply(Event::ReceiverFault).iter(),
            radio,
            speaker,
            configured,
            metrics,
        );
        ApplyOutcome::Faulted
    }
}

fn tune_and_sample<B: RegisterBus>(
    radio: &mut Bk4819<B>,
    channel: Pmr446Channel,
    audio: bool,
    squelch: SquelchLevel,
) -> (Option<u16>, Option<ReceiveMetrics>) {
    if radio
        .configure_receive(&channel.setup_with_squelch(audio, squelch))
        .is_err()
    {
        return (None, None);
    }
    let configured = radio.read_back(ReadbackRegister::FilterBandwidth).ok();
    let metrics = radio.receive_metrics(Tone::None).ok();
    (configured, metrics)
}

/// Applies a pre-existing effect batch from serial control through the same
/// target adapter used by keypad intents.
pub fn apply_effects<B: RegisterBus, S: SpeakerOutput>(
    effects: impl Iterator<Item = Effect>,
    radio: &mut Bk4819<B>,
    speaker: &mut S,
    configured: &mut Option<u16>,
    metrics: &mut Option<ReceiveMetrics>,
) -> bool {
    let mut complete = true;
    for effect in effects {
        match effect {
            Effect::Tune {
                channel,
                frequency_hz,
                audio,
                squelch,
            } => {
                let Some(channel) = Pmr446Channel::new(channel) else {
                    complete = false;
                    continue;
                };
                if channel.frequency().as_hz() != frequency_hz {
                    complete = false;
                    continue;
                }
                (*configured, *metrics) = tune_and_sample(radio, channel, audio, squelch);
                if configured.is_none() || metrics.is_none() {
                    complete = false;
                }
            }
            Effect::SetChipAudio(audio) => {
                let output = if audio {
                    AfOutput::Demodulated
                } else {
                    AfOutput::Mute
                };
                if radio.set_af_output(Modulation::Fm, output).is_err() {
                    complete = false;
                }
            }
            Effect::SetSpeaker(enabled) => speaker.set_enabled(enabled),
            Effect::ConfigurationChange(_) | Effect::ConfigurationAcknowledged(_) => {}
            Effect::Redraw(view) => {
                if !view.receiver_ok {
                    *configured = None;
                    *metrics = None;
                }
            }
        }
    }
    if !complete {
        speaker.set_enabled(false);
    }
    complete
}
