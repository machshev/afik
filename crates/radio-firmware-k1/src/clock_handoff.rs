//! Fail-closed contract for the K1 bootloader-provided clock state.

/// Clock frequency required by the existing K1 peripheral witnesses.
pub const K1_INHERITED_CLOCK_HZ: u32 = 48_000_000;

/// Enable/readiness state reported for one clock source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockSourceState {
    /// Source is disabled and not ready.
    Disabled,
    /// Source is enabled but not ready.
    Starting,
    /// Source is enabled and ready.
    Ready,
    /// Hardware reports ready while the source is disabled.
    Inconsistent,
}

impl ClockSourceState {
    /// Decodes separate RCC enable and ready flags without discarding an
    /// inconsistent combination.
    #[must_use]
    pub const fn from_flags(enabled: bool, ready: bool) -> Self {
        match (enabled, ready) {
            (false, false) => Self::Disabled,
            (true, false) => Self::Starting,
            (true, true) => Self::Ready,
            (false, true) => Self::Inconsistent,
        }
    }
}

/// Read-only RCC fields needed to establish the inherited clock contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSnapshot {
    /// HSI oscillator state.
    pub hsi: ClockSourceState,
    /// HSI frequency-selection encoding.
    pub hsi_frequency: u8,
    /// PLL state.
    pub pll: ClockSourceState,
    /// PLL source encoding.
    pub pll_source: u8,
    /// PLL multiplier encoding.
    pub pll_multiplier: u8,
    /// Requested system-clock source encoding.
    pub system_source: u8,
    /// Active system-clock source encoding.
    pub active_system_source: u8,
    /// AHB prescaler encoding.
    pub ahb_prescaler: u8,
    /// APB prescaler encoding.
    pub apb_prescaler: u8,
}

/// Decodes the relevant PY32F071 RCC fields from a read-only register sample.
#[must_use]
pub const fn snapshot_from_registers(
    cr: u32,
    icscr: u32,
    cfgr: u32,
    pllcfgr: u32,
) -> ClockSnapshot {
    ClockSnapshot {
        hsi: ClockSourceState::from_flags(cr & (1 << 8) != 0, cr & (1 << 10) != 0),
        hsi_frequency: ((icscr >> 13) & 0x07) as u8,
        pll: ClockSourceState::from_flags(cr & (1 << 24) != 0, cr & (1 << 25) != 0),
        pll_source: (pllcfgr & 0x03) as u8,
        pll_multiplier: ((pllcfgr >> 2) & 0x03) as u8,
        system_source: (cfgr & 0x07) as u8,
        active_system_source: ((cfgr >> 3) & 0x07) as u8,
        ahb_prescaler: ((cfgr >> 8) & 0x0f) as u8,
        apb_prescaler: ((cfgr >> 12) & 0x07) as u8,
    }
}

/// A validated inherited clock tree. Construction is fail-closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InheritedClocks {
    /// System clock frequency in hertz.
    sys: u32,
    /// AHB clock frequency in hertz.
    hclk1: u32,
    /// APB clock frequency in hertz.
    pclk1: u32,
    /// APB timer clock frequency in hertz.
    pclk1_tim: u32,
}

impl InheritedClocks {
    /// Returns the validated system clock frequency.
    #[must_use]
    pub const fn sys_hz(self) -> u32 {
        self.sys
    }

    /// Returns the validated AHB clock frequency.
    #[must_use]
    pub const fn hclk1_hz(self) -> u32 {
        self.hclk1
    }

    /// Returns the validated APB clock frequency.
    #[must_use]
    pub const fn pclk1_hz(self) -> u32 {
        self.pclk1
    }

    /// Returns the validated APB timer clock frequency.
    #[must_use]
    pub const fn pclk1_tim_hz(self) -> u32 {
        self.pclk1_tim
    }
}

/// Why an inherited RCC snapshot cannot be adopted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockHandoffError {
    /// HSI is disabled.
    HsiOff,
    /// HSI is not ready.
    HsiNotReady,
    /// HSI is not configured for the required 24 MHz PLL input.
    HsiFrequency,
    /// PLL is disabled.
    PllOff,
    /// PLL is not ready.
    PllNotReady,
    /// PLL does not use HSI.
    PllSource,
    /// PLL does not multiply the 16 MHz HSI by three.
    PllMultiplier,
    /// Requested system clock is not PLL.
    RequestedSystemSource,
    /// Active system clock is not PLL.
    ActiveSystemSource,
    /// AHB is prescaled.
    AhbPrescaler,
    /// APB is prescaled.
    ApbPrescaler,
}

