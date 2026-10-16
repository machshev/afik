//! UV-K5 V1 commands: image inspection, normal-mode control, EEPROM restore,
//! and AFIK application flashing.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use radio_flasher::k5::{control as k5_control, ControlRequest, ControlResponse, LogicalKey, View};
use radio_flasher::{
    apply_eeprom_restore, flash_afik_application, flash_application, prepare_eeprom_restore,
    sha256, ApplicationImage, EepromBackup, EepromRestorePrerequisites, FirmwareVersion,
    FlashPrerequisites, FlashPurpose, VersionCheck, APPLICATION_BYTES, EEPROM_BYTES,
    K5_RECOVERY_SOURCE_BYTES,
};

use radio_programmer_serial::LinuxSerialTransport;

use crate::prompt::Confirm;
use crate::{
    confirm_write, fresh_transaction_id, open_serial, parse_crc32, read_bounded, set_once,
    CliError, DetectedFlash, K5_BAUD,
};

/// The opt-in recovery-gated AFIK flash, selected by `--recovery`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GatedAfikArguments {
    pub(crate) image: PathBuf,
    pub(crate) recovery: PathBuf,
    pub(crate) backup: PathBuf,
    pub(crate) version: String,
    pub(crate) target_confirmation: String,
    pub(crate) image_crc32_confirmation: String,
    pub(crate) recovery_rehearsed_confirmation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoreArguments {
    pub(crate) restore: PathBuf,
    pub(crate) current: PathBuf,
    pub(crate) recovery: PathBuf,
    pub(crate) pre_write_backup: PathBuf,
    pub(crate) version: String,
    pub(crate) target_confirmation: String,
    pub(crate) recovery_crc32_confirmation: String,
    pub(crate) recovery_sha256_confirmation: String,
    pub(crate) restore_confirmation: String,
}

pub(crate) fn parse_control(arguments: &[String]) -> Result<ControlRequest, String> {
    match arguments {
        [operation] if operation == "view" => Ok(ControlRequest::View),
        [operation] if operation == "capabilities" => Ok(ControlRequest::Capabilities),
        [operation, key] if operation == "key" => match key.as_str() {
            "AUDIO" => Ok(ControlRequest::ToggleAudio),
            "UP" => Ok(ControlRequest::Key(LogicalKey::Up)),
            "DOWN" => Ok(ControlRequest::Key(LogicalKey::Down)),
            _ => Err("control key requires AUDIO, UP, or DOWN".into()),
        },
        _ => Err("control requires view, capabilities, or key AUDIO|UP|DOWN".into()),
    }
}

pub(crate) fn parse_restore(arguments: &[String]) -> Result<RestoreArguments, String> {
    let restore = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "restore-eeprom requires RESTORE_IMAGE".to_owned())?;
    let mut current = None;
    let mut recovery = None;
    let mut pre_write_backup = None;
    let mut version = None;
    let mut target_confirmation = None;
    let mut recovery_crc32_confirmation = None;
    let mut recovery_sha256_confirmation = None;
    let mut restore_confirmation = None;
    let mut offset = 1;
    while offset < arguments.len() {
        let option = &arguments[offset];
        let value = arguments
            .get(offset + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--current" => set_once(&mut current, PathBuf::from(value), option)?,
            "--recovery" => set_once(&mut recovery, PathBuf::from(value), option)?,
            "--pre-write-backup" => set_once(&mut pre_write_backup, PathBuf::from(value), option)?,
            "--version" => set_once(&mut version, value.clone(), option)?,
            "--confirm-target" => set_once(&mut target_confirmation, value.clone(), option)?,
            "--confirm-recovery-crc32" => {
                set_once(&mut recovery_crc32_confirmation, value.clone(), option)?;
            }
            "--confirm-recovery-sha256" => {
                set_once(&mut recovery_sha256_confirmation, value.clone(), option)?;
            }
            "--confirm-restore" => set_once(&mut restore_confirmation, value.clone(), option)?,
            _ => return Err(format!("unknown restore-eeprom option: {option}")),
        }
        offset += 2;
    }
    Ok(RestoreArguments {
        restore: PathBuf::from(restore),
        current: current.ok_or_else(|| "restore-eeprom requires --current BACKUP".to_owned())?,
        recovery: recovery.ok_or_else(|| "restore-eeprom requires --recovery IMAGE".to_owned())?,
        pre_write_backup: pre_write_backup
            .ok_or_else(|| "restore-eeprom requires --pre-write-backup OUTPUT".to_owned())?,
        version: version.ok_or_else(|| "restore-eeprom requires --version VERSION".to_owned())?,
        target_confirmation: target_confirmation
            .ok_or_else(|| "restore-eeprom requires --confirm-target TARGET".to_owned())?,
        recovery_crc32_confirmation: recovery_crc32_confirmation
            .ok_or_else(|| "restore-eeprom requires --confirm-recovery-crc32 CRC32".to_owned())?,
        recovery_sha256_confirmation: recovery_sha256_confirmation
            .ok_or_else(|| "restore-eeprom requires --confirm-recovery-sha256 SHA256".to_owned())?,
        restore_confirmation: restore_confirmation
            .ok_or_else(|| "restore-eeprom requires --confirm-restore TARGET:CRC32".to_owned())?,
    })
}

