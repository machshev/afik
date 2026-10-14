//! The K5's 8 KiB EEPROM as the application's memory view (`ADR-100`).
//!
//! The view is the EEPROM itself, address for address, so a backup taken
//! from AFIK is the same bytes a backup from the stock firmware would be.

use radio_app::drivers::Memory;
use radio_platform::battery::{Calibration, CALIBRATION_BYTES};
use radio_platform::memory_link::WRITE_BYTES;
use radio_platform::transmit::TxCalibration;

use crate::eeprom::{self, Bus};

/// EEPROM address of the six calibration half-words the stock firmware reads
/// (`EVID-K5-034`).
pub const BATTERY_CALIBRATION_ADDRESS: u16 = 0x1F40;
/// The low, mid and high bias rows for the 400–470 MHz band, three bytes
/// each: `0x1ED0 + band 5 * 16 + power * 3` (`radio.c:456-482`).
pub const TX_CALIBRATION_ADDRESS: u16 = 0x1F20;

const _: () = assert!(WRITE_BYTES == eeprom::WRITE_BLOCK_BYTES);

/// The EEPROM behind the memory view.
pub struct K5Memory<B> {
    bus: B,
}

impl<B: Bus> K5Memory<B> {
    /// Wraps the EEPROM bus.
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Returns the bus, for tests that inspect the device behind it.
    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }
}

impl<B: Bus> Memory for K5Memory<B> {
    fn read(&mut self, address: u16, output: &mut [u8]) -> bool {
        eeprom::read(&mut self.bus, address, output).is_ok()
    }

    /// Writes one block, skipping it when the EEPROM already holds it: a host
    /// rewriting a whole image then costs only the blocks that changed.
    fn write(&mut self, address: u16, data: &[u8; WRITE_BYTES]) -> bool {
        let mut current = [0_u8; WRITE_BYTES];
        if eeprom::read(&mut self.bus, address, &mut current).is_err() {
            return false;
        }
        current == *data || eeprom::write_verified(&mut self.bus, address, &current, data).is_ok()
    }

    fn read_battery_calibration(&mut self) -> Option<Calibration> {
        let mut block = [0_u8; CALIBRATION_BYTES];
        self.read(BATTERY_CALIBRATION_ADDRESS, &mut block)
            .then(|| Calibration::from_vendor_block(&block))
            .flatten()
    }

    fn read_tx_calibration(&mut self) -> Option<TxCalibration> {
        let mut bias = [0_u8; 9];
        self.read(TX_CALIBRATION_ADDRESS, &mut bias).then_some(bias)
    }
}
