//! PY32F071 pin adapter for the K1 BK1080 two-wire bus on `PF5`/`PF6`.
//!
//! The edge order lives in [`crate::bk1080_bus`] and is host tested there.

use py32_hal::gpio::{Flex, Level, Output, Pull, Speed};

use crate::bk1080_bus::TwoWirePins;

/// Core cycles per microsecond at the validated 48 MHz handoff.
const CYCLES_PER_MICROSECOND: u32 = 48;

/// The BK1080 clock and data pins.
pub struct Bk1080Pins {
    clock: Output<'static>,
    data: Flex<'static>,
}

impl Bk1080Pins {
    /// Binds the pins with both lines driven high, the bus idle.
    pub fn new(clock: Output<'static>, data: Flex<'static>) -> Self {
        let mut pins = Self { clock, data };
        pins.clock.set_level(Level::High);
        pins.data.set_high();
        pins.data.set_as_output(Speed::VeryHigh);
        pins
    }
}

impl TwoWirePins for Bk1080Pins {
    fn set_clock(&mut self, high: bool) {
        self.clock
            .set_level(if high { Level::High } else { Level::Low });
    }

    fn drive_data(&mut self, high: bool) {
        if high {
            self.data.set_high();
        } else {
            self.data.set_low();
        }
        self.data.set_as_output(Speed::VeryHigh);
    }

    fn release_data(&mut self) {
        self.data.set_as_input(Pull::Up);
    }

    fn read_data(&mut self) -> bool {
        self.data.is_high()
    }

    fn tick(&mut self) {
        cortex_m::asm::delay(CYCLES_PER_MICROSECOND);
    }
}
