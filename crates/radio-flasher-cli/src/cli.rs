//! The AFIK flasher command line: one tool for the K1 and the K5 V1.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use radio_flasher::k5::ControlRequest;
use radio_flasher::{
    backup_eeprom, crc32, detect_bootloader, flash_application, observe_bootloader,
    probe_clock_control, probe_clock_register, probe_clock_snapshot, probe_keypad_matrix,
    probe_normal_firmware, probe_rf, set_rf_audio, ApplicationImage, BootloaderFamily,
    EepromBackup, FlashPrerequisites, FlashPurpose, APPLICATION_BYTES,
};
use radio_programmer_serial::{discover_usb_serial_devices, LinuxSerialTransport};

use crate::k5::{self, GatedAfikArguments, RestoreArguments};
use crate::{
    confirm_write, fresh_transaction_id, open_serial, parse_crc32, read_bounded, set_once,
    write_failure, write_private_file, write_success, CliError, DetectedFlash, EXIT_OPERATION,
    EXIT_SUCCESS, EXIT_USAGE, K5_BAUD,
};

use radio_flasher::k1::K1RecoveryImage;

use crate::prompt::{self, AssumeYes, Confirm};

const K1_MAX_IMAGE_BYTES: usize =
    (radio_flasher::k1::K1_APPLICATION_END - radio_flasher::k1::K1_APPLICATION_ORIGIN) as usize;

/// Stable help text.
pub const HELP: &str = "AFIK K1/K5 flasher\n\
\n\
Usage:\n\
  afik-flasher inspect IMAGE\n\
  afik-flasher [--device PATH|auto] identify\n\
  afik-flasher [--device PATH|auto] flash-afik [IMAGE] [--yes] [--version VERSION]\n\
  afik-flasher [--device PATH|auto] backup-eeprom OUTPUT [--force]\n\
  afik-flasher [--device PATH|auto] flash-recovery IMAGE --backup EEPROM \\\n    --confirm-target TARGET --confirm-image-crc32 CRC32 [--version VERSION]\n\
  afik-flasher [--device PATH|auto] flash-afik IMAGE --recovery RAW --backup EEPROM \\\n    --version VERSION --confirm-target UV-K5-V1-DP32G030 \\\n    --confirm-image-crc32 CRC32 \\\n    --confirm-recovery-rehearsed RECOVERY-REHEARSED-ON-THIS-UNIT\n\
  afik-flasher [--device PATH|auto] restore-eeprom RESTORE --current BACKUP \\\n    --recovery IMAGE --pre-write-backup OUTPUT --version NORMAL_VERSION \\\n    --confirm-target TARGET --confirm-recovery-crc32 CRC32 \\\n    --confirm-recovery-sha256 SHA256 --confirm-restore TARGET:CRC32\n\
  afik-flasher [--device PATH|auto] control view|capabilities|key AUDIO|UP|DOWN\n\
  afik-flasher [--device PATH|auto] probe-normal\n\
  afik-flasher [--device PATH|auto] probe-keypad\n\
  afik-flasher [--device PATH|auto] probe-rf\n\
  afik-flasher [--device PATH|auto] rf-audio on|off\n\
  afik-flasher [--device PATH|auto] probe-clock\n\
  afik-flasher [--device PATH|auto] probe-clock-register CR|ICSCR|CFGR|PLLCFGR\n\
  afik-flasher [--device PATH|auto] probe-clock-control\n\
  afik-flasher --help\n\
  afik-flasher --version\n\
\n\
The default device selector is auto. One USB serial candidate is used, and\n\
several are offered as a choice on a terminal or fail closed without one.\n\
Serial is fixed at 38400 8-N-1.\n\
\n\
Bootloader commands. identify reports the beacon as observed: its command, the\n\
protocol that command announces, and the printable bootloader version. It makes\n\
no claim about which radio or processor this is. flash-afik reads the beacon\n\
and writes to whichever radio it finds: a K5 V1 2.* bootloader takes a K5\n\
image carrying an AFIK-K5- identity, and a K1 7.03.* bootloader takes a K1\n\
image. Without IMAGE it takes the packaged image for the detected radio from\n\
the workspace target directory. An image built for the other radio is refused.\n\
Before writing, it shows the device, bootloader, image and CRC-32 for\n\
confirmation; --yes skips that prompt, and without a terminal the write is\n\
refused rather than assumed. It cannot reach either bootloader or EEPROM, so\n\
an application which does not boot is recovered by flashing again. --version\n\
sets the K5 negotiation version (default 2.01.23). Passing --recovery selects\n\
the K5 recovery-gated form, which requires every listed option. flash-recovery\n\
and restore-eeprom put a unit at risk and still require their backups and\n\
exact phrases. Writes report page acknowledgements, not read-back. inspect\n\
prints a K5 image's vectors and CRC-32 without a radio.\n\
\n\
Normal-mode commands talk to running AFIK firmware. control (K5) sends one\n\
bounded read or key request and never writes EEPROM, persists configuration,\n\
transmits or flashes. probe-normal sends one hello. probe-keypad prints four\n\
raw active-low row masks. probe-rf prints the raw receive observation, and\n\
rf-audio routes or mutes receive audio; neither can key the radio. probe-clock\n\
prints the inherited RCC clock registers and the fail-closed contract result,\n\
probe-clock-register reads one named register, and probe-clock-control returns\n\
a constant without MMIO.\n\
Exit codes: 0 success, 1 operation failure, 2 usage failure.\n";

