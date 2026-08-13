//! K1 translation of shared receive-application effects.

use radio_channel_control::ChannelReceiveSetup;
use radio_domain::{Bandwidth, Frequency, FrequencyStep, Modulation, SquelchLevel, Tone};
use radio_platform::configuration::Pmr446Channel;
use radio_platform::configuration_change::{ConfigurationAcknowledgement, ConfigurationChange};
use radio_platform::receive_app::{Effect, View};

/// One operation expressed in the K1 receiver/display adapter vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum K1Effect {
    /// Identify the exact candidate which the target must acknowledge after
    /// the preceding tune operation completes.
    ConfigurationChange(ConfigurationChange),
    /// Apply the existing complete K1 receive setup.
    Tune {
        /// Complete validated receive setup.
        setup: ChannelReceiveSetup,
        /// Whether demodulated chip audio should be routed after tuning.
        audio: bool,
    },
    /// Route or mute demodulated BK4829 audio.
    SetChipAudio(bool),
    /// Drive the K1 receive speaker amplifier.
    SetSpeaker(bool),
    /// Render the shared semantic view through the K1 display adapter.
    Redraw(View),
    /// Carry the shared application acknowledgement across the target seam.
    ConfigurationAcknowledged(ConfigurationAcknowledgement),
}

/// A shared effect was inconsistent with its validated PMR channel identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterError;

/// Identifies the shared PMR446 example channel represented by a complete K1
/// receive setup.
///
/// The K1 continues to accept arbitrary programmed channels and VFO settings;
/// those do not silently become shared-example channels merely because their
/// frequency happens to be nearby.
#[must_use]
pub fn shared_channel(setup: ChannelReceiveSetup) -> Option<u8> {
    if setup.modulation != Modulation::Fm
        || setup.bandwidth != Bandwidth::Narrow
        || setup.tone != Tone::None
        || setup.step.as_hz() != 12_500
    {
        return None;
    }
    Some(Pmr446Channel::from_frequency_hz(setup.frequency.as_hz())?.number())
}

/// Translates one shared effect without touching K1 hardware.
pub fn translate(effect: Effect) -> Result<K1Effect, AdapterError> {
    match effect {
        Effect::Tune {
            channel,
            frequency_hz,
            audio,
        } => {
            let selected = Pmr446Channel::new(channel).ok_or(AdapterError)?;
            if frequency_hz != selected.frequency_hz() {
                return Err(AdapterError);
            }
            Ok(K1Effect::Tune {
                setup: ChannelReceiveSetup {
                    frequency: Frequency::from_hz(frequency_hz).map_err(|_| AdapterError)?,
                    modulation: Modulation::Fm,
                    bandwidth: Bandwidth::Narrow,
                    tone: Tone::None,
                    squelch: SquelchLevel::CONSERVATIVE,
                    step: FrequencyStep::from_hz(12_500).map_err(|_| AdapterError)?,
                },
                audio,
            })
        }
        Effect::ConfigurationChange(change) => Ok(K1Effect::ConfigurationChange(change)),
        Effect::SetChipAudio(enabled) => Ok(K1Effect::SetChipAudio(enabled)),
        Effect::SetSpeaker(enabled) => Ok(K1Effect::SetSpeaker(enabled)),
        Effect::Redraw(view) => Ok(K1Effect::Redraw(view)),
        Effect::ConfigurationAcknowledged(ack) => Ok(K1Effect::ConfigurationAcknowledged(ack)),
    }
}

#[cfg(test)]
mod tests {
    use super::{shared_channel, translate, AdapterError, K1Effect};
    use radio_channel_control::ChannelReceiveSetup;
    use radio_domain::{Bandwidth, Frequency, FrequencyStep, Modulation, SquelchLevel, Tone};
    use radio_platform::receive_app::{Effect, Event, ReceiveApp};

    #[test]
    fn shared_start_becomes_the_existing_k1_receive_vocabulary() {
        let mut app = ReceiveApp::new();
        let translated: std::vec::Vec<_> = app
            .apply(Event::Start)
            .iter()
            .map(translate)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(translated[0], K1Effect::SetSpeaker(false));
        let K1Effect::Tune { setup, audio } = translated[1] else {
            panic!("second effect must tune");
        };
        assert!(audio);
        assert_eq!(setup.frequency.as_hz(), 446_006_250);
        assert_eq!(setup.modulation, Modulation::Fm);
        assert_eq!(setup.bandwidth, Bandwidth::Narrow);
        assert_eq!(setup.tone, Tone::None);
        assert_eq!(setup.squelch, SquelchLevel::CONSERVATIVE);
    }

