//! Thin K5-compatible and generic K1/K5 recovery-flasher front ends.

#![forbid(unsafe_code)]

mod auto;
mod parse;

use parse::{
    parse, AfikApplicationArguments, Command, FlashArguments, Parsed, Purpose, RestoreArguments,
};
use prompt::Confirm;
use radio_flasher::k5::{control as k5_control, ControlRequest, ControlResponse, LogicalKey, View};
use radio_flasher::{
    apply_eeprom_restore, backup_eeprom, flash_afik_application, flash_application,
    prepare_eeprom_restore, probe_bootloader_v2, sha256, ApplicationImage, EepromBackup,
    EepromRestorePrerequisites, FirmwareVersion, FlashPrerequisites, FlashPurpose,
    APPLICATION_BYTES, EEPROM_BYTES, K5_RECOVERY_SOURCE_BYTES,
};
use radio_programmer_serial::LinuxSerialTransport;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// Bootloader negotiation version sent by the default AFIK flash.
pub const DEFAULT_NEGOTIATION_VERSION: &str = "2.01.23";

/// Successful process exit code.
pub const EXIT_SUCCESS: i32 = 0;
/// File, validation, transport, protocol, or device failure exit code.
pub const EXIT_OPERATION: i32 = 1;
/// Command-line usage failure exit code.
pub const EXIT_USAGE: i32 = 2;
/// Exact serial speed observed for the supported legacy protocol.
pub const K5_BAUD: u32 = 38_400;

/// Runs the generic auto-detecting K1/K5 CLI.
pub mod prompt;

pub use auto::{run_to as run_auto_to, run_with as run_auto_with, HELP as AUTO_HELP};

/// Stable command help text.
pub const HELP: &str = "AFIK UV-K5 V1 recovery-gated flasher\n\
\n\
Usage:\n\
  afik-k5 inspect IMAGE\n\
  afik-k5 --device PATH probe\n\
  afik-k5 --device PATH control view|capabilities|key AUDIO|UP|DOWN\n\
  afik-k5 --device PATH backup-eeprom OUTPUT [--force]\n\
  afik-k5 --device PATH restore-eeprom RESTORE --current BACKUP --recovery IMAGE \\\n\
    --pre-write-backup OUTPUT --version NORMAL_VERSION --confirm-target TARGET \\\n\
    --confirm-recovery-crc32 CRC32 --confirm-recovery-sha256 SHA256 \\\n\
    --confirm-restore TARGET:CRC32\n\
  afik-k5 --device PATH flash-recovery IMAGE --backup EEPROM --version VERSION \\\n    --confirm-target UV-K5-V1-DP32G030 --confirm-image-crc32 CRC32\n\
  afik-k5 --device PATH flash-afik IMAGE [--yes] [--version VERSION]\n\
  afik-k5 --device PATH flash-afik IMAGE --recovery RAW --backup EEPROM \\\n    --version VERSION --confirm-target UV-K5-V1-DP32G030 \\\n    --confirm-image-crc32 CRC32 \\\n    --confirm-recovery-rehearsed RECOVERY-REHEARSED-ON-THIS-UNIT\n\
  afik-k5 --help\n\
  afik-k5 --version\n\
\n\
Only an inspected UV-K5 V1 with DP32G030 and bootloader v2 is supported.\n\
Serial is fixed at 38400 8-N-1. The normal-mode control command sends one\n\
bounded read/control request and validates one exact response; it never writes\n\
EEPROM, persists configuration, transmits, or flashes. Flashing writes all 240\n\
application pages, never the stock bootloader, and reports acknowledgements\n\
rather than read-back.\n\
The default flash-afik takes an AFIK image only: it must carry an AFIK-K5-\n\
identity, the radio is probed read-only, and the write is confirmed at a\n\
prompt (--yes skips it; without a terminal the write is refused). It needs no\n\
backup or recovery image because it touches neither the bootloader nor EEPROM;\n\
an image that does not boot is recovered by flashing again. The negotiation\n\
version defaults to 2.01.23. Passing --recovery selects the recovery-gated form,\n\
which requires every listed option.\n\
Exit codes: 0 success, 1 operation failure, 2 usage failure.\n";

/// Runs one CLI invocation against supplied output streams.
pub fn run_to<W: Write, E: Write>(arguments: &[String], stdout: &mut W, stderr: &mut E) -> i32 {
    run_k5_with(arguments, &mut prompt::terminal(), stdout, stderr)
}

/// Runs one invocation against an explicit confirmer.
pub fn run_k5_with<C: Confirm, W: Write, E: Write>(
    arguments: &[String],
    confirm: &mut C,
    stdout: &mut W,
    stderr: &mut E,
) -> i32 {
    match parse(arguments) {
        Ok(Parsed::Help) => write_success(stdout, HELP),
        Ok(Parsed::Version) => {
            write_success(stdout, &format!("afik-k5 {}\n", env!("CARGO_PKG_VERSION")))
        }
        Ok(parsed) => match execute(parsed, confirm, stdout) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => write_failure(stderr, EXIT_OPERATION, error),
        },
        Err(error) => write_failure(stderr, EXIT_USAGE, error),
    }
}

