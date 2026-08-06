//! Guarded firmware and EEPROM operations driven from the native editor.
//!
//! Every write reuses the recovery-gated `radio-flasher` workflows unchanged,
//! including their exact confirmation phrases. This module only collects
//! operator input, runs one operation on a worker thread, and reports progress.

use std::{
    fmt,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use radio_flasher::{
    k1, k1::K1RecoveryImage, ApplicationImage, EepromBackup, FirmwareVersion, FlashPrerequisites,
    FlashPurpose,
};
use radio_programmer_serial::LinuxSerialTransport;

/// Serial baud used by every supported bootloader path.
pub const FLASH_BAUD: u32 = 38_400;
/// Largest firmware image the editor will read from disk.
pub const MAX_FIRMWARE_BYTES: usize = 256 * 1024;
/// Exact EEPROM backup size for the UV-K5 V1 path.
pub const EEPROM_BYTES: usize = 8 * 1024;

/// Which guarded operation to run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashOperation {
    /// Read the complete EEPROM without writing anything.
    BackupEeprom,
    /// Write the known-good recovery image to a UV-K1.
    K1Recovery,
    /// Write an AFIK application to a UV-K1 after a recovery rehearsal.
    K1Application,
    /// Write an application to a UV-K5 V1 after a recovery rehearsal.
    K5Application,
}

impl FlashOperation {
    /// Returns the editor label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::BackupEeprom => "Back up EEPROM (read only)",
            Self::K1Recovery => "Flash K1 recovery image",
            Self::K1Application => "Flash K1 AFIK application",
            Self::K5Application => "Flash K5 V1 application",
        }
    }

    /// Reports whether the operation writes to the radio.
    pub const fn is_write(self) -> bool {
        !matches!(self, Self::BackupEeprom)
    }

    /// Returns every operation in display order.
    pub const fn all() -> [Self; 4] {
        [
            Self::BackupEeprom,
            Self::K1Recovery,
            Self::K1Application,
            Self::K5Application,
        ]
    }
}

/// Complete operator-supplied inputs for one guarded operation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlashRequest {
    /// Explicit serial device path.
    pub device: PathBuf,
    /// Firmware image to write, when the operation writes firmware.
    pub firmware: PathBuf,
    /// Known-good recovery image retained for the exact unit.
    pub recovery: PathBuf,
    /// Retained EEPROM/calibration backup for the exact unit.
    pub eeprom_backup: PathBuf,
    /// Destination for a fresh EEPROM backup.
    pub eeprom_output: PathBuf,
    /// Exact target confirmation phrase.
    pub target_confirmation: String,
    /// Exact recovery-rehearsed confirmation phrase.
    pub recovery_rehearsed_confirmation: String,
    /// Bootloader version observed while identifying the radio.
    pub bootloader_version: String,
    /// Non-zero per-run transaction identifier.
    pub transaction_id: u32,
    /// Operator-entered CRC-32 of the selected K5 image.
    pub image_crc32: u32,
    /// Negotiated K5 firmware version string.
    pub firmware_version: String,
}

/// One progress or completion message from the worker thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlashProgress {
    /// A page or block was acknowledged.
    Step {
        /// Completed units.
        done: u16,
        /// Total units for this operation, when known.
        total: u16,
    },
    /// The operation finished successfully with a summary line.
    Finished(String),
    /// The operation failed without completing.
    Failed(String),
}

/// A guarded operation could not start or complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlashRequestError(String);

impl fmt::Display for FlashRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FlashRequestError {
    fn new(detail: impl Into<String>) -> Self {
        Self(detail.into())
    }
}

/// A running guarded operation.
pub struct FlashJob {
    operation: FlashOperation,
    progress: Receiver<FlashProgress>,
}

impl FlashJob {
    /// Returns the running operation.
    pub const fn operation(&self) -> FlashOperation {
        self.operation
    }

    /// Drains every progress message available without blocking.
    pub fn drain(&self) -> Vec<FlashProgress> {
        self.progress.try_iter().collect()
    }
}

