//! K1 battery sense input on the PY32F071's analogue-to-digital converter.
//!
//! `EVID-K1-063` records the wiring and the conversion contract: the pack is
//! divided onto `PB0`, which is converter channel eight, and the pinned board
//! initialisation selects a twelve-bit right-aligned single conversion off the
//! peripheral clock. That is the whole of what this adapter does; the arithmetic
//! that turns a count into volts and percent lives in [`crate::battery`], where
//! it can be tested without a radio.
//!
//! Nothing here can transmit or reach the radio bus. It reads one pin.

use py32_hal::adc::{Adc, SampleTime};
use py32_hal::peripherals::{ADC1, PB0};
use py32_hal::Peripheral;

/// The K1's battery sense input.
pub struct BatterySense {
    adc: Adc<'static, ADC1>,
    input: PB0,
}

impl BatterySense {
    /// Claims the converter and the sense pin.
    ///
    /// The pinned board initialisation samples this channel over 41.5 converter
    /// cycles, which is the longest window it uses anywhere, because the divider
    /// behind the pin is high impedance. AFIK takes the nearest window the
    /// vendored driver offers rather than a shorter one.
    pub fn new(adc: impl Peripheral<P = ADC1> + 'static, input: PB0) -> Self {
        let mut adc = Adc::new(adc);
        adc.set_sample_time(SampleTime::CYCLES41_5);
        Self { adc, input }
    }

    /// Takes one conversion.
    ///
    /// A converter which does not answer is not distinguishable from one
    /// reading zero here, and both mean the same thing to the caller: the
    /// battery module refuses to report a reading it cannot justify.
    pub fn read(&mut self) -> u16 {
        self.adc.blocking_read(&mut self.input)
    }
}
