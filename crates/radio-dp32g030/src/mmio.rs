//! The one place this crate touches memory-mapped registers.
//!
//! Every other module builds values with pure functions and hands them here.
//! Keeping the raw access in a single module means the unsafe surface is four
//! lines long and every caller is a named register operation.

/// One 32-bit memory-mapped register at a fixed address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Register {
    address: u32,
}

impl Register {
    /// Names the register at `base + offset`.
    pub const fn new(base: u32, offset: u32) -> Self {
        Self {
            address: base + offset,
        }
    }

    /// Returns the address this register was named at.
    pub const fn address(self) -> u32 {
        self.address
    }

    /// Reads the register.
    ///
    /// # Safety note
    ///
    /// The address comes from a manual-sourced constant in this crate, is
    /// word-aligned by construction, and names a peripheral register whose read
    /// has no side effect except where the manual says so (`UART_RDR` pops the
    /// receive FIFO). Callers on a host build must not call this: nothing maps
    /// these addresses there.
    #[allow(unsafe_code)]
    pub fn read(self) -> u32 {
        // SAFETY: see the safety note above.
        unsafe { core::ptr::read_volatile(self.address as *const u32) }
    }

    /// Writes the register.
    ///
    /// # Safety note
    ///
    /// As [`Register::read`]. The written value is produced by this crate's
    /// pure field encoders, which cannot name a bit the evidence does not
    /// record.
    #[allow(unsafe_code)]
    pub fn write(self, value: u32) {
        // SAFETY: see the safety note above.
        unsafe { core::ptr::write_volatile(self.address as *mut u32, value) }
    }

    /// Reads, applies `change`, and writes the result back.
    pub fn modify<F: FnOnce(u32) -> u32>(self, change: F) {
        self.write(change(self.read()));
    }
}

#[cfg(test)]
mod tests {
    use super::Register;

    #[test]
    fn register_addresses_are_base_plus_offset() {
        assert_eq!(Register::new(0x4006_B800, 0x04).address(), 0x4006_B804);
    }
}