/// Validates one request without opening the serial device.
///
/// This runs the same phrase, identifier, and file checks the worker performs,
/// so the editor can refuse an incomplete request before touching hardware.
pub fn validate_request(
    operation: FlashOperation,
    request: &FlashRequest,
) -> Result<(), FlashRequestError> {
    if request.device.as_os_str().is_empty() {
        return Err(FlashRequestError::new("select a serial device path"));
    }
    match operation {
        FlashOperation::BackupEeprom => {
            if request.eeprom_output.as_os_str().is_empty() {
                return Err(FlashRequestError::new(
                    "choose an EEPROM backup output path",
                ));
            }
            if request.eeprom_output.exists() {
                return Err(FlashRequestError::new(
                    "the EEPROM backup output path already exists",
                ));
            }
        }
        FlashOperation::K1Recovery | FlashOperation::K1Application => {
            require_file(&request.firmware, "firmware image")?;
            if request.transaction_id == 0 {
                return Err(FlashRequestError::new(
                    "enter a non-zero transaction identifier",
                ));
            }
            if !k1::is_supported_bootloader_version(&request.bootloader_version) {
                return Err(FlashRequestError::new(
                    "enter the observed 7.03.x bootloader version",
                ));
            }
            if request.target_confirmation.trim().is_empty() {
                return Err(FlashRequestError::new("enter the target confirmation"));
            }
            if matches!(operation, FlashOperation::K1Application) {
                require_file(&request.recovery, "recovery image")?;
                if request.recovery_rehearsed_confirmation.trim().is_empty() {
                    return Err(FlashRequestError::new(
                        "enter the recovery-rehearsed confirmation",
                    ));
                }
            }
        }
        FlashOperation::K5Application => {
            require_file(&request.firmware, "firmware image")?;
            require_file(&request.recovery, "recovery image")?;
            require_file(&request.eeprom_backup, "EEPROM backup")?;
            if request.transaction_id == 0 {
                return Err(FlashRequestError::new(
                    "enter a non-zero transaction identifier",
                ));
            }
            if request.target_confirmation.trim().is_empty() {
                return Err(FlashRequestError::new("enter the target confirmation"));
            }
            if request.recovery_rehearsed_confirmation.trim().is_empty() {
                return Err(FlashRequestError::new(
                    "enter the recovery-rehearsed confirmation",
                ));
            }
            FirmwareVersion::new(&request.firmware_version)
                .map_err(|error| FlashRequestError::new(error.to_string()))?;
        }
    }
    Ok(())
}

/// Starts one guarded operation on a worker thread.
pub fn start(
    operation: FlashOperation,
    request: FlashRequest,
) -> Result<FlashJob, FlashRequestError> {
    validate_request(operation, &request)?;
    let (sender, progress) = mpsc::channel();
    thread::Builder::new()
        .name("afik-flash".to_owned())
        .spawn(move || {
            let outcome = match run(operation, &request, &sender) {
                Ok(summary) => FlashProgress::Finished(summary),
                Err(error) => FlashProgress::Failed(error.to_string()),
            };
            let _ignored = sender.send(outcome);
        })
        .map_err(|error| FlashRequestError::new(format!("could not start worker: {error}")))?;
    Ok(FlashJob {
        operation,
        progress,
    })
}

