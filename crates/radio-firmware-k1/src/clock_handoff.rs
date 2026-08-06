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
    /// Requested system-clock source encoding.
    pub system_source: u8,
    /// Active system-clock source encoding.
    pub active_system_source: u8,
    /// AHB prescaler encoding.
    pub ahb_prescaler: u8,
    /// APB prescaler encoding.
    pub apb_prescaler: u8,
}

/// A validated inherited clock tree. Construction is fail-closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InheritedClocks {
    /// System clock frequency in hertz.
    pub sys_hz: u32,
    /// AHB clock frequency in hertz.
    pub hclk1_hz: u32,
    /// APB clock frequency in hertz.
    pub pclk1_hz: u32,
    /// APB timer clock frequency in hertz.
    pub pclk1_tim_hz: u32,
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
    /// Requested system clock is not PLL.
    RequestedSystemSource,
    /// Active system clock is not PLL.
    ActiveSystemSource,
    /// AHB is prescaled.
    AhbPrescaler,
    /// APB is prescaled.
    ApbPrescaler,
}

/// Validates the exact 24 MHz HSI, fixed x2 PLL, undivided-bus contract.
pub const fn validate(snapshot: ClockSnapshot) -> Result<InheritedClocks, ClockHandoffError> {
    match snapshot.hsi {
        ClockSourceState::Ready => {}
        ClockSourceState::Starting => return Err(ClockHandoffError::HsiNotReady),
        ClockSourceState::Disabled | ClockSourceState::Inconsistent => {
            return Err(ClockHandoffError::HsiOff);
        }
    }
    if snapshot.hsi_frequency != 4 {
        return Err(ClockHandoffError::HsiFrequency);
    }
    match snapshot.pll {
        ClockSourceState::Ready => {}
        ClockSourceState::Starting => return Err(ClockHandoffError::PllNotReady),
        ClockSourceState::Disabled | ClockSourceState::Inconsistent => {
            return Err(ClockHandoffError::PllOff);
        }
    }
    if snapshot.pll_source != 0 {
        return Err(ClockHandoffError::PllSource);
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
        sys_hz: K1_INHERITED_CLOCK_HZ,
        hclk1_hz: K1_INHERITED_CLOCK_HZ,
        pclk1_hz: K1_INHERITED_CLOCK_HZ,
        pclk1_tim_hz: K1_INHERITED_CLOCK_HZ,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        validate, ClockHandoffError, ClockSnapshot, ClockSourceState, K1_INHERITED_CLOCK_HZ,
    };

    const VALID: ClockSnapshot = ClockSnapshot {
        hsi: ClockSourceState::Ready,
        hsi_frequency: 4,
        pll: ClockSourceState::Ready,
        pll_source: 0,
        system_source: 2,
        active_system_source: 2,
        ahb_prescaler: 0,
        apb_prescaler: 0,
    };

    #[test]
    fn accepts_only_the_undivided_48mhz_contract() {
        let clocks = validate(VALID).unwrap();
        assert_eq!(clocks.sys_hz, K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.hclk1_hz, K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.pclk1_hz, K1_INHERITED_CLOCK_HZ);
        assert_eq!(clocks.pclk1_tim_hz, K1_INHERITED_CLOCK_HZ);
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
                    pll_source: 1,
                    ..VALID
                },
                ClockHandoffError::PllSource,
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