/// Parses the recovery-gated `flash-afik` form, which `--recovery` selects.
pub(crate) fn parse_gated_afik(arguments: &[String]) -> Result<GatedAfikArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "flash-afik requires IMAGE".to_owned())?;
    let mut recovery = None;
    let mut backup = None;
    let mut version = None;
    let mut target_confirmation = None;
    let mut image_crc32_confirmation = None;
    let mut recovery_rehearsed_confirmation = None;
    let mut offset = 1;
    while offset < arguments.len() {
        let option = &arguments[offset];
        let value = arguments
            .get(offset + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--recovery" => set_once(&mut recovery, PathBuf::from(value), option)?,
            "--backup" => set_once(&mut backup, PathBuf::from(value), option)?,
            "--version" => set_once(&mut version, value.clone(), option)?,
            "--confirm-target" => set_once(&mut target_confirmation, value.clone(), option)?,
            "--confirm-image-crc32" => {
                set_once(&mut image_crc32_confirmation, value.clone(), option)?;
            }
            "--confirm-recovery-rehearsed" => {
                set_once(&mut recovery_rehearsed_confirmation, value.clone(), option)?;
            }
            _ => return Err(format!("unknown flash-afik option: {option}")),
        }
        offset += 2;
    }
    Ok(GatedAfikArguments {
        image: PathBuf::from(image),
        recovery: recovery.ok_or_else(|| "flash-afik requires --recovery RAW".to_owned())?,
        backup: backup.ok_or_else(|| "flash-afik requires --backup EEPROM".to_owned())?,
        version: version.ok_or_else(|| "flash-afik requires --version VERSION".to_owned())?,
        target_confirmation: target_confirmation
            .ok_or_else(|| "flash-afik requires --confirm-target PHRASE".to_owned())?,
        image_crc32_confirmation: image_crc32_confirmation
            .ok_or_else(|| "flash-afik requires --confirm-image-crc32 CRC32".to_owned())?,
        recovery_rehearsed_confirmation: recovery_rehearsed_confirmation
            .ok_or_else(|| "flash-afik requires --confirm-recovery-rehearsed PHRASE".to_owned())?,
    })
}

pub(crate) fn inspect<W: Write>(path: &Path, stdout: &mut W) -> Result<(), CliError> {
    let raw = read_bounded(path, APPLICATION_BYTES, "application")?;
    let image = ApplicationImage::from_raw(&raw).map_err(CliError::operation)?;
    writeln!(stdout, "source={}", path.display()).map_err(CliError::operation)?;
    writeln!(stdout, "source_bytes={}", image.source_len()).map_err(CliError::operation)?;
    writeln!(stdout, "application_bytes={}", image.bytes().len()).map_err(CliError::operation)?;
    writeln!(stdout, "initial_sp=0x{:08x}", image.initial_stack()).map_err(CliError::operation)?;
    writeln!(stdout, "reset_vector=0x{:08x}", image.reset_vector()).map_err(CliError::operation)?;
    writeln!(stdout, "crc32={:08x}", image.crc32()).map_err(CliError::operation)
}

