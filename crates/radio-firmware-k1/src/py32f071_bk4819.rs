//! PY32F071 pin adapter for the K1 BK4819 three-wire register bus.
//!
//! The pin assignment comes from `EVID-K1-054`: chip select `PF9`, clock `PB8`,
//! and the shared bidirectional data line `PB9`. This module only binds those
//! pins and a busy-wait delay; the transfer order lives in
//! [`crate::bk4819_bus`] and is host tested there.

use py32_hal::gpio::{Flex, Level, Output, Pull, Speed};

use crate::bk4819_bus::{DataDirection, ThreeWirePins};

/// Core clock the busy-wait delay assumes, matching the validated handoff.
pub const ASSUMED_CORE_HZ: u32 = 48_000_000;
/// Core cycles per microsecond at [`ASSUMED_CORE_HZ`].
pub const CYCLES_PER_MICROSECOND: u32 = ASSUMED_CORE_HZ / 1_000_000;

/// The three-wire pins bound to their PY32F071 peripherals.
pub struct Bk4819Pins {
    chip_select: Output<'static>,
    clock: Output<'static>,
    data: Flex<'static>,
}

impl Bk4819Pins {
    /// Binds the pins with the bus idle: chip select released, clock and data
    /// high, and the data line driven by this device.
    pub fn new(chip_select: Output<'static>, clock: Output<'static>, data: Flex<'static>) -> Self {
        let mut pins = Self {
            chip_select,
            clock,
            data,
        };
        pins.data.set_as_output(Speed::High);
        pins.chip_select.set_level(Level::High);
        pins.clock.set_level(Level::High);
        pins.data.set_high();
        pins
    }
}

impl ThreeWirePins for Bk4819Pins {
    /// Driving a bound pin cannot fail, so no transfer can fail on this board.
    type Error = core::convert::Infallible;

    fn set_chip_select(&mut self, asserted: bool) -> Result<(), Self::Error> {
        // The chip select is active low.
        self.chip_select
            .set_level(if asserted { Level::Low } else { Level::High });
        Ok(())
    }

    fn set_clock(&mut self, high: bool) -> Result<(), Self::Error> {
        self.clock
            .set_level(if high { Level::High } else { Level::Low });
        Ok(())
    }

    fn set_data(&mut self, high: bool) -> Result<(), Self::Error> {
        if high {
            self.data.set_high();
        } else {
            self.data.set_low();
        }
        Ok(())
    }

    fn set_data_direction(&mut self, direction: DataDirection) -> Result<(), Self::Error> {
        match direction {
            DataDirection::Output => self.data.set_as_output(Speed::High),
            DataDirection::Input => self.data.set_as_input(Pull::None),
        }
        Ok(())
    }

    fn read_data(&mut self) -> Result<bool, Self::Error> {
        Ok(self.data.is_high())
    }

    fn delay_microseconds(&mut self, microseconds: u32) -> Result<(), Self::Error> {
        cortex_m::asm::delay(microseconds.saturating_mul(CYCLES_PER_MICROSECOND));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ASSUMED_CORE_HZ, CYCLES_PER_MICROSECOND};

    #[test]
    fn the_busy_wait_matches_the_validated_core_clock() {
        assert_eq!(ASSUMED_CORE_HZ, 48_000_000);
        assert_eq!(CYCLES_PER_MICROSECOND, 48);
    }
}
