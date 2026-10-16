//! Guarded firmware and EEPROM operations driven from the native editor.
//!
//! Every write reuses the recovery-gated `radio-flasher` workflows unchanged,
//! including their exact confirmation phrases. This module only collects
//! operator input, runs one operation on a worker thread, and reports progress.
//!
//! Firmware cannot be read back. The bootloader protocols this crate drives
//! carry no flash-read command, so a page acknowledgement is the only evidence a
//! write produced and there is no firmware backup to take. The default AFIK
//! write is image-only, exactly as `afik-flasher flash-afik` (`ADR-098`): it
//! reads the bootloader beacon, refuses an image built for the other radio, and
//! writes only the application pages, so an image which does not boot is
//! recovered by writing another one. The K1 recovery write and the
//! recovery-gated K5 write keep their retained artefacts and phrases.

use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::SystemTime,
};

use radio_flasher::{
    afik_image_version, crc32, detect_bootloader, flash_afik_application, force_bootloader_family,
    identify_radio, k1, k1::K1RecoveryImage, ApplicationImage, BootloaderFamily, EepromBackup,
    FirmwareVersion, FlashPrerequisites, FlashPurpose, ForcedFamily, RadioPresence, VersionCheck,
    DEFAULT_NEGOTIATION_VERSION, FLASH_PAGE_COUNT,
};
use radio_programmer_serial::LinuxSerialTransport;

/// Serial baud used by every supported bootloader path.
pub const FLASH_BAUD: u32 = 38_400;
/// Largest firmware image the editor will read from disk.
pub const MAX_FIRMWARE_BYTES: usize = 256 * 1024;
/// Application pages every K5 V1 write sends.
#[allow(clippy::cast_possible_truncation)]
const K5_PAGES: u16 = FLASH_PAGE_COUNT as u16;
/// Exact EEPROM backup size for the UV-K5 V1 path.
pub const EEPROM_BYTES: usize = 8 * 1024;

/// Which guarded operation to run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashOperation {
    /// Read the complete EEPROM without writing anything.
    BackupEeprom,
    /// Write an AFIK application to whichever radio's bootloader answers.
    AfikApplication,
    /// Write the known-good recovery image to a UV-K1.
    K1Recovery,
    /// Write an application to a UV-K5 V1 after a recovery rehearsal.
    K5Application,
}

impl FlashOperation {
    /// Returns the editor label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::BackupEeprom => "Back up EEPROM (read only)",
            Self::AfikApplication => "Flash AFIK application (K5 V1 or K1)",
            Self::K1Recovery => "Flash K1 recovery image",
            Self::K5Application => "Flash K5 V1 application (recovery-gated)",
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
            Self::AfikApplication,
            Self::K1Recovery,
            Self::K5Application,
        ]
    }

    /// Reports whether the operator must type the image CRC-32 to confirm it.
    pub const fn needs_crc32(self) -> bool {
        matches!(self, Self::K1Recovery | Self::K5Application)
    }
}

/// Parses an operator-typed CRC-32: eight hex digits, optionally `0x`-prefixed.
pub fn parse_crc32(text: &str) -> Option<u32> {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
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
    /// Operator-entered CRC-32 of the selected image, where one is required.
    pub image_crc32: u32,
    /// Negotiated K5 firmware version string.
    pub firmware_version: String,
    /// The family the operator forces the AFIK write to, accepting any
    /// bootloader version on a page-write beacon; `None` detects it.
    pub forced_family: Option<RadioFamily>,
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

/// Which radio family a device belongs to, as far as flashing is concerned.
///
/// The K5 and K6 share the K5 V1 bootloader and image layout, so they are one
/// family here; the K1 has its own bootloader, image layout, and storage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RadioFamily {
    /// UV-K5 V1 and UV-K6: `2.*` or `4.*` bootloader.
    K5,
    /// UV-K1: `7.03.*` bootloader.
    K1,
}

impl RadioFamily {
    /// Returns every family in display order.
    pub const fn all() -> [Self; 2] {
        [Self::K5, Self::K1]
    }

