//! Generic auto-detecting K1/K5 flasher front end.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use radio_flasher::{
    backup_eeprom, crc32, detect_bootloader, flash_application, observe_bootloader,
    probe_clock_control, probe_clock_register, probe_clock_snapshot, probe_keypad_matrix,
    probe_normal_firmware, probe_rf, set_rf_audio, ApplicationImage, EepromBackup,
    FlashPrerequisites, FlashPurpose,
};
use radio_programmer_serial::{discover_usb_serial_devices, LinuxSerialTransport};

use crate::{
    fresh_transaction_id, open_serial, parse_crc32, read_bounded, write_failure,
    write_private_file, write_success, CliError, EXIT_OPERATION, EXIT_SUCCESS, EXIT_USAGE, K5_BAUD,
};

use radio_flasher::k1::K1RecoveryImage;

use crate::prompt::{self, AssumeYes, Confirm};

const K1_MAX_IMAGE_BYTES: usize =
    (radio_flasher::k1::K1_APPLICATION_END - radio_flasher::k1::K1_APPLICATION_ORIGIN) as usize;

/// Stable help text for the generic device-selecting flasher.
pub const HELP: &str = "AFIK K1/K5 auto-detecting recovery flasher\n\
\n\
Usage:\n\
  afik-flasher [--device PATH|auto] identify\n\
  afik-flasher [--device PATH|auto] probe-normal\n\
  afik-flasher [--device PATH|auto] probe-keypad\n\
  afik-flasher [--device PATH|auto] probe-rf\n\
  afik-flasher [--device PATH|auto] rf-audio on|off\n\
  afik-flasher [--device PATH|auto] probe-clock\n\
  afik-flasher [--device PATH|auto] probe-clock-register CR|ICSCR|CFGR|PLLCFGR\n\
  afik-flasher [--device PATH|auto] probe-clock-control\n\
  afik-flasher [--device PATH|auto] backup-eeprom OUTPUT [--force]\n\
  afik-flasher [--device PATH|auto] flash-recovery IMAGE --backup EEPROM \\\n    --confirm-target TARGET --confirm-image-crc32 CRC32 [--version VERSION]\n\
  afik-flasher [--device PATH|auto] flash-afik-k1 IMAGE \\\n\
    [--yes] [--recovery RAW] [--backup EEPROM]\n\
  afik-flasher --help\n\
  afik-flasher --version\n\
\n\
The default device selector is auto. One USB serial candidate is used, and\n\
several are offered as a choice on a terminal or fail closed without one.\n\
identify then reports the beacon as observed: its command, the protocol that\n\
command announces, and the printable bootloader version, whatever that version\n\
is. It makes no claim about which radio or processor this is, and an unfamiliar\n\
version is reported rather than refused. Write paths remain separately gated to\n\
the qualified K5 V1 2.* and pinned K1 7.03.* targets.\n\
Recovery flashing remains separately gated. The K1 AFIK application command\n\
takes the image and nothing else. It cannot reach the bootloader: the protocol\n\
addresses a page index, not an address, and the image is bounded to the\n\
application region, so an application which does not boot is recovered by\n\
flashing again. Before writing, it classifies the radio read-only and shows the\n\
device, bootloader, image, size and CRC-32 for confirmation. Pass --yes to skip\n\
that prompt; without a terminal to ask, the write is refused rather than\n\
assumed. A retained recovery image and EEPROM backup are optional and are\n\
validated when supplied. Recovery flashing, which does put a unit at risk,\n\
still requires the backup and its exact phrases.\n\
The read-only probe-normal command sends one normal-mode hello and is the\n\
serial witness command for an AFIK application. The read-only probe-keypad\n\
command prints four raw active-low row masks without interpreting them as keys.\n\
The rf-audio command routes or mutes demodulated receive audio. It drives the\n\
receive audio chain only and cannot key the radio.\n\
The read-only probe-rf command prints the raw receive observation: the\n\
read-back register, the bring-up stage, and the latest RSSI, glitch, noise,\n\
and squelch sample. It cannot request a transmission.\n\
The read-only probe-clock command prints the inherited RCC clock registers and\n\
the target's fail-closed contract result without changing the clock tree.\n\
The diagnostic probe-clock-register command reads exactly one named register.\n\
The diagnostic probe-clock-control command returns a constant without MMIO.\n\
Serial is fixed at 38400 8-N-1.\n";

/// Runs one generic flasher invocation against supplied output streams.
pub fn run_to<W: Write, E: Write>(arguments: &[String], stdout: &mut W, stderr: &mut E) -> i32 {
    run_with(arguments, &mut prompt::terminal(), stdout, stderr)
}

