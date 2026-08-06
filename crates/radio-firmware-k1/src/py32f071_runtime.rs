//! Compile-only ownership composition for the prospective K1 async runtime.

use embassy_executor::Executor;
use py32_hal::mode::Async;
use py32_hal::peripherals::{DMA1_CH1, DMA1_CH2, PA10, PA5, PA7, PA9, SPI1, USART1};
use py32_hal::spi::SpiTx;
use py32_hal::usart::{ConfigError, Uart};

use crate::embassy_runtime::executor;
use crate::py32f071_spi1::new_k1_display_spi;
use crate::py32f071_usart1::new_k1_usart1;

/// Caller-supplied peripheral ownership required by the prospective runtime.
pub struct K1RuntimePeripherals {
    /// USART1 peripheral token.
    pub usart: USART1,
    /// USART1 receive pin, PA10 AF1.
    pub usart_rx: PA10,
    /// USART1 transmit pin, PA9 AF1.
    pub usart_tx: PA9,
    /// Bounded USART1 transmit DMA channel.
    pub usart_tx_dma: DMA1_CH1,
    /// Bounded USART1 receive DMA channel.
    pub usart_rx_dma: DMA1_CH2,
    /// SPI1 peripheral token.
    pub display_spi: SPI1,
    /// Display SPI clock pin, PA5 AF0.
    pub display_sck: PA5,
    /// Display SPI controller-output pin, PA7 AF0.
    pub display_mosi: PA7,
}

/// Owned driver bundle for the prospective K1 async runtime.
pub struct K1Runtime {
    /// Heap-free Cortex-M thread executor.
    pub executor: Executor,
    /// Async USART1 driver with bounded DMA ownership.
    pub usart: Uart<'static, Async>,
    /// Cooperative transmit-only display SPI driver.
    pub display_spi: SpiTx<'static, SPI1>,
}

/// Composes the proven async surfaces without initializing the HAL or clocks.
///
/// This function is compile-only and is not called by the polling firmware
/// entry point. Its arguments make every peripheral ownership transfer
/// explicit and leave TIM15, display A0/CS, keypad GPIO, and all other board
/// surfaces outside the bundle.
pub fn compose(peripherals: K1RuntimePeripherals) -> Result<K1Runtime, ConfigError> {
    let usart = new_k1_usart1(
        peripherals.usart,
        peripherals.usart_rx,
        peripherals.usart_tx,
        peripherals.usart_tx_dma,
        peripherals.usart_rx_dma,
    )?;
    let display_spi = new_k1_display_spi(
        peripherals.display_spi,
        peripherals.display_sck,
        peripherals.display_mosi,
    );

    Ok(K1Runtime {
        executor: executor(),
        usart,
        display_spi,
    })
}
