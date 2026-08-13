//! Relative, power-loss-safe persistence for canonical configuration images.
//!
//! This is a storage model, not a device driver. Addresses are block numbers
//! relative to the caller's allocation; a target adapter supplies the actual
//! non-volatile transaction later.

use crate::decode_configuration_image;

/// The fixed physical programming unit established by the EEPROM evidence.
pub const BLOCK_BYTES: usize = 8;
/// Number of complete slots in the bounded model.
pub const SLOT_COUNT: usize = 2;
/// K1-parity maximum canonical image accepted by one slot.
pub const MAX_CANONICAL_IMAGE_BYTES: usize = 1_280;
const HEADER_BYTES: usize = 24;
const COMMIT_BYTES: usize = BLOCK_BYTES;
/// Bytes reserved by one relative slot, including its final commit block.
pub const SLOT_BYTES: usize = HEADER_BYTES + MAX_CANONICAL_IMAGE_BYTES + COMMIT_BYTES;
/// Blocks reserved by one relative slot.
pub const SLOT_BLOCKS: usize = SLOT_BYTES / BLOCK_BYTES;
/// Blocks reserved by this model in total.
pub const TOTAL_BLOCKS: usize = SLOT_COUNT * SLOT_BLOCKS;
/// Bytes reserved by this model in total.
pub const TOTAL_BYTES: usize = TOTAL_BLOCKS * BLOCK_BYTES;
/// Documented model-region bound; this is not an EEPROM allocation claim.
pub const MODEL_REGION_BYTES: usize = 8_192;

const MAGIC: [u8; 4] = *b"AFIP";
const FORMAT_VERSION: u8 = 1;
const EMPTY_BYTE: u8 = 0xFF;

const _: [(); 0] = [(); SLOT_BYTES % BLOCK_BYTES];
const _: [(); 0] = [(); HEADER_BYTES % BLOCK_BYTES];
const _: [(); 0] = [(); MAX_CANONICAL_IMAGE_BYTES % BLOCK_BYTES];
const _: [(); 0] = [(); TOTAL_BYTES % BLOCK_BYTES];
const _: [(); 0] = [(); (TOTAL_BYTES > MODEL_REGION_BYTES) as usize];

/// A relative block backend with target-faithful atomic expected-byte writes.
pub trait BlockBackend {
    /// Backend-specific failure, including stale expected-byte rejection.
    type Error;

    /// Reads one eight-byte block at a relative block index.
    fn read_block(&mut self, block: u16, output: &mut [u8; BLOCK_BYTES])
        -> Result<(), Self::Error>;

    /// Atomically compares `expected` and programs `replacement`.
    ///
    /// The adapter must perform the compare and program as one bounded
    /// operation so a concurrent change cannot pass a split read/compare.
    fn write_verified(
        &mut self,
        block: u16,
        expected: &[u8; BLOCK_BYTES],
        replacement: &[u8; BLOCK_BYTES],
    ) -> Result<(), Self::Error>;
}

/// Failure while inspecting or updating the logical persistence region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceError<E> {
    /// The backend rejected or could not complete an operation.
    Backend(E),
    /// A read-back did not equal the bytes just written.
    VerificationFailed,
    /// The image is not an exact supported canonical image.
    InvalidImage,
    /// A caller supplied an image larger than the fixed slot bound.
    ImageTooLarge,
}

/// A validated image selected from one slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedImage<'a> {
    /// Exact canonical image bytes, without slot padding or metadata.
    pub bytes: &'a [u8],
    /// Wrapping slot generation; zero is reserved for erased/empty state.
    pub generation: u32,
    /// Relative slot index which supplied the image.
    pub slot: u8,
}

/// Caller-provided fixed scratch for one complete slot.
pub type SlotBuffer = [[u8; BLOCK_BYTES]; SLOT_BLOCKS];

/// Returns an erased slot snapshot for tests and future adapter setup.
#[must_use]
pub const fn empty_slot() -> SlotBuffer {
    [[EMPTY_BYTE; BLOCK_BYTES]; SLOT_BLOCKS]
}

