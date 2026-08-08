//! Guarded firmware and EEPROM operations driven from the native editor.
//!
//! Every write reuses the recovery-gated `radio-flasher` workflows unchanged,
//! including their exact confirmation phrases. This module only collects
//! operator input, runs one operation on a worker thread, and reports progress.
//!
//! Firmware cannot be read back. The bootloader protocols this crate drives
//! carry no flash-read command, so a page acknowledgement is the only evidence a
//! write produced and there is no firmware backup to take. What protects a unit
//! is the retained known-good recovery image and the retained EEPROM backup,
//! which the recovery and K5 paths require and report the digest of before
//! starting. The K1 application path does not: it addresses a page index rather
//! than an address and issues no EEPROM operation, so it cannot reach the
//! bootloader and a bad application is recovered by writing another one. Both
//! artefacts stay optional there and are validated when supplied.

use std::{
    fmt,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use radio_flasher::{
    crc32, detect_bootloader, k1, k1::K1RecoveryImage, ApplicationImage, BootloaderFamily,
    EepromBackup, FirmwareVersion, FlashPrerequisites, FlashPurpose,
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

/// What a read-only identification found on the serial port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadioIdentity {
    /// Bootloader protocol family label.
    pub family: String,
    /// Bootloader version the device reported.
    pub version: String,
}

/// Classifies the connected bootloader without writing anything.
///
/// This is the same read-only classification the flasher CLI performs before a
/// write, so the operator can read the bootloader version off the radio instead
/// of typing it from memory into a confirmation field.
pub fn identify(device: &Path) -> Result<RadioIdentity, FlashRequestError> {
    if device.as_os_str().is_empty() {
        return Err(FlashRequestError::new("select a serial device path"));
    }
    let mut serial = LinuxSerialTransport::open(device, FLASH_BAUD)
        .map_err(|error| FlashRequestError::new(format!("serial setup failed: {error}")))?;
    let family = detect_bootloader(&mut serial)
        .map_err(|error| FlashRequestError::new(error.to_string()))?;
    let version = match &family {
        BootloaderFamily::K1(info) | BootloaderFamily::K5V1(info) => info.version().to_owned(),
    };
    Ok(RadioIdentity {
        family: family.label().to_owned(),
        version,
    })
}

/// Returns the byte length and CRC-32 of one retained artefact.
///
/// The operator confirms the retained recovery image and EEPROM backup by their
/// digest, which is the only evidence available that the files on disk are the
/// pair taken from this exact unit.
pub fn artefact_digest(path: &Path) -> Result<(usize, u32), FlashRequestError> {
    let bytes = read_bounded(path, MAX_FIRMWARE_BYTES, "artefact")?;
    Ok((bytes.len(), crc32(&bytes)))
}

/// Returns one fresh non-zero transaction identifier.
///
/// The bootloader ties every page acknowledgement to this word, so it must not
/// be reused between runs. Generating it removes the operator's opportunity to
/// retype a previous one.
pub fn fresh_transaction_id() -> Result<u32, FlashRequestError> {
    let mut random = File::open("/dev/urandom")
        .map_err(|error| FlashRequestError::new(format!("could not read /dev/urandom: {error}")))?;
    for _ in 0..4 {
        let mut bytes = [0_u8; 4];
        random.read_exact(&mut bytes).map_err(|error| {
            FlashRequestError::new(format!("could not read /dev/urandom: {error}"))
        })?;
        let transaction_id = u32::from_le_bytes(bytes);
        if transaction_id != 0 {
            return Ok(transaction_id);
        }
    }
    Err(FlashRequestError::new(
        "could not obtain a nonzero transaction identifier",
    ))
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
                    "identify the radio, or enter the observed 7.03.x bootloader version",
                ));
            }
            if request.target_confirmation.trim().is_empty() {
                return Err(FlashRequestError::new("enter the target confirmation"));
            }
            if matches!(operation, FlashOperation::K1Application) {
                // Neither retained artefact is required here, exactly as the
                // flasher CLI's `flash-afik-k1` no longer requires them. This
                // path addresses a page index rather than an address and issues
                // no EEPROM operation, so it cannot reach the bootloader and an
                // application which does not boot is recovered by writing
                // another one through the same passive beacon. Both are still
                // validated when the operator does supply them.
                optional_file(&request.recovery, "recovery image")?;
                optional_file(&request.eeprom_backup, "EEPROM backup")?;
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
            confirm_image_crc32(&image, request.image_crc32)?;
            classify_k1(&mut serial, &request.bootloader_version)?;
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
            // Both retained artefacts are optional for this operation and are
            // validated only when the operator supplied one.
            let recovery = read_optional(&request.recovery, read_k1_image)?;
            let backup = read_optional(&request.eeprom_backup, read_eeprom_backup)?;
            confirm_image_crc32(&image, request.image_crc32)?;
            classify_k1(&mut serial, &request.bootloader_version)?;
            let total = image.page_count();
            let report = k1::flash_application(
                &mut serial,
                &image,
                recovery.as_ref(),
                &request.bootloader_version,
                k1::K1ApplicationConfirmations {
                    target: &request.target_confirmation,
                    recovery_rehearsed: &request.recovery_rehearsed_confirmation,
                },
                request.transaction_id,
                |page| step(sender, page, total),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(match backup {
                Some(backup) => format!(
                    "wrote {} application pages under transaction {:08x} with retained backup CRC-32 {:08x}",
                    report.pages_acknowledged,
                    report.transaction_id,
                    backup.crc32()
                ),
                None => format!(
                    "wrote {} application pages under transaction {:08x} with no retained backup",
                    report.pages_acknowledged, report.transaction_id
                ),
            })
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

/// Refuses a K1 write unless the operator's CRC-32 matches the exact image.
fn confirm_image_crc32(
    image: &K1RecoveryImage,
    confirmation: u32,
) -> Result<(), FlashRequestError> {
    let expected = crc32(image.bytes());
    if confirmation == expected {
        return Ok(());
    }
    Err(FlashRequestError::new(
        "the image CRC-32 confirmation does not match the selected image",
    ))
}

/// Classifies the bootloader and consumes the first beacon before a K1 write.
///
/// `radio-flasher`'s K1 workflows require their caller to have consumed the
/// classification beacon, and a K5 in bootloader mode must never be sent a K1
/// page sequence. The detected version must equal the confirmation the operator
/// supplied, so a mistyped version stops the run before any page is written.
fn classify_k1(
    serial: &mut LinuxSerialTransport,
    confirmed_version: &str,
) -> Result<(), FlashRequestError> {
    let family =
        detect_bootloader(serial).map_err(|error| FlashRequestError::new(error.to_string()))?;
    let info = match &family {
        BootloaderFamily::K1(info) => info,
        BootloaderFamily::K5V1(info) => {
            return Err(FlashRequestError::new(format!(
                "a K1 write needs bootloader 7.03.x; this radio reports K5 V1 {}",
                info.version()
            )));
        }
    };
    if info.version() != confirmed_version {
        return Err(FlashRequestError::new(format!(
            "bootloader version confirmation mismatch: the radio reports {}",
            info.version()
        )));
    }
    Ok(())
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

/// Reads and validates one retained artefact, if the operator supplied a path.
///
/// An unset field is the operator saying they are not retaining this artefact
/// for this operation, which the K1 application path allows. A path which is set
/// is always read and validated.
fn read_optional<T>(
    path: &Path,
    read: impl Fn(&Path) -> Result<T, FlashRequestError>,
) -> Result<Option<T>, FlashRequestError> {
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    read(path).map(Some)
}

/// Accepts an unset path, and checks any path the operator did set.
///
/// Leaving the field blank is a choice the operation allows. Typing something
/// which is not a readable file is not: that is a mistake, and it is caught here
/// rather than after the write has started.
fn optional_file(path: &Path, description: &str) -> Result<(), FlashRequestError> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    require_file(path, description)
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

#[cfg(test)]
mod tests {
    use super::{
        artefact_digest, confirm_image_crc32, crc32, fresh_transaction_id, validate_request,
        FlashOperation, FlashRequest, K1RecoveryImage,
    };
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

        // The K1 application path cannot reach the bootloader and issues no
        // EEPROM operation, so neither retained artefact is required and a
        // failed write is recovered by writing another image.
        let mut no_artefacts = request.clone();
        no_artefacts.eeprom_backup = PathBuf::new();
        no_artefacts.recovery = PathBuf::new();
        validate_request(FlashOperation::K1Application, &no_artefacts)
            .expect("recovery from a bad K1 application is another flash");
        validate_request(FlashOperation::K1Recovery, &no_artefacts)
            .expect("a recovery write restores the known-good image itself");

        // Optional is not unchecked: a path which was typed and does not exist
        // is a mistake, not a decision, and is still refused.
        let mut missing_backup = request.clone();
        missing_backup.eeprom_backup = PathBuf::from("/nonexistent/eeprom.bin");
        assert!(
            validate_request(FlashOperation::K1Application, &missing_backup)
                .unwrap_err()
                .to_string()
                .contains("EEPROM backup")
        );
        let mut missing_recovery = request.clone();
        missing_recovery.recovery = PathBuf::from("/nonexistent/recovery.raw");
        assert!(
            validate_request(FlashOperation::K1Application, &missing_recovery)
                .unwrap_err()
                .to_string()
                .contains("recovery image")
        );

        let mut bad_version = request.clone();
        bad_version.firmware_version = "*".to_owned();
        assert!(validate_request(FlashOperation::K5Application, &bad_version).is_err());
        let mut k5 = request;
        k5.firmware_version = "2.01.26".to_owned();
        validate_request(FlashOperation::K5Application, &k5).unwrap();
    }

    #[test]
    fn a_k1_write_requires_the_exact_image_crc32() {
        // This is the same fixed 256-byte page shape the K1 bootloader takes, so
        // the image the gate compares against is a real one.
        let mut raw = vec![0_u8; 512];
        raw[..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&0x0800_28c1_u32.to_le_bytes());
        let image = K1RecoveryImage::from_raw(&raw).expect("a valid vector image");
        let expected = crc32(image.bytes());
        assert!(confirm_image_crc32(&image, expected).is_ok());
        assert!(confirm_image_crc32(&image, expected ^ 1)
            .unwrap_err()
            .to_string()
            .contains("CRC-32"));
    }

    #[test]
    fn a_fresh_transaction_identifier_is_never_zero() {
        let first = fresh_transaction_id().expect("/dev/urandom is readable");
        assert_ne!(first, 0);
    }

    #[test]
    fn an_artefact_digest_reports_its_size_and_checksum() {
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/flash.rs"));
        let (bytes, checksum) = artefact_digest(&path).expect("this source file is readable");
        assert!(bytes > 0);
        assert_eq!(artefact_digest(&path).unwrap(), (bytes, checksum));
        assert!(artefact_digest(&PathBuf::from("/definitely/missing")).is_err());
    }

    #[test]
    fn only_the_backup_operation_is_read_only() {
        assert!(!FlashOperation::BackupEeprom.is_write());
        assert!(FlashOperation::K1Recovery.is_write());
        assert!(FlashOperation::K1Application.is_write());
        assert!(FlashOperation::K5Application.is_write());
    }
}