fn execute<C: Confirm, W: Write>(
    parsed: Parsed,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    match parsed {
        Parsed::Inspect { image } => inspect(&image, stdout),
        Parsed::Hardware { device, command } => match command {
            Command::Probe => probe(&device, stdout),
            Command::Backup { output, force } => backup(&device, &output, force, stdout),
            Command::Restore(arguments) => restore(&device, &arguments, stdout),
            Command::Control(request) => control(&device, request, stdout),
            Command::Flash(arguments) => flash(&device, &arguments, stdout),
            Command::FlashAfikApplication(arguments) => {
                if arguments.yes {
                    flash_afik_default(&device, &arguments, &mut prompt::AssumeYes, stdout)
                } else {
                    flash_afik_default(&device, &arguments, confirm, stdout)
                }
            }
        },
        Parsed::Help | Parsed::Version => unreachable!("handled before execution"),
    }
}

fn inspect<W: Write>(path: &Path, stdout: &mut W) -> Result<(), CliError> {
    let raw = read_bounded(path, APPLICATION_BYTES, "application")?;
    let image = ApplicationImage::from_raw(&raw).map_err(CliError::operation)?;
    writeln!(stdout, "source={}", path.display()).map_err(CliError::operation)?;
    writeln!(stdout, "source_bytes={}", image.source_len()).map_err(CliError::operation)?;
    writeln!(stdout, "application_bytes={}", image.bytes().len()).map_err(CliError::operation)?;
    writeln!(stdout, "initial_sp=0x{:08x}", image.initial_stack()).map_err(CliError::operation)?;
    writeln!(stdout, "reset_vector=0x{:08x}", image.reset_vector()).map_err(CliError::operation)?;
    writeln!(stdout, "crc32={:08x}", image.crc32()).map_err(CliError::operation)
}

fn probe<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let info = probe_bootloader_v2(&mut serial).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", info.version()).map_err(CliError::operation)?;
    writeln!(stdout, "hardware_identity=not_proven_by_beacon").map_err(CliError::operation)
}

fn control<W: Write>(
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

fn backup<W: Write>(
    device: &Path,
    output: &Path,
    force: bool,
    stdout: &mut W,
) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let (info, backup) = backup_eeprom(&mut serial).map_err(CliError::operation)?;
    write_private_file(output, backup.bytes(), force)?;
    writeln!(stdout, "firmware={}", info.version()).map_err(CliError::operation)?;
    writeln!(stdout, "backup={}", output.display()).map_err(CliError::operation)?;
    writeln!(stdout, "bytes={}", backup.bytes().len()).map_err(CliError::operation)?;
    writeln!(stdout, "crc32={:08x}", backup.crc32()).map_err(CliError::operation)
}

