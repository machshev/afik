//! Host block writes into NOR sectors, staged so nothing is lost.
//!
//! A host programs a radio in small blocks, and only the blocks that changed.
//! NOR memory can only clear bits until a whole sector is erased, so a block
//! cannot simply be written in place, and erasing the sector first would lose
//! every block the host did not send. Each written block therefore goes into
//! a spare staging region of the same size, a bitmap records which, and reads
//! during the write see the staged block over the live one. When the host has
//! finished, [`Staged::commit`] fills the staging region with every block the
//! host did not write, erases the live region, and copies the staging region
//! back.
//!
//! The bitmap is 128 bytes, not an 8 KiB copy of the region in RAM. If power
//! fails between the live erase and the end of the copy, the staging region
//! still holds the whole new contents.

use crate::{Eeprom, EepromError, NorBus, Region, ERASED_BYTE, PAGE_BYTES, SECTOR_BYTES};

/// Bytes in one host write block.
pub const BLOCK_BYTES: usize = 8;
/// Largest region staged, in sectors.
pub const MAX_SECTORS: u32 = 2;
/// Host blocks in the largest region.
const BLOCKS: usize = (MAX_SECTORS * SECTOR_BYTES) as usize / BLOCK_BYTES;
// `u32` offsets below are built from these.
const BLOCK_BYTES_U32: u32 = 8;
const _: () = assert!(BLOCK_BYTES_U32 as usize == BLOCK_BYTES);

/// A live region and the spare region host writes are staged in.
#[derive(Clone, Debug)]
pub struct Staged {
    live: Region,
    staging: Region,
    /// One bit per block written since the staging sector was erased, or
    /// `None` when no host write is open.
    written: Option<[u8; BLOCKS / 8]>,
}

impl Staged {
    /// Pairs a live region with its staging region of the same size.
    ///
    /// # Errors
    ///
    /// [`EepromError::OutOfRange`] when the regions differ in size, are larger
    /// than [`MAX_SECTORS`], or overlap.
    pub fn new<E>(live: Region, staging: Region) -> Result<Self, EepromError<E>> {
        let apart = live.origin() + live.len() <= staging.origin()
            || staging.origin() + staging.len() <= live.origin();
        if live.len() != staging.len() || live.sectors() > MAX_SECTORS || !apart {
            return Err(EepromError::OutOfRange);
        }
        Ok(Self {
            live,
            staging,
            written: None,
        })
    }