/// Runs one invocation against an explicit confirmer.
///
/// Separating this from [`run_to`] is what lets the confirmation behaviour be
/// tested without a terminal attached.
pub fn run_with<C: Confirm, W: Write, E: Write>(
    arguments: &[String],
    confirm: &mut C,
    stdout: &mut W,
    stderr: &mut E,
) -> i32 {
    match parse(arguments) {
        Ok(Parsed::Help) => write_success(stdout, HELP),
        Ok(Parsed::Version) => write_success(
            stdout,
            &format!("afik-flasher {}\n", env!("CARGO_PKG_VERSION")),
        ),
        Ok(parsed) => match execute(parsed, confirm, stdout) {
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
    ProbeRf,
    RfAudio(bool),
    ProbeClock,
    ProbeClockRegister(usize),
    ProbeClockControl,
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
    /// Optional retained recovery image, checked against the image being
    /// written when it is supplied. This command cannot reach the bootloader,
    /// so recovery from a bad application is a second flash.
    recovery: Option<PathBuf>,
    /// Optional retained EEPROM backup, validated and logged when supplied.
    /// This command issues no EEPROM operation.
    backup: Option<PathBuf>,
    /// Skip the confirmation prompt because the operator already decided.
    assume_yes: bool,
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
        "a command is required: identify, probe-normal, probe-keypad, probe-rf, probe-clock, probe-clock-register, probe-clock-control, backup-eeprom, flash-recovery, or flash-afik-k1"
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
        "rf-audio" => match tail {
            [state] if state == "on" => Command::RfAudio(true),
            [state] if state == "off" => Command::RfAudio(false),
            _ => return Err("rf-audio requires on or off".into()),
        },
        "probe-rf" => {
            if !tail.is_empty() {
                return Err("probe-rf does not accept arguments".into());
            }
            Command::ProbeRf
        }
        "probe-keypad" => {
            if !tail.is_empty() {
                return Err("probe-keypad does not accept arguments".into());
            }
            Command::ProbeKeypad
        }
        "probe-clock" => {
            if !tail.is_empty() {
                return Err("probe-clock does not accept arguments".into());
            }
            Command::ProbeClock
        }
        "probe-clock-register" => {
            let index = match tail {
                [name] if name == "CR" => 0,
                [name] if name == "ICSCR" => 1,
                [name] if name == "CFGR" => 2,
                [name] if name == "PLLCFGR" => 3,
                _ => return Err("probe-clock-register requires CR, ICSCR, CFGR, or PLLCFGR".into()),
            };
            Command::ProbeClockRegister(index)
        }
        "probe-clock-control" => {
            if !tail.is_empty() {
                return Err("probe-clock-control does not accept arguments".into());
            }
            Command::ProbeClockControl
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
    let mut assume_yes = false;
    let mut offset = 1;
    while offset < arguments.len() {
        let option = &arguments[offset];
        if option == "--yes" {
            if assume_yes {
                return Err("--yes was provided more than once".to_owned());
            }
            assume_yes = true;
            offset += 1;
            continue;
        }
        let value = arguments
            .get(offset + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option.as_str() {
            "--recovery" => set_once(&mut recovery, PathBuf::from(value), option)?,
            "--backup" => set_once(&mut backup, PathBuf::from(value), option)?,
            _ => return Err(format!("unknown flash-afik-k1 option: {option}")),
        }
        offset += 2;
    }
    Ok(K1AfikFlashArguments {
        image: PathBuf::from(image),
        recovery,
        backup,
        assume_yes,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{option} was provided more than once"))
    } else {
        Ok(())
    }
}

fn execute<W: Write, C: Confirm>(
    parsed: Parsed,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let Parsed::Hardware { device, command } = parsed else {
        unreachable!("handled before execution");
    };
    let device = resolve_device(&device, confirm)?;
    match command {
        Command::Identify => identify(&device, stdout),
        Command::ProbeNormal => probe_normal(&device, stdout),
        Command::ProbeKeypad => probe_keypad(&device, stdout),
        Command::ProbeRf => probe_rf_observation(&device, stdout),
        Command::RfAudio(routed) => rf_audio(&device, routed, stdout),
        Command::ProbeClock => probe_clock(&device, stdout),
        Command::ProbeClockRegister(index) => probe_clock_register_named(&device, index, stdout),
        Command::ProbeClockControl => probe_clock_control_marker(&device, stdout),
        Command::Backup { output, force } => backup(&device, &output, force, stdout),
        Command::Flash(arguments) => flash(&device, &arguments, stdout),
        Command::FlashAfikK1(arguments) => {
            if arguments.assume_yes {
                flash_afik_k1(&device, &arguments, &mut AssumeYes, stdout)
            } else {
                flash_afik_k1(&device, &arguments, confirm, stdout)
            }
        }
    }
}

fn resolve_device<C: Confirm>(
    selector: &DeviceSelector,
    confirm: &mut C,
) -> Result<PathBuf, CliError> {
    match selector {
        DeviceSelector::Explicit(path) => Ok(path.clone()),
        DeviceSelector::Auto => {
            let candidates = discover_usb_serial_devices().map_err(CliError::operation)?;
            select_auto_device(&candidates, confirm).map_err(CliError::Operation)
        }
    }
}

/// Picks the radio to talk to, asking only when the answer is genuinely unclear.
///
/// One candidate is not a question worth asking: the operator plugged in one
/// radio and every command then names the device it used in its own output.
/// Several candidates is a real ambiguity, and guessing there could write to the
/// wrong device, so it is either answered by the operator or refused.
fn select_auto_device<C: Confirm>(
    candidates: &[PathBuf],
    confirm: &mut C,
) -> Result<PathBuf, String> {
    match candidates {
        [] => Err("no USB serial devices detected; supply --device PATH".into()),
        [device] => Ok(device.clone()),
        _ => {
            let options = candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            match confirm.choose("Several USB serial devices are present:", &options) {
                Ok(Some(index)) => Ok(candidates[index].clone()),
                Ok(None) | Err(_) => Err(format!(
                    "multiple USB serial devices detected; choose one with --device PATH:\n{}",
                    options
                        .iter()
                        .map(|option| format!("  {option}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )),
            }
        }
    }
}

fn identify<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let beacon = observe_bootloader(&mut serial).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "beacon_command=0x{:04x}", beacon.command()).map_err(CliError::operation)?;
    writeln!(stdout, "protocol={}", beacon.protocol().label()).map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", beacon.version()).map_err(CliError::operation)?;
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

fn rf_audio<W: Write>(device: &Path, routed: bool, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let report = set_rf_audio(&mut serial, routed).map_err(CliError::operation)?;
    write_rf_report(device, &report, stdout)
}

fn probe_rf_observation<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let report = probe_rf(&mut serial).map_err(CliError::operation)?;
    write_rf_report(device, &report, stdout)
}

fn write_rf_report<W: Write>(
    device: &Path,
    report: &radio_flasher::RfReport,
    stdout: &mut W,
) -> Result<(), CliError> {
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-receive-raw").map_err(CliError::operation)?;
    writeln!(stdout, "stage={}", report.stage()).map_err(CliError::operation)?;
    writeln!(
        stdout,
        "readback_register={:02x}={:04x}",
        report.identity_address(),
        report.identity_register()
    )
    .map_err(CliError::operation)?;
    writeln!(stdout, "frequency_hz={}", report.frequency_hz()).map_err(CliError::operation)?;
    writeln!(stdout, "samples={}", report.samples()).map_err(CliError::operation)?;
    writeln!(stdout, "rssi_dbm_x2={}", report.rssi_dbm_x2()).map_err(CliError::operation)?;
    writeln!(stdout, "glitch={}", report.glitch()).map_err(CliError::operation)?;
    writeln!(stdout, "noise={}", report.noise()).map_err(CliError::operation)?;
    writeln!(stdout, "squelch_open={}", report.squelch_open()).map_err(CliError::operation)?;
    writeln!(stdout, "audio_routed={}", report.audio_routed()).map_err(CliError::operation)
}

fn probe_keypad<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let report = probe_keypad_matrix(&mut serial).map_err(CliError::operation)?;
    let idr = report.gpio_b_idr_by_column();
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-keypad-raw").map_err(CliError::operation)?;
    writeln!(stdout, "scan_valid={}", report.scan_valid()).map_err(CliError::operation)?;
    writeln!(stdout, "captured={}", report.captured()).map_err(CliError::operation)?;
    writeln!(stdout, "pb6_idr={:04x}", idr[0]).map_err(CliError::operation)?;
    writeln!(stdout, "pb5_idr={:04x}", idr[1]).map_err(CliError::operation)?;
    writeln!(stdout, "pb4_idr={:04x}", idr[2]).map_err(CliError::operation)?;
    writeln!(stdout, "pb3_idr={:04x}", idr[3]).map_err(CliError::operation)
}

fn probe_clock<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let report = probe_clock_snapshot(&mut serial).map_err(CliError::operation)?;
    let registers = report.registers();
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-clock-raw").map_err(CliError::operation)?;
    writeln!(stdout, "contract_valid={}", report.contract_valid()).map_err(CliError::operation)?;
    writeln!(stdout, "rcc_cr={:08x}", registers[0]).map_err(CliError::operation)?;
    writeln!(stdout, "rcc_icscr={:08x}", registers[1]).map_err(CliError::operation)?;
    writeln!(stdout, "rcc_cfgr={:08x}", registers[2]).map_err(CliError::operation)?;
    writeln!(stdout, "rcc_pllcfgr={:08x}", registers[3]).map_err(CliError::operation)
}

fn probe_clock_register_named<W: Write>(
    device: &Path,
    index: usize,
    stdout: &mut W,
) -> Result<(), CliError> {
    const NAMES: [&str; 4] = ["CR", "ICSCR", "CFGR", "PLLCFGR"];
    let mut serial = open_serial(device)?;
    let value = probe_clock_register(&mut serial, index).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-clock-register").map_err(CliError::operation)?;
    writeln!(stdout, "register={}", NAMES[index]).map_err(CliError::operation)?;
    writeln!(stdout, "value={value:08x}").map_err(CliError::operation)
}

fn probe_clock_control_marker<W: Write>(device: &Path, stdout: &mut W) -> Result<(), CliError> {
    let mut serial = open_serial(device)?;
    let marker = probe_clock_control(&mut serial).map_err(CliError::operation)?;
    writeln!(stdout, "device={}", device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol=afik-k1-clock-control").map_err(CliError::operation)?;
    writeln!(stdout, "marker={marker:08x}").map_err(CliError::operation)
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

fn flash_afik_k1<W: Write, C: Confirm>(
    device: &Path,
    arguments: &K1AfikFlashArguments,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image_raw = read_bounded(&arguments.image, K1_MAX_IMAGE_BYTES, "AFIK K1 image")?;
    let image = K1RecoveryImage::from_raw(&image_raw).map_err(CliError::operation)?;
    // Both retained artefacts are optional for this command. It addresses a page
    // index rather than an address, so it cannot reach the bootloader, and it
    // issues no EEPROM operation. Recovery from an application which does not
    // boot is another flash through the same passive beacon. When either is
    // supplied it is still fully validated, so a caller who does retain them
    // gets the same accidental-selection checks as before.
    let recovery_raw = arguments
        .recovery
        .as_ref()
        .map(|path| read_bounded(path, K1_MAX_IMAGE_BYTES, "K1 recovery image"))
        .transpose()?;
    let recovery = recovery_raw
        .as_deref()
        .map(K1RecoveryImage::from_raw)
        .transpose()
        .map_err(CliError::operation)?;
    let backup = arguments
        .backup
        .as_ref()
        .map(|path| read_bounded(path, radio_flasher::EEPROM_BYTES, "EEPROM backup"))
        .transpose()?
        .as_deref()
        .map(EepromBackup::from_raw)
        .transpose()
        .map_err(CliError::operation)?;
    let image_crc = crc32(image.bytes());
    let transaction_id = fresh_transaction_id()?;
    let mut serial = open_serial(device)?;
    // Classification is read-only, so it runs before the operator is asked. That
    // way the confirmation names the radio actually on the other end of the
    // cable rather than the one the operator meant to plug in.
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

    let summary = format!(
        "About to write an AFIK application to a radio:\n\
         \x20 device:     {}\n\
         \x20 bootloader: K1 {}\n\
         \x20 image:      {}\n\
         \x20 bytes:      {} in {} pages\n\
         \x20 crc32:      {image_crc:08x}\n\
         This replaces the application and cannot be read back.",
        device.display(),
        info.version(),
        arguments.image.display(),
        image.bytes().len(),
        image.page_count(),
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
    writeln!(stdout, "protocol_family={}", family.label()).map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", info.version()).map_err(CliError::operation)?;
    writeln!(stdout, "image={}", arguments.image.display()).map_err(CliError::operation)?;
    match &arguments.recovery {
        Some(path) => writeln!(stdout, "recovery_image={}", path.display()),
        None => writeln!(stdout, "recovery_image=none_retained"),
    }
    .map_err(CliError::operation)?;
    match &backup {
        Some(backup) => writeln!(stdout, "backup_crc32={:08x}", backup.crc32()),
        None => writeln!(stdout, "backup_crc32=none_retained"),
    }
    .map_err(CliError::operation)?;
    writeln!(stdout, "image_crc32={image_crc:08x}").map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={transaction_id:08x}").map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    let report = radio_flasher::k1::flash_application(
        &mut serial,
        &image,
        recovery.as_ref(),
        info.version(),
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

    use super::{parse, run_with, select_auto_device, Command, DeviceSelector, Parsed};
    use crate::prompt::TerminalConfirm;
    use crate::EXIT_SUCCESS;

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
        assert!(matches!(
            parse(&strings(&["probe-clock"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::ProbeClock,
            }
        ));
        assert!(parse(&strings(&["probe-clock", "extra"])).is_err());
        assert!(matches!(
            parse(&strings(&["probe-clock-register", "PLLCFGR"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::ProbeClockRegister(3),
            }
        ));
        assert!(parse(&strings(&["probe-clock-register", "RCC"])).is_err());
        assert!(matches!(
            parse(&strings(&["probe-clock-control"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::ProbeClockControl,
            }
        ));
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

    /// The image is the whole command. Everything else is optional.
    #[test]
    fn k1_afik_parser_takes_the_image_and_nothing_else() {
        assert!(
            matches!(
                parse(&strings(&["flash-afik-k1", "image.raw"])).unwrap(),
                Parsed::Hardware {
                    command: Command::FlashAfikK1(_),
                    ..
                }
            ),
            "an image alone is a complete command"
        );

        let arguments = strings(&[
            "flash-afik-k1",
            "image.raw",
            "--yes",
            "--recovery",
            "recovery.raw",
            "--backup",
            "eeprom.raw",
        ]);
        let Parsed::Hardware {
            command: Command::FlashAfikK1(parsed),
            ..
        } = parse(&arguments).unwrap()
        else {
            panic!("expected a K1 AFIK flash command");
        };
        assert!(parsed.assume_yes);
        assert_eq!(parsed.recovery, Some(PathBuf::from("recovery.raw")));
        assert_eq!(parsed.backup, Some(PathBuf::from("eeprom.raw")));

        // The phrases this command used to demand are gone, so passing one is a
        // mistake worth reporting rather than something silently ignored.
        assert!(parse(&strings(&[
            "flash-afik-k1",
            "image.raw",
            "--confirm-target",
            "UV-K1-AFIK-7.03.01",
        ]))
        .is_err());
        assert!(
            parse(&strings(&["flash-afik-k1"])).is_err(),
            "IMAGE is required"
        );
    }

    /// A write must not proceed unless something actually approved it.
    #[test]
    fn a_declined_or_unanswerable_confirmation_writes_nothing() {
        // No device is opened and no image is read: the refusal happens before
        // any of that, so these run without hardware.
        let arguments = strings(&["--device", "/dev/null", "flash-afik-k1", "/nonexistent.raw"]);
        let mut declined = TerminalConfirm::new(&b"n\n"[..], Vec::new(), true);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_ne!(
            run_with(&arguments, &mut declined, &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );

        let mut silent = TerminalConfirm::new(&b"y\n"[..], Vec::new(), false);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_ne!(
            run_with(&arguments, &mut silent, &mut stdout, &mut stderr),
            EXIT_SUCCESS
        );
    }

    #[test]
    fn auto_selection_asks_only_when_the_radio_is_ambiguous() {
        let mut never = TerminalConfirm::new(&b""[..], Vec::new(), false);
        assert!(select_auto_device(&[], &mut never)
            .unwrap_err()
            .contains("no USB"));

        // One candidate is not a question: the operator plugged in one radio.
        let one = vec![PathBuf::from("/dev/ttyUSB0")];
        assert_eq!(
            select_auto_device(&one, &mut never).unwrap(),
            PathBuf::from("/dev/ttyUSB0")
        );

        let candidates = vec![PathBuf::from("/dev/ttyUSB0"), PathBuf::from("/dev/ttyUSB1")];
        let error = select_auto_device(&candidates, &mut never).unwrap_err();
        assert!(error.contains("multiple USB"));
        assert!(error.contains("ttyUSB0"));
        assert!(error.contains("ttyUSB1"));

        // On a terminal the ambiguity is a question, and the answer is used.
        let mut second = TerminalConfirm::new(&b"2\n"[..], Vec::new(), true);
        assert_eq!(
            select_auto_device(&candidates, &mut second).unwrap(),
            PathBuf::from("/dev/ttyUSB1")
        );

        // A nonsense answer is not a selection.
        let mut nonsense = TerminalConfirm::new(&b"9\n"[..], Vec::new(), true);
        assert!(select_auto_device(&candidates, &mut nonsense).is_err());
    }
}
