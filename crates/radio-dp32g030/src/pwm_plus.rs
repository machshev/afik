//! Bounded `PWM_PLUS` support for the V1 display backlight.

use crate::mmio::Register;

const CFG_COUNTER_ENABLE: u32 = 1 << 0;
const CFG_REPEAT: u32 = 1 << 2;
const GEN_CH0_OUTPUT_INVERT: u32 = 1 << 16;
const GEN_CH0_OUTPUT_ENABLE: u32 = 1 << 24;

/// One `PWM_PLUS` controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PwmPlus {
    base: u32,
}

impl PwmPlus {
    /// Names the controller at `base`.
    pub const fn new(base: u32) -> Self {
        Self { base }
    }

    const fn configuration(self) -> Register {
        Register::new(self.base, 0x00)
    }

    const fn generation(self) -> Register {
        Register::new(self.base, 0x04)
    }

    const fn clock_source(self) -> Register {
        Register::new(self.base, 0x08)
    }

    const fn period(self) -> Register {
        Register::new(self.base, 0x1C)
    }

    const fn channel_zero_compare(self) -> Register {
        Register::new(self.base, 0x20)
    }

    /// Drives channel zero at the fixed full diagnostic brightness.
    pub fn enable_diagnostic_backlight(self) {
        self.configuration().write(0);
        self.clock_source().write(46 << 16);
        self.period().write(1_023);
        self.channel_zero_compare().write(1_023);
        self.generation()
            .write(GEN_CH0_OUTPUT_ENABLE | GEN_CH0_OUTPUT_INVERT);
        self.configuration().write(CFG_REPEAT | CFG_COUNTER_ENABLE);
    }
}

#[cfg(test)]
mod tests {
    use super::PwmPlus;

    #[test]
    fn register_addresses_match_the_manual() {
        let pwm = PwmPlus::new(0x400B_4000);
        assert_eq!(pwm.configuration().address(), 0x400B_4000);
        assert_eq!(pwm.generation().address(), 0x400B_4004);
        assert_eq!(pwm.clock_source().address(), 0x400B_4008);
        assert_eq!(pwm.period().address(), 0x400B_401C);
        assert_eq!(pwm.channel_zero_compare().address(), 0x400B_4020);
    }
}
