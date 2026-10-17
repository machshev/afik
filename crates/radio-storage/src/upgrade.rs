//! One-way upgrade of older project files (`ADR-115`).
//!
//! A project file is a canonical configuration image. Version 4 stored each
//! channel's bank as a 16-bit membership mask, which caps a project at 16
//! banks; version 5 stores the bank's number. [`project`] rewrites a version 4
//! file as version 5 in place, and a studio saves only version 5, so the
//! conversion runs once per file and never back.

use crate::{
    configuration_image_crc, decode_configuration_image, decode_image_entry, ObjectKind,
    StorageError, CHANNEL_ENCODED_LEN, CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_MAGIC,
    CONFIGURATION_IMAGE_OBJECT_HEADER_LEN, CONFIGURATION_IMAGE_VERSION, NO_BANK_ID,
    STORAGE_FORMAT_VERSION,
};

/// The object version that stored a channel's bank as a membership mask.
pub const MASK_FORMAT_VERSION: u8 = 4;
/// Where a channel object holds its bank.
const CHANNEL_BANK_AT: usize = 40;

/// Rewrites a version 4 project file as version 5, in place.
///
/// Returns whether anything was rewritten: a current file is left as it is
/// and any other version is refused. The old file's checksum is checked
/// before a byte changes, a channel's mask becomes the number of its lowest
/// bank (an older studio could name several; the channel stays in the
/// first), every object and the header take the new version, the checksum is
/// recomputed, and the result is decoded once more as any current file is.
/// On an error the bytes may be part rewritten and must not be used.
pub fn project(bytes: &mut [u8]) -> Result<bool, StorageError> {
    if bytes.len() < CONFIGURATION_IMAGE_HEADER_LEN || bytes[..4] != CONFIGURATION_IMAGE_MAGIC {
        return Err(StorageError::MalformedImage);
    }
    if bytes[4] != CONFIGURATION_IMAGE_VERSION {
        return Err(StorageError::UnsupportedImageVersion);
    }
    match bytes[5] {
        STORAGE_FORMAT_VERSION => return Ok(false),
        MASK_FORMAT_VERSION => {}
        _ => return Err(StorageError::UnsupportedImageVersion),
    }
    let payload_len = usize::try_from(u32::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11],
    ]))
    .map_err(|_| StorageError::ImageTooLarge)?;
    if CONFIGURATION_IMAGE_HEADER_LEN.checked_add(payload_len) != Some(bytes.len()) {
        return Err(StorageError::MalformedImage);
    }
    let checksum = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let (header, payload) = bytes.split_at_mut(CONFIGURATION_IMAGE_HEADER_LEN);
    if configuration_image_crc(&header[..12], payload) != checksum {
        return Err(StorageError::ImageIntegrity);
    }

    let count = u16::from_le_bytes([header[6], header[7]]);
    let mut offset = 0;
    for _ in 0..count {
        let (object, end) = decode_image_entry(payload, offset)?;
        let kind = object.key().kind;
        let data = &mut payload[offset + CONFIGURATION_IMAGE_OBJECT_HEADER_LEN..end];
        if data.first() != Some(&MASK_FORMAT_VERSION) {
            return Err(StorageError::MalformedObject);
        }
        data[0] = STORAGE_FORMAT_VERSION;
        if kind == ObjectKind::Channel && data.len() == CHANNEL_ENCODED_LEN {
            let field = &mut data[CHANNEL_BANK_AT..CHANNEL_BANK_AT + 2];
            let mask = u16::from_le_bytes([field[0], field[1]]);
            // The lowest set bit of a 16-bit mask is at most 15.
            let bank = if mask == 0 {
                NO_BANK_ID
            } else {
                u16::try_from(mask.trailing_zeros()).unwrap_or(NO_BANK_ID)
            };
            field.copy_from_slice(&bank.to_le_bytes());
        }
        offset = end;
    }
    if offset != payload.len() {
        return Err(StorageError::MalformedImage);
    }
    header[5] = STORAGE_FORMAT_VERSION;
    let checksum = configuration_image_crc(&header[..12], payload);
    header[12..16].copy_from_slice(&checksum.to_le_bytes());
    decode_configuration_image(bytes)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{project, MASK_FORMAT_VERSION};
    use crate::{
        configuration_image_crc, decode_channel, decode_configuration_image, encode_channel,
        encode_channel_bank, encode_configuration_image, ObjectKind, StorageError, StorageObject,
        CONFIGURATION_IMAGE_HEADER_LEN, CONFIGURATION_IMAGE_OBJECT_HEADER_LEN,
        STORAGE_FORMAT_VERSION,
    };
    use radio_channel_plan::{
        BankFlags, BankName, ChannelBank, ChannelDefinition, ChannelFlags, ChannelName,
        ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, BankId, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, Tone,
    };
    use std::vec::Vec;

    fn channel(id: u16, bank: Option<u16>) -> ChannelRecord {
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("GB3CF").unwrap(),
            receive: Frequency::from_hz(145_612_500).unwrap(),
            transmit: Frequency::from_hz(145_012_500).unwrap(),
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low1,
            step: FrequencyStep::from_hz(12_500).unwrap(),
            flags: ChannelFlags::default(),
            bank: bank.map(BankId::new),
        })
        .unwrap()
    }

    /// A project file as a version 4 studio wrote it: the current encoding
    /// with each object's version byte, the channels' masks and the header
    /// set back, and the checksum resealed.
    fn version_four(masks: &[(u16, u16)]) -> Vec<u8> {
        let mut objects: Vec<StorageObject> = std::vec![encode_channel_bank(
            ChannelBank::new(
                BankId::new(3),
                BankName::new("RPT").unwrap(),
                BankFlags::default(),
            )
            .unwrap()
        )
        .unwrap()];
        for (id, _) in masks {
            objects.push(encode_channel(channel(*id, None)).unwrap());
        }
        objects.sort_by_key(|object| object.key());
        let mut bytes = std::vec![0_u8; 1024];
        let len = encode_configuration_image(&objects, &mut bytes).unwrap();
        bytes.truncate(len);
        bytes[5] = MASK_FORMAT_VERSION;
        let mut offset = CONFIGURATION_IMAGE_HEADER_LEN;
        while offset < bytes.len() {
            let id = u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]);
            let len = usize::from(u16::from_le_bytes([bytes[offset + 3], bytes[offset + 4]]));
            let data = offset + CONFIGURATION_IMAGE_OBJECT_HEADER_LEN;
            bytes[data] = MASK_FORMAT_VERSION;
            if bytes[offset] == ObjectKind::Channel as u8 {
                let (_, mask) = masks.iter().find(|(channel, _)| *channel == id).unwrap();
                bytes[data + 40..data + 42].copy_from_slice(&mask.to_le_bytes());
            }
            offset = data + len;
        }
        let checksum = configuration_image_crc(&bytes[..12], &bytes[16..]);
        bytes[12..16].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn a_version_four_project_becomes_version_five_once() {
        // In bank 3; in banks 2 and 9, kept in 2; in no bank; in bank 15.
        let mut bytes = version_four(&[(1, 1 << 3), (2, 1 << 2 | 1 << 9), (3, 0), (4, 1 << 15)]);
        assert_eq!(
            decode_configuration_image(&bytes).map(|_| ()),
            Err(StorageError::UnsupportedImageVersion),
            "the current decoder refuses it"
        );
        assert_eq!(project(&mut bytes), Ok(true));
        let image = decode_configuration_image(&bytes).expect("now current");
        let banks: Vec<_> = image
            .objects()
            .filter(|object| object.key().kind == ObjectKind::Channel)
            .map(|object| decode_channel(&object).unwrap().bank().map(BankId::get))
            .collect();
        assert_eq!(banks, [Some(3), Some(2), None, Some(15)]);
        assert_eq!(bytes[5], STORAGE_FORMAT_VERSION);

        // A current file is left alone.
        let upgraded = bytes.clone();
        assert_eq!(project(&mut bytes), Ok(false));
        assert_eq!(bytes, upgraded);
    }

    #[test]
    fn a_damaged_or_foreign_file_is_refused() {
        let mut damaged = version_four(&[(1, 1)]);
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert_eq!(project(&mut damaged), Err(StorageError::ImageIntegrity));

        let mut foreign = version_four(&[(1, 1)]);
        foreign[5] = 3;
        assert_eq!(
            project(&mut foreign),
            Err(StorageError::UnsupportedImageVersion)
        );

        assert_eq!(project(&mut [0_u8; 4]), Err(StorageError::MalformedImage));
    }
}