/// Inspects both relative slots and copies the newest exact image to `output`.
pub fn load<'a, B: BlockBackend>(
    backend: &mut B,
    scratch: &mut SlotBuffer,
    output: &'a mut [u8; MAX_CANONICAL_IMAGE_BYTES],
) -> Result<Option<LoadedImage<'a>>, PersistenceError<B::Error>> {
    let first = inspect_slot(backend, 0, scratch, output)?;
    let second = inspect_slot(backend, 1, scratch, output)?;
    let Some(slot) = newest_slot([first, second]) else {
        return Ok(None);
    };
    let meta = [first, second][slot];
    if slot == 0 && inspect_slot(backend, 0, scratch, output)? != meta {
        return Ok(None);
    }
    let Some(meta) = meta else {
        return Ok(None);
    };
    Ok(Some(LoadedImage {
        bytes: &output[..meta.length],
        generation: meta.generation,
        slot: match slot {
            0 => 0,
            1 => 1,
            _ => return Ok(None),
        },
    }))
}

/// Updates the inactive slot using the actual read snapshot of that slot.
///
/// `scratch` and `image_scratch` are caller-owned so an embedded target can
/// place them in static storage rather than creating a large hidden stack
/// frame. The active slot is never a write target; an interrupted update may
/// tear only the inactive candidate.
pub fn update<B: BlockBackend>(
    backend: &mut B,
    image: &[u8],
    scratch: &mut SlotBuffer,
    image_scratch: &mut [u8; MAX_CANONICAL_IMAGE_BYTES],
) -> Result<u32, PersistenceError<B::Error>> {
    validate_image(image)?;
    let first = inspect_slot(backend, 0, scratch, image_scratch)?;
    let second = inspect_slot(backend, 1, scratch, image_scratch)?;
    let metas = [first, second];
    let (target, generation) = match newest_slot(metas) {
        Some(active) => (
            1 - active,
            next_generation(metas[active].map_or(1, |m| m.generation)),
        ),
        None => (0, 1),
    };

    let commit_index = SLOT_BLOCKS - 1;
    let commit_block = relative_block(target, commit_index);
    let mut expected_commit = [0; BLOCK_BYTES];
    backend
        .read_block(commit_block, &mut expected_commit)
        .map_err(PersistenceError::Backend)?;
    // Tombstone first: an old valid marker must not authenticate any partially
    // replaced header or payload. Erased all-FF is the unambiguous invalid tag.
    write_checked(
        backend,
        commit_block,
        expected_commit,
        [EMPTY_BYTE; BLOCK_BYTES],
    )?;

    for block_index in 0..commit_index {
        let block = relative_block(target, block_index);
        let mut expected = [0; BLOCK_BYTES];
        backend
            .read_block(block, &mut expected)
            .map_err(PersistenceError::Backend)?;
        let mut replacement = [EMPTY_BYTE; BLOCK_BYTES];
        encode_block(block_index, generation, image, &mut replacement);
        write_checked(backend, block, expected, replacement)?;
    }
    let mut marker = [EMPTY_BYTE; BLOCK_BYTES];
    encode_block(commit_index, generation, image, &mut marker);
    write_checked(backend, commit_block, [EMPTY_BYTE; BLOCK_BYTES], marker)?;
    Ok(generation)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SlotMeta {
    generation: u32,
    length: usize,
}

fn validate_image<E>(image: &[u8]) -> Result<(), PersistenceError<E>> {
    if image.len() > MAX_CANONICAL_IMAGE_BYTES {
        return Err(PersistenceError::ImageTooLarge);
    }
    decode_configuration_image(image)
        .map(|_| ())
        .map_err(|_| PersistenceError::InvalidImage)
}

fn inspect_slot<B: BlockBackend>(
    backend: &mut B,
    slot: usize,
    scratch: &mut SlotBuffer,
    image: &mut [u8; MAX_CANONICAL_IMAGE_BYTES],
) -> Result<Option<SlotMeta>, PersistenceError<B::Error>> {
    for (index, block) in scratch.iter_mut().enumerate() {
        backend
            .read_block(relative_block(slot, index), block)
            .map_err(PersistenceError::Backend)?;
    }
    Ok(valid_meta(scratch, image))
}

fn valid_meta(slot: &SlotBuffer, image: &mut [u8; MAX_CANONICAL_IMAGE_BYTES]) -> Option<SlotMeta> {
    if slot_byte(slot, 0) != MAGIC[0]
        || slot_byte(slot, 1) != MAGIC[1]
        || slot_byte(slot, 2) != MAGIC[2]
        || slot_byte(slot, 3) != MAGIC[3]
        || slot_byte(slot, 4) != FORMAT_VERSION
        || slot_byte(slot, 5) != 0
        || slot_byte(slot, 20) != 0
        || slot_byte(slot, 21) != 0
        || slot_byte(slot, 22) != 0
        || slot_byte(slot, 23) != 0
    {
        return None;
    }
    let commit = SLOT_BLOCKS - 1;
    if slot[commit].iter().all(|byte| *byte == EMPTY_BYTE) {
        return None;
    }
    let length = usize::from(u16::from_le_bytes([slot_byte(slot, 6), slot_byte(slot, 7)]));
    if length == 0 || length > MAX_CANONICAL_IMAGE_BYTES {
        return None;
    }
    let generation = u32::from_le_bytes([
        slot_byte(slot, 8),
        slot_byte(slot, 9),
        slot_byte(slot, 10),
        slot_byte(slot, 11),
    ]);
    if generation == 0 {
        return None;
    }
    let stored_crc = u32::from_le_bytes([
        slot_byte(slot, 12),
        slot_byte(slot, 13),
        slot_byte(slot, 14),
        slot_byte(slot, 15),
    ]);
    let header_crc = u32::from_le_bytes([
        slot_byte(slot, 16),
        slot_byte(slot, 17),
        slot_byte(slot, 18),
        slot_byte(slot, 19),
    ]);
    let mut header = [0; 16];
    for (index, byte) in header.iter_mut().enumerate() {
        *byte = slot_byte(slot, index);
    }
    copy_image(slot, length, image);
    if crc32(&header) != header_crc
        || crc32(&image[..length]) != stored_crc
        || decode_configuration_image(&image[..length]).is_err()
    {
        return None;
    }
    let commit_generation = u32::from_le_bytes([
        slot_byte(slot, commit * BLOCK_BYTES),
        slot_byte(slot, commit * BLOCK_BYTES + 1),
        slot_byte(slot, commit * BLOCK_BYTES + 2),
        slot_byte(slot, commit * BLOCK_BYTES + 3),
    ]);
    let commit_tag = u32::from_le_bytes([
        slot_byte(slot, commit * BLOCK_BYTES + 4),
        slot_byte(slot, commit * BLOCK_BYTES + 5),
        slot_byte(slot, commit * BLOCK_BYTES + 6),
        slot_byte(slot, commit * BLOCK_BYTES + 7),
    ]);
    if commit_generation != generation
        || commit_tag != commit_tag_for(generation, length, stored_crc)
    {
        return None;
    }
    Some(SlotMeta { generation, length })
}

fn newest_slot(metas: [Option<SlotMeta>; SLOT_COUNT]) -> Option<usize> {
    let mut newest = None;
    for slot in 0..SLOT_COUNT {
        let Some(candidate) = metas[slot] else {
            continue;
        };
        let Some(current_slot) = newest else {
            newest = Some(slot);
            continue;
        };
        let Some(current) = metas[current_slot] else {
            continue;
        };
        let difference = candidate.generation.wrapping_sub(current.generation);
        if difference != 0 && difference < 0x8000_0000 {
            newest = Some(slot);
        }
        // Equal and exact half-range ambiguity deterministically keeps the
        // lower relative slot index.
    }
    newest
}

fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

fn write_checked<B: BlockBackend>(
    backend: &mut B,
    block: u16,
    expected: [u8; BLOCK_BYTES],
    replacement: [u8; BLOCK_BYTES],
) -> Result<(), PersistenceError<B::Error>> {
    backend
        .write_verified(block, &expected, &replacement)
        .map_err(PersistenceError::Backend)?;
    let mut read_back = [0; BLOCK_BYTES];
    backend
        .read_block(block, &mut read_back)
        .map_err(PersistenceError::Backend)?;
    if read_back != replacement {
        return Err(PersistenceError::VerificationFailed);
    }
    Ok(())
}

fn encode_block(index: usize, generation: u32, image: &[u8], output: &mut [u8; BLOCK_BYTES]) {
    output.fill(EMPTY_BYTE);
    let offset = index * BLOCK_BYTES;
    if index == SLOT_BLOCKS - 1 {
        output[..4].copy_from_slice(&generation.to_le_bytes());
        output[4..8]
            .copy_from_slice(&commit_tag_for(generation, image.len(), crc32(image)).to_le_bytes());
    } else if offset < HEADER_BYTES {
        let mut header = [0; HEADER_BYTES];
        header[..4].copy_from_slice(&MAGIC);
        header[4] = FORMAT_VERSION;
        header[5] = 0;
        header[6..8].copy_from_slice(&u16::try_from(image.len()).unwrap_or(u16::MAX).to_le_bytes());
        header[8..12].copy_from_slice(&generation.to_le_bytes());
        header[12..16].copy_from_slice(&crc32(image).to_le_bytes());
        let header_crc = crc32(&header[..16]);
        header[16..20].copy_from_slice(&header_crc.to_le_bytes());
        output.copy_from_slice(&header[offset..offset + BLOCK_BYTES]);
    } else {
        let payload_offset = offset - HEADER_BYTES;
        if payload_offset < image.len() {
            let count = core::cmp::min(BLOCK_BYTES, image.len() - payload_offset);
            output[..count].copy_from_slice(&image[payload_offset..payload_offset + count]);
        }
    }
}

fn commit_tag_for(generation: u32, length: usize, image_crc: u32) -> u32 {
    let mut covered = [0; 15];
    covered[..4].copy_from_slice(&MAGIC);
    covered[4] = FORMAT_VERSION;
    covered[5..9].copy_from_slice(&generation.to_le_bytes());
    covered[9..11].copy_from_slice(&u16::try_from(length).unwrap_or(u16::MAX).to_le_bytes());
    covered[11..].copy_from_slice(&image_crc.to_le_bytes());
    let tag = crc32(&covered);
    if tag == 0 || tag == u32::MAX {
        tag ^ 1
    } else {
        tag
    }
}

fn copy_image(slot: &SlotBuffer, length: usize, output: &mut [u8; MAX_CANONICAL_IMAGE_BYTES]) {
    for (index, byte) in output.iter_mut().enumerate().take(length) {
        *byte = slot_byte(slot, HEADER_BYTES + index);
    }
}

fn relative_block(slot: usize, index: usize) -> u16 {
    u16::try_from(slot * SLOT_BLOCKS + index).unwrap_or(u16::MAX)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let low_bit_mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & low_bit_mask);
        }
    }
    !crc
}

