//! Recovery-gated host flashing for qualified UV-K5 V1 and UV-K1 radios.
//!
//! This crate implements bounded legacy serial protocols from recorded wire
//! evidence. Normal-mode backup is read-only; the explicitly selected,
//! independently guarded restore API can write EEPROM and is never part of
//! auto-detection. The crate does not identify hardware, replace the stock
//! bootloader, read flash back, or prove that an acknowledged application
//! boots.

#![forbid(unsafe_code)]

mod codec;
mod image;
pub mod k1;
pub mod k5;
mod workflow;

pub use codec::{receive_packet, send_packet, Packet};
pub use image::{
    crc32, sha256, ApplicationImage, EepromBackup, ImageError, APPLICATION_BYTES, EEPROM_BYTES,
    FLASH_PAGE_BYTES, FLASH_PAGE_COUNT,
};
pub use workflow::{
    afik_image_version, apply_eeprom_restore, backup_eeprom, detect_bootloader,
    flash_afik_application, flash_application, force_bootloader_family, identify_radio,
    observe_bootloader, prepare_eeprom_restore, probe_bootloader_v2, probe_clock_control,
    probe_clock_register, probe_clock_registers, probe_clock_snapshot, probe_keypad_matrix,
    probe_normal_firmware, probe_rf, read_afik_image, serves_eeprom_reads, set_rf_audio,
    write_afik_image, write_afik_place, BeaconObservation, BeaconProtocol, BootloaderFamily,
    BootloaderInfo, ClockSnapshotReport, EepromRestorePlan, EepromRestorePrerequisites,
    EepromRestoreReport, FirmwareVersion, FlashError, FlashPrerequisites, FlashPurpose,
    FlashReport, ForcedFamily, ImageWriteReport, KeypadMatrixReport, NormalFirmwareInfo,
    RadioPresence, RfReport, VersionCheck, DEFAULT_NEGOTIATION_VERSION, K5_RECOVERY_SOURCE_BYTES,
    QUALIFIED_TARGET_CONFIRMATION, RECOVERY_REHEARSED_CONFIRMATION, WILDCARD_NEGOTIATION_VERSION,
};
