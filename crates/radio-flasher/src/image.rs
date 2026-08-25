use std::{error::Error, fmt};

/// Complete qualified UV-K5 V1 application region below the stock bootloader.
pub const APPLICATION_BYTES: usize = 0xF000;
/// Complete external EEPROM backup length observed on the qualified target.
pub const EEPROM_BYTES: usize = 0x2000;
/// One version-2 bootloader application page.
pub const FLASH_PAGE_BYTES: usize = 0x100;
/// Number of complete pages in the qualified application region.
pub const FLASH_PAGE_COUNT: usize = APPLICATION_BYTES / FLASH_PAGE_BYTES;

const RAM_START: u32 = 0x2000_0000;
const RAM_END_EXCLUSIVE: u32 = 0x2000_4000;

/// Rejection from bounded application-image or EEPROM-backup validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageError {
    /// A raw application is shorter than its two required reset vectors.
    ApplicationTooShort(usize),
    /// A raw application would extend into the preserved bootloader.
    ApplicationTooLong(usize),
    /// The initial stack pointer is not aligned inside the evidenced RAM.
    InvalidInitialStack(u32),
    /// The Reset vector is not a Thumb address inside supplied application data.
    InvalidResetVector(u32),
    /// An EEPROM backup is not exactly the complete observed EEPROM length.
    InvalidEepromLength(usize),
    /// A uniform EEPROM file is not accepted as a credible physical backup.
    UniformEeprom(u8),
}

impl fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApplicationTooShort(length) => {
                write!(
                    formatter,
                    "application is shorter than two vectors: {length}"
                )
            }
            Self::ApplicationTooLong(length) => write!(
                formatter,
                "application reaches the reserved bootloader region: {length}"
            ),
            Self::InvalidInitialStack(value) => {
                write!(
                    formatter,
                    "invalid DP32G030 initial stack vector: 0x{value:08x}"
                )
            }
            Self::InvalidResetVector(value) => {
                write!(formatter, "invalid DP32G030 Reset vector: 0x{value:08x}")
            }
            Self::InvalidEepromLength(length) => {
                write!(formatter, "EEPROM backup is not 0x2000 bytes: {length}")
            }
            Self::UniformEeprom(value) => write!(
                formatter,
                "EEPROM backup is uniformly 0x{value:02x}, so it is not credible"
            ),
        }
    }
}

impl Error for ImageError {}

/// A vector-checked, fully padded application ready for all 240 page writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationImage {
    bytes: Vec<u8>,
    source_len: usize,
    crc32: u32,
    initial_stack: u32,
    reset_vector: u32,
}

impl ApplicationImage {
    /// Validates raw application bytes and pads unused application space with
    /// `0xFF`. The preserved bootloader is never represented in this value.
    pub fn from_raw(raw: &[u8]) -> Result<Self, ImageError> {
        if raw.len() < 8 {
            return Err(ImageError::ApplicationTooShort(raw.len()));
        }
        if raw.len() > APPLICATION_BYTES {
            return Err(ImageError::ApplicationTooLong(raw.len()));
        }

        let initial_stack = read_u32(raw, 0);
        if initial_stack <= RAM_START || initial_stack > RAM_END_EXCLUSIVE || initial_stack % 4 != 0
        {
            return Err(ImageError::InvalidInitialStack(initial_stack));
        }

        let reset_vector = read_u32(raw, 4);
        let reset_address = reset_vector & !1;
        if reset_vector & 1 == 0 || reset_address < 8 || reset_address as usize >= raw.len() {
            return Err(ImageError::InvalidResetVector(reset_vector));
        }

        let mut bytes = vec![0xFF; APPLICATION_BYTES];
        bytes[..raw.len()].copy_from_slice(raw);
        let crc32 = crc32(&bytes);
        Ok(Self {
            bytes,
            source_len: raw.len(),
            crc32,
            initial_stack,
            reset_vector,
        })
    }

    /// Returns the complete application bytes; this is always exactly 60 KiB.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of raw bytes supplied before `0xFF` padding.
    pub const fn source_len(&self) -> usize {
        self.source_len
    }

    /// Returns the exact raw bytes supplied before application padding.
    pub fn source_bytes(&self) -> &[u8] {
        &self.bytes[..self.source_len]
    }

    /// Returns the complete padded-image CRC-32 selection guard.
    pub const fn crc32(&self) -> u32 {
        self.crc32
    }

    /// Returns the checked initial stack vector.
    pub const fn initial_stack(&self) -> u32 {
        self.initial_stack
    }

    /// Returns the checked Thumb Reset vector.
    pub const fn reset_vector(&self) -> u32 {
        self.reset_vector
    }
}

/// A complete, non-uniform, read-only EEPROM/calibration backup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EepromBackup {
    bytes: Vec<u8>,
    crc32: u32,
}

