//! Generic auto-detecting K1/K5 flasher front end.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use radio_flasher::{
    backup_eeprom, crc32, detect_bootloader, flash_application, probe_keypad_matrix,
    probe_normal_firmware, ApplicationImage, EepromBackup, FlashPrerequisites, FlashPurpose,
};
use radio_programmer_serial::{discover_usb_serial_devices, LinuxSerialTransport};

use crate::{
    fresh_transaction_id, open_serial, parse_crc32, read_bounded, write_failure,
    write_private_file, write_success, CliError, EXIT_OPERATION, EXIT_SUCCESS, EXIT_USAGE, K5_BAUD,
};

use radio_flasher::k1::K1RecoveryImage;

const K1_MAX_IMAGE_BYTES: usize =
    (radio_flasher::k1::K1_APPLICATION_END - radio_flasher::k1::K1_APPLICATION_ORIGIN) as usize;

/// Stable help text for the generic device-selecting flasher.
pub const HELP: &str = "AFIK K1/K5 auto-detecting recovery flasher\n\
\n\
Usage:\n\
  afik-flasher [--device PATH|auto] identify\n\
  afik-flasher [--device PATH|auto] probe-normal\n\
  afik-flasher [--device PATH|auto] probe-keypad\n\
  afik-flasher [--device PATH|auto] backup-eeprom OUTPUT [--force]\n\
  afik-flasher [--device PATH|auto] flash-recovery IMAGE --backup EEPROM \\\n    --confirm-target TARGET --confirm-image-crc32 CRC32 [--version VERSION]\n\
  afik-flasher [--device PATH|auto] flash-afik-k1 IMAGE --recovery RAW \\\n\
    --backup EEPROM --version VERSION --confirm-target TARGET \\\n\
    --confirm-image-crc32 CRC32 --confirm-recovery-rehearsed PHRASE\n\
  afik-flasher --help\n\
  afik-flasher --version\n\
\n\
The default device selector is auto. It accepts exactly one USB serial\n\
candidate, then classifies the bootloader protocol: K5 V1 2.* or the pinned\n\
K1 7.03.* family. Zero or multiple candidates fail closed.\n\
Recovery flashing remains separately gated. The K1 AFIK application command\n\
also requires a distinct recovery image, a known EEPROM backup, the exact AFIK\n\
target phrase, and confirmation that recovery was rehearsed on this unit.\n\
The read-only probe-normal command sends one normal-mode hello and is the\n\
serial witness command for an AFIK application. The read-only probe-keypad\n\
command prints four raw active-low row masks without interpreting them as keys.\n\
Serial is fixed at 38400 8-N-1.\n";

