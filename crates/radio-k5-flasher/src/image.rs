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

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"))
}

#[cfg(test)]
mod tests {
    use super::{crc32, ApplicationImage, EepromBackup, ImageError, APPLICATION_BYTES};

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
}
