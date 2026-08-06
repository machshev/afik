//! Hardware-independent register plan for the bounded K1 backlight witness.

/// Register masks and values needed to hold the evidenced PF8 backlight high.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConstantBacklightPlan {
    /// GPIO peripheral clock bit to set.
    pub clock_enable: u32,
    /// Two-bit PF8 mode field to clear.
    pub mode_clear: u32,
    /// Output-mode value to set for PF8.
    pub mode_set: u32,
    /// PF8 output-type bit to clear for push-pull.
    pub output_type_clear: u32,
    /// Two-bit PF8 speed field to clear.
    pub speed_clear: u32,
    /// High-speed value used by the pinned board observation.
    pub speed_set: u32,
    /// Two-bit PF8 pull field to clear.
    pub pull_clear: u32,
    /// Pull-up value used by the pinned board observation.
    pub pull_set: u32,
    /// BSRR bit which drives the active-high backlight on.
    pub output_high: u32,
}

/// Returns the exact GPIOF/PF8-only constant illumination plan.
#[must_use]
pub const fn constant_on_plan() -> ConstantBacklightPlan {
    ConstantBacklightPlan {
        clock_enable: 1 << 5,
        mode_clear: 0b11 << 16,
        mode_set: 0b01 << 16,
        output_type_clear: 1 << 8,
        speed_clear: 0b11 << 16,
        speed_set: 0b11 << 16,
        pull_clear: 0b11 << 16,
        pull_set: 0b01 << 16,
        output_high: 1 << 8,
    }
}

#[cfg(test)]
mod tests {
    use super::constant_on_plan;

    #[test]
    fn constant_plan_touches_only_gpiof_clock_and_pf8() {
        let plan = constant_on_plan();
        assert_eq!(plan.clock_enable, 0x20);
        assert_eq!(plan.mode_clear, 0x0003_0000);
        assert_eq!(plan.mode_set, 0x0001_0000);
        assert_eq!(plan.output_type_clear, 0x0000_0100);
        assert_eq!(plan.speed_clear, 0x0003_0000);
        assert_eq!(plan.speed_set, 0x0003_0000);
        assert_eq!(plan.pull_clear, 0x0003_0000);
        assert_eq!(plan.pull_set, 0x0001_0000);
        assert_eq!(plan.output_high, 0x0000_0100);
    }
}