/// Runs one generic flasher invocation against supplied output streams.
pub fn run_to<W: Write, E: Write>(arguments: &[String], stdout: &mut W, stderr: &mut E) -> i32 {
    match parse(arguments) {
        Ok(Parsed::Help) => write_success(stdout, HELP),
        Ok(Parsed::Version) => write_success(
            stdout,
            &format!("afik-flasher {}\n", env!("CARGO_PKG_VERSION")),
        ),
        Ok(parsed) => match execute(parsed, stdout) {
            Ok(()) => EXIT_SUCCESS,
            Err(error) => write_failure(stderr, EXIT_OPERATION, error),
        },
        Err(error) => write_failure(stderr, EXIT_USAGE, error),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Parsed {
    Help,
    Version,
    Hardware {
        device: DeviceSelector,
        command: Command,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DeviceSelector {
    Auto,
    Explicit(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Identify,
    ProbeNormal,
    ProbeKeypad,
    Backup { output: PathBuf, force: bool },
    Flash(FlashArguments),
    FlashAfikK1(K1AfikFlashArguments),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FlashArguments {
    image: PathBuf,
    backup: PathBuf,
    version: Option<String>,
    target_confirmation: String,
    image_crc32_confirmation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct K1AfikFlashArguments {
    image: PathBuf,
    recovery: PathBuf,
    backup: PathBuf,
    version: String,
    target_confirmation: String,
    image_crc32_confirmation: String,
    recovery_rehearsed_confirmation: String,
}

struct FlashContext<'a> {
    image_raw: &'a [u8],
    backup: &'a EepromBackup,
    target_confirmation: &'a str,
    supplied_crc: u32,
    transaction_id: u32,
}

fn parse(arguments: &[String]) -> Result<Parsed, String> {
    if arguments.is_empty()
        || matches!(arguments, [argument] if matches!(argument.as_str(), "--help" | "-h" | "help"))
    {
        return Ok(Parsed::Help);
    }
    if matches!(arguments, [argument] if argument == "--version") {
        return Ok(Parsed::Version);
    }

    let (device, command_index) = if arguments
        .first()
        .is_some_and(|argument| argument == "--device")
    {
        let path = arguments
            .get(1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| "--device requires PATH or auto".to_owned())?;
        let device = if path == "auto" {
            DeviceSelector::Auto
        } else {
            DeviceSelector::Explicit(PathBuf::from(path))
        };
        (device, 2)
    } else {
        (DeviceSelector::Auto, 0)
    };
    let command = arguments.get(command_index).ok_or_else(|| {
        "a command is required: identify, probe-normal, probe-keypad, backup-eeprom, flash-recovery, or flash-afik-k1"
            .to_owned()
    })?;
    let tail = &arguments[command_index + 1..];
    let command = match command.as_str() {
        "identify" => {
            if !tail.is_empty() {
                return Err("identify does not accept arguments".into());
            }
            Command::Identify
        }
        "probe-normal" => {
            if !tail.is_empty() {
                return Err("probe-normal does not accept arguments".into());
            }
            Command::ProbeNormal
        }
        "probe-keypad" => {
            if !tail.is_empty() {
                return Err("probe-keypad does not accept arguments".into());
            }
            Command::ProbeKeypad
        }
        "backup-eeprom" => parse_backup(tail)?,
        "flash-recovery" => Command::Flash(parse_flash(tail)?),
        "flash-afik-k1" => Command::FlashAfikK1(parse_flash_afik_k1(tail)?),
        other => return Err(format!("unknown command: {other}")),
    };
    Ok(Parsed::Hardware { device, command })
}

fn parse_backup(arguments: &[String]) -> Result<Command, String> {
    let output = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "backup-eeprom requires OUTPUT".to_owned())?;
    let force = match &arguments[1..] {
        [] => false,
        [argument] if argument == "--force" => true,
        _ => return Err("backup-eeprom accepts only OUTPUT and optional --force".into()),
    };
    Ok(Command::Backup {
        output: PathBuf::from(output),
        force,
    })
}

fn parse_flash(arguments: &[String]) -> Result<FlashArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "flash-recovery requires IMAGE".to_owned())?;
    let mut backup = None;
    let mut version = None;
    let mut target_confirmation = None;
    let mut image_crc32_confirmation = None;
    let mut offset = 1;
    while offset < arguments.len() {
        let option = &arguments[offset];
        let value = arguments
            .get(offset + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--backup" => set_once(&mut backup, PathBuf::from(value), option)?,
            "--version" => set_once(&mut version, value.clone(), option)?,
            "--confirm-target" => set_once(&mut target_confirmation, value.clone(), option)?,
            "--confirm-image-crc32" => {
                set_once(&mut image_crc32_confirmation, value.clone(), option)?;
            }
            _ => return Err(format!("unknown flash option: {option}")),
        }
        offset += 2;
    }
    Ok(FlashArguments {
        image: PathBuf::from(image),
        backup: backup.ok_or_else(|| "flash-recovery requires --backup EEPROM".to_owned())?,
        version,
        target_confirmation: target_confirmation
            .ok_or_else(|| "flash-recovery requires --confirm-target TARGET".to_owned())?,
        image_crc32_confirmation: image_crc32_confirmation
            .ok_or_else(|| "flash-recovery requires --confirm-image-crc32 CRC32".to_owned())?,
    })
}

fn parse_flash_afik_k1(arguments: &[String]) -> Result<K1AfikFlashArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "flash-afik-k1 requires IMAGE".to_owned())?;
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
            _ => return Err(format!("unknown flash-afik-k1 option: {option}")),
        }
        offset += 2;
    }
    Ok(K1AfikFlashArguments {
        image: PathBuf::from(image),
        recovery: recovery.ok_or_else(|| "flash-afik-k1 requires --recovery RAW".to_owned())?,
        backup: backup.ok_or_else(|| "flash-afik-k1 requires --backup EEPROM".to_owned())?,
        version: version.ok_or_else(|| "flash-afik-k1 requires --version VERSION".to_owned())?,
        target_confirmation: target_confirmation
            .ok_or_else(|| "flash-afik-k1 requires --confirm-target TARGET".to_owned())?,
        image_crc32_confirmation: image_crc32_confirmation
            .ok_or_else(|| "flash-afik-k1 requires --confirm-image-crc32 CRC32".to_owned())?,
        recovery_rehearsed_confirmation: recovery_rehearsed_confirmation.ok_or_else(|| {
            "flash-afik-k1 requires --confirm-recovery-rehearsed PHRASE".to_owned()
        })?,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{option} was provided more than once"))
    } else {
        Ok(())
    }
}

