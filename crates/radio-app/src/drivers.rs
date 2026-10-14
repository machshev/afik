//! What the application needs from a radio's hardware.
//!
//! Each target implements these over its own pins and peripherals and hands
//! them to [`crate::interface::run`] and [`crate::serial::run`]. The radio chip
//! itself needs no trait here: the application drives any
//! [`radio_bk4819::Bk4819`] whose register bus the target provides.

use radio_platform::battery::Calibration;
use radio_platform::operator_display::FRAME_BYTES;
use radio_platform::transmit::Pmr446Target;

use radio_platform::memory_link::{PLACE_ORIGIN, WRITE_BYTES};

use crate::keys::Sample;
use crate::operator_state::{OperatorState, OPERATOR_STATE_BYTES};

/// The 128×64 operator display.
#[allow(async_fn_in_trait)]
pub trait Display {
    /// Brings the controller out of reset and switches it on.
    async fn initialise(&mut self) -> bool;
    /// Writes one whole frame.
    async fn frame(&mut self, frame: &[u8; FRAME_BYTES]) -> bool;
    /// Switches the backlight on or off. [`Self::initialise`] leaves it on.
    fn set_backlight(&mut self, on: bool);
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

/// The radio's 8 KiB memory view (`radio_platform::memory_link`).
///
/// The configuration image, the operator's place, and the vendor calibration
/// all live here at the addresses the memory map gives. A target maps the view
/// onto whatever memory it has.
///
/// Both tasks use it: the interface reads channels on demand and the serial
/// task answers the host. Every operation is synchronous, so neither task can
/// hold it across an `await`; a write waits for the memory to finish.
pub trait Memory {
    /// Reads `output.len()` bytes from `address`.
    fn read(&mut self, address: u16, output: &mut [u8]) -> bool;
    /// Stores one block at an aligned `address` and reports whether it took.
    fn write(&mut self, address: u16, data: &[u8; WRITE_BYTES]) -> bool;
    /// Reads the battery calibration from the vendor block.
    fn read_battery_calibration(&mut self) -> Option<Calibration>;
    /// Reads the PMR446 low-power TX bias bytes from the vendor block.
    fn read_tx_calibration(&mut self) -> Option<[u8; 3]>;

    /// Reads where the operator left the radio.
    fn read_operator_state(&mut self) -> Option<OperatorState> {
        let mut bytes = [0_u8; OPERATOR_STATE_BYTES];
        if !self.read(PLACE_ORIGIN, &mut bytes) {
            return None;
        }
        OperatorState::decode(&bytes)
    }

    /// Records where the operator left the radio.
    fn write_operator_state(&mut self, state: OperatorState) -> bool {
        let bytes = state.encode();
        let mut stored = true;
        for (index, block) in bytes.chunks_exact(WRITE_BYTES).enumerate() {
            let mut data = [0_u8; WRITE_BYTES];
            data.copy_from_slice(block);
            let offset = u16::try_from(index * WRITE_BYTES).unwrap_or(0);
            stored &= self.write(PLACE_ORIGIN + offset, &data);
        }
        stored
    }
}

/// What differs between targets beyond their drivers.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Identity shown on the information screen.
    pub identity: &'static [u8],
    /// PMR446 TX constants for this radio.
    pub tx: Pmr446Target,
}