    /// Returns the operator-facing name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::K5 => "K5/K6",
            Self::K1 => "K1",
        }
    }

    /// Returns the directory name used for this family's files.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::K5 => "k5",
            Self::K1 => "k1",
        }
    }

    /// Returns the file name `tool/package-*-image.sh` gives this family's image.
    pub const fn image_name(self) -> &'static str {
        match self {
            Self::K5 => "afik-k5-v1.raw",
            Self::K1 => "radio-firmware-k1.raw",
        }
    }

    /// Returns the command which builds and packages this family's image.
    pub const fn build_hint(self) -> &'static str {
        match self {
            Self::K5 => "tool/build-k5.sh --release",
            Self::K1 => "tool/build-k1.sh --release",
        }
    }

    /// Returns the library's name for this family when forcing it.
    pub const fn forced(self) -> ForcedFamily {
        match self {
            Self::K5 => ForcedFamily::K5V1,
            Self::K1 => ForcedFamily::K1,
        }
    }

    /// Returns the family a bootloader beacon belongs to.
    pub const fn of_bootloader(family: &BootloaderFamily) -> Self {
        match family {
            BootloaderFamily::K5V1(_) => Self::K5,
            BootloaderFamily::K1(_) => Self::K1,
        }
    }

    /// Recognises the family from a running firmware's hello version.
    ///
    /// AFIK names its target; stock K5 firmware reports `k5_…`. Anything else
    /// is left for the operator to choose rather than guessed.
    pub fn of_firmware(version: &str) -> Option<Self> {
        let lower = version.to_ascii_lowercase();
        if lower.starts_with("afik-k5-") || lower.starts_with("k5_") || lower.starts_with("k6_") {
            Some(Self::K5)
        } else if lower.starts_with("afik-k1-") || lower.starts_with("k1_") {
            Some(Self::K1)
        } else {
            None
        }
    }

    /// Returns the family of whatever answered, when it can be told.
    pub fn of_presence(presence: &RadioPresence) -> Option<Self> {
        match presence {
            RadioPresence::Bootloader(family) => Some(Self::of_bootloader(family)),
            RadioPresence::Firmware(info) => Self::of_firmware(info.version()),
        }
    }
}

/// Finds out what is on the serial port without writing to a bootloader.
///
/// A radio in flashing mode is only listened to; a radio running its firmware
/// is sent the read-only hello. See [`radio_flasher::identify_radio`].
pub fn identify(device: &Path) -> Result<RadioPresence, FlashRequestError> {
    if device.as_os_str().is_empty() {
        return Err(FlashRequestError::new("select a serial device path"));
    }
    let mut serial = LinuxSerialTransport::open(device, FLASH_BAUD)
        .map_err(|error| FlashRequestError::new(format!("serial setup failed: {error}")))?;
    identify_radio(&mut serial).map_err(|error| FlashRequestError::new(error.to_string()))
}

/// A read-only identification running on a worker thread.
///
/// Waiting for a beacon can take seconds when no bootloader answers, so it never
/// runs on the thread drawing the editor.
pub struct IdentifyJob {
    outcome: Receiver<Result<RadioPresence, FlashRequestError>>,
}

impl IdentifyJob {
    /// Returns the outcome once the worker has finished, without blocking.
    pub fn poll(&self) -> Option<Result<RadioPresence, FlashRequestError>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(FlashRequestError::new(
                "the identification worker stopped without a result",
            ))),
        }
    }
}

