//! System clock selection and the frequency an image may derive timing from.
//!
//! Per `EVID-DP32-005` the part comes out of reset on RCHF at 24 MHz, but a
//! bootloader runs before an application does, so nothing about the state an
//! image inherits is known. [`configure`] therefore states the clock rather
//! than reading it: RCHF, enabled, at 48 MHz, driving the system clock
//! undivided.

use crate::mmio::Register;
use crate::{PMU_BASE, SYSCON_BASE};

/// `PMU_SRC_CFG`, the clock-source configuration register.
const SRC_CFG: Register = Register::new(PMU_BASE, 0x10);
/// `SYSCON_CLK_SEL`, the clock-selection register.
const CLK_SEL: Register = Register::new(SYSCON_BASE, 0x00);
/// `SYSCON_RC_FREQ_DELTA`, the measured RC deviation register.
const RC_FREQ_DELTA: Register = Register::new(SYSCON_BASE, 0x78);

/// `SRC_CFG` bit 0: RCHF enable.
const SRC_CFG_RCHF_EN: u32 = 1 << 0;
/// `SRC_CFG` bit 1: RCHF frequency select, set for 24 MHz and clear for 48 MHz.
const SRC_CFG_RCHF_FSEL_24MHZ: u32 = 1 << 1;
/// `CLK_SEL` bit 0: system clock select, clear for RCHF.
const CLK_SEL_SYS_CLK_DIVIDED: u32 = 1 << 0;

/// `RC_FREQ_DELTA` bit 31: the RCHF deviation is positive when set.
const RC_FREQ_DELTA_RCHF_POSITIVE: u32 = 1 << 31;
/// `RC_FREQ_DELTA` bits 30:11: the RCHF deviation magnitude in hertz.
const RC_FREQ_DELTA_RCHF_MASK: u32 = 0x7FFF_F800;
/// Bit position of the RCHF deviation magnitude.
const RC_FREQ_DELTA_RCHF_SHIFT: u32 = 11;

/// Nominal RCHF frequency in hertz once 48 MHz is selected.
pub const NOMINAL_RCHF_HZ: u32 = 48_000_000;

/// The system clock an image configured with [`configure`] runs at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemClock {
    hertz: u32,
}

impl SystemClock {
    /// Returns the frequency in hertz, corrected by the part's measured
    /// deviation from nominal.
    pub const fn hertz(self) -> u32 {
        self.hertz
    }
}

/// Selects RCHF at 48 MHz as the undivided system clock and reports it.
///
/// The system clock is pointed at RCHF before the RCHF frequency changes, so
/// the part is never running from a source while that source is reconfigured
/// behind it. Neither `SRC_CLK_SEL` nor `DIV_CLK_SEL` is touched, which is what
/// makes the manual's divided-clock switching procedure inapplicable here.
pub fn configure() -> SystemClock {
    CLK_SEL.modify(|value| value & !CLK_SEL_SYS_CLK_DIVIDED);
    SRC_CFG.modify(|value| (value & !SRC_CFG_RCHF_FSEL_24MHZ) | SRC_CFG_RCHF_EN);
    SystemClock {
        hertz: corrected_rchf_hz(RC_FREQ_DELTA.read()),
    }
}

/// Applies a `RC_FREQ_DELTA` reading to the nominal 48 MHz.
///
/// The deviation is in hertz with a separate sign bit. A part whose deviation
/// register has never been written reads zero, which yields the nominal
/// frequency, so an uncalibrated part degrades to the number an image would
/// have assumed anyway.
pub const fn corrected_rchf_hz(delta_register: u32) -> u32 {
    let magnitude = (delta_register & RC_FREQ_DELTA_RCHF_MASK) >> RC_FREQ_DELTA_RCHF_SHIFT;
    if delta_register & RC_FREQ_DELTA_RCHF_POSITIVE == 0 {
        NOMINAL_RCHF_HZ.saturating_sub(magnitude)
    } else {
        NOMINAL_RCHF_HZ.saturating_add(magnitude)
    }
}

#[cfg(test)]
mod tests {
    use super::{corrected_rchf_hz, NOMINAL_RCHF_HZ};

    #[test]
    fn an_unwritten_deviation_register_yields_the_nominal_frequency() {
        assert_eq!(corrected_rchf_hz(0), NOMINAL_RCHF_HZ);
    }

    #[test]
    fn a_positive_deviation_is_added() {
        let delta = (1 << 31) | (12_345 << 11);
        assert_eq!(corrected_rchf_hz(delta), NOMINAL_RCHF_HZ + 12_345);
    }

    #[test]
    fn a_negative_deviation_is_subtracted() {
        let delta = 12_345 << 11;
        assert_eq!(corrected_rchf_hz(delta), NOMINAL_RCHF_HZ - 12_345);
    }

    #[test]
    fn the_low_field_of_the_register_describes_rclf_and_is_ignored() {
        assert_eq!(corrected_rchf_hz(0x0000_07FF), NOMINAL_RCHF_HZ);
    }

    #[test]
    fn the_widest_deviation_the_field_can_hold_stays_near_nominal() {
        // The magnitude field is twenty bits, so it cannot express more than
        // about one megahertz and can never drive the frequency to zero.
        assert_eq!(corrected_rchf_hz(0x7FFF_F800), NOMINAL_RCHF_HZ - 1_048_575);
        assert_eq!(corrected_rchf_hz(0xFFFF_F800), NOMINAL_RCHF_HZ + 1_048_575);
    }
}
