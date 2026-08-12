//! Receive-audio gate for the K5 V1 speaker path.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

/// PC4 speaker-path gate, initialized muted.
pub struct SpeakerGate {
    enabled: bool,
}
impl SpeakerGate {
    /// Configures PC4 as GPIO output and forces the audio path off.
    pub fn initialise() -> Self {
        portcon::select_gpio(Port::C, 4);
        gpio::set_output(Port::C, 4);
        gpio::write_pin(Port::C, 4, false);
        Self { enabled: false }
    }
    /// Enables or mutes only the board speaker path.
    pub fn set_enabled(&mut self, enabled: bool) {
        gpio::write_pin(Port::C, 4, enabled);
        self.enabled = enabled;
    }
    /// Reports the last commanded gate state.
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}