    /// Reports whether a host write is open and not yet committed.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.written.is_some()
    }

    /// Reads the sector as the host has written it so far.
    ///
    /// # Errors
    ///
    /// Any bus error, or [`EepromError::OutOfRange`] past the sector.
    pub fn read<B: NorBus>(
        &self,
        eeprom: &mut Eeprom<B>,
        offset: u32,
        output: &mut [u8],
    ) -> Result<(), EepromError<B::Error>> {
        let Some(written) = &self.written else {
            return eeprom.read(self.live, offset, output);
        };
        let mut at = offset;
        let mut rest = output;
        while !rest.is_empty() {
            // Up to the end of this block, so each piece has one source.
            let into =
                usize::try_from(at % BLOCK_BYTES_U32).map_err(|_| EepromError::OutOfRange)?;
            let take = rest.len().min(BLOCK_BYTES - into);
            let (piece, tail) = rest.split_at_mut(take);
            let block =
                usize::try_from(at / BLOCK_BYTES_U32).map_err(|_| EepromError::OutOfRange)?;
            let source = if block < BLOCKS && is_set(written, block) {
                self.staging
            } else {
                self.live
            };
            eeprom.read(source, at, piece)?;
            at += u32::try_from(take).map_err(|_| EepromError::OutOfRange)?;
            rest = tail;
        }
        Ok(())
    }

    /// Stages one aligned block, erasing the staging sector on the first.
    ///
    /// # Errors
    ///
    /// [`EepromError::OutOfRange`] for an unaligned or out-of-sector block,
    /// [`EepromError::Rewritten`] when a block already staged in this write
    /// would need bits set again, or any bus error.
    pub fn write<B: NorBus>(
        &mut self,
        eeprom: &mut Eeprom<B>,
        offset: u32,
        data: &[u8; BLOCK_BYTES],
    ) -> Result<(), EepromError<B::Error>> {
        if offset % BLOCK_BYTES_U32 != 0 || offset >= self.live.len() {
            return Err(EepromError::OutOfRange);
        }
        let block =
            usize::try_from(offset / BLOCK_BYTES_U32).map_err(|_| EepromError::OutOfRange)?;
        if self.written.is_none() {
            eeprom.erase(self.staging)?;
        }
        let written = self.written.get_or_insert([0; BLOCKS / 8]);
        if is_set(written, block) {
            // Programming only clears bits, so the same block twice works when
            // the new bytes need nothing set again.
            let mut held = [0_u8; BLOCK_BYTES];
            eeprom.read(self.staging, offset, &mut held)?;
            if held.iter().zip(data).any(|(held, new)| held & new != *new) {
                return Err(EepromError::Rewritten);
            }
        }
        eeprom.program(self.staging, offset, data)?;
        written[block / 8] |= 1 << (block % 8);
        Ok(())
    }

    /// Makes the staged write the live region, when one is open.
    ///
    /// # Errors
    ///
    /// Any bus error. The write stays open, so a later commit tries again.
    pub fn commit<B: NorBus>(
        &mut self,
        eeprom: &mut Eeprom<B>,
    ) -> Result<(), EepromError<B::Error>> {
        let Some(written) = self.written else {
            return Ok(());
        };
        let blocks = self.live.len() / BLOCK_BYTES_U32;
        let blocks = usize::try_from(blocks).map_err(|_| EepromError::OutOfRange)?;
        let mut block = [0_u8; BLOCK_BYTES];
        for index in (0..blocks).filter(|&index| !is_set(&written, index)) {
            let offset = u32::try_from(index * BLOCK_BYTES).map_err(|_| EepromError::OutOfRange)?;
            eeprom.read(self.live, offset, &mut block)?;
            if block.iter().any(|&byte| byte != ERASED_BYTE) {
                eeprom.program(self.staging, offset, &block)?;
            }
        }
        eeprom.erase(self.live)?;
        let mut page = [0_u8; PAGE_BYTES as usize];
        for offset in (0..self.live.len()).step_by(PAGE_BYTES as usize) {
            eeprom.read(self.staging, offset, &mut page)?;
            if page.iter().any(|&byte| byte != ERASED_BYTE) {
                eeprom.program(self.live, offset, &page)?;
            }
        }
        self.written = None;
        Ok(())
    }
}