fn run(
    operation: FlashOperation,
    request: &FlashRequest,
    sender: &Sender<FlashProgress>,
) -> Result<String, FlashRequestError> {
    let mut serial = LinuxSerialTransport::open(&request.device, FLASH_BAUD)
        .map_err(|error| FlashRequestError::new(format!("serial setup failed: {error}")))?;

    match operation {
        FlashOperation::BackupEeprom => {
            let (info, backup) = radio_flasher::backup_eeprom(&mut serial)
                .map_err(|error| FlashRequestError::new(error.to_string()))?;
            write_new_file(&request.eeprom_output, backup.bytes())?;
            Ok(format!(
                "backed up {} EEPROM bytes from firmware {}",
                backup.bytes().len(),
                info.version()
            ))
        }
        FlashOperation::K1Recovery => {
            let image = read_k1_image(&request.firmware)?;
            let total = image.page_count();
            let report = k1::flash_recovery(
                &mut serial,
                &image,
                &request.bootloader_version,
                &request.target_confirmation,
                request.transaction_id,
                |page| step(sender, page, total),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} recovery pages under transaction {:08x}",
                report.pages_acknowledged, report.transaction_id
            ))
        }
        FlashOperation::K1Application => {
            let image = read_k1_image(&request.firmware)?;
            let recovery = read_k1_image(&request.recovery)?;
            let total = image.page_count();
            let report = k1::flash_application(
                &mut serial,
                &image,
                &recovery,
                &request.bootloader_version,
                k1::K1ApplicationConfirmations {
                    target: &request.target_confirmation,
                    recovery_rehearsed: &request.recovery_rehearsed_confirmation,
                },
                request.transaction_id,
                |page| step(sender, page, total),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} application pages under transaction {:08x}",
                report.pages_acknowledged, report.transaction_id
            ))
        }
        FlashOperation::K5Application => {
            let image = read_k5_image(&request.firmware)?;
            let recovery = read_k5_image(&request.recovery)?;
            let backup = read_eeprom_backup(&request.eeprom_backup)?;
            let version = FirmwareVersion::new(&request.firmware_version)
                .map_err(|error| FlashRequestError::new(error.to_string()))?;
            let report = radio_flasher::flash_application(
                &mut serial,
                FlashPrerequisites {
                    image: &image,
                    recovery_image: &recovery,
                    eeprom_backup: &backup,
                    version: &version,
                    target_confirmation: &request.target_confirmation,
                    image_crc32_confirmation: request.image_crc32,
                    transaction_id: request.transaction_id,
                    purpose: FlashPurpose::Afik {
                        recovery_rehearsed_confirmation: &request.recovery_rehearsed_confirmation,
                    },
                },
                |page| step(sender, page, 240),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} application pages with CRC-32 {:08x}",
                report.pages_acknowledged, report.image_crc32
            ))
        }
    }
}

fn step(sender: &Sender<FlashProgress>, page: u16, total: u16) {
    let _ignored = sender.send(FlashProgress::Step {
        done: page.saturating_add(1),
        total,
    });
}

fn require_file(path: &Path, description: &str) -> Result<(), FlashRequestError> {
    if path.as_os_str().is_empty() {
        return Err(FlashRequestError::new(format!("select a {description}")));
    }
    if !path.is_file() {
        return Err(FlashRequestError::new(format!(
            "{description} {} is not a readable file",
            path.display()
        )));
    }
    Ok(())
}

fn read_bounded(
    path: &Path,
    maximum: usize,
    description: &str,
) -> Result<Vec<u8>, FlashRequestError> {
    let file = File::open(path).map_err(|error| {
        FlashRequestError::new(format!("could not open {description}: {error}"))
    })?;
    let limit = u64::try_from(maximum + 1).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).map_err(|error| {
        FlashRequestError::new(format!("could not read {description}: {error}"))
    })?;
    if bytes.len() > maximum {
        return Err(FlashRequestError::new(format!(
            "{description} exceeds {maximum} bytes"
        )));
    }
    Ok(bytes)
}

fn read_k1_image(path: &Path) -> Result<K1RecoveryImage, FlashRequestError> {
    let bytes = read_bounded(path, MAX_FIRMWARE_BYTES, "firmware image")?;
    K1RecoveryImage::from_raw(&bytes).map_err(|error| FlashRequestError::new(error.to_string()))
}

fn read_k5_image(path: &Path) -> Result<ApplicationImage, FlashRequestError> {
    let bytes = read_bounded(path, MAX_FIRMWARE_BYTES, "firmware image")?;
    ApplicationImage::from_raw(&bytes).map_err(|error| FlashRequestError::new(error.to_string()))
}

