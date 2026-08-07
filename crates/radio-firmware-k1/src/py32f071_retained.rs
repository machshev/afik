//! Bounded retained-configuration region in the K1's internal flash.
//!
//! The last erase sector of the evidenced application flash holds one canonical
//! configuration image, so a radio programmed by the host tooling keeps its
//! channels across a power cycle. The application image cannot reach into this
//! sector: the linker script stops before it and every access here is bounded to
//! it, so a failed retain can lose the retained configuration and nothing else.

use py32_hal::flash::values::{PAGE_SIZE, SECTOR_SIZE};
use py32_hal::flash::{Error as FlashError, Flash};
use py32_hal::mode::Blocking;
use py32_hal::peripherals::FLASH;
use radio_storage::configuration_image_len_from_header;

use crate::configuration::RETAINED_IMAGE_BYTES;

/// Offset of the retained-configuration sector from the start of flash.
///
/// `EVID-K1-020` places the application at `0x0800_2800`; this is the last
/// 8 KiB erase sector of the 128 KiB device, `0x0801_E000`.
pub const RETAINED_OFFSET: u32 = 0x1_E000;

/// Bytes reserved for the retained-configuration sector.
pub const RETAINED_SECTOR_BYTES: u32 = SECTOR_SIZE as u32;

/// End of the application flash the linker script may fill.
pub const APPLICATION_FLASH_END_OFFSET: u32 = RETAINED_OFFSET;

// The retained region is exactly one erase sector, so retaining a
// configuration never erases a byte of anything else.
const _: () = assert!(RETAINED_IMAGE_BYTES <= SECTOR_SIZE);
const _: () = assert!(RETAINED_IMAGE_BYTES % PAGE_SIZE == 0);

/// Why a retained configuration could not be read or written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainError {
    /// The image does not fit the reserved sector.
    TooLarge,
    /// The flash controller reported a failure.
    Flash(FlashError),
}

/// Bounded accessor for the retained-configuration sector.
pub struct RetainedConfiguration<'d> {
    flash: Flash<'d, Blocking>,
}

impl<'d> RetainedConfiguration<'d> {
    /// Takes exclusive ownership of the internal flash controller.
    pub fn new(flash: FLASH) -> Self {
        Self {
            flash: Flash::new_blocking(flash),
        }
    }

    /// Reads a retained image into `buffer` and returns its exact length.
    ///
    /// Only the container header is inspected here so the exact image length
    /// can be read back; the complete checksum, ordering, and object validation
    /// happen when the caller loads it. An erased or foreign sector yields
    /// `None` rather than an error, because an unprogrammed radio is normal.
    pub fn read(&mut self, buffer: &mut [u8; RETAINED_IMAGE_BYTES]) -> Option<usize> {
        self.flash.blocking_read(RETAINED_OFFSET, buffer).ok()?;
        let length = configuration_image_len_from_header(buffer).ok()?;
        if length > RETAINED_IMAGE_BYTES {
            return None;
        }
        Some(length)
    }

    /// Replaces the retained image with the first `length` bytes of `buffer`.
    ///
    /// The complete reserved sector is erased first and whole write pages are
    /// programmed, so no partly overwritten previous image can survive to be
    /// read back as a valid configuration.
    pub fn write(
        &mut self,
        buffer: &[u8; RETAINED_IMAGE_BYTES],
        length: usize,
    ) -> Result<(), RetainError> {
        if length > RETAINED_IMAGE_BYTES {
            return Err(RetainError::TooLarge);
        }
        self.flash
            .blocking_erase(RETAINED_OFFSET, RETAINED_OFFSET + RETAINED_SECTOR_BYTES)
            .map_err(RetainError::Flash)?;
        let pages = length.div_ceil(PAGE_SIZE);
        self.flash
            .blocking_write(RETAINED_OFFSET, &buffer[..pages * PAGE_SIZE])
            .map_err(RetainError::Flash)
    }
}
