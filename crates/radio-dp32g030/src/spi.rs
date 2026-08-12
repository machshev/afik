//! CPU-driven SPI master support, per `EVID-DP32-010`.

use crate::mmio::Register;

const CONTROL_SPE: u32 = 1 << 3;
const CONTROL_CPHA: u32 = 1 << 4;
const CONTROL_CPOL: u32 = 1 << 5;
const CONTROL_MSTR: u32 = 1 << 6;
const CONTROL_MSR_SSN: u32 = 1 << 12;
const CONTROL_RF_CLR: u32 = 1 << 15;
const CONTROL_TF_CLR: u32 = 1 << 16;
const FIFO_TRANSMIT_FULL: u32 = 1 << 4;
const FIFO_TRANSMIT_EMPTY: u32 = 1 << 3;

/// One SPI controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Spi {
    base: u32,
}

impl Spi {
    /// Names the controller at `base`.
    pub const fn new(base: u32) -> Self {
        Self { base }
    }

    const fn control(self) -> Register {
        Register::new(self.base, 0x00)
    }

    const fn write_data(self) -> Register {
        Register::new(self.base, 0x04)
    }

    const fn interrupt_enable(self) -> Register {
        Register::new(self.base, 0x10)
    }

    const fn fifo_status(self) -> Register {
        Register::new(self.base, 0x18)
    }

    /// Configures an enabled, MSB-first master at pclk/16, CPOL=1, CPHA=1.
    pub fn configure_display(self) {
        self.control().write(0);
        self.interrupt_enable().write(0);
        self.control()
            .write(display_control(false) | CONTROL_RF_CLR | CONTROL_TF_CLR);
        self.control().write(display_control(false));
    }

    /// Selects or releases the hardware SSN output.
    pub fn select(self, selected: bool) {
        self.control().modify(|value| {
            if selected {
                value & !CONTROL_MSR_SSN
            } else {
                value | CONTROL_MSR_SSN
            }
        });
    }

    /// Queues one byte after waiting for transmit FIFO room.
    pub fn write_byte(self, byte: u8) {
        while self.fifo_status().read() & FIFO_TRANSMIT_FULL != 0 {
            core::hint::spin_loop();
        }
        self.write_data().write(u32::from(byte));
    }

    /// Waits until every queued byte has left the transmit FIFO.
    pub fn flush(self) {
        while self.fifo_status().read() & FIFO_TRANSMIT_EMPTY == 0 {
            core::hint::spin_loop();
        }
    }
}

/// Produces the complete display-mode control word.
pub const fn display_control(selected: bool) -> u32 {
    let divider_pclk_16 = 0b010;
    CONTROL_SPE
        | CONTROL_CPHA
        | CONTROL_CPOL
        | CONTROL_MSTR
        | if selected { 0 } else { CONTROL_MSR_SSN }
        | divider_pclk_16
}

#[cfg(test)]
mod tests {
    use super::{display_control, Spi};

    #[test]
    fn display_mode_is_master_msb_first_mode_three_at_pclk_divided_by_sixteen() {
        assert_eq!(display_control(false), 0x107A);
        assert_eq!(display_control(true), 0x007A);
    }

    #[test]
    fn register_addresses_match_the_manual() {
        let spi = Spi::new(0x400B_8000);
        assert_eq!(spi.control().address(), 0x400B_8000);
        assert_eq!(spi.write_data().address(), 0x400B_8004);
        assert_eq!(spi.fifo_status().address(), 0x400B_8018);
    }
}