pub(crate) fn control<W: Write>(
    device: &Path,
    request: ControlRequest,
    stdout: &mut W,
) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let response = k5_control(&mut serial, request).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "request={}", request_label(request)).map_err(CliError::operation)?;
    render_control_response(response, stdout)
}

fn request_label(request: ControlRequest) -> &'static str {
    match request {
        ControlRequest::View => "view command=0x0600",
        ControlRequest::Capabilities => "capabilities command=0x0601",
        ControlRequest::ToggleAudio => "audio command=0x0615",
        ControlRequest::Key(LogicalKey::Up) => "key UP command=0x0606",
        ControlRequest::Key(LogicalKey::Down) => "key DOWN command=0x0607",
    }
}

fn render_control_response<W: Write>(
    response: ControlResponse,
    stdout: &mut W,
) -> Result<(), CliError> {
    match response {
        ControlResponse::View(view) => render_view(view, stdout),
        ControlResponse::Capabilities(bits) => {
            writeln!(stdout, "response=capabilities command=0x0620")
                .map_err(CliError::operation)?;
            writeln!(stdout, "capability_bits=0x{bits:04x}").map_err(CliError::operation)?;
            writeln!(stdout, "capabilities={}", capability_labels(bits))
                .map_err(CliError::operation)
        }
    }
}

fn render_view<W: Write>(view: View, stdout: &mut W) -> Result<(), CliError> {
    writeln!(stdout, "response=view command=0x0620").map_err(CliError::operation)?;
    writeln!(stdout, "channel={}", view.channel).map_err(CliError::operation)?;
    writeln!(stdout, "audio={}", if view.audio { "on" } else { "off" })
        .map_err(CliError::operation)?;
    writeln!(
        stdout,
        "monitoring={}",
        if view.monitoring { "on" } else { "off" }
    )
    .map_err(CliError::operation)?;
    writeln!(
        stdout,
        "squelch={}",
        if view.squelch_open { "open" } else { "closed" }
    )
    .map_err(CliError::operation)?;
    writeln!(
        stdout,
        "receiver={}",
        if view.receiver_ok { "ok" } else { "fault" }
    )
    .map_err(CliError::operation)
}

fn capability_labels(bits: u16) -> String {
    const LABELS: [&str; 9] = [
        "shared-receive",
        "view",
        "keys",
        "persistent-configuration",
        "object-configuration",
        "arbitrary-channels",
        "vfo",
        "scanning",
        "raw-metrics",
    ];
    let mut labels = Vec::new();
    for (index, label) in LABELS.iter().enumerate() {
        if bits & (1_u16 << index) != 0 {
            labels.push(*label);
        }
    }
    labels.join(",")
}