/// Runs one invocation against supplied output streams.
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
#[allow(clippy::large_enum_variant)]
enum Parsed {
    Help,
    Version,
    Inspect {
        image: PathBuf,
    },
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
#[allow(clippy::large_enum_variant)]
enum Command {
    Identify,
    ProbeNormal,
    ProbeKeypad,
    ProbeRf,
    RfAudio(bool),
    ProbeClock,
    ProbeClockRegister(usize),
    ProbeClockControl,
    Control(ControlRequest),
    Backup { output: PathBuf, force: bool },
    Restore(RestoreArguments),
    Flash(FlashArguments),
    FlashAfik(AfikArguments),
    FlashAfikGated(GatedAfikArguments),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FlashArguments {
    image: PathBuf,
    backup: PathBuf,
    version: Option<String>,
    target_confirmation: String,
    image_crc32_confirmation: String,
}

/// The default AFIK application flash: the image and nothing else.
#[derive(Clone, Debug, Eq, PartialEq)]
struct AfikArguments {
    /// The image to write; without one, the packaged image for the detected
    /// radio is taken from the workspace `target` directory.
    image: Option<PathBuf>,
    /// K5 bootloader negotiation version; refused for a K1.
    version: Option<String>,
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
    if arguments
        .first()
        .is_some_and(|argument| argument == "inspect")
    {
        return match arguments {
            [_, image] => Ok(Parsed::Inspect {
                image: PathBuf::from(image),
            }),
            _ => Err("inspect requires exactly one IMAGE".into()),
        };
    }

