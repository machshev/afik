//! Compile-only contract for the locally pinned PY32F071 HAL inventory.

use py32_hal::peripherals::{
    PA10, PA5, PA7, PA9, PB12, PB15, PB2, PB3, PB6, PF8, RCC, SPI1, TIM1, TIM15, TIM3, USART1,
};

/// Peripheral singletons required by the evidenced K1 migration.
///
/// Type-checking this alias proves only that the local metadata and HAL
/// generator expose these exact F071 surfaces; it does not initialize or
/// validate any peripheral behavior.
pub type K1Inventory = (
    RCC,
    USART1,
    SPI1,
    TIM1,
    TIM3,
    TIM15,
    PA5,
    PA7,
    PA9,
    PA10,
    PB2,
    PB3,
    PB6,
    PB12,
    PB15,
    PF8,
);