pub(crate) fn restore<W: Write>(
    device: &Path,
    arguments: &RestoreArguments,
    stdout: &mut W,
) -> Result<(), CliError> {
    validate_restore_paths(arguments)?;
    let restore_raw = read_bounded(&arguments.restore, EEPROM_BYTES, "restore EEPROM image")?;
    let restore_image = EepromBackup::from_raw(&restore_raw).map_err(CliError::operation)?;
    let current_raw = read_bounded(
        &arguments.current,
        EEPROM_BYTES,
        "expected current EEPROM backup",
    )?;
    let expected_current = EepromBackup::from_raw(&current_raw).map_err(CliError::operation)?;
    let recovery_raw = read_bounded(
        &arguments.recovery,
        APPLICATION_BYTES,
        "recovery application",
    )?;
    let recovery_image = ApplicationImage::from_raw(&recovery_raw).map_err(CliError::operation)?;
    let recovery_crc = parse_crc32(&arguments.recovery_crc32_confirmation)?;
    if recovery_crc != recovery_image.crc32() {
        return Err(CliError::Operation(format!(
            "recovery CRC-32 confirmation mismatch: expected {:08x}, supplied {:08x}",
            recovery_image.crc32(),
            recovery_crc
        )));
    }
    if recovery_image.source_len() != K5_RECOVERY_SOURCE_BYTES {
        return Err(CliError::Operation(format!(
            "recovery must contain exactly {K5_RECOVERY_SOURCE_BYTES} raw bytes"
        )));
    }
    let recovery_sha = parse_sha256(&arguments.recovery_sha256_confirmation)?;
    if sha256(recovery_image.source_bytes()) != recovery_sha {
        return Err(CliError::Operation(
            "trusted recovery SHA-256 confirmation mismatch".into(),
        ));
    }
    let version = FirmwareVersion::normal(&arguments.version).map_err(CliError::operation)?;

    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "restore_crc32={:08x}", restore_image.crc32()).map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    let mut serial = open_serial(device)?;
    let plan = prepare_eeprom_restore(
        &mut serial,
        EepromRestorePrerequisites {
            restore_image: &restore_image,
            expected_current: &expected_current,
            recovery_image: &recovery_image,
            recovery_source_len: recovery_image.source_len(),
            recovery_crc32: recovery_crc,
            recovery_sha256: recovery_sha,
            expected_firmware: &version,
            target_confirmation: &arguments.target_confirmation,
            restore_confirmation: &arguments.restore_confirmation,
        },
    )
    .map_err(CliError::operation)?;
    writeln!(stdout, "firmware={}", plan.firmware().version()).map_err(CliError::operation)?;
    writeln!(stdout, "live_crc32={:08x}", plan.live_backup().crc32())
        .map_err(CliError::operation)?;
    writeln!(stdout, "changed_blocks={}", plan.changed_offsets().len())
        .map_err(CliError::operation)?;
    write_private_file_atomic(&arguments.pre_write_backup, plan.live_backup().bytes())?;
    writeln!(
        stdout,
        "pre_write_backup={}",
        arguments.pre_write_backup.display()
    )
    .map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;
    let report = apply_eeprom_restore(&mut serial, &plan).map_err(CliError::operation)?;
    writeln!(
        stdout,
        "status={}",
        if report.changed_blocks == 0 {
            "verified_no_write"
        } else if report.verified_read_back {
            "verified_read_back"
        } else {
            "not_verified"
        }
    )
    .map_err(CliError::operation)
}

fn validate_restore_paths(arguments: &RestoreArguments) -> Result<(), CliError> {
    for (left, right, label) in [
        (&arguments.restore, &arguments.current, "restore/current"),
        (&arguments.restore, &arguments.recovery, "restore/recovery"),
        (&arguments.current, &arguments.recovery, "current/recovery"),
        (
            &arguments.restore,
            &arguments.pre_write_backup,
            "restore/output",
        ),
        (
            &arguments.current,
            &arguments.pre_write_backup,
            "current/output",
        ),
        (
            &arguments.recovery,
            &arguments.pre_write_backup,
            "recovery/output",
        ),
    ] {
        if paths_same(left, right) {
            return Err(CliError::Operation(format!(
                "restore-eeprom inputs must be distinct: {label}"
            )));
        }
    }
    if fs::symlink_metadata(&arguments.pre_write_backup).is_ok() {
        return Err(CliError::Operation(
            "pre-write backup output already exists; refusing overwrite".into(),
        ));
    }
    Ok(())
}