    let (device, command_index) = if arguments
        .first()
        .is_some_and(|argument| argument == "--device")
    {
        let path = arguments
            .get(1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
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
        "a command is required: identify, flash-afik, backup-eeprom, flash-recovery, restore-eeprom, control, probe-normal, probe-keypad, probe-rf, rf-audio, probe-clock, probe-clock-register, or probe-clock-control"
            .to_owned()
    })?;
    let command = parse_command(command, &arguments[command_index + 1..])?;
    Ok(Parsed::Hardware { device, command })
}

fn parse_command(command: &str, tail: &[String]) -> Result<Command, String> {
    let bare = match command {
        "identify" => Some(Command::Identify),
        "probe-normal" => Some(Command::ProbeNormal),
        "probe-rf" => Some(Command::ProbeRf),
        "probe-keypad" => Some(Command::ProbeKeypad),
        "probe-clock" => Some(Command::ProbeClock),
        "probe-clock-control" => Some(Command::ProbeClockControl),
        _ => None,
    };
    if let Some(bare) = bare {
        return if tail.is_empty() {
            Ok(bare)
        } else {
            Err(format!("{command} does not accept arguments"))
        };
    }
    Ok(match command {
        "rf-audio" => match tail {
            [state] if state == "on" => Command::RfAudio(true),
            [state] if state == "off" => Command::RfAudio(false),
            _ => return Err("rf-audio requires on or off".into()),
        },
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
        "control" => Command::Control(k5::parse_control(tail)?),
        "backup-eeprom" => parse_backup(tail)?,
        "restore-eeprom" => Command::Restore(k5::parse_restore(tail)?),
        "flash-recovery" => Command::Flash(parse_flash(tail)?),
        "flash-afik" if tail.iter().any(|argument| argument == "--recovery") => {
            Command::FlashAfikGated(k5::parse_gated_afik(tail)?)
        }
        "flash-afik" => Command::FlashAfik(parse_flash_afik(tail)?),
        other => return Err(format!("unknown command: {other}")),
    })
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

fn parse_flash_afik(arguments: &[String]) -> Result<AfikArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .map(PathBuf::from);
    let mut version = None;
    let mut assume_yes = false;
    let mut offset = usize::from(image.is_some());
    while offset < arguments.len() {
        match arguments[offset].as_str() {
            "--yes" if !assume_yes => assume_yes = true,
            "--yes" => return Err("--yes was provided more than once".into()),
            "--version" => {
                let value = arguments
                    .get(offset + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "--version requires a value".to_owned())?;
                set_once(&mut version, value.clone(), "--version")?;
                offset += 1;
            }
            option => {
                return Err(format!(
                    "unknown flash-afik option: {option}; pass --recovery to select the K5 recovery-gated form"
                ))
            }
        }
        offset += 1;
    }
    Ok(AfikArguments {
        image,
        version,
        assume_yes,
    })
}

fn execute<W: Write, C: Confirm>(
    parsed: Parsed,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let (device, command) = match parsed {
        Parsed::Inspect { image } => return k5::inspect(&image, stdout),
        Parsed::Hardware { device, command } => (device, command),
        Parsed::Help | Parsed::Version => unreachable!("handled before execution"),
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
        Command::Control(request) => k5::control(&device, request, stdout),
        Command::Backup { output, force } => backup(&device, &output, force, stdout),
        Command::Restore(arguments) => k5::restore(&device, &arguments, stdout),
        Command::Flash(arguments) => flash(&device, &arguments, stdout),
        Command::FlashAfikGated(arguments) => k5::flash_afik_gated(&device, &arguments, stdout),
        Command::FlashAfik(arguments) => {
            if arguments.assume_yes {
                flash_afik(&device, &arguments, &mut AssumeYes, stdout)
            } else {
                flash_afik(&device, &arguments, confirm, stdout)
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
        BootloaderFamily::K1(info) => flash_k1(&mut serial, &context, &info, stdout),
        BootloaderFamily::K5V1(_info) => {
            flash_k5(&mut serial, &context, arguments.version.as_deref(), stdout)
        }
    }
}

/// Writes an AFIK application to whichever radio's bootloader answers.
fn flash_afik<W: Write, C: Confirm>(
    device: &Path,
    arguments: &AfikArguments,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let transaction_id = fresh_transaction_id()?;
    let mut serial = open_serial(device)?;
    // Classification is read-only, so it runs before the operator is asked. That
    // way the confirmation names the radio actually on the other end of the
    // cable rather than the one the operator meant to plug in.
    let family = detect_bootloader(&mut serial).map_err(CliError::operation)?;
    let radio = Radio::of(&family);
    let image_path = arguments
        .image
        .clone()
        .unwrap_or_else(|| radio.default_image());
    if arguments.image.is_none() && !image_path.is_file() {
        return Err(CliError::Operation(format!(
            "no IMAGE given and the default {} image {} does not exist; build it with {}",
            radio.label(),
            image_path.display(),
            radio.build_hint()
        )));
    }
    let image_raw = read_bounded(
        &image_path,
        APPLICATION_BYTES.max(K1_MAX_IMAGE_BYTES),
        "AFIK image",
    )?;
    check_afik_image(radio, &image_raw).map_err(|error| {
        CliError::Operation(format!(
            "{error}; detected bootloader {}",
            family.info().version()
        ))
    })?;
    let flash = DetectedFlash {
        device,
        image_path: &image_path,
        image_raw: &image_raw,
        bootloader: family.info(),
        transaction_id,
    };
    match &family {
        BootloaderFamily::K5V1(_) => k5::flash_afik_detected(
            &mut serial,
            &flash,
            arguments.version.as_deref(),
            confirm,
            stdout,
        ),
        BootloaderFamily::K1(_) if arguments.version.is_some() => Err(CliError::Operation(
            "--version sets the K5 negotiation version; a K1 bootloader was detected".into(),
        )),
        BootloaderFamily::K1(_) => flash_afik_k1(&mut serial, &flash, confirm, stdout),
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

    const fn label(self) -> &'static str {
        match self {
            Self::K5V1 => "K5-V1",
            Self::K1 => "K1",
        }
    }

    /// The packaged image `tool/package-*-image.sh` writes for this radio.
    fn default_image(self) -> PathBuf {
        let name = match self {
            Self::K5V1 => "afik-k5-v1.raw",
            Self::K1 => "radio-firmware-k1.raw",
        };
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/thumbv6m-none-eabi/release")
            .join(name)
    }

    const fn build_hint(self) -> &'static str {
        match self {
            Self::K5V1 => "tool/build-k5.sh --release && tool/package-k5-image.sh",
            Self::K1 => "tool/build-k1.sh --release && tool/package-k1-image.sh",
        }
    }
}

/// Refuses an image built for the other radio before anything is asked or sent.
///
/// The two layouts cannot be confused: a K5 image's reset vector lies inside its
/// own 60 KiB from address zero, and a K1 image's lies in the K1 flash region.
fn check_afik_image(radio: Radio, image_raw: &[u8]) -> Result<(), String> {
    let fits = match radio {
        Radio::K5V1 => ApplicationImage::from_raw(image_raw)
            .map(drop)
            .map_err(|error| error.to_string()),
        Radio::K1 => K1RecoveryImage::from_raw(image_raw)
            .map(drop)
            .map_err(|error| error.to_string()),
    };
    fits.map_err(|error| format!("image is not a {} application ({error})", radio.label()))
}

fn flash_afik_k1<W: Write, C: Confirm>(
    serial: &mut LinuxSerialTransport,
    flash: &DetectedFlash<'_>,
    confirm: &mut C,
    stdout: &mut W,
) -> Result<(), CliError> {
    let image = K1RecoveryImage::from_raw(flash.image_raw).map_err(CliError::operation)?;
    let image_crc = crc32(image.bytes());
    let summary = format!(
        "About to write an AFIK application to a K1:\n\
         \x20 device:     {}\n\
         \x20 bootloader: {}\n\
         \x20 image:      {}\n\
         \x20 bytes:      {} in {} pages\n\
         \x20 crc32:      {image_crc:08x}\n\
         This replaces the application and cannot be read back.",
        flash.device.display(),
        flash.bootloader.version(),
        flash.image_path.display(),
        image.bytes().len(),
        image.page_count(),
    );
    confirm_write(confirm, &summary)?;

    writeln!(stdout, "device={}", flash.device.display()).map_err(CliError::operation)?;
    writeln!(stdout, "baud={K5_BAUD}").map_err(CliError::operation)?;
    writeln!(stdout, "protocol_family=K1").map_err(CliError::operation)?;
    writeln!(stdout, "bootloader={}", flash.bootloader.version()).map_err(CliError::operation)?;
    writeln!(stdout, "image={}", flash.image_path.display()).map_err(CliError::operation)?;
    writeln!(stdout, "image_crc32={image_crc:08x}").map_err(CliError::operation)?;
    writeln!(stdout, "transaction_id={:08x}", flash.transaction_id).map_err(CliError::operation)?;
    stdout.flush().map_err(CliError::operation)?;

    let report = radio_flasher::k1::flash_application(
        serial,
        &image,
        None,
        flash.bootloader.version(),
        flash.transaction_id,
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

    use super::{
        check_afik_image, parse, run_to, run_with, select_auto_device, Command, DeviceSelector,
        Parsed, Radio, HELP,
    };
    use crate::prompt::TerminalConfirm;
    use crate::{EXIT_SUCCESS, EXIT_USAGE};
    use radio_flasher::k5::ControlRequest;

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
        assert!(HELP.contains("control view|capabilities|key AUDIO|UP|DOWN"));
        assert!(stderr.is_empty());

        stdout.clear();
        assert_eq!(
            run_to(&strings(&["unknown"]), &mut stdout, &mut stderr),
            EXIT_USAGE
        );
        assert!(stdout.is_empty());
        assert!(stderr.starts_with(b"error: "));
    }

    #[test]
    fn inspect_needs_no_device_and_prints_the_vector_and_crc_contract() {
        assert!(parse(&strings(&["inspect"])).is_err());
        assert!(parse(&strings(&["inspect", "a.raw", "b.raw"])).is_err());

        let path = std::env::temp_dir().join(format!("afik-inspect-{}", std::process::id()));
        let mut raw = vec![0xA5; 32];
        raw[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&9_u32.to_le_bytes());
        std::fs::write(&path, raw).unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_to(
            &["inspect".into(), path.display().to_string()],
            &mut stdout,
            &mut stderr,
        );
        std::fs::remove_file(path).unwrap();
        assert_eq!(code, EXIT_SUCCESS);
        let output = String::from_utf8(stdout).unwrap();
        assert!(output.contains("source_bytes=32\n"));
        assert!(output.contains("application_bytes=61440\n"));
        assert!(output.contains("initial_sp=0x20004000\n"));
        assert!(output.contains("reset_vector=0x00000009\n"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn k5_normal_mode_and_restore_commands_take_the_device_selector() {
        assert!(matches!(
            parse(&strings(&["control", "key", "AUDIO"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Auto,
                command: Command::Control(ControlRequest::ToggleAudio),
            }
        ));
        assert!(matches!(
            parse(&strings(&["--device", "/dev/x", "control", "view"])).unwrap(),
            Parsed::Hardware {
                device: DeviceSelector::Explicit(_),
                command: Command::Control(ControlRequest::View),
            }
        ));
        assert!(parse(&strings(&["control", "key", "PTT"])).is_err());
        assert!(parse(&strings(&["restore-eeprom", "restore.raw"])).is_err());
    }

    /// The image is the whole command; the radio decides which protocol is used.
    #[test]
    fn flash_afik_takes_the_image_and_nothing_else() {
        let Parsed::Hardware {
            command: Command::FlashAfik(parsed),
            ..
        } = parse(&strings(&["flash-afik", "image.raw"])).unwrap()
        else {
            panic!("an image alone is a complete command");
        };
        assert!(!parsed.assume_yes);
        assert_eq!(parsed.version, None);
        assert_eq!(parsed.image, Some(PathBuf::from("image.raw")));

        let Parsed::Hardware {
            command: Command::FlashAfik(parsed),
            ..
        } = parse(&strings(&[
            "flash-afik",
            "image.raw",
            "--yes",
            "--version",
            "2.01.26",
        ]))
        .unwrap()
        else {
            panic!("expected an AFIK flash command");
        };
        assert!(parsed.assume_yes);
        assert_eq!(parsed.version.as_deref(), Some("2.01.26"));

        // A strict-gate option without --recovery is refused, not ignored.
        assert!(parse(&strings(&[
            "flash-afik",
            "image.raw",
            "--backup",
            "eeprom.raw"
        ]))
        .is_err());
        assert!(parse(&strings(&["flash-afik", "image.raw", "--yes", "--yes"])).is_err());
        // Without IMAGE the detected radio's packaged image is used.
        let Parsed::Hardware {
            command: Command::FlashAfik(parsed),
            ..
        } = parse(&strings(&["flash-afik", "--yes"])).unwrap()
        else {
            panic!("IMAGE is optional");
        };
        assert_eq!(parsed.image, None);
        assert!(parsed.assume_yes);
        // --recovery selects the K5 recovery-gated form, which needs every gate.
        assert!(parse(&strings(&[
            "flash-afik",
            "image.raw",
            "--recovery",
            "stock.raw"
        ]))
        .is_err());
        // The K1-only command is gone; flash-afik detects the radio.
        assert!(parse(&strings(&["flash-afik-k1", "image.raw"])).is_err());
    }

    /// The defaults are the files the packaging scripts write, in the workspace
    /// target directory rather than wherever the command happens to run.
    #[test]
    fn default_images_are_the_packaged_ones_for_each_radio() {
        let k5 = Radio::K5V1.default_image();
        let k1 = Radio::K1.default_image();
        assert!(k5.ends_with("target/thumbv6m-none-eabi/release/afik-k5-v1.raw"));
        assert!(k1.ends_with("target/thumbv6m-none-eabi/release/radio-firmware-k1.raw"));
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(k5.starts_with(&workspace));
        assert!(workspace.join("Cargo.toml").is_file());
    }

    /// An image built for one radio is refused when the other one answers.
    #[test]
    fn an_image_for_the_other_radio_is_refused() {
        let (k5, k1) = (Radio::K5V1, Radio::K1);
        let mut k5_image = vec![0xA5; 32];
        k5_image[0..4].copy_from_slice(&0x2000_4000_u32.to_le_bytes());
        k5_image[4..8].copy_from_slice(&9_u32.to_le_bytes());
        let mut k1_image = vec![0xA5; 32];
        k1_image[0..4].copy_from_slice(&radio_flasher::k1::K1_SRAM_END.to_le_bytes());
        k1_image[4..8]
            .copy_from_slice(&(radio_flasher::k1::K1_APPLICATION_ORIGIN + 9).to_le_bytes());

        assert!(check_afik_image(k5, &k5_image).is_ok());
        assert!(check_afik_image(k1, &k1_image).is_ok());
        let error = check_afik_image(k5, &k1_image).unwrap_err();
        assert!(error.contains("not a K5-V1 application"), "{error}");
        let error = check_afik_image(k1, &k5_image).unwrap_err();
        assert!(error.contains("not a K1 application"), "{error}");
    }

    /// A write must not proceed unless something actually approved it.
    #[test]
    fn a_declined_or_unanswerable_confirmation_writes_nothing() {
        // No device is opened and no image is read: the refusal happens before
        // any of that, so these run without hardware.
        let arguments = strings(&["--device", "/dev/null", "flash-afik", "/nonexistent.raw"]);
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
