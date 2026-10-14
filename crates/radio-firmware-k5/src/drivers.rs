//! The K5's keypad, speaker and battery behind the application's traits.

use radio_app::drivers::{BatterySense, Keypad, Speaker};
use radio_app::keys::Sample;
use radio_dp32g030::saradc;

use crate::audio::SpeakerGate;
use crate::keypad::{self, K5Matrix, MatrixIo};

/// The keypad matrix, the side keys and PTT.
pub struct K5Keypad;

impl K5Keypad {
    /// Configures the keypad pins.
    pub fn initialise() -> Self {
        let _ = K5Matrix::initialise();
        Self
    }
}

impl Keypad for K5Keypad {
    async fn scan(&mut self) -> Sample {
        // The EEPROM shares two of the row pins (`EVID-K5-025`), and the serial
        // task may have used them since the last scan, so every scan takes the
        // matrix back first.
        match keypad::scan(&mut K5Matrix::initialise()) {
            Some(key) => Sample::Key(key.shared()),
            None => Sample::Released,
        }
    }

    fn ptt_pressed(&mut self) -> bool {
        K5Matrix.ptt_pressed()
    }
}

impl Speaker for SpeakerGate {
    /// The amplifier pin is configured, and held off, at boot.
    fn claim(&mut self) {}

    fn set_enabled(&mut self, enabled: bool) {
        Self::set_enabled(self, enabled);
    }
}

/// The battery divider on SARADC channel 4 (`EVID-K5-034`).
pub struct K5Battery;

impl BatterySense for K5Battery {
    fn read(&mut self) -> Option<u16> {
        saradc::read_voltage()
    }
}
