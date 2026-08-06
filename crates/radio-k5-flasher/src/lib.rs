//! Recovery-gated host deployment for a qualified UV-K5 V1/DP32G030 radio.
//!
//! This crate implements a bounded legacy serial protocol from recorded wire
//! evidence. It does not identify hardware, replace the stock bootloader, write
//! EEPROM, read flash back, or prove that an acknowledged application boots.

#![forbid(unsafe_code)]

mod codec;
mod image;
mod workflow;

pub use image::{
    crc32, ApplicationImage, EepromBackup, ImageError, APPLICATION_BYTES, EEPROM_BYTES,
    FLASH_PAGE_BYTES, FLASH_PAGE_COUNT,
};
pub use workflow::{
    backup_eeprom, flash_application, probe_bootloader_v2, BootloaderInfo, FirmwareVersion,
    FlashError, FlashPrerequisites, FlashPurpose, FlashReport, NormalFirmwareInfo,
    QUALIFIED_TARGET_CONFIRMATION, RECOVERY_REHEARSED_CONFIRMATION,
};
