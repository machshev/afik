//! A polled UART, per `EVID-DP32-009`.
//!
//! Polled and CPU-driven on purpose: this image has no interrupt table beyond
//! reset and no DMA evidence, and a driver which cannot lose a byte to a
//! handler that does not exist is the one worth having first. Both FIFOs are
//! eight bytes deep, so a caller which polls between bytes never overruns the
//! transmitter.

use crate::clock::SystemClock;
use crate::mmio::Register;

/// `UART_CTRL` bit 0: module enable.
const CTRL_UARTEN: u32 = 1 << 0;
/// `UART_CTRL` bit 1: receive enable.
const CTRL_RXEN: u32 = 1 << 1;
/// `UART_CTRL` bit 2: transmit enable.
const CTRL_TXEN: u32 = 1 << 2;
/// `UART_CTRL` bit 3: DMA owns receive-data reads.
const CTRL_RXDMAEN: u32 = 1 << 3;
const CTRL_RECEIVE_DMA_PREPARED: u32 = CTRL_RXEN | CTRL_TXEN | CTRL_RXDMAEN;

/// `UART_IF` bit 10: the receive FIFO is empty.
const IF_RXFIFO_EMPTY: u32 = 1 << 10;
/// `UART_IF` bit 14: the transmit FIFO is full.
const IF_TXFIFO_FULL: u32 = 1 << 14;
/// `UART_IF` bit 16: the transmitter is busy.
const IF_TXBUSY: u32 = 1 << 16;
/// `UART_IF` bit 5: receive timeout; write one to clear before DMA starts.
const IF_RXTO: u32 = 1 << 5;

/// `UART_FIFO` bit 6: clear the receive FIFO.
const FIFO_RF_CLR: u32 = 1 << 6;
/// `UART_FIFO` bit 7: clear the transmit FIFO.
const FIFO_TF_CLR: u32 = 1 << 7;
/// Receive FIFO trigger encoding `111`: eight bytes.
const FIFO_RF_LEVEL_EIGHT: u32 = 0b111;

/// One UART instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Uart {
    base: u32,
}

impl Uart {
    /// Names the UART at `base`.
    pub const fn new(base: u32) -> Self {
        Self { base }
    }

    /// `UART_CTRL`.
    const fn control(self) -> Register {
        Register::new(self.base, 0x00)
    }

    /// `UART_BAUD`.
    const fn baud(self) -> Register {
        Register::new(self.base, 0x04)
    }

    /// `UART_TDR`, the transmit data register.
    const fn transmit(self) -> Register {
        Register::new(self.base, 0x08)
    }

    /// `UART_RDR`, the receive data register.
    const fn receive(self) -> Register {
        Register::new(self.base, 0x0C)
    }

    /// Address DMA must use as its UART receive source.
    pub const fn receive_address(self) -> u32 {
        self.receive().address()
    }

    /// `UART_IE`, the interrupt-enable register.
    const fn interrupt_enable(self) -> Register {
        Register::new(self.base, 0x10)
    }

    /// `UART_IF`, which this driver reads as status rather than as interrupts.
    const fn status(self) -> Register {
        Register::new(self.base, 0x14)
    }

    /// `UART_FIFO`.
    const fn fifo(self) -> Register {
        Register::new(self.base, 0x18)
    }

    /// `UART_FC`, the flow-control register.
    const fn flow_control(self) -> Register {
        Register::new(self.base, 0x1C)
    }

    /// `UART_RXTO`, in received-character times.
    const fn receive_timeout(self) -> Register {
        Register::new(self.base, 0x20)
    }

    /// Configures 8-N-1 at `baud`, with both FIFOs emptied and no interrupt.
    ///
    /// The module is disabled first so the divider never changes underneath a
    /// transfer, and every field this driver depends on is written rather than
    /// inherited: an image cannot see what a bootloader left behind.
    pub fn configure(self, clock: SystemClock, baud: u32) {
        self.configure_with_divider(divider(clock.hertz(), baud));
    }

    /// Configures 8-N-1 at an explicitly supplied divider.
    ///
    /// A diagnostic which does not know the clock it is running at cannot
    /// compute a divider, but it can try several. That is the only reason this
    /// exists; an image which knows its clock should use [`Uart::configure`].
    pub fn configure_with_divider(self, divider: u16) {
        self.control().write(0);
        self.interrupt_enable().write(0);
        self.flow_control().write(0);
        self.baud().write(u32::from(divider));
        self.fifo()
            .write(FIFO_RF_LEVEL_EIGHT | FIFO_RF_CLR | FIFO_TF_CLR);
        self.control().write(CTRL_UARTEN | CTRL_RXEN | CTRL_TXEN);
    }

    /// Prepares receive DMA while keeping the UART itself disabled.
    ///
    /// The known-running V1 firmware programs the DMA channel between this
    /// preparation and [`Uart::start_receive_dma`]. Keeping that ordering here
    /// avoids accepting bytes before DMA owns the receive-data register.
    pub fn prepare_receive_dma_with_divider(self, divider: u16) {
        self.control().write(0);
        self.baud().write(u32::from(divider));
        self.receive_timeout().write(4);
        self.flow_control().write(0);
        self.fifo()
            .write(FIFO_RF_LEVEL_EIGHT | FIFO_RF_CLR | FIFO_TF_CLR);
        self.interrupt_enable().write(0);
        self.control().write(CTRL_RECEIVE_DMA_PREPARED);
    }