fn slot_byte(slot: &SlotBuffer, offset: usize) -> u8 {
    slot[offset / BLOCK_BYTES][offset % BLOCK_BYTES]
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{encode_configuration_image, ObjectKey, ObjectKind, StorageObject};
    use radio_domain::{RadioConfig, RadioFlags, ScanResume, SquelchLevel};
    use std::{mem::size_of, vec, vec::Vec};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FakeError {
        Fault,
        Stale,
    }

    #[derive(Clone)]
    struct Fake {
        bytes: [u8; TOTAL_BYTES],
        operation: usize,
        fail_at: Option<usize>,
        interleave_once: bool,
        tamper_next_write: bool,
        fail_write_call: Option<usize>,
        write_calls: usize,
        writes: usize,
    }

    impl Fake {
        fn blank() -> Self {
            Self {
                bytes: [EMPTY_BYTE; TOTAL_BYTES],
                operation: 0,
                fail_at: None,
                interleave_once: false,
                tamper_next_write: false,
                fail_write_call: None,
                write_calls: 0,
                writes: 0,
            }
        }

        fn faulted(&self) -> bool {
            self.fail_at == Some(self.operation)
        }
    }

    impl BlockBackend for Fake {
        type Error = FakeError;

        fn read_block(
            &mut self,
            block: u16,
            output: &mut [u8; BLOCK_BYTES],
        ) -> Result<(), FakeError> {
            if self.faulted() {
                self.operation += 1;
                return Err(FakeError::Fault);
            }
            self.operation += 1;
            let start = usize::from(block) * BLOCK_BYTES;
            output.copy_from_slice(&self.bytes[start..start + BLOCK_BYTES]);
            Ok(())
        }

        fn write_verified(
            &mut self,
            block: u16,
            expected: &[u8; BLOCK_BYTES],
            replacement: &[u8; BLOCK_BYTES],
        ) -> Result<(), FakeError> {
            self.write_calls += 1;
            if self.faulted() {
                self.operation += 1;
                let start = usize::from(block) * BLOCK_BYTES;
                self.bytes[start..start + 3].copy_from_slice(&replacement[..3]);
                self.bytes[start + 3..start + BLOCK_BYTES].fill(EMPTY_BYTE);
                return Err(FakeError::Fault);
            }
            if self.fail_write_call == Some(self.write_calls) {
                let start = usize::from(block) * BLOCK_BYTES;
                self.bytes[start..start + 3].copy_from_slice(&replacement[..3]);
                self.bytes[start + 3..start + BLOCK_BYTES].fill(EMPTY_BYTE);
                return Err(FakeError::Fault);
            }
            self.operation += 1;
            let start = usize::from(block) * BLOCK_BYTES;
            if self.interleave_once {
                self.bytes[start] ^= 1;
                self.interleave_once = false;
            }
            if self.bytes[start..start + BLOCK_BYTES] != expected[..] {
                return Err(FakeError::Stale);
            }
            self.bytes[start..start + BLOCK_BYTES].copy_from_slice(replacement);
            self.writes += 1;
            if self.tamper_next_write {
                self.bytes[start] ^= 1;
                self.tamper_next_write = false;
            }
            Ok(())
        }
    }

    fn image() -> Vec<u8> {
        let config = RadioConfig::conservative();
        let object = StorageObject::new(
            ObjectKey {
                kind: ObjectKind::RadioConfig,
                id: 0,
            },
            crate::encode_radio_config(config).unwrap().data(),
        )
        .unwrap();
        let mut output = vec![0; crate::configuration_image_len(&[object]).unwrap()];
        encode_configuration_image(&[object], &mut output).unwrap();
        output
    }

    fn update_once(fake: &mut Fake, image: &[u8]) -> Result<u32, PersistenceError<FakeError>> {
        let mut slot = empty_slot();
        let mut image_scratch = [0; MAX_CANONICAL_IMAGE_BYTES];
        update(fake, image, &mut slot, &mut image_scratch)
    }

    #[test]
    fn geometry_is_aligned_and_within_model_region() {
        assert_eq!(SLOT_BYTES, 1_312);
        assert_eq!(TOTAL_BYTES, 2_624);
        assert!(size_of::<SlotBuffer>() <= 4_096);
    }

    #[test]
    fn exact_round_trip_and_newest_selection() {
        let mut fake = Fake::blank();
        let original = image();
        assert_eq!(update_once(&mut fake, &original), Ok(1));
        assert_eq!(update_once(&mut fake, &original), Ok(2));
        let mut slot = empty_slot();
        let mut output = [0; MAX_CANONICAL_IMAGE_BYTES];
        let loaded = load(&mut fake, &mut slot, &mut output).unwrap().unwrap();
        assert_eq!(loaded.bytes, original.as_slice());
        assert_eq!(loaded.generation, 2);
        assert_eq!(
            crate::decode_configuration_image(loaded.bytes)
                .unwrap()
                .object_count(),
            1
        );
    }

    #[test]
    fn atomic_stale_interleave_and_wrong_readback_are_fail_safe() {
        let original = image();
        let mut fake = Fake::blank();
        assert_eq!(update_once(&mut fake, &original), Ok(1));
        let writes = fake.writes;
        fake.interleave_once = true;
        assert_eq!(
            update_once(&mut fake, &original),
            Err(PersistenceError::Backend(FakeError::Stale))
        );
        assert_eq!(fake.writes, writes);
        fake.interleave_once = false;
        fake.tamper_next_write = true;
        assert_eq!(
            update_once(&mut fake, &original),
            Err(PersistenceError::VerificationFailed)
        );
    }

    #[test]
    fn commit_collision_and_reserved_bytes_are_rejected() {
        let original = image();
        let mut fake = Fake::blank();
        assert_eq!(update_once(&mut fake, &original), Ok(1));
        let header = 0;
        fake.bytes[header + 8..header + 12].copy_from_slice(&65_537_u32.to_le_bytes());
        let crc = crc32(&fake.bytes[header..header + 16]);
        fake.bytes[header + 16..header + 20].copy_from_slice(&crc.to_le_bytes());
        let mut slot = empty_slot();
        let mut output = [0; MAX_CANONICAL_IMAGE_BYTES];
        assert!(load(&mut fake, &mut slot, &mut output).unwrap().is_none());
        let mut fake = Fake::blank();
        assert_eq!(update_once(&mut fake, &original), Ok(1));
        fake.bytes[5] = 1;
        assert!(load(&mut fake, &mut slot, &mut output).unwrap().is_none());
    }

    #[test]
    fn all_ff_tombstone_is_rejected_before_generation_max_tag_parsing() {
        let original = image();
        let mut fake = Fake::blank();
        update_once(&mut fake, &original).unwrap();
        let mut slot = empty_slot();
        for (index, block) in slot.iter_mut().enumerate() {
            fake.read_block(relative_block(0, index), block).unwrap();
        }
        slot[1][0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut header = [0; 16];
        for (index, byte) in header.iter_mut().enumerate() {
            *byte = slot_byte(&slot, index);
        }
        let header_crc = crc32(&header);
        slot[2][0..4].copy_from_slice(&header_crc.to_le_bytes());
        slot[SLOT_BLOCKS - 1].fill(EMPTY_BYTE);
        let mut image_scratch = [0; MAX_CANONICAL_IMAGE_BYTES];
        assert!(valid_meta(&slot, &mut image_scratch).is_none());

        let mut marker = [0; BLOCK_BYTES];
        encode_block(SLOT_BLOCKS - 1, u32::MAX, &original, &mut marker);
        assert_ne!(marker, [EMPTY_BYTE; BLOCK_BYTES]);
    }

    #[test]
    fn newest_corruption_falls_back_and_both_corrupt_is_empty() {
        let original = image();
        let mut fake = Fake::blank();
        update_once(&mut fake, &original).unwrap();
        update_once(&mut fake, &original).unwrap();
        fake.bytes[SLOT_BYTES + HEADER_BYTES] ^= 1;
        let mut slot = empty_slot();
        let mut output = [0; MAX_CANONICAL_IMAGE_BYTES];
        assert_eq!(
            load(&mut fake, &mut slot, &mut output)
                .unwrap()
                .unwrap()
                .generation,
            1
        );
        fake.bytes[HEADER_BYTES] ^= 1;
        assert!(load(&mut fake, &mut slot, &mut output).unwrap().is_none());
    }

    #[test]
    fn every_interruption_and_pre_final_marker_preserves_active_image() {
        let original = image();
        let mut prepared = Fake::blank();
        update_once(&mut prepared, &original).unwrap();
        let mut probe = prepared.clone();
        probe.operation = 0;
        probe.fail_at = None;
        update_once(&mut probe, &original).unwrap();
        for failure in 0..probe.operation {
            let mut fake = prepared.clone();
            fake.operation = 0;
            fake.fail_at = Some(failure);
            let _ = update_once(&mut fake, &original);
            fake.fail_at = None;
            let mut slot = empty_slot();
            let mut output = [0; MAX_CANONICAL_IMAGE_BYTES];
            let loaded = load(&mut fake, &mut slot, &mut output).unwrap().unwrap();
            assert_eq!(loaded.bytes, original.as_slice());
            assert!(loaded.generation == 1 || loaded.generation == 2);
        }
    }

    #[test]
    fn tombstone_precedes_replacement_and_old_marker_cannot_survive() {
        let original = image();
        let mut prepared = Fake::blank();
        update_once(&mut prepared, &original).unwrap();
        update_once(&mut prepared, &original).unwrap();

        // The selected target is an already-valid old slot. A torn tombstone
        // must make it invalid, leaving the other generation loadable.
        let mut tombstone_fault = prepared.clone();
        tombstone_fault.operation = 0;
        tombstone_fault.write_calls = 0;
        tombstone_fault.fail_write_call = Some(1);
        let _ = update_once(&mut tombstone_fault, &original);
        tombstone_fault.fail_write_call = None;
        let mut slot = empty_slot();
        let mut output = [0; MAX_CANONICAL_IMAGE_BYTES];
        assert_eq!(
            load(&mut tombstone_fault, &mut slot, &mut output)
                .unwrap()
                .unwrap()
                .generation,
            2
        );

        // Read-back failure during the tombstone is also fail-safe.
        let mut tombstone_readback = prepared.clone();
        tombstone_readback.operation = 0;
        tombstone_readback.write_calls = 0;
        tombstone_readback.tamper_next_write = true;
        assert_eq!(
            update_once(&mut tombstone_readback, &original),
            Err(PersistenceError::VerificationFailed)
        );
        tombstone_readback.tamper_next_write = false;
        assert_eq!(
            load(&mut tombstone_readback, &mut slot, &mut output)
                .unwrap()
                .unwrap()
                .generation,
            2
        );

        // Tombstone + 163 header/payload writes precede the final marker.
        let mut final_marker_fault = prepared;
        final_marker_fault.operation = 0;
        final_marker_fault.write_calls = 0;
        final_marker_fault.fail_write_call = Some(165);
        let _ = update_once(&mut final_marker_fault, &original);
        final_marker_fault.fail_write_call = None;
        assert_eq!(
            load(&mut final_marker_fault, &mut slot, &mut output)
                .unwrap()
                .unwrap()
                .generation,
            2
        );
    }

    #[test]
    fn generation_zero_is_reserved_and_wrap_skips_it() {
        assert_eq!(next_generation(0), 1);
        assert_eq!(next_generation(u32::MAX), 1);
        let old = SlotMeta {
            generation: u32::MAX,
            length: 1,
        };
        let new = SlotMeta {
            generation: 1,
            length: 1,
        };
        assert_eq!(newest_slot([Some(old), Some(new)]), Some(1));
        let tie = SlotMeta {
            generation: 0x7FFF_FFFF,
            length: 1,
        };
        assert_eq!(newest_slot([Some(old), Some(tie)]), Some(0));
    }

    #[test]
    fn invalid_image_is_refused_before_backend_mutation() {
        let mut fake = Fake::blank();
        assert_eq!(
            update_once(&mut fake, b"not-canonical"),
            Err(PersistenceError::InvalidImage)
        );
        assert_eq!(fake.writes, 0);
    }

    #[allow(dead_code)]
    fn _keep_domain_imports_checked(_: (RadioConfig, RadioFlags, ScanResume, SquelchLevel)) {}
}
