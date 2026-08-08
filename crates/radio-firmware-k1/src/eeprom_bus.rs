//! K1 external-memory transfer framing over hardware SPI.
//!
//! The device and its pin assignment come from the pinned K1 reference firmware
//! recorded in `docs/hardware-evidence.md`: a PY25Q16 serial NOR memory on
//! `SPI2`, with `SCK` on `PA0`, `MOSI` on `PA1`, `MISO` on `PA2`, and an
//! active-low chip select on `PA3`. The vendored PY32 HAL generates exactly
//! those alternate functions for `SPI2`, so the bytes are shifted by the
//! peripheral rather than in software.
//!
//! What this module owns is the framing the memory requires and the peripheral
//! does not provide: one chip-select assertion spanning a command, its address,
//! its payload, and the response. A board adapter supplies the port, so that
//! framing is host testable without a radio.

use radio_eeprom::NorBus;

/// Port A pin carrying the peripheral clock.
pub const SCK_PIN: u8 = 0;
/// Port A pin carrying host-to-memory data.
pub const MOSI_PIN: u8 = 1;
/// Port A pin carrying memory-to-host data.
pub const MISO_PIN: u8 = 2;
/// Port A pin driving the active-low chip select.
pub const CS_PIN: u8 = 3;

/// Board primitives required to frame one external-memory transfer.
pub trait EepromPort {
    /// Adapter-specific failure. Any failure abandons the transfer.
    type Error;

    /// Drives the active-low chip select.
    fn select(&mut self, asserted: bool) -> Result<(), Self::Error>;

    /// Shifts one byte out through the peripheral and returns the byte in.
    fn transfer_byte(&mut self, value: u8) -> Result<u8, Self::Error>;

    /// Waits for at least the requested number of microseconds.
    fn delay_microseconds(&mut self, microseconds: u32);
}

/// An external-memory bus framed over one hardware SPI port.
pub struct SpiEepromBus<P: EepromPort> {
    port: P,
}

impl<P: EepromPort> SpiEepromBus<P> {
    /// Wraps a board port without performing any transfer.
    pub const fn new(port: P) -> Self {
        Self { port }
    }

    /// Returns the port, so a caller can release what it owns.
    pub fn release(self) -> P {
        self.port
    }
}

impl<P: EepromPort> NorBus for SpiEepromBus<P> {
    type Error = P::Error;

    fn transfer(
        &mut self,
        header: &[u8],
        payload: &[u8],
        response: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.port.select(true)?;

        let mut result = Ok(());
        for byte in header.iter().chain(payload.iter()) {
            if let Err(error) = self.port.transfer_byte(*byte) {
                result = Err(error);
                break;
            }
        }
        if result.is_ok() {
            for slot in response.iter_mut() {
                match self.port.transfer_byte(0) {
                    Ok(byte) => *slot = byte,
                    Err(error) => {
                        result = Err(error);
                        break;
                    }
                }
            }
        }

        // Chip select is released whatever happened, so a failed transfer
        // cannot leave the memory selected and the next one desynchronised.
        let released = self.port.select(false);
        result.and(released)
    }

    fn delay_us(&mut self, micros: u32) {
        self.port.delay_microseconds(micros);
    }
}

#[cfg(test)]
mod tests {
    use super::{EepromPort, SpiEepromBus};
    use radio_eeprom::NorBus;
    use std::vec::Vec;

    /// Records the framing and replays a fixed response.
    #[derive(Default)]
    struct RecordingPort {
        selected: bool,
        selections: Vec<bool>,
        sent: Vec<u8>,
        /// Bytes the memory presents, consumed in order.
        replies: Vec<u8>,
        sent_while_deselected: bool,
        failures: usize,
    }

    impl EepromPort for RecordingPort {
        type Error = ();

        fn select(&mut self, asserted: bool) -> Result<(), Self::Error> {
            self.selected = asserted;
            self.selections.push(asserted);
            Ok(())
        }

        fn transfer_byte(&mut self, value: u8) -> Result<u8, Self::Error> {
            // A byte shifted while the memory is not selected reaches nothing,
            // so this is what proves the framing spans the whole transfer.
            self.sent_while_deselected = self.sent_while_deselected || !self.selected;
            if self.failures > 0 {
                self.failures -= 1;
                return Err(());
            }
            self.sent.push(value);
            Ok(if self.replies.is_empty() {
                0
            } else {
                self.replies.remove(0)
            })
        }

        fn delay_microseconds(&mut self, _microseconds: u32) {}
    }

    #[test]
    fn one_selection_spans_the_header_payload_and_response() {
        let port = RecordingPort {
            // The memory drives nothing while the command byte is shifted in,
            // then presents its three identification bytes.
            replies: std::vec![0x00, 0x85, 0x60, 0x15],
            ..RecordingPort::default()
        };
        let mut bus = SpiEepromBus::new(port);

        let mut response = [0_u8; 3];
        bus.transfer(&[0x9F], &[], &mut response).expect("transfer");
        assert_eq!(response, [0x85, 0x60, 0x15]);

        let port = bus.release();
        assert_eq!(
            port.selections,
            std::vec![true, false],
            "one transfer asserts and releases chip select exactly once"
        );
        assert!(!port.sent_while_deselected);
        assert_eq!(
            port.sent,
            std::vec![0x9F, 0x00, 0x00, 0x00],
            "the response is clocked out with zeros after the command"
        );
    }

    #[test]
    fn the_header_is_shifted_before_the_payload() {
        let mut bus = SpiEepromBus::new(RecordingPort::default());
        bus.transfer(&[0x02, 0x10, 0x00, 0x00], &[0xAB, 0xCD], &mut [])
            .expect("transfer");
        let port = bus.release();
        assert_eq!(port.sent, std::vec![0x02, 0x10, 0x00, 0x00, 0xAB, 0xCD]);
    }

    #[test]
    fn a_failed_transfer_still_releases_chip_select() {
        let port = RecordingPort {
            failures: 1,
            ..RecordingPort::default()
        };
        let mut bus = SpiEepromBus::new(port);
        assert_eq!(bus.transfer(&[0x03], &[], &mut []), Err(()));
        let port = bus.release();
        assert!(
            !port.selected,
            "a memory left selected would desynchronise the next transfer"
        );
        assert_eq!(port.selections, std::vec![true, false]);
    }
}