/// Starts [`identify`] on a worker thread.
pub fn start_identify(device: PathBuf) -> Result<IdentifyJob, FlashRequestError> {
    if device.as_os_str().is_empty() {
        return Err(FlashRequestError::new("select a serial device path"));
    }
    let (sender, outcome) = mpsc::channel();
    thread::Builder::new()
        .name("afik-identify".to_owned())
        .spawn(move || {
            let _ignored = sender.send(identify(&device));
        })
        .map_err(|error| FlashRequestError::new(format!("could not start worker: {error}")))?;
    Ok(IdentifyJob { outcome })
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

/// Most selected files whose digests are remembered at once.
const MAX_CACHED_DIGESTS: usize = 16;

/// What identifies one version of a file without reading it.
type FileStamp = Option<(SystemTime, u64)>;
/// One file's size and CRC-32, or why it could not be read.
type Digest = Result<(usize, u32), FlashRequestError>;

/// What one read of a file found.
#[derive(Debug)]
struct Facts {
    stamp: FileStamp,
    digest: Digest,
    /// AFIK version the file carries, when it is an AFIK image.
    version: Option<String>,
}

/// Digests of the selected artefacts, recomputed only when a file changes.
///
/// The editor shows each digest on every frame, and reading up to 256 KiB per
/// file per frame is wasted work while a flash keeps the window repainting.
/// A file is re-read when its size or modification time changes.
#[derive(Debug, Default)]
pub struct DigestCache {
    entries: HashMap<PathBuf, Facts>,
    reads: usize,
}

impl DigestCache {
    /// Returns the digest of one file, reading it only if it has changed.
    pub fn digest(&mut self, path: &Path) -> Digest {
        self.facts(path).digest.clone()
    }

    /// Returns the AFIK version an image carries, such as `AFIK-K5-7.0.22`.
    pub fn version(&mut self, path: &Path) -> Option<String> {
        self.facts(path).version.clone()
    }

    fn facts(&mut self, path: &Path) -> &Facts {
        let stamp = fs::metadata(path).ok().map(|metadata| {
            (
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                metadata.len(),
            )
        });
        let fresh = self
            .entries
            .get(path)
            .is_some_and(|facts| facts.stamp == stamp);
        if !fresh {
            // A path typed one character at a time leaves a trail of prefixes.
            if self.entries.len() >= MAX_CACHED_DIGESTS {
                self.entries.clear();
            }
            let bytes = read_bounded(path, MAX_FIRMWARE_BYTES, "artefact");
            self.reads += 1;
            let facts = Facts {
                stamp,
                version: bytes.as_deref().ok().and_then(afik_image_version),
                digest: bytes.map(|bytes| (bytes.len(), crc32(&bytes))),
            };
            self.entries.insert(path.to_path_buf(), facts);
        }
        &self.entries[path]
    }

    /// Returns how many times a file has actually been read.
    pub const fn reads(&self) -> usize {
        self.reads
    }
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
        FlashOperation::AfikApplication => {
            // The image is the only input: the radio is identified from its
            // beacon at write time, as `afik-flasher flash-afik` does.
            require_file(&request.firmware, "firmware image")?;
            if request.transaction_id == 0 {
                return Err(FlashRequestError::new(
                    "enter a non-zero transaction identifier",
                ));
            }
        }
        FlashOperation::K1Recovery => {
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
            // The recovery write can leave a unit needing rescue, so it still
            // asks for its phrase.
            if request.target_confirmation.trim().is_empty() {
                return Err(FlashRequestError::new("enter the target confirmation"));
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
        FlashOperation::AfikApplication => flash_afik(&mut serial, request, sender),
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
                |page| step(sender, page, K5_PAGES),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} application pages with CRC-32 {:08x}",
                report.pages_acknowledged, report.image_crc32
            ))
        }
    }
}

/// Writes an AFIK application to whichever radio's bootloader answers.
fn flash_afik(
    serial: &mut LinuxSerialTransport,
    request: &FlashRequest,
    sender: &Sender<FlashProgress>,
) -> Result<String, FlashRequestError> {
    let raw = read_bounded(&request.firmware, MAX_FIRMWARE_BYTES, "firmware image")?;
    // Classification is read-only and consumes the first beacon, which
    // both write paths require of their caller. A forced family still needs a
    // page-write beacon, and then writes whatever version it reports.
    let (family, check) = match request.forced_family {
        None => (detect_bootloader(serial), VersionCheck::Known),
        Some(forced) => (
            force_bootloader_family(serial, forced.forced()),
            VersionCheck::Any,
        ),
    };
    let family = family.map_err(|error| FlashRequestError::new(error.to_string()))?;
    match afik_image_for(Radio::of(&family), family.info().version(), &raw)? {
        AfikImage::K5V1(image) => {
            let version = FirmwareVersion::new(DEFAULT_NEGOTIATION_VERSION)
                .map_err(|error| FlashRequestError::new(error.to_string()))?;
            let report = flash_afik_application(
                serial,
                &image,
                &version,
                request.transaction_id,
                check,
                |page| {
                    step(sender, page, K5_PAGES);
                },
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} K5 V1 application pages with CRC-32 {:08x} under transaction \
                 {:08x}; acknowledged, not read back",
                report.pages_acknowledged, report.image_crc32, report.transaction_id
            ))
        }
        AfikImage::K1(image) => {
            let total = image.page_count();
            let report = k1::flash_application(
                serial,
                &image,
                None,
                family.info().version(),
                request.transaction_id,
                check,
                |page| step(sender, page, total),
            )
            .map_err(|error| FlashRequestError::new(error.to_string()))?;
            Ok(format!(
                "wrote {} K1 application pages with CRC-32 {:08x} under transaction \
                 {:08x}; acknowledged, not read back",
                report.pages_acknowledged,
                crc32(image.bytes()),
                report.transaction_id
            ))
        }
    }
}

