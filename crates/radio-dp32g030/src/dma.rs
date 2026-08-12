//! Bounded DMA channel-zero receive support, per `EVID-DP32-012`.

use crate::mmio::Register;
use crate::DMA_BASE;

const CHANNEL_ZERO: u32 = DMA_BASE + 0x100;
const CHANNEL_ENABLE: u32 = 1 << 0;
const CHANNEL_LOOP: u32 = 1 << 13;
const CHANNEL_MEDIUM_PRIORITY: u32 = 1 << 14;
const MODE_DESTINATION_INCREMENT: u32 = 1 << 8;
const MODE_UART1_RX_SOURCE_REQUEST: u32 = 1 << 3;

/// A circular byte receiver backed by DMA channel zero.
pub struct CircularReceiver<const BYTES: usize> {
    buffer: *mut u8,
    consumed: usize,
}

impl<const BYTES: usize> CircularReceiver<BYTES> {
    /// Configures channel zero to copy UART1 receive bytes into `buffer`.
    ///
    /// # Safety
    ///
    /// `buffer` must name `BYTES` bytes of writable SRAM which remain uniquely
    /// owned by this receiver for its entire lifetime. `BYTES` must be in
    /// `1..=4096`; the hardware transfer count is twelve bits plus one.
    ///
    /// # Panics
    ///
    /// Panics when `BYTES` is zero or exceeds the hardware's 4096-byte count.
    #[allow(unsafe_code)]
    pub unsafe fn new(buffer: *mut u8, uart_receive_register: u32) -> Self {
        assert!(BYTES > 0 && BYTES <= 4096);
        control().write(0);
        interrupt_enable().write(0);
        interrupt_status().write(0xF0F);
        channel_control().write(0);
        channel_source().write(uart_receive_register);
        channel_destination().write(buffer as u32);
        channel_mode().write(MODE_DESTINATION_INCREMENT | MODE_UART1_RX_SOURCE_REQUEST);
        channel_control().write(
            CHANNEL_ENABLE
                | CHANNEL_LOOP
                | CHANNEL_MEDIUM_PRIORITY
                | ((u32::try_from(BYTES).unwrap_or(1) - 1) << 1),
        );
        control().write(1);
        Self {
            buffer,
            consumed: 0,
        }
    }

    /// Returns the next byte DMA has completed, if any.
    #[allow(unsafe_code)]
    pub fn read_byte(&mut self) -> Option<u8> {
        let produced = usize::try_from(channel_status().read() & 0x0FFF).unwrap_or(0) % BYTES;
        if produced == self.consumed {
            return None;
        }
        // SAFETY: `new` requires a live uniquely owned BYTES-byte buffer, and
        // `consumed` is always reduced modulo BYTES.
        let byte = unsafe { self.buffer.add(self.consumed).read_volatile() };
        self.consumed = (self.consumed + 1) % BYTES;
        Some(byte)
    }
}

const fn control() -> Register {
    Register::new(DMA_BASE, 0x00)
}

const fn interrupt_enable() -> Register {
    Register::new(DMA_BASE, 0x04)
}

const fn interrupt_status() -> Register {
    Register::new(DMA_BASE, 0x08)
}

const fn channel_control() -> Register {
    Register::new(CHANNEL_ZERO, 0x00)
}

const fn channel_mode() -> Register {
    Register::new(CHANNEL_ZERO, 0x04)
}

const fn channel_source() -> Register {
    Register::new(CHANNEL_ZERO, 0x08)
}

const fn channel_destination() -> Register {
    Register::new(CHANNEL_ZERO, 0x0C)
}

const fn channel_status() -> Register {
    Register::new(CHANNEL_ZERO, 0x10)
}

#[cfg(test)]
mod tests {
    use super::{
        channel_control, channel_destination, channel_mode, channel_source, channel_status,
    };

    #[test]
    fn channel_zero_registers_match_the_manual() {
        assert_eq!(channel_control().address(), 0x4000_1100);
        assert_eq!(channel_mode().address(), 0x4000_1104);
        assert_eq!(channel_source().address(), 0x4000_1108);
        assert_eq!(channel_destination().address(), 0x4000_110C);
        assert_eq!(channel_status().address(), 0x4000_1110);
    }
}