fn read_eeprom_backup(path: &Path) -> Result<EepromBackup, FlashRequestError> {
    let bytes = read_bounded(path, EEPROM_BYTES, "EEPROM backup")?;
    EepromBackup::from_raw(&bytes).map_err(|error| FlashRequestError::new(error.to_string()))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), FlashRequestError> {
    if path.exists() {
        return Err(FlashRequestError::new(format!(
            "{} already exists",
            path.display()
        )));
    }
    fs::write(path, bytes)
        .map_err(|error| FlashRequestError::new(format!("could not write backup: {error}")))
}

/// Lists plausible serial device paths without opening any of them.
pub fn discover_serial_devices() -> Vec<PathBuf> {
    let mut devices = Vec::new();
    for directory in ["/dev/serial/by-id", "/dev"] {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let plausible = directory == "/dev/serial/by-id"
                || name.starts_with("ttyUSB")
                || name.starts_with("ttyACM");
            if plausible {
                devices.push(path);
            }
        }
    }
    devices.sort();
    devices.dedup();
    devices
}

#[cfg(test)]
mod tests {
    use super::{validate_request, FlashOperation, FlashRequest};
    use std::path::PathBuf;

    fn request() -> FlashRequest {
        FlashRequest {
            device: PathBuf::from("/dev/ttyUSB0"),
            transaction_id: 1,
            bootloader_version: "7.03.01".to_owned(),
            target_confirmation: "confirmed".to_owned(),
            recovery_rehearsed_confirmation: "rehearsed".to_owned(),
            ..FlashRequest::default()
        }
    }

    #[test]
    fn every_operation_requires_a_device() {
        let mut request = request();
        request.device = PathBuf::new();
        for operation in FlashOperation::all() {
            assert!(validate_request(operation, &request).is_err());
        }
    }

    #[test]
    fn backups_require_a_fresh_output_path() {
        let mut request = request();
        assert!(validate_request(FlashOperation::BackupEeprom, &request).is_err());
        request.eeprom_output = PathBuf::from("/dev/null");
        assert!(validate_request(FlashOperation::BackupEeprom, &request)
            .unwrap_err()
            .to_string()
            .contains("already exists"));
    }

    #[test]
    fn writes_require_files_identifiers_and_confirmations() {
        let mut request = request();
        // No firmware file exists yet.
        assert!(validate_request(FlashOperation::K1Recovery, &request).is_err());

        let existing = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/flash.rs"));
        request.firmware = existing.clone();
        request.recovery = existing.clone();
        request.eeprom_backup = existing;
        validate_request(FlashOperation::K1Recovery, &request).unwrap();

        let mut no_transaction = request.clone();
        no_transaction.transaction_id = 0;
        assert!(validate_request(FlashOperation::K1Recovery, &no_transaction).is_err());

        let mut bad_bootloader = request.clone();
        bad_bootloader.bootloader_version = "2.00.06".to_owned();
        assert!(validate_request(FlashOperation::K1Recovery, &bad_bootloader).is_err());

        let mut no_rehearsal = request.clone();
        no_rehearsal.recovery_rehearsed_confirmation = String::new();
        assert!(validate_request(FlashOperation::K1Application, &no_rehearsal).is_err());
        validate_request(FlashOperation::K1Application, &request).unwrap();

        let mut bad_version = request.clone();
        bad_version.firmware_version = "*".to_owned();
        assert!(validate_request(FlashOperation::K5Application, &bad_version).is_err());
        let mut k5 = request;
        k5.firmware_version = "2.01.26".to_owned();
        validate_request(FlashOperation::K5Application, &k5).unwrap();
    }

    #[test]
    fn only_the_backup_operation_is_read_only() {
        assert!(!FlashOperation::BackupEeprom.is_write());
        assert!(FlashOperation::K1Recovery.is_write());
        assert!(FlashOperation::K1Application.is_write());
        assert!(FlashOperation::K5Application.is_write());
    }
}
