//! The K5 V1's BK1080 broadcast receiver (`EVID-BCFM-066-001`).
//!
//! It shares the EEPROM's PA10/PA11 two-wire bus, so every transaction claims
//! the pins back from the keypad as an EEPROM one does. PB15 is its enable
//! line, low to run the chip; the reference firmware drives it high at boot.

use radio_bk1080::TwoWire;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

use crate::eeprom::{Bus, K5Bus};

/// Port B pin enabling the BK1080, active low.
pub const ENABLE_PIN: u8 = 15;

/// The BK1080's view of the shared bus and its enable line.
pub struct K5Broadcast(K5Bus);

impl K5Broadcast {
    /// Drives the enable line high, the chip off, and takes the bus pins.
    pub fn initialise() -> Self {
        portcon::select_gpio(Port::B, ENABLE_PIN);
        gpio::write_pin(Port::B, ENABLE_PIN, true);
        gpio::set_output(Port::B, ENABLE_PIN);
        Self(K5Bus::initialise())
    }
}

impl TwoWire for K5Broadcast {
    fn start(&mut self) {
        self.0.start();
    }
    fn stop(&mut self) {
        self.0.stop();
    }
    fn write(&mut self, byte: u8) -> bool {
        self.0.write(byte)
    }
    fn read(&mut self, final_byte: bool) -> u8 {
        self.0.read(final_byte)
    }
    fn claim(&mut self) {
        self.0.claim_conservative();
    }
    fn set_enabled(&mut self, enabled: bool) {
        gpio::write_pin(Port::B, ENABLE_PIN, !enabled);
    }
}
