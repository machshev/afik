//! K1 translation of shared receive-application effects.

use radio_channel_control::ChannelReceiveSetup;
use radio_domain::{Bandwidth, Frequency, FrequencyStep, Modulation, SquelchLevel, Tone};
use radio_platform::receive_app::{Effect, View};

/// One operation expressed in the K1 receiver/display adapter vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum K1Effect {
    /// Apply the existing complete K1 receive setup.
    Tune(ChannelReceiveSetup),
    /// Route or mute demodulated BK4829 audio.
    SetChipAudio(bool),
    /// Drive the K1 receive speaker amplifier.
    SetSpeaker(bool),
    /// Render the shared semantic view through the K1 display adapter.
    Redraw(View),
}

/// A shared effect was inconsistent with its validated PMR channel identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterError;

/// Translates one shared effect without touching K1 hardware.
pub fn translate(effect: Effect) -> Result<K1Effect, AdapterError> {
    match effect {
        Effect::Tune {
            channel,
            frequency_hz,
            audio: _,
        } => {
            let expected = 446_006_250_u32
                .checked_add(u32::from(channel.saturating_sub(1)) * 12_500)
                .ok_or(AdapterError)?;
            if !(1..=16).contains(&channel) || frequency_hz != expected {
                return Err(AdapterError);
            }
            Ok(K1Effect::Tune(ChannelReceiveSetup {
                frequency: Frequency::from_hz(frequency_hz).map_err(|_| AdapterError)?,
                modulation: Modulation::Fm,
                bandwidth: Bandwidth::Narrow,
                tone: Tone::None,
                squelch: SquelchLevel::CONSERVATIVE,
                step: FrequencyStep::from_hz(12_500).map_err(|_| AdapterError)?,
            }))
        }
        Effect::SetChipAudio(enabled) => Ok(K1Effect::SetChipAudio(enabled)),
        Effect::SetSpeaker(enabled) => Ok(K1Effect::SetSpeaker(enabled)),
        Effect::Redraw(view) => Ok(K1Effect::Redraw(view)),
    }
}

#[cfg(test)]
mod tests {
    use super::{translate, AdapterError, K1Effect};
    use radio_domain::{Bandwidth, Modulation, SquelchLevel, Tone};
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
        let K1Effect::Tune(setup) = translated[1] else {
            panic!("second effect must tune");
        };
        assert_eq!(setup.frequency.as_hz(), 446_006_250);
        assert_eq!(setup.modulation, Modulation::Fm);
        assert_eq!(setup.bandwidth, Bandwidth::Narrow);
        assert_eq!(setup.tone, Tone::None);
        assert_eq!(setup.squelch, SquelchLevel::CONSERVATIVE);
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
}
