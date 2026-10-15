//! Where each part of the shared 8 KiB memory view lives on the K1.
//!
//! The K5 view is its EEPROM: the image, the operator's place, and the vendor
//! calibration from `0x1E00`. The K1 keeps the same view over its SPI NOR
//! memory so one host backup reads either radio. The image is the AFIK
//! configuration region, the place is the latest record from its own sector,
//! and the calibration window is the vendor block at `0x010000`
//! (`EVID-K1-064`), which the stock K1 firmware addresses at the same offsets
//! the K5 uses from `0x1E00`: TX bias at `+0xD0`, battery at `+0x140`.
//!
//! The window is read through the vendor read, which takes no region and so
//! can never become a write.

use radio_app::operator_state::OPERATOR_STATE_BYTES;
use radio_platform::memory_link::{CALIBRATION_ORIGIN, PLACE_ORIGIN, VIEW_BYTES};

/// External-memory address of the vendor calibration block.
pub const VENDOR_CALIBRATION_ORIGIN: u32 = 0x01_0000;

/// Where one read of the view is served from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewSpan {
    /// Offset into the AFIK configuration region.
    Image(u32),
    /// Offset into the view's place record.
    Place(usize),
    /// Absolute external-memory address in the vendor calibration block.
    Vendor(u32),
}

/// Locates a read of `length` bytes at `address`, or `None` when it leaves the
/// view or crosses from one part into another.
#[must_use]
pub fn locate(address: u16, length: usize) -> Option<ViewSpan> {
    let start = usize::from(address);
    let end = start.checked_add(length)?;
    let place = usize::from(PLACE_ORIGIN);
    let calibration = usize::from(CALIBRATION_ORIGIN);
    if length == 0 || end > usize::from(VIEW_BYTES) {
        None
    } else if end <= place {
        Some(ViewSpan::Image(u32::from(address)))
    } else if start >= place && end <= calibration {
        Some(ViewSpan::Place(start - place))
    } else if start >= calibration {
        Some(ViewSpan::Vendor(
            VENDOR_CALIBRATION_ORIGIN + u32::from(address - CALIBRATION_ORIGIN),
        ))
    } else {
        None
    }
}

/// Fills `output` from `offset` in the place part: the record first, then
/// erased bytes, as a K5 holds it.
pub fn fill_place(offset: usize, record: Option<[u8; OPERATOR_STATE_BYTES]>, output: &mut [u8]) {
    output.fill(0xFF);
    let Some(record) = record else {
        return;
    };
    for (index, byte) in output.iter_mut().enumerate() {
        if let Some(&value) = record.get(offset + index) {
            *byte = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fill_place, locate, ViewSpan, VENDOR_CALIBRATION_ORIGIN};
    use crate::battery::CALIBRATION_ADDRESS;

    #[test]
    fn each_part_of_the_view_maps_to_its_own_storage() {
        assert_eq!(locate(0x0000, 0x80), Some(ViewSpan::Image(0)));
        assert_eq!(locate(0x0F80, 0x80), Some(ViewSpan::Image(0x0F80)));
        assert_eq!(locate(0x1000, 0x80), Some(ViewSpan::Image(0x1000)));
        assert_eq!(locate(0x1C80, 0x80), Some(ViewSpan::Image(0x1C80)));
        assert_eq!(locate(0x1D00, 0x80), Some(ViewSpan::Place(0)));
        assert_eq!(locate(0x1D80, 0x80), Some(ViewSpan::Place(0x80)));
        assert_eq!(
            locate(0x1E00, 0x80),
            Some(ViewSpan::Vendor(VENDOR_CALIBRATION_ORIGIN))
        );
        assert_eq!(locate(0x1F80, 0x80), Some(ViewSpan::Vendor(0x01_0180)));
    }

    #[test]
    fn calibration_sits_at_the_k5_offsets() {
        // K5 TX bias at 0x1ED0 and battery at 0x1F40; K1 at 0x100D0 and 0x10140.
        assert_eq!(locate(0x1ED0, 16), Some(ViewSpan::Vendor(0x01_00D0)));
        assert_eq!(
            locate(0x1F40, 8),
            Some(ViewSpan::Vendor(CALIBRATION_ADDRESS))
        );
    }

    #[test]
    fn reads_outside_or_across_parts_are_refused() {
        assert_eq!(locate(0x0000, 0), None);
        assert_eq!(locate(0x1FF0, 0x20), None);
        assert_eq!(locate(0x2000, 1), None);
        assert_eq!(locate(0xFFFF, 0x80), None);
        assert_eq!(locate(0x0FF0, 0x20), Some(ViewSpan::Image(0x0FF0)));
        assert_eq!(locate(0x1CF0, 0x20), None);
        assert_eq!(locate(0x1DF0, 0x20), None);
    }

    #[test]
    fn the_place_reads_as_its_record_then_erased() {
        let record = core::array::from_fn(|index| u8::try_from(index).unwrap_or(0));
        let mut block = [0_u8; 0x80];
        fill_place(0, Some(record), &mut block);
        assert_eq!(&block[..32], &record);
        assert!(block[32..].iter().all(|&byte| byte == 0xFF));

        fill_place(16, Some(record), &mut block[..32]);
        assert_eq!(&block[..16], &record[16..]);
        assert!(block[16..32].iter().all(|&byte| byte == 0xFF));

        fill_place(0, None, &mut block);
        assert!(block.iter().all(|&byte| byte == 0xFF));
    }
}