fn execute<W: Write>(parsed: Parsed, stdout: &mut W) -> Result<(), CliError> {
    let Parsed::Hardware { device, command } = parsed else {
        unreachable!("handled before execution");
    };
    let device = resolve_device(&device)?;
    match command {
        Command::Identify => identify(&device, stdout),
        Command::ProbeNormal => probe_normal(&device, stdout),
        Command::ProbeKeypad => probe_keypad(&device, stdout),
        Command::Backup { output, force } => backup(&device, &output, force, stdout),
        Command::Flash(arguments) => flash(&device, &arguments, stdout),
        Command::FlashAfikK1(arguments) => flash_afik_k1(&device, &arguments, stdout),
    }
}

fn resolve_device(selector: &DeviceSelector) -> Result<PathBuf, CliError> {
    match selector {
        DeviceSelector::Explicit(path) => Ok(path.clone()),
        DeviceSelector::Auto => {
            let candidates = discover_usb_serial_devices().map_err(CliError::operation)?;
            select_auto_device(&candidates).map_err(CliError::Operation)
        }
    }
}

fn select_auto_device(candidates: &[PathBuf]) -> Result<PathBuf, String> {
    match candidates {
        [] => Err("no USB serial devices detected; supply --device PATH".into()),
        [device] => Ok(device.clone()),
        _ => {
            let list = candidates
                .iter()
                .map(|path| format!("  {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n");
            Err(format!(
                "multiple USB serial devices detected; choose one with --device PATH:\n{list}"
            ))
        }
    }
}

fn identify<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let family = detect_bootloader(&mut serial).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol_family={}", family.label()).map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", family.info().version()).map_err(CliError::operation)?;
    writeln!(stdout, "hardware_identity=not_proven_by_beacon").map_err(CliError::operation)
}

fn probe_normal<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let info = probe_normal_firmware(&mut serial).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=normal-firmware-hello").map_err(CliError::operation)?;
    writeln!(stdout, "firmware={}", info.version()).map_err(CliError::operation)
}

fn probe_keypad<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let report = probe_keypad_matrix(&mut serial).map_err(CliError::operation)?;
    let rows = report.row_low_by_column();
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-keypad-raw").map_err(CliError::operation)?;
    writeln!(stdout, "scan_valid={}", report.scan_valid()).map_err(CliError::operation)?;
    writeln!(stdout, "pb6_rows={:01x}", rows[0]).map_err(CliError::operation)?;
    writeln!(stdout, "pb5_rows={:01x}", rows[1]).map_err(CliError::operation)?;
    writeln!(stdout, "pb4_rows={:01x}", rows[2]).map_err(CliError::operation)?;
    writeln!(stdout, "pb3_rows={:01x}", rows[3]).map_err(CliError::operation)
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
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
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
    let image_raw = read_bounded(&arguments.image, K1_MAX_IMAGE_BYTES, "recovery image")?;
    let backup_raw = read_bounded(
        &arguments.backup,
        radio_flasher::EEPROM_BYTES,
        "EEPROM backup",
    )?;
    let backup = EepromBackup::from_raw(&backup_raw).map_err(CliError::operation)?;
    let supplied_crc = parse_crc32(&arguments.image_crc32_confirmation)?;
    let transaction_id = fresh_transaction_id()?;
    let mut serial = open_serial(device)?;
    let family = detect_bootloader(&mut serial).map_err(CliError::operation)?;
    let context = FlashContext {
        image_raw: &image_raw,
        backup: &backup,
        target_confirmation: &arguments.target_confirmation,
        supplied_crc,
        transaction_id,
    };

    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol_family={}", family.label()).map_err(CliError::operation)?;
    writeln!(stdout, "image={}", arguments.image.display()).map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={transaction_id:08x}").map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    match family {
        radio_flasher::BootloaderFamily::K1(info) => flash_k1(&mut serial, &context, &info, stdout),
        radio_flasher::BootloaderFamily::K5V1(_info) => {
            flash_k5(&mut serial, &context, arguments.version.as_deref(), stdout)
        }
    }
}