    #[test]
    fn muted_navigation_keeps_chip_audio_muted_across_tune() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        app.apply(Event::ToggleAudio);
        let translated: std::vec::Vec<_> = app
            .apply(Event::NextChannel)
            .iter()
            .map(translate)
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(matches!(translated[1], K1Effect::Tune { audio: false, .. }));
    }

    #[test]
    fn k1_monitor_intent_uses_the_shared_state_boundary() {
        let mut app = ReceiveApp::new();
        let translated: std::vec::Vec<_> = app
            .apply(Event::SetMonitor(true))
            .iter()
            .map(translate)
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(app.view().monitoring);
        assert!(translated.contains(&K1Effect::SetSpeaker(true)));
        let translated: std::vec::Vec<_> = app
            .apply(Event::SetMonitor(false))
            .iter()
            .map(translate)
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!app.view().monitoring);
        assert!(translated.contains(&K1Effect::SetSpeaker(false)));
    }

    #[test]
    fn k1_configuration_recovery_can_resynchronize_controller_monitor_state() {
        use radio_platform::configuration_change::{
            ConfigurationAcknowledgement, ConfigurationCandidate,
        };

        let mut app = ReceiveApp::new();
        app.apply(Event::ReceiverFault);
        let candidate = ConfigurationCandidate::from_channel(2, 8, true).expect("candidate");
        let request = app.apply(Event::ConfigurationCandidate(candidate));
        let change = request
            .iter()
            .find_map(|effect| match effect {
                Effect::ConfigurationChange(change) => Some(change),
                _ => None,
            })
            .expect("configuration change");
        app.apply(Event::ConfigurationAcknowledged(
            ConfigurationAcknowledgement::applied(change),
        ));
        assert!(app.view().receiver_ok);
        // This is the same synchronization performed by the K1 UI boundary
        // after its banked controller reports an applied recovery.
        let effects = app.apply(Event::SetMonitor(true));
        assert!(app.view().monitoring);
        assert!(effects
            .iter()
            .any(|effect| effect == Effect::SetSpeaker(true)));
    }

    #[test]
    fn mismatched_channel_frequency_fails_before_hardware() {
        assert_eq!(
            translate(Effect::Tune {
                channel: 1,
                frequency_hz: 446_018_750,
                audio: true,
            }),
            Err(AdapterError)
        );
        assert_eq!(
            translate(Effect::Tune {
                channel: 0,
                frequency_hz: 446_006_250,
                audio: true,
            }),
            Err(AdapterError)
        );
    }

    #[test]
    fn audio_speaker_and_view_effects_are_lossless() {
        let mut app = ReceiveApp::new();
        app.apply(Event::Start);
        for effect in app.apply(Event::ToggleAudio).iter() {
            match (effect, translate(effect).unwrap()) {
                (Effect::SetSpeaker(value), K1Effect::SetSpeaker(adapted))
                | (Effect::SetChipAudio(value), K1Effect::SetChipAudio(adapted)) => {
                    assert_eq!(value, adapted);
                }
                (Effect::Redraw(value), K1Effect::Redraw(adapted)) => {
                    assert_eq!(value, adapted);
                }
                _ => panic!("adapter changed effect kind"),
            }
        }
    }

    #[test]
    fn only_complete_shared_pmr_setups_enter_the_common_path() {
        let setup = ChannelReceiveSetup {
            frequency: Frequency::from_hz(446_093_750).unwrap(),
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            tone: Tone::None,
            squelch: SquelchLevel::new(8).unwrap(),
            step: FrequencyStep::from_hz(12_500).unwrap(),
        };
        assert_eq!(shared_channel(setup), Some(8));
        assert_eq!(
            shared_channel(ChannelReceiveSetup {
                bandwidth: Bandwidth::Wide,
                ..setup
            }),
            None
        );
        assert_eq!(
            shared_channel(ChannelReceiveSetup {
                frequency: Frequency::from_hz(446_100_000).unwrap(),
                ..setup
            }),
            None
        );
    }
}