fn restore<W: Write>(
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

fn flash<W: Write>(
    device: &Path,
    arguments: &FlashArguments,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image_raw = read_bounded(&arguments.image, APPLICATION_BYTES, "application")?;
    let image = ApplicationImage::from_raw(&image_raw).map_err(CliError::operation)?;
    let backup_raw = read_bounded(&arguments.backup, EEPROM_BYTES, "EEPROM backup")?;
    let backup = EepromBackup::from_raw(&backup_raw).map_err(CliError::operation)?;
    let recovery_storage = match &arguments.recovery {
        Some(path) => {
            let raw = read_bounded(path, APPLICATION_BYTES, "recovery application")?;
            Some(ApplicationImage::from_raw(&raw).map_err(CliError::operation)?)
        }
        None => None,
    };
    let recovery = recovery_storage.as_ref().unwrap_or(&image);
    let version = FirmwareVersion::new(&arguments.version).map_err(CliError::operation)?;
    let confirmation = parse_crc32(&arguments.image_crc32_confirmation)?;
    let transaction_id = fresh_transaction_id()?;
    let purpose = match arguments.purpose {
        Purpose::Recovery => FlashPurpose::RecoveryRehearsal,
        Purpose::Afik => FlashPurpose::Afik {
            recovery_rehearsed_confirmation: arguments
                .recovery_rehearsed_confirmation
                .as_deref()
                .unwrap_or(""),
        },
    };
    let prerequisites = FlashPrerequisites {
        image: &image,
        recovery_image: recovery,
        eeprom_backup: &backup,
        version: &version,
        target_confirmation: &arguments.target_confirmation,
        image_crc32_confirmation: confirmation,
        transaction_id,
        purpose,
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

fn flash_afik_default<C: Confirm, W: Write>(
    device: &Path,
    arguments: &AfikApplicationArguments,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image_raw = read_bounded(&arguments.image, APPLICATION_BYTES, "application")?;
    let image = ApplicationImage::from_raw(&image_raw).map_err(CliError::operation)?;
    let version = FirmwareVersion::new(
        arguments
            .version
            .as_deref()
            .unwrap_or(DEFAULT_NEGOTIATION_VERSION),
    )
    .map_err(CliError::operation)?;
    let transaction_id = fresh_transaction_id()?;
    let mut serial = open_serial(device)?;
    // Read-only, so the confirmation names the radio actually on the cable.
    let bootloader = probe_bootloader_v2(&mut serial).map_err(CliError::operation)?;
    let summary = format!(
        "About to write an AFIK application to a K5 V1:\n\
         \x20 device:     {}\n\
         \x20 bootloader: {}\n\
         \x20 image:      {}\n\
         \x20 crc32:      {:08x}\n\
         This replaces the application, not the bootloader or EEPROM, and cannot be read back.",
        device.display(),
        bootloader.version(),
        arguments.image.display(),
        image.crc32(),
    );
    if !confirm.confirm(&summary).map_err(CliError::operation)? {
        return Err(CliError::Operation(if confirm.is_interactive() {
            "write declined".to_owned()
        } else {
            "nothing can answer a confirmation prompt here; pass --yes to write unattended"
                .to_owned()
        }));
    }

    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "image={}", arguments.image.display()).map_err(CliError::operation)?;
    writeln!(stdout, "image_crc32={:08x}", image.crc32()).map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={transaction_id:08x}").map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;
    let report = flash_afik_application(&mut serial, &image, &version, transaction_id, |page| {
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

fn open_serial(path: &Path) -> Result<LinuxSerialTransport, CliError> {
    LinuxSerialTransport::open(path, K5_BAUD).map_err(CliError::operation)
}

fn read_bounded(path: &Path, maximum: usize, description: &str) -> Result<Vec<u8>, CliError> {
    let file = File::open(path)
        .map_err(|error| CliError::Operation(format!("could not open {description}: {error}")))?;
    let limit = u64::try_from(maximum + 1).expect("bounded host file limit");
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::Operation(format!("could not read {description}: {error}")))?;
    if bytes.len() > maximum {
        return Err(CliError::Operation(format!(
            "{description} exceeds {maximum} bytes"
        )));
    }
    Ok(bytes)
}

fn write_private_file(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let mut options = OpenOptions::new();
    options.write(true).mode(0o600);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(path)
        .map_err(|error| CliError::Operation(format!("could not create backup: {error}")))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| CliError::Operation(format!("could not write backup: {error}")))
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

fn parse_crc32(value: &str) -> Result<u32, CliError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Operation(
            "--confirm-image-crc32 requires exactly eight hexadecimal digits".into(),
        ));
    }
    u32::from_str_radix(value, 16).map_err(CliError::operation)
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

fn fresh_transaction_id() -> Result<u32, CliError> {
    let mut random = File::open("/dev/urandom").map_err(CliError::operation)?;
    for _ in 0..4 {
        let mut bytes = [0_u8; 4];
        random.read_exact(&mut bytes).map_err(CliError::operation)?;
        let transaction_id = u32::from_le_bytes(bytes);
        if transaction_id != 0 {
            return Ok(transaction_id);
        }
    }
    Err(CliError::Operation(
        "could not obtain a nonzero transaction identifier".into(),
    ))
}

fn write_success<W: Write>(output: &mut W, text: &str) -> i32 {
    output
        .write_all(text.as_bytes())
        .map_or(EXIT_OPERATION, |()| EXIT_SUCCESS)
}

fn write_failure<E: Write>(output: &mut E, code: i32, error: impl fmt::Display) -> i32 {
    let _ = writeln!(output, "error: {error}");
    code
}

#[derive(Debug)]
enum CliError {
    Operation(String),
}

impl CliError {
    fn operation(error: impl fmt::Display) -> Self {
        Self::Operation(error.to_string())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::{
        render_view, run_to, validate_restore_paths, write_private_file_atomic, EXIT_SUCCESS,
        EXIT_USAGE, HELP,
    };
    use crate::parse::RestoreArguments;
    use radio_flasher::k5::View;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("afik-k5-cli-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn help_and_usage_status_are_stable() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run_to(&["--help".into()], &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
        assert_eq!(stdout, HELP.as_bytes());
        assert!(HELP.contains("--confirm-recovery-sha256 SHA256"));
        assert!(!HELP.contains("\\n+"));
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(run_to(&[], &mut stdout, &mut stderr), EXIT_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.starts_with(b"error: "));
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
    fn inspect_prints_normalised_vector_and_crc_contract() {
        let path = temp_path("inspect");
        let mut raw = vec![0xA5; 32];
        raw[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&9_u32.to_le_bytes());
        fs::write(&path, raw).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_to(
            &["inspect".into(), path.display().to_string()],
            &mut stdout,
            &mut stderr,
        );
        fs::remove_file(path).unwrap();
        assert_eq!(code, EXIT_SUCCESS);
        let output = String::from_utf8(stdout).unwrap();
        assert!(output.contains("source_bytes=32\n"));
        assert!(output.contains("application_bytes=61440\n"));
        assert!(output.contains("initial_sp=0x20004000\n"));
        assert!(output.contains("reset_vector=0x00000009\n"));
        assert!(output.contains("crc32="));
        assert!(stderr.is_empty());
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