impl EepromBackup {
    /// Validates one complete raw EEPROM backup.
    pub fn from_raw(raw: &[u8]) -> Result<Self, ImageError> {
        if raw.len() != EEPROM_BYTES {
            return Err(ImageError::InvalidEepromLength(raw.len()));
        }
        if raw.iter().all(|byte| *byte == raw[0]) {
            return Err(ImageError::UniformEeprom(raw[0]));
        }
        Ok(Self {
            bytes: raw.to_vec(),
            crc32: crc32(raw),
        })
    }

    /// Returns the exact 8 KiB backup bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the backup CRC-32 for recording and accidental-selection checks.
    pub const fn crc32(&self) -> u32 {
        self.crc32
    }
}

/// Computes reflected CRC-32/ISO-HDLC with the standard initial/final XOR.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Computes SHA-256 for a bounded host artifact.
///
/// # Panics
///
/// This function only uses fixed-size chunks and cannot panic for a valid
/// slice; the internal conversion assertions document that invariant.
#[allow(clippy::too_many_lines)]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = (bytes.len() as u64) * 8;
    let padded_len = (bytes.len() + 9).div_ceil(64) * 64;
    let mut padded = vec![0_u8; padded_len];
    padded[..bytes.len()].copy_from_slice(bytes);
    padded[bytes.len()] = 0x80;
    padded[padded_len - 8..].copy_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0_u32; 64];
        for (word, bytes) in schedule[..16].iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("four bytes"));
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }
        let mut working = state;
        for index in 0..64 {
            let choice = (working[4] & working[5]) ^ (!working[4] & working[6]);
            let majority =
                (working[0] & working[1]) ^ (working[0] & working[2]) ^ (working[1] & working[2]);
            let big0 = working[0].rotate_right(2)
                ^ working[0].rotate_right(13)
                ^ working[0].rotate_right(22);
            let big1 = working[4].rotate_right(6)
                ^ working[4].rotate_right(11)
                ^ working[4].rotate_right(25);
            let temp1 = working[7]
                .wrapping_add(big1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let temp2 = big0.wrapping_add(majority);
            working[7] = working[6];
            working[6] = working[5];
            working[5] = working[4];
            working[4] = working[3].wrapping_add(temp1);
            working[3] = working[2];
            working[2] = working[1];
            working[1] = working[0];
            working[0] = temp1.wrapping_add(temp2);
        }
        for (value, addend) in state.iter_mut().zip(working) {
            *value = (*value).wrapping_add(addend);
        }
    }
    let mut output = [0_u8; 32];
    for (bytes, value) in output.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }
    output
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

#[cfg(test)]
mod tests {
    use super::{crc32, sha256, ApplicationImage, EepromBackup, ImageError, APPLICATION_BYTES};

    fn valid_raw(length: usize) -> Vec<u8> {
        let mut bytes = vec![0xAA; length];
        bytes[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&9_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn image_is_vector_checked_and_fully_ff_padded() {
        let image = ApplicationImage::from_raw(&valid_raw(32)).unwrap();
        assert_eq!(image.source_len(), 32);
        assert_eq!(image.bytes().len(), APPLICATION_BYTES);
        assert!(image.bytes()[32..].iter().all(|byte| *byte == 0xFF));
        assert_eq!(image.initial_stack(), 0x2000_4000);
        assert_eq!(image.reset_vector(), 9);
        assert_eq!(image.crc32(), crc32(image.bytes()));
    }

    #[test]
    fn image_rejects_bounds_stack_and_reset_failures() {
        assert_eq!(
            ApplicationImage::from_raw(&[0; 7]),
            Err(ImageError::ApplicationTooShort(7))
        );
        assert!(matches!(
            ApplicationImage::from_raw(&valid_raw(APPLICATION_BYTES + 1)),
            Err(ImageError::ApplicationTooLong(_))
        ));

        let mut raw = valid_raw(32);
        raw[0..4].copy_from_slice(&0x2000_0000_u32.to_le_bytes());
        assert!(matches!(
            ApplicationImage::from_raw(&raw),
            Err(ImageError::InvalidInitialStack(_))
        ));

        let mut raw = valid_raw(32);
        raw[4..8].copy_from_slice(&8_u32.to_le_bytes());
        assert!(matches!(
            ApplicationImage::from_raw(&raw),
            Err(ImageError::InvalidResetVector(_))
        ));
    }

    #[test]
    fn eeprom_backup_is_complete_and_non_uniform() {
        let mut raw = vec![0xFF; 0x2000];
        assert_eq!(
            EepromBackup::from_raw(&raw),
            Err(ImageError::UniformEeprom(0xFF))
        );
        raw[0] = 0;
        let backup = EepromBackup::from_raw(&raw).unwrap();
        assert_eq!(backup.bytes(), raw);
        assert_eq!(backup.crc32(), crc32(&raw));
        assert!(matches!(
            EepromBackup::from_raw(&raw[..0x1FFF]),
            Err(ImageError::InvalidEepromLength(_))
        ));
    }

    #[test]
    fn crc32_matches_standard_check_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn sha256_matches_standard_check_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }
}