fn is_set(written: &[u8; BLOCKS / 8], block: usize) -> bool {
    written[block / 8] & (1 << (block % 8)) != 0
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{Staged, BLOCK_BYTES, MAX_SECTORS};
    use crate::tests::ModelBus;
    use crate::{Eeprom, EepromError, Region, SECTOR_BYTES};

    const SPAN: u32 = MAX_SECTORS * SECTOR_BYTES;
    const LIVE: u32 = 0x10_0000;
    const STAGING: u32 = LIVE + SPAN;

    fn setup() -> (Eeprom<ModelBus>, Staged) {
        let mut eeprom = Eeprom::new(ModelBus::new(LIVE, 2 * SPAN as usize));
        let live = Region::new(LIVE, SPAN).unwrap();
        let staging = Region::new(STAGING, SPAN).unwrap();
        let old: std::vec::Vec<u8> = (0..SPAN).map(|at| (at % 251) as u8).collect();
        eeprom.write(live, &old).unwrap();
        (eeprom, Staged::new::<()>(live, staging).unwrap())
    }

    fn read_all(eeprom: &mut Eeprom<ModelBus>, staged: &Staged) -> std::vec::Vec<u8> {
        let mut bytes = std::vec![0_u8; SPAN as usize];
        for (index, chunk) in bytes.chunks_mut(0x80).enumerate() {
            staged
                .read(eeprom, u32::try_from(index * 0x80).unwrap(), chunk)
                .unwrap();
        }
        bytes
    }

    #[test]
    fn changed_blocks_replace_only_themselves() {
        let (mut eeprom, mut staged) = setup();
        let mut wanted = read_all(&mut eeprom, &staged);
        // Bits set and cleared, which no in-place program could do.
        for (offset, fill) in [
            (0_u32, 0xFF_u8),
            (8, 0x00),
            (0x7F8, 0xA5),
            (0xFF8, 0x5A),
            (0x1000, 0x3C),
            (0x1FF8, 0xC3),
        ] {
            let data = [fill; BLOCK_BYTES];
            staged.write(&mut eeprom, offset, &data).unwrap();
            let at = offset as usize;
            wanted[at..at + BLOCK_BYTES].copy_from_slice(&data);
        }
        assert!(staged.is_open());
        // The host's read-back sees its write before the commit.
        assert_eq!(read_all(&mut eeprom, &staged), wanted);

        staged.commit(&mut eeprom).unwrap();
        assert!(!staged.is_open());
        assert_eq!(read_all(&mut eeprom, &staged), wanted);
        let mut live = std::vec![0_u8; SPAN as usize];
        eeprom
            .read(Region::new(LIVE, SPAN).unwrap(), 0, &mut live)
            .unwrap();
        assert_eq!(live, wanted);
    }

    #[test]
    fn an_unaligned_read_straddling_staged_and_live_blocks_is_whole() {
        let (mut eeprom, mut staged) = setup();
        let before = read_all(&mut eeprom, &staged);
        staged.write(&mut eeprom, 16, &[0x11; BLOCK_BYTES]).unwrap();
        let mut piece = [0_u8; 20];
        staged.read(&mut eeprom, 12, &mut piece).unwrap();
        assert_eq!(&piece[..4], &before[12..16]);
        assert_eq!(&piece[4..12], &[0x11; BLOCK_BYTES]);
        assert_eq!(&piece[12..], &before[24..32]);
    }

    #[test]
    fn a_block_is_restaged_only_when_no_bit_must_be_set() {
        let (mut eeprom, mut staged) = setup();
        staged.write(&mut eeprom, 0, &[0x0F; BLOCK_BYTES]).unwrap();
        staged.write(&mut eeprom, 0, &[0x0F; BLOCK_BYTES]).unwrap();
        staged.write(&mut eeprom, 0, &[0x03; BLOCK_BYTES]).unwrap();
        assert_eq!(
            staged.write(&mut eeprom, 0, &[0xF0; BLOCK_BYTES]),
            Err(EepromError::Rewritten)
        );
    }

    #[test]
    fn blocks_outside_the_region_or_unaligned_are_refused() {
        let (mut eeprom, mut staged) = setup();
        let data = [0_u8; BLOCK_BYTES];
        assert_eq!(
            staged.write(&mut eeprom, 4, &data),
            Err(EepromError::OutOfRange)
        );
        assert_eq!(
            staged.write(&mut eeprom, SPAN, &data),
            Err(EepromError::OutOfRange)
        );
        assert!(!staged.is_open());
    }

    #[test]
    fn nothing_open_commits_nothing() {
        let (mut eeprom, mut staged) = setup();
        let erases = eeprom_erases(&mut eeprom);
        staged.commit(&mut eeprom).unwrap();
        assert_eq!(eeprom_erases(&mut eeprom), erases);
    }

    #[test]
    fn regions_must_match_fit_and_not_overlap() {
        let live = Region::new(LIVE, SPAN).unwrap();
        assert!(Staged::new::<()>(live, live).is_err());
        let overlapping = Region::new(LIVE + SECTOR_BYTES, SPAN).unwrap();
        assert!(Staged::new::<()>(live, overlapping).is_err());
        let smaller = Region::new(STAGING, SECTOR_BYTES).unwrap();
        assert!(Staged::new::<()>(live, smaller).is_err());
        let three = Region::new(LIVE, 3 * SECTOR_BYTES).unwrap();
        let after = Region::new(LIVE + 3 * SECTOR_BYTES, 3 * SECTOR_BYTES).unwrap();
        assert!(Staged::new::<()>(three, after).is_err());
        let one = Region::new(LIVE, SECTOR_BYTES).unwrap();
        assert!(Staged::new::<()>(one, Region::new(STAGING, SECTOR_BYTES).unwrap()).is_ok());
    }

    fn eeprom_erases(eeprom: &mut Eeprom<ModelBus>) -> usize {
        // The model counts erases; reach it through the driver's bus.
        let bus = core::mem::replace(eeprom, Eeprom::new(ModelBus::new(0, 0))).release();
        let erases = bus.erases;
        *eeprom = Eeprom::new(bus);
        erases
    }
}