fn flash_afik_k1<W: Write>(
    device: &Path,
    arguments: &K1AfikFlashArguments,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image_raw = read_bounded(&arguments.image, K1_MAX_IMAGE_BYTES, "AFIK K1 image")?;
    let recovery_raw = read_bounded(&arguments.recovery, K1_MAX_IMAGE_BYTES, "K1 recovery image")?;
    let image = K1RecoveryImage::from_raw(&image_raw).map_err(CliError::operation)?;
    let recovery = K1RecoveryImage::from_raw(&recovery_raw).map_err(CliError::operation)?;
    let backup_raw = read_bounded(
        &arguments.backup,
        radio_flasher::EEPROM_BYTES,
        "EEPROM backup",
    )?;
    let backup = EepromBackup::from_raw(&backup_raw).map_err(CliError::operation)?;
    let supplied_crc = parse_crc32(&arguments.image_crc32_confirmation)?;
    let expected_crc = crc32(image.bytes());
    if supplied_crc != expected_crc {
        return Err(CliError::Operation(format!(
            "image CRC-32 confirmation mismatch: expected {expected_crc:08x}, supplied {supplied_crc:08x}"
        )));
    }
    let transaction_id = fresh_transaction_id()?;
    let mut serial = open_serial(device)?;
    let family = detect_bootloader(&mut serial).map_err(CliError::operation)?;
    let info = match &family {
        radio_flasher::BootloaderFamily::K1(info) => info,
        radio_flasher::BootloaderFamily::K5V1(info) => {
            return Err(CliError::Operation(format!(
                "flash-afik-k1 requires K1 bootloader 7.03.*, detected K5 V1 {}",
                info.version()
            )));
        }
    };
    if arguments.version != info.version() {
        return Err(CliError::Operation(format!(
            "K1 bootloader version confirmation mismatch: detected {}, supplied {}",
            info.version(),
            arguments.version
        )));
    }

    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol_family={}", family.label()).map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", info.version()).map_err(CliError::operation)?;
    writeln!(stdout, "image={}", arguments.image.display()).map_err(CliError::operation)?;
    writeln!(stdout, "recovery_image={}", arguments.recovery.display())
        .map_err(CliError::operation)?;
    writeln!(stdout, "backup_crc32={:08x}", backup.crc32()).map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={transaction_id:08x}").map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    let report = radio_flasher::k1::flash_application(
        &mut serial,
        &image,
        &recovery,
        info.version(),
        radio_flasher::k1::K1ApplicationConfirmations {
            target: &arguments.target_confirmation,
            recovery_rehearsed: &arguments.recovery_rehearsed_confirmation,
        },
        transaction_id,
        |page| {
            let complete = page + 1;
            if complete % 16 == 0 || complete == image.page_count() {
                let _ = writeln!(
                    stdout,
                    "acknowledged_pages={complete}/{}",
                    image.page_count()
                );
                let _ = stdout.flush();
            }
        },
    )
    .map_err(CliError::operation)?;
    writeln!(stdout, "pages_acknowledged={}", report.pages_acknowledged)
        .map_err(CliError::operation)?;
    writeln!(stdout, "status=acknowledged_not_read_back").map_err(CliError::operation)
}

fn flash_k1<W: Write>(
    serial: &mut LinuxSerialTransport,
    context: &FlashContext<'_>,
    info: &radio_flasher::BootloaderInfo,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image = K1RecoveryImage::from_raw(context.image_raw).map_err(CliError::operation)?;
    let expected_crc = crc32(image.bytes());
    if context.supplied_crc != expected_crc {
        return Err(CliError::Operation(format!(
            "image CRC-32 confirmation mismatch: expected {expected_crc:08x}, supplied {:08x}",
            context.supplied_crc
        )));
    }
    let report = radio_flasher::k1::flash_recovery(
        serial,
        &image,
        info.version(),
        context.target_confirmation,
        context.transaction_id,
        |page| {
            let complete = page + 1;
            if complete % 16 == 0 || complete == image.page_count() {
                let _ = writeln!(
                    stdout,
                    "acknowledged_pages={complete}/{}",
                    image.page_count()
                );
                let _ = stdout.flush();
            }
        },
    )
    .map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", report.bootloader_version).map_err(CliError::operation)?;
    writeln!(stdout, "pages_acknowledged={}", report.pages_acknowledged)
        .map_err(CliError::operation)?;
    writeln!(stdout, "status=acknowledged_not_read_back").map_err(CliError::operation)
}