    /// Starts a prepared receive-DMA UART after its DMA channel is enabled.
    pub fn start_receive_dma(self) {
        self.status().write(IF_RXTO);
        self.control().modify(|value| value | CTRL_UARTEN);
    }

    /// Sends one byte, waiting for room in the transmit FIFO.
    pub fn write_byte(self, byte: u8) {
        while self.status().read() & IF_TXFIFO_FULL != 0 {
            core::hint::spin_loop();
        }
        self.transmit().write(u32::from(byte));
    }

    /// Sends every byte in order.
    pub fn write(self, bytes: &[u8]) {
        for byte in bytes {
            self.write_byte(*byte);
        }
    }

    /// Waits until the transmitter has finished the last bit on the wire.
    pub fn flush(self) {
        while self.status().read() & IF_TXBUSY != 0 {
            core::hint::spin_loop();
        }
    }

    /// Returns the raw `UART_IF` status word.
    ///
    /// This is for diagnostics which have to report what the peripheral says
    /// rather than what a driver concluded from it.
    pub fn status_bits(self) -> u32 {
        self.status().read()
    }

    /// Takes one received byte if the receive FIFO holds any.
    ///
    /// This deliberately does not write `UART_IF`. `AFIK-K5-1.2` added a
    /// write-one-to-clear interpretation for its error bits and then produced
    /// no boot banner on the exact unit. That does not prove the write caused
    /// the failure, because this method runs after the banner, but it does mean
    /// the interpretation has not earned a place in the booting baseline.
    pub fn read_byte(self) -> Option<u8> {
        if self.status().read() & IF_RXFIFO_EMPTY != 0 {
            return None;
        }
        Some((self.receive().read() & 0xFF) as u8)
    }
}

/// Returns the `UART_BAUD` divider for `baud` from a module clock of
/// `clock_hz`.
///
/// The manual prints its formula as an image and works one example, dividing
/// and rounding; this rounds to nearest for the same reason. A divider of zero
/// would stop the baud generator, so the result is held at one, and the field
/// is sixteen bits wide, so it saturates rather than wrapping into a faster
/// rate than the caller asked for.
// The cast below is guarded by the comparison above it, which is the whole
// point of the function; `u16::try_from` is not available in a const context.
#[allow(clippy::cast_possible_truncation)]
pub const fn divider(clock_hz: u32, baud: u32) -> u16 {
    if baud == 0 {
        return u16::MAX;
    }
    let rounded = (clock_hz as u64 + (baud as u64) / 2) / (baud as u64);
    if rounded < 1 {
        return 1;
    }
    if rounded > u16::MAX as u64 {
        return u16::MAX;
    }
    rounded as u16
}

/// Returns the board-proven V1 programming-port divider.
///
/// The known-running V1 firmware uses 39,053 as the empirical divisor target
/// for a nominal 38,400-baud link. `EVID-K5-023` keeps this board correction
/// separate from the manual's generic divider arithmetic.
pub const fn k5_programming_divider(clock_hz: u32) -> u16 {
    divider(clock_hz, 39_053)
}

#[cfg(test)]
mod tests {
    use super::{divider, k5_programming_divider, CTRL_RECEIVE_DMA_PREPARED, CTRL_UARTEN};

    #[test]
    fn the_manuals_worked_example_is_reproduced() {
        assert_eq!(divider(48_000_000, 115_200), 417);
    }

    #[test]
    fn the_programming_baud_divides_exactly_at_nominal_rchf() {
        assert_eq!(divider(48_000_000, 38_400), 1_250);
    }

    #[test]
    fn a_corrected_clock_moves_the_divider_by_less_than_a_percent() {
        let divider = divider(48_120_000, 38_400);
        assert_eq!(divider, 1_253);
        let error_parts_per_thousand =
            (i64::from(48_120_000 / i32::from(divider)) - 38_400) * 1_000 / 38_400;
        assert!(error_parts_per_thousand.abs() < 10);
    }

    #[test]
    fn a_divider_is_never_zero_and_never_wraps() {
        assert_eq!(divider(1_000, 100_000_000), 1);
        assert_eq!(divider(48_000_000, 1), u16::MAX);
        assert_eq!(divider(48_000_000, 0), u16::MAX);
    }

    #[test]
    fn the_k5_programming_port_uses_its_board_proven_correction() {
        assert_eq!(k5_programming_divider(47_796_863), 1_224);
    }

    #[test]
    fn receive_dma_is_prepared_before_the_uart_starts() {
        assert_eq!(CTRL_RECEIVE_DMA_PREPARED, 0b1110);
        assert_eq!(CTRL_RECEIVE_DMA_PREPARED | CTRL_UARTEN, 0b1111);
    }
}