/// Which radio a detected bootloader belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Radio {
    K5V1,
    K1,
}

impl Radio {
    const fn of(family: &BootloaderFamily) -> Self {
        match family {
            BootloaderFamily::K5V1(_) => Self::K5V1,
            BootloaderFamily::K1(_) => Self::K1,
        }
    }
}

/// An AFIK application image checked against the radio that answered.
enum AfikImage {
    K5V1(ApplicationImage),
    K1(K1RecoveryImage),
}

/// Refuses an image built for the other radio before any page is sent.
///
/// The layouts cannot be confused: a K5 image's reset vector lies inside its own
/// 60 KiB from address zero, and a K1 image's lies in the K1 flash region.
fn afik_image_for(
    radio: Radio,
    bootloader: &str,
    raw: &[u8],
) -> Result<AfikImage, FlashRequestError> {
    let (label, image) = match radio {
        Radio::K5V1 => (
            "K5 V1",
            ApplicationImage::from_raw(raw)
                .map(AfikImage::K5V1)
                .map_err(|error| error.to_string()),
        ),
        Radio::K1 => (
            "K1",
            K1RecoveryImage::from_raw(raw)
                .map(AfikImage::K1)
                .map_err(|error| error.to_string()),
        ),
    };
    image.map_err(|error| {
        FlashRequestError::new(format!(
            "the image is not a {label} application ({error}); detected bootloader {bootloader}"
        ))
    })
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
    // The default lives under the XDG data directory, which may not exist yet.
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            FlashRequestError::new(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    fs::write(path, bytes)
        .map_err(|error| FlashRequestError::new(format!("could not write backup: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{
        afik_image_for, artefact_digest, confirm_image_crc32, crc32, fresh_transaction_id,
        parse_crc32, start_identify, validate_request, write_new_file, AfikImage, DigestCache,
        FlashOperation, FlashRequest, K1RecoveryImage, Radio,
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

        // The AFIK write takes the image and nothing else, as `flash-afik`
        // does: no phrases, no bootloader version, no retained artefacts.
        let afik = FlashRequest {
            device: request.device.clone(),
            firmware: request.firmware.clone(),
            transaction_id: 1,
            ..FlashRequest::default()
        };
        validate_request(FlashOperation::AfikApplication, &afik).unwrap();
        let mut no_image = afik.clone();
        no_image.firmware = PathBuf::new();
        assert!(validate_request(FlashOperation::AfikApplication, &no_image)
            .unwrap_err()
            .to_string()
            .contains("firmware image"));

        // The recovery write can leave a unit needing rescue, so it still asks.
        let mut no_phrases = request.clone();
        no_phrases.target_confirmation = String::new();
        assert!(validate_request(FlashOperation::K1Recovery, &no_phrases)
            .unwrap_err()
            .to_string()
            .contains("target confirmation"));
        let mut no_artefacts = request.clone();
        no_artefacts.eeprom_backup = PathBuf::new();
        no_artefacts.recovery = PathBuf::new();
        validate_request(FlashOperation::K1Recovery, &no_artefacts)
            .expect("a recovery write restores the known-good image itself");

        // The recovery-gated K5 write keeps its retained artefacts.
        let mut k5_no_backup = request.clone();
        k5_no_backup.firmware_version = "2.01.26".to_owned();
        k5_no_backup.eeprom_backup = PathBuf::new();
        assert!(
            validate_request(FlashOperation::K5Application, &k5_no_backup)
                .unwrap_err()
                .to_string()
                .contains("EEPROM backup")
        );

        let mut bad_version = request.clone();
        bad_version.firmware_version = "*".to_owned();
        assert!(validate_request(FlashOperation::K5Application, &bad_version).is_err());
        let mut k5 = request;
        k5.firmware_version = "2.01.26".to_owned();
        validate_request(FlashOperation::K5Application, &k5).unwrap();
    }

    /// A raw K1 image: a stack pointer and a reset vector in the K1 flash region.
    fn k1_raw() -> Vec<u8> {
        let mut raw = vec![0_u8; 512];
        raw[..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&0x0800_28c1_u32.to_le_bytes());
        raw
    }

    #[test]
    fn an_afik_image_must_belong_to_the_radio_which_answered() {
        assert!(matches!(
            afik_image_for(Radio::K1, "7.03.01", &k1_raw()),
            Ok(AfikImage::K1(_))
        ));
        let refused = afik_image_for(Radio::K5V1, "2.00.06", &k1_raw())
            .err()
            .unwrap()
            .to_string();
        assert!(refused.contains("not a K5 V1 application"), "{refused}");
        assert!(refused.contains("2.00.06"), "{refused}");
        assert!(afik_image_for(Radio::K1, "7.03.01", &[0_u8; 16])
            .err()
            .unwrap()
            .to_string()
            .contains("not a K1 application"));
    }

    #[test]
    fn typed_crc32s_are_hex_or_refused() {
        assert_eq!(parse_crc32("deadBEEF"), Some(0xdead_beef));
        assert_eq!(parse_crc32(" 0x0000002a "), Some(42));
        assert_eq!(parse_crc32("0XFF"), Some(0xff));
        assert_eq!(parse_crc32(""), None);
        assert_eq!(parse_crc32("0x"), None);
        assert_eq!(parse_crc32("12345678g"), None);
        assert_eq!(parse_crc32("123456789"), None);
        assert!(FlashOperation::K1Recovery.needs_crc32());
        assert!(FlashOperation::K5Application.needs_crc32());
        assert!(!FlashOperation::AfikApplication.needs_crc32());
        assert!(!FlashOperation::BackupEeprom.needs_crc32());
    }

    #[test]
    fn a_k1_write_requires_the_exact_image_crc32() {
        // This is the same fixed 256-byte page shape the K1 bootloader takes, so
        // the image the gate compares against is a real one.
        let image = K1RecoveryImage::from_raw(&k1_raw()).expect("a valid vector image");
        let expected = crc32(image.bytes());
        assert!(confirm_image_crc32(&image, expected).is_ok());
        assert!(confirm_image_crc32(&image, expected ^ 1)
            .unwrap_err()
            .to_string()
            .contains("CRC-32"));
    }

    #[test]
    fn identification_runs_off_the_calling_thread_and_reports_its_failure() {
        assert!(start_identify(PathBuf::new())
            .err()
            .unwrap()
            .to_string()
            .contains("serial device"));
        let job = start_identify(PathBuf::from("/definitely/missing")).unwrap();
        let outcome = loop {
            if let Some(outcome) = job.poll() {
                break outcome;
            }
            std::thread::yield_now();
        };
        assert!(outcome
            .unwrap_err()
            .to_string()
            .contains("serial setup failed"));
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
    fn a_digest_is_read_once_and_again_only_when_the_file_changes() {
        let path = std::env::temp_dir().join(format!("afik-digest-{}.bin", std::process::id()));
        std::fs::write(&path, [1_u8; 10]).unwrap();
        let mut cache = DigestCache::default();
        let first = cache.digest(&path).unwrap();
        assert_eq!(first.0, 10);
        assert_eq!(cache.digest(&path).unwrap(), first);
        assert_eq!(cache.reads(), 1);

        std::fs::write(&path, [2_u8; 12]).unwrap();
        assert_eq!(cache.digest(&path).unwrap().0, 12);
        assert_eq!(cache.reads(), 2);

        // A missing file is remembered too, and read once it appears.
        std::fs::remove_file(&path).unwrap();
        assert!(cache.digest(&path).is_err());
        assert!(cache.digest(&path).is_err());
        assert_eq!(cache.reads(), 3);
        std::fs::write(&path, [3_u8; 4]).unwrap();
        assert_eq!(cache.digest(&path).unwrap().0, 4);
        assert_eq!(cache.version(&path), None);

        // An image's AFIK version is read along with its digest.
        std::fs::write(&path, b"\0\0AFIK-K5-7.0.22\0").unwrap();
        assert_eq!(cache.version(&path).as_deref(), Some("AFIK-K5-7.0.22"));
        assert_eq!(cache.digest(&path).unwrap().0, 17);
        assert_eq!(cache.reads(), 5);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_backup_creates_its_directory_and_never_overwrites() {
        let directory = std::env::temp_dir().join(format!("afik-backup-{}", std::process::id()));
        let path = directory.join("eeprom").join("eeprom-19700101T000000Z.bin");
        write_new_file(&path, &[7_u8; 4]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [7_u8; 4]);
        assert!(write_new_file(&path, &[8_u8; 4])
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        assert_eq!(std::fs::read(&path).unwrap(), [7_u8; 4]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn only_the_backup_operation_is_read_only() {
        assert!(!FlashOperation::BackupEeprom.is_write());
        assert!(FlashOperation::AfikApplication.is_write());
        assert!(FlashOperation::K1Recovery.is_write());
        assert!(FlashOperation::K5Application.is_write());
    }
}