pub(crate) fn flash_afik_gated<W: Write>(
    device: &Path,
    arguments: &GatedAfikArguments,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image_raw = read_bounded(&arguments.image, APPLICATION_BYTES, "application")?;
    let image = ApplicationImage::from_raw(&image_raw).map_err(CliError::operation)?;
    let backup_raw = read_bounded(&arguments.backup, EEPROM_BYTES, "EEPROM backup")?;
    let backup = EepromBackup::from_raw(&backup_raw).map_err(CliError::operation)?;
    let recovery_raw = read_bounded(
        &arguments.recovery,
        APPLICATION_BYTES,
        "recovery application",
    )?;
    let recovery = ApplicationImage::from_raw(&recovery_raw).map_err(CliError::operation)?;
    let version = FirmwareVersion::new(&arguments.version).map_err(CliError::operation)?;
    let confirmation = parse_crc32(&arguments.image_crc32_confirmation)?;
    let transaction_id = fresh_transaction_id()?;
    let prerequisites = FlashPrerequisites {
        image: &image,
        recovery_image: &recovery,
        eeprom_backup: &backup,
        version: &version,
        target_confirmation: &arguments.target_confirmation,
        image_crc32_confirmation: confirmation,
        transaction_id,
        purpose: FlashPurpose::Afik {
            recovery_rehearsed_confirmation: &arguments.recovery_rehearsed_confirmation,
        },
    };

    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "image={}", arguments.image.display()).map_err(CliError::operation)?;
    writeln!(stdout, "image_crc32={:08x}", image.crc32()).map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={transaction_id:08x}").map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    let mut serial = open_serial(device)?;
    let report = flash_application(&mut serial, prerequisites, |page| {
        let complete = page + 1;
        if complete % 16 == 0 || complete == 240 {
            let _ = writeln!(stdout, "acknowledged_pages={complete}/240");
            let _ = stdout.flush();
        }
    })
    .map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", report.bootloader.version()).map_err(CliError::operation)?;
    writeln!(stdout, "pages_acknowledged={}", report.pages_acknowledged)
        .map_err(CliError::operation)?;
    writeln!(stdout, "status=acknowledged_not_read_back").map_err(CliError::operation)
}

