//! What the application needs from a radio's hardware.
//!
//! Each target implements these over its own pins and peripherals and hands
//! them to [`crate::interface::run`] and [`crate::serial::run`]. The radio chip
//! itself needs no trait here: the application drives any
//! [`radio_bk4819::Bk4819`] whose register bus the target provides.

use radio_platform::battery::Calibration;
use radio_platform::operator_display::FRAME_BYTES;
use radio_platform::transmit::Pmr446Target;

use crate::configuration::RETAINED_IMAGE_BYTES;
use crate::keys::Sample;
use crate::operator_state::OperatorState;

/// The 128×64 operator display.
#[allow(async_fn_in_trait)]
pub trait Display {
    /// Brings the controller out of reset and switches it on.
    async fn initialise(&mut self) -> bool;
    /// Writes one whole frame.
    async fn frame(&mut self, frame: &[u8; FRAME_BYTES]) -> bool;
}

/// The keypad and the push-to-talk input.
#[allow(async_fn_in_trait)]
pub trait Keypad {
    /// Scans every key once and reports the single key held, if any.
    async fn scan(&mut self) -> Sample;
    /// Reads the raw push-to-talk level, `true` while pressed.
    fn ptt_pressed(&mut self) -> bool;
}

/// The receive audio amplifier.
pub trait Speaker {
    /// Takes the amplifier, switched off. Until then [`Self::set_enabled`]
    /// leaves it alone.
    fn claim(&mut self);
    /// Switches a claimed amplifier on or off.
    fn set_enabled(&mut self, enabled: bool);
}

/// The battery voltage input.
pub trait BatterySense {
    /// Takes one raw conversion, if the converter produced one.
    fn read(&mut self) -> Option<u16>;
}

/// The serial link to the programming cable.
#[allow(async_fn_in_trait)]
pub trait SerialLink {
    /// Waits for a burst of bytes and returns how many arrived before the line
    /// went idle.
    async fn read_until_idle(&mut self, buffer: &mut [u8]) -> Result<usize, LinkError>;
    /// Sends every byte.
    async fn write(&mut self, bytes: &[u8]);
}

/// The serial link received bytes it could not frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkError;

/// Non-volatile memory holding the configuration, the operator's place, and
/// the vendor calibration the application reads.
#[allow(async_fn_in_trait)]
pub trait RetainedMemory {
    /// Reads the battery calibration from the vendor block.
    fn read_battery_calibration(&mut self) -> Option<Calibration>;
    /// Reads the PMR446 low-power TX bias bytes from the vendor block.
    fn read_tx_calibration(&mut self) -> Option<[u8; 3]>;
    /// Reads the retained configuration image and returns its length.
    fn read(&mut self, image: &mut [u8; RETAINED_IMAGE_BYTES]) -> Option<usize>;
    /// Replaces the retained configuration image.
    async fn write(&mut self, image: &[u8; RETAINED_IMAGE_BYTES], length: usize) -> bool;
    /// Reads where the operator left the radio.
    fn read_operator_state(&mut self) -> Option<OperatorState>;
    /// Records where the operator left the radio.
    async fn write_operator_state(&mut self, state: OperatorState) -> bool;
}

/// What differs between targets beyond their drivers.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Identity shown on the information screen.
    pub identity: &'static [u8],
    /// PMR446 TX constants for this radio.
    pub tx: Pmr446Target,
}
