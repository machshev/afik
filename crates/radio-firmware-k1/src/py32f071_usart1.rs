//! Compile-only contract for the evidenced K1 USART1 path.

use py32_hal::bind_interrupts;
use py32_hal::mode::Async;
use py32_hal::peripherals::{DMA1_CH1, DMA1_CH2, PA10, PA9, USART1};
use py32_hal::usart::{Config, ConfigError, InterruptHandler, Uart};

/// Baud rate physically observed on the K1 USART1 application path.
pub const K1_USART1_BAUD: u32 = 38_400;

bind_interrupts!(
    struct K1UsartInterrupts {
        USART1 => InterruptHandler<USART1>;
    }
);

/// Constructs the prospective async USART1 driver from the evidenced pins.
///
/// Type-checking this function proves the local F071 HAL exposes USART1 on
/// PA9 TX / PA10 RX AF1, its interrupt, and bounded DMA channels. This function
/// is deliberately not called by the current firmware entry point: clock
/// ownership, interrupt delivery, DMA operation, and physical behavior remain
/// separate runtime evidence boundaries.
pub fn new_k1_usart1(
    usart: USART1,
    rx: PA10,
    tx: PA9,
    tx_dma: DMA1_CH1,
    rx_dma: DMA1_CH2,
) -> Result<Uart<'static, Async>, ConfigError> {
    let mut config = Config::default();
    config.baudrate = K1_USART1_BAUD;
    Uart::new(usart, rx, tx, K1UsartInterrupts, tx_dma, rx_dma, config)
}