/// Validates the exact 16 MHz HSI, x3 PLL, undivided-bus contract.
pub const fn validate(snapshot: ClockSnapshot) -> Result<InheritedClocks, ClockHandoffError> {
    match snapshot.hsi {
        ClockSourceState::Ready => {}
        ClockSourceState::Starting => return Err(ClockHandoffError::HsiNotReady),
        ClockSourceState::Disabled | ClockSourceState::Inconsistent => {
            return Err(ClockHandoffError::HsiOff);
        }
    }
    if snapshot.hsi_frequency != 2 {
        return Err(ClockHandoffError::HsiFrequency);
    }
    match snapshot.pll {
        ClockSourceState::Ready => {}
        ClockSourceState::Starting => return Err(ClockHandoffError::PllNotReady),
        ClockSourceState::Disabled | ClockSourceState::Inconsistent => {
            return Err(ClockHandoffError::PllOff);
        }
    }
    if snapshot.pll_source != 2 {
        return Err(ClockHandoffError::PllSource);
    }
    if snapshot.pll_multiplier != 1 {
        return Err(ClockHandoffError::PllMultiplier);
    }
    if snapshot.system_source != 2 {
        return Err(ClockHandoffError::RequestedSystemSource);
    }
    if snapshot.active_system_source != 2 {
        return Err(ClockHandoffError::ActiveSystemSource);
    }
    if snapshot.ahb_prescaler != 0 {
        return Err(ClockHandoffError::AhbPrescaler);
    }
    if snapshot.apb_prescaler != 0 {
        return Err(ClockHandoffError::ApbPrescaler);
    }

    Ok(InheritedClocks {
        sys: K1_INHERITED_CLOCK_HZ,
        hclk1: K1_INHERITED_CLOCK_HZ,
        pclk1: K1_INHERITED_CLOCK_HZ,
        pclk1_tim: K1_INHERITED_CLOCK_HZ,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        snapshot_from_registers, validate, ClockHandoffError, ClockSnapshot, ClockSourceState,
        K1_INHERITED_CLOCK_HZ,
    };

    const VALID: ClockSnapshot = ClockSnapshot {
        hsi: ClockSourceState::Ready,
        hsi_frequency: 2,
        pll: ClockSourceState::Ready,
        pll_source: 2,
        pll_multiplier: 1,
        system_source: 2,
        active_system_source: 2,
        ahb_prescaler: 0,
        apb_prescaler: 0,
    };

    #[test]
    fn accepts_only_the_undivided_48mhz_contract() {
        let clocks = validate(VALID).unwrap();
        assert_eq!(clocks.sys_hz(), K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.hclk1_hz(), K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.pclk1_hz(), K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.pclk1_tim_hz(), K1_INHERITED_CLOCK_HZ);
    }

    #[test]
    fn exact_unit_snapshot_satisfies_the_corrected_contract() {
        let snapshot = snapshot_from_registers(0x0300_0500, 0x00e6_4d14, 0x0000_0012, 0x0000_0006);
        assert_eq!(snapshot, VALID);
        assert!(validate(snapshot).is_ok());
    }

    #[test]
    fn raw_register_decoder_preserves_every_contract_field() {
        let cr = (1 << 8) | (1 << 10) | (1 << 24) | (1 << 25);
        let icscr = 2 << 13;
        let cfgr = 2 | (2 << 3);
        let pllcfgr = 2 | (1 << 2);
        assert_eq!(snapshot_from_registers(cr, icscr, cfgr, pllcfgr), VALID);

        let varied = snapshot_from_registers(1 << 10, 1 << 13, 4 << 12, 3 | (2 << 2));
        assert_eq!(varied.hsi, ClockSourceState::Inconsistent);
        assert_eq!(varied.hsi_frequency, 1);
        assert_eq!(varied.pll, ClockSourceState::Disabled);
        assert_eq!(varied.pll_source, 3);
        assert_eq!(varied.pll_multiplier, 2);
        assert_eq!(varied.apb_prescaler, 4);
    }

    #[test]
    fn rejects_every_mismatched_field() {
        let cases = [
            (
                ClockSnapshot {
                    hsi: ClockSourceState::Disabled,
                    ..VALID
                },
                ClockHandoffError::HsiOff,
            ),
            (
                ClockSnapshot {
                    hsi: ClockSourceState::Starting,
                    ..VALID
                },
                ClockHandoffError::HsiNotReady,
            ),
            (
                ClockSnapshot {
                    hsi_frequency: 1,
                    ..VALID
                },
                ClockHandoffError::HsiFrequency,
            ),
            (
                ClockSnapshot {
                    pll: ClockSourceState::Disabled,
                    ..VALID
                },
                ClockHandoffError::PllOff,
            ),
            (
                ClockSnapshot {
                    pll: ClockSourceState::Starting,
                    ..VALID
                },
                ClockHandoffError::PllNotReady,
            ),
            (
                ClockSnapshot {
                    pll_source: 3,
                    ..VALID
                },
                ClockHandoffError::PllSource,
            ),
            (
                ClockSnapshot {
                    pll_multiplier: 0,
                    ..VALID
                },
                ClockHandoffError::PllMultiplier,
            ),
            (
                ClockSnapshot {
                    system_source: 0,
                    ..VALID
                },
                ClockHandoffError::RequestedSystemSource,
            ),
            (
                ClockSnapshot {
                    active_system_source: 0,
                    ..VALID
                },
                ClockHandoffError::ActiveSystemSource,
            ),
            (
                ClockSnapshot {
                    ahb_prescaler: 8,
                    ..VALID
                },
                ClockHandoffError::AhbPrescaler,
            ),
            (
                ClockSnapshot {
                    apb_prescaler: 4,
                    ..VALID
                },
                ClockHandoffError::ApbPrescaler,
            ),
        ];

        for (snapshot, expected) in cases {
            assert_eq!(validate(snapshot), Err(expected));
        }
    }
}
