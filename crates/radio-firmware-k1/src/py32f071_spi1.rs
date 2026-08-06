//! Compile-only contract for the evidenced K1 display SPI1 path.

use py32_hal::peripherals::{PA5, PA7, SPI1};
use py32_hal::spi::SpiTx;

const _: [(); crate::display::ASYNC_WRITE_CHUNK_BYTES] =
    [(); py32_hal::spi::ASYNC_WRITE_CHUNK_BYTES];

/// Constructs the prospective cooperative async display SPI interface.
///
/// Type-checking this function proves only the bounded local HAL surface. The
/// current firmware entry point does not call it.
#[must_use]
pub fn new_k1_display_spi(spi: SPI1, sck: PA5, mosi: PA7) -> SpiTx<'static, SPI1> {
    SpiTx::new(spi, sck, mosi)
}