fn flash_k5<W: Write>(
    serial: &mut LinuxSerialTransport,
    context: &FlashContext<'_>,
    version: Option<&str>,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image = ApplicationImage::from_raw(context.image_raw).map_err(CliError::operation)?;
    if context.supplied_crc != image.crc32() {
        return Err(CliError::Operation(format!(
            "image CRC-32 confirmation mismatch: expected {:08x}, supplied {:08x}",
            image.crc32(),
            context.supplied_crc
        )));
    }
    let version = version.ok_or_else(|| {
        CliError::Operation("K5 recovery requires --version VERSION (for example 2.01.23)".into())
    })?;
    let version = radio_flasher::FirmwareVersion::new(version).map_err(CliError::operation)?;
    let prerequisites = FlashPrerequisites {
        image: &image,
        recovery_image: &image,
        eeprom_backup: context.backup,
        version: &version,
        target_confirmation: context.target_confirmation,
        image_crc32_confirmation: image.crc32(),
        transaction_id: context.transaction_id,
        purpose: FlashPurpose::RecoveryRehearsal,
    };
    let report = flash_application(serial, prerequisites, |page| {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{parse, select_auto_device, Command, DeviceSelector, Parsed};

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_to_auto_and_accepts_explicit_device() {
        assert!(matches!(
            parse(&strings(&["identify"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::Identify,
            }
        ));
        assert!(matches!(
            parse(&strings(&["--device", "/dev/ttyUSB0", "identify"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Explicit(_),
                command: Command::Identify,
            }
        ));
        assert!(matches!(
            parse(&strings(&["probe-keypad"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::ProbeKeypad,
            }
        ));
        assert!(parse(&strings(&["probe-keypad", "extra"])).is_err());
    }

    #[test]
    fn recovery_parser_requires_selection_guards() {
        assert!(parse(&strings(&["flash-recovery", "image.raw"])).is_err());
        let arguments = strings(&[
            "flash-recovery",
            "image.raw",
            "--backup",
            "eeprom.raw",
            "--confirm-target",
            "UV-K1-F4HWN-7.03.01",
            "--confirm-image-crc32",
            "12345678",
        ]);
        assert!(matches!(
            parse(&arguments).unwrap(),
            Parsed::Hardware {
                command: Command::Flash(_),
                ..
            }
        ));
    }

    #[test]
    fn k1_afik_parser_requires_recovery_and_rehearsal_guards() {
        assert!(parse(&strings(&["flash-afik-k1", "image.raw"])).is_err());
        let arguments = strings(&[
            "flash-afik-k1",
            "image.raw",
            "--recovery",
            "recovery.raw",
            "--backup",
            "eeprom.raw",
            "--version",
            "7.03.01",
            "--confirm-target",
            "UV-K1-AFIK-7.03.01",
            "--confirm-image-crc32",
            "12345678",
            "--confirm-recovery-rehearsed",
            "K1-RECOVERY-REHEARSED-ON-THIS-UNIT",
        ]);
        assert!(matches!(
            parse(&arguments).unwrap(),
            Parsed::Hardware {
                command: Command::FlashAfikK1(_),
                ..
            }
        ));
    }

    #[test]
    fn auto_selection_fails_closed_for_zero_or_multiple_candidates() {
        assert!(select_auto_device(&[]).unwrap_err().contains("no USB"));
        let candidates = vec![PathBuf::from("/dev/ttyUSB0"), PathBuf::from("/dev/ttyUSB1")];
        let error = select_auto_device(&candidates).unwrap_err();
        assert!(error.contains("multiple USB"));
        assert!(error.contains("ttyUSB0"));
        assert!(error.contains("ttyUSB1"));
    }
}
