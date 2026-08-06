//! Thin explicit-device CLI for recovery-gated UV-K5 V1 deployment.

#![forbid(unsafe_code)]

mod parse;

use parse::{parse, Command, FlashArguments, Parsed, Purpose};
use radio_k5_flasher::{
    backup_eeprom, flash_application, probe_bootloader_v2, ApplicationImage, EepromBackup,
    FirmwareVersion, FlashPrerequisites, FlashPurpose, APPLICATION_BYTES, EEPROM_BYTES,
};
use radio_programmer_serial::LinuxSerialTransport;
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// Successful process exit code.
pub const EXIT_SUCCESS: i32 = 0;
/// File, validation, transport, protocol, or device failure exit code.
pub const EXIT_OPERATION: i32 = 1;
/// Command-line usage failure exit code.
pub const EXIT_USAGE: i32 = 2;
/// Exact serial speed observed for the supported legacy protocol.
pub const K5_BAUD: u32 = 38_400;

/// Stable command help text.
pub const HELP: &str = "AFIK UV-K5 V1 recovery-gated flasher\n\
\n\
Usage:\n\
  afik-k5 inspect IMAGE\n\
  afik-k5 --device PATH probe\n\
  afik-k5 --device PATH backup-eeprom OUTPUT [--force]\n\
  afik-k5 --device PATH flash-recovery IMAGE --backup EEPROM --version VERSION \\\n+    --confirm-target UV-K5-V1-DP32G030 --confirm-image-crc32 CRC32\n\
  afik-k5 --device PATH flash-afik IMAGE --recovery RAW --backup EEPROM \\\n+    --version VERSION --confirm-target UV-K5-V1-DP32G030 \\\n+    --confirm-image-crc32 CRC32 \\\n+    --confirm-recovery-rehearsed RECOVERY-REHEARSED-ON-THIS-UNIT\n\
  afik-k5 --help\n\
  afik-k5 --version\n\
\n\
Only an inspected UV-K5 V1 with DP32G030 and bootloader v2 is supported.\n\
Serial is fixed at 38400 8-N-1. Flashing writes all 240 application pages,\n\
never the stock bootloader, and reports acknowledgements rather than read-back.\n\
Exit codes: 0 success, 1 operation failure, 2 usage failure.\n";

/// Runs one CLI invocation against supplied output streams.
pub fn run_to<W: Write, E: Write>(arguments: &[String], stdout: &mut W, stderr: &mut E) -> i32 {
    match parse(arguments) {
        Ok(Parsed::Help) => write_success(stdout, HELP),
        Ok(Parsed::Version) => {
            write_success(stdout, &format!("afik-k5 {}\n", env!("CARGO_PKG_VERSION")))
        }
        Ok(parsed) => match execute(parsed, stdout) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => write_failure(stderr, EXIT_OPERATION, error),
        },
        Err(error) => write_failure(stderr, EXIT_USAGE, error),
    }
}

fn execute<W: Write>(parsed: Parsed, stdout: &mut W) -> Result<(), CliError> {
    match parsed {
        Parsed::Inspect { image } => inspect(&image, stdout),
        Parsed::Hardware { device, command } => match command {
            Command::Probe => probe(&device, stdout),
            Command::Backup { output, force } => backup(&device, &output, force, stdout),
            Command::Flash(arguments) => flash(&device, &arguments, stdout),
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

fn parse_crc32(value: &str) -> Result<u32, CliError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Operation(
            "--confirm-image-crc32 requires exactly eight hexadecimal digits".into(),
        ));
    }
    u32::from_str_radix(value, 16).map_err(CliError::operation)
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

    use super::{run_to, EXIT_SUCCESS, EXIT_USAGE, HELP};

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
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(run_to(&[], &mut stdout, &mut stderr), EXIT_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.starts_with(b"error: "));
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
}
