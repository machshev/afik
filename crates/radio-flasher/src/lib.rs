//! Recovery-gated host flashing for qualified UV-K5 V1 and UV-K1 radios.
//!
//! This crate implements bounded legacy serial protocols from recorded wire
//! evidence. It classifies supported bootloader protocol families, but does not
//! identify hardware, replace the stock bootloader, write EEPROM, read flash
//! back, or prove that an acknowledged application boots.

#![forbid(unsafe_code)]

mod codec;
mod image;
pub mod k1;
mod workflow;

pub use codec::{receive_packet, send_packet, Packet};
pub use image::{
    crc32, ApplicationImage, EepromBackup, ImageError, APPLICATION_BYTES, EEPROM_BYTES,
    FLASH_PAGE_BYTES, FLASH_PAGE_COUNT,
};
pub use workflow::{
    backup_eeprom, detect_bootloader, flash_application, probe_bootloader_v2, probe_clock_control,
    probe_clock_register, probe_clock_registers, probe_clock_snapshot, probe_keypad_matrix,
    probe_normal_firmware, BootloaderFamily, BootloaderInfo, ClockSnapshotReport, FirmwareVersion,
    FlashError, FlashPrerequisites, FlashPurpose, FlashReport, KeypadMatrixReport,
    NormalFirmwareInfo, QUALIFIED_TARGET_CONFIRMATION, RECOVERY_REHEARSED_CONFIRMATION,
};