/// Writes an AFIK application to a K5 V1 whose bootloader was already detected.
pub(crate) fn flash_afik_detected<C: Confirm, W: Write>(
    serial: &mut LinuxSerialTransport,
    flash: &DetectedFlash<'_>,
    version: Option<&str>,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image = ApplicationImage::from_raw(flash.image_raw).map_err(CliError::operation)?;
    let version = version
        .map_or_else(
            || Ok(FirmwareVersion::for_bootloader(flash.bootloader.version())),
            FirmwareVersion::new,
        )
        .map_err(CliError::operation)?;
    let summary = format!(
        "About to write an AFIK application to a K5 V1:\n\
         \x20 device:     {}\n\
         \x20 bootloader: {}\n\
         \x20 image:      {}\n\
         \x20 crc32:      {:08x}\n\
         This replaces the application, not the bootloader or EEPROM, and cannot be read back.",
        flash.device.display(),
        flash.bootloader.version(),
        flash.image_path.display(),
        image.crc32(),
    );
    confirm_write(confirm, &summary)?;

    writeln!(stdout, "device={}", flash.device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol_family=K5-V1").map_err(CliError::operation)?;
    writeln!(stdout, "image={}", flash.image_path.display()).map_err(CliError::operation)?;
    writeln!(stdout, "image_crc32={:08x}", image.crc32()).map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={:08x}", flash.transaction_id).map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;
    let report = flash_afik_application(
        serial,
        &image,
        &version,
        flash.transaction_id,
        VersionCheck::Known,
        |page| {
            let complete = page + 1;
            if complete % 16 == 0 || complete == 240 {
                let _ = writeln!(stdout, "acknowledged_pages={complete}/240");
                let _ = stdout.flush();
            }
        },
    )
    .map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", report.bootloader.version()).map_err(CliError::operation)?;
    writeln!(stdout, "pages_acknowledged={}", report.pages_acknowledged)
        .map_err(CliError::operation)?;
    writeln!(stdout, "status=acknowledged_not_read_back").map_err(CliError::operation)
}

fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let temporary = path.with_extension(format!("restore-{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&temporary).map_err(|error| {
        CliError::Operation(format!(
            "could not create temporary pre-write backup: {error}"
        ))
    })?;
    let result = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|error| CliError::Operation(format!("could not write pre-write backup: {error}")));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::hard_link(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::Operation(format!(
            "could not publish pre-write backup without overwrite: {error}"
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let result = File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| CliError::Operation(format!("could not sync backup directory: {error}")));
    let _ = fs::remove_file(&temporary);
    result
}

fn paths_same(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    #[cfg(unix)]
    if let (Ok(left), Ok(right)) = (fs::metadata(left), fs::metadata(right)) {
        if left.dev() == right.dev() && left.ino() == right.ino() {
            return true;
        }
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn parse_sha256(value: &str) -> Result<[u8; 32], CliError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Operation(
            "--confirm-recovery-sha256 requires exactly 64 hexadecimal digits".into(),
        ));
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(CliError::operation)?;
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::{
        parse_control, parse_gated_afik, parse_restore, render_view, validate_restore_paths,
        write_private_file_atomic, ControlRequest, RestoreArguments,
    };
    use radio_flasher::k5::View;

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(ToString::to_string).collect()
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("afik-k5-cli-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn control_requests_are_strict() {
        assert_eq!(
            parse_control(&strings(&["key", "AUDIO"])).unwrap(),
            ControlRequest::ToggleAudio
        );
        assert_eq!(
            parse_control(&strings(&["view"])).unwrap(),
            ControlRequest::View
        );
        assert!(parse_control(&strings(&["key", "PTT"])).is_err());
        assert!(parse_control(&strings(&["view", "extra"])).is_err());
    }

    #[test]
    fn gated_afik_requires_every_gate() {
        assert!(parse_gated_afik(&strings(&["afik.raw", "--recovery", "stock.raw"])).is_err());
        assert!(parse_gated_afik(&strings(&[
            "afik.raw",
            "--backup",
            "eeprom.raw",
            "--version",
            "F4HWN v4.3",
            "--confirm-target",
            "UV-K5-V1-DP32G030",
            "--confirm-image-crc32",
            "12345678",
            "--recovery",
            "stock.raw",
            "--confirm-recovery-rehearsed",
            "RECOVERY-REHEARSED-ON-THIS-UNIT",
        ]))
        .is_ok());
    }

    #[test]
    fn restore_requires_all_confirmation_gates() {
        assert!(parse_restore(&strings(&[
            "restore.raw",
            "--current",
            "current.raw",
            "--recovery",
            "recovery.raw",
            "--pre-write-backup",
            "before.raw",
            "--version",
            "2.01.23",
            "--confirm-target",
            "UV-K5-V1-DP32G030",
            "--confirm-recovery-crc32",
            "12345678",
            "--confirm-recovery-sha256",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--confirm-restore",
            "UV-K5-V1-DP32G030:90abcdef",
        ]))
        .is_ok());
        assert!(parse_restore(&strings(&["restore.raw"])).is_err());
    }

    #[test]
    fn view_output_reports_monitoring_separately_from_audio() {
        let mut output = Vec::new();
        render_view(
            View {
                channel: 3,
                audio: true,
                squelch_open: false,
                monitoring: true,
                receiver_ok: true,
            },
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert!(output.contains("audio=on\n"));
        assert!(output.contains("monitoring=on\n"));
        assert!(output.contains("receiver=ok\n"));
    }

    #[test]
    fn restore_preflight_rejects_hardlink_aliases_without_serial() {
        let original = temp_path("restore-original");
        let alias = temp_path("restore-alias");
        fs::write(&original, [1_u8]).unwrap();
        fs::hard_link(&original, &alias).unwrap();
        let arguments = RestoreArguments {
            restore: original.clone(),
            current: alias.clone(),
            recovery: temp_path("restore-recovery"),
            pre_write_backup: temp_path("restore-output"),
            version: "F4HWN v4.3".into(),
            target_confirmation: "UV-K5-V1-DP32G030".into(),
            recovery_crc32_confirmation: "00000000".into(),
            recovery_sha256_confirmation: "00".repeat(32),
            restore_confirmation: "UV-K5-V1-DP32G030:00000000".into(),
        };
        assert!(validate_restore_paths(&arguments).is_err());
        fs::remove_file(alias).unwrap();
        fs::remove_file(original).unwrap();
    }

    #[test]
    fn atomic_pre_write_backup_is_create_new_and_directory_synced() {
        let path = temp_path("atomic-backup");
        write_private_file_atomic(&path, b"backup").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"backup");
        assert!(write_private_file_atomic(&path, b"replacement").is_err());
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_pre_write_backup_rejects_dangling_symlink() {
        let path = temp_path("atomic-dangling");
        let target = temp_path("atomic-target");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(write_private_file_atomic(&path, b"backup").is_err());
        assert!(fs::symlink_metadata(&path).is_ok());
        fs::remove_file(path).unwrap();
    }
}
