use std::path::PathBuf;

use radio_flasher::k5::{ControlRequest, LogicalKey};

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Parsed {
    Help,
    Version,
    Inspect { image: PathBuf },
    Hardware { device: PathBuf, command: Command },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Probe,
    Backup { output: PathBuf, force: bool },
    Control(ControlRequest),
    Flash(FlashArguments),
    FlashAfikApplication(AfikApplicationArguments),
    Restore(RestoreArguments),
}

/// The default AFIK application flash: image only, with a confirmation prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AfikApplicationArguments {
    pub(crate) image: PathBuf,
    pub(crate) version: Option<String>,
    pub(crate) yes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FlashArguments {
    pub(crate) purpose: Purpose,
    pub(crate) image: PathBuf,
    pub(crate) recovery: Option<PathBuf>,
    pub(crate) backup: PathBuf,
    pub(crate) version: String,
    pub(crate) target_confirmation: String,
    pub(crate) image_crc32_confirmation: String,
    pub(crate) recovery_rehearsed_confirmation: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Purpose {
    Recovery,
    Afik,
}

pub(crate) fn parse(arguments: &[String]) -> Result<Parsed, String> {
    if matches!(arguments, [argument] if matches!(argument.as_str(), "--help" | "-h" | "help")) {
        return Ok(Parsed::Help);
    }
    if matches!(arguments, [argument] if argument == "--version") {
        return Ok(Parsed::Version);
    }
    if matches!(arguments, [command, _] if command == "inspect") {
        return Ok(Parsed::Inspect {
            image: PathBuf::from(&arguments[1]),
        });
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "inspect")
    {
        return Err("inspect requires exactly one IMAGE".into());
    }
    if arguments.len() < 3 || arguments[0] != "--device" {
        return Err("hardware commands require --device PATH before the command".into());
    }
    let device = PathBuf::from(&arguments[1]);
    if device.as_os_str().is_empty() || arguments[1].starts_with("--") {
        return Err("--device requires an explicit path".into());
    }
    let command = match arguments[2].as_str() {
        "probe" => {
            if arguments.len() != 3 {
                return Err("probe does not accept arguments".into());
            }
            Command::Probe
        }
        "backup-eeprom" => parse_backup(&arguments[3..])?,
        "restore-eeprom" => parse_restore(&arguments[3..])?,
        "control" => parse_control(&arguments[3..])?,
        "flash-recovery" => Command::Flash(parse_flash(Purpose::Recovery, &arguments[3..])?),
        "flash-afik"
            if arguments[3..]
                .iter()
                .any(|argument| argument == "--recovery") =>
        {
            Command::Flash(parse_flash(Purpose::Afik, &arguments[3..])?)
        }
        "flash-afik" => {
            Command::FlashAfikApplication(parse_flash_afik_application(&arguments[3..])?)
        }
        command => return Err(format!("unknown command: {command}")),
    };
    Ok(Parsed::Hardware { device, command })
}

fn parse_control(arguments: &[String]) -> Result<Command, String> {
    let request = match arguments {
        [operation] if operation == "view" => ControlRequest::View,
        [operation] if operation == "capabilities" => ControlRequest::Capabilities,
        [operation, key] if operation == "key" => match key.as_str() {
            "AUDIO" => ControlRequest::ToggleAudio,
            "UP" => ControlRequest::Key(LogicalKey::Up),
            "DOWN" => ControlRequest::Key(LogicalKey::Down),
            _ => return Err("control key requires AUDIO, UP, or DOWN".into()),
        },
        _ => return Err("control requires view, capabilities, or key AUDIO|UP|DOWN".into()),
    };
    Ok(Command::Control(request))
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

fn parse_restore(arguments: &[String]) -> Result<Command, String> {
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
    Ok(Command::Restore(RestoreArguments {
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
    }))
}

fn parse_flash(purpose: Purpose, arguments: &[String]) -> Result<FlashArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "flash command requires IMAGE".to_owned())?;
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
            "--confirm-target" => {
                set_once(&mut target_confirmation, value.clone(), option)?;
            }
            "--confirm-image-crc32" => {
                set_once(&mut image_crc32_confirmation, value.clone(), option)?;
            }
            "--confirm-recovery-rehearsed" => {
                set_once(&mut recovery_rehearsed_confirmation, value.clone(), option)?;
            }
            _ => return Err(format!("unknown flash option: {option}")),
        }
        offset += 2;
    }

    match purpose {
        Purpose::Recovery => {
            if recovery.is_some() || recovery_rehearsed_confirmation.is_some() {
                return Err(
                    "flash-recovery does not accept --recovery or --confirm-recovery-rehearsed"
                        .into(),
                );
            }
        }
        Purpose::Afik => {
            if recovery.is_none() {
                return Err("flash-afik requires --recovery RAW".into());
            }
            if recovery_rehearsed_confirmation.is_none() {
                return Err("flash-afik requires --confirm-recovery-rehearsed PHRASE".into());
            }
        }
    }

    Ok(FlashArguments {
        purpose,
        image: PathBuf::from(image),
        recovery,
        backup: backup.ok_or_else(|| "flash command requires --backup EEPROM".to_owned())?,
        version: version.ok_or_else(|| "flash command requires --version VERSION".to_owned())?,
        target_confirmation: target_confirmation
            .ok_or_else(|| "flash command requires --confirm-target PHRASE".to_owned())?,
        image_crc32_confirmation: image_crc32_confirmation
            .ok_or_else(|| "flash command requires --confirm-image-crc32 CRC32".to_owned())?,
        recovery_rehearsed_confirmation,
    })
}

fn parse_flash_afik_application(arguments: &[String]) -> Result<AfikApplicationArguments, String> {
    let image = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| "flash-afik requires IMAGE".to_owned())?;
    let mut version = None;
    let mut yes = false;
    let mut offset = 1;
    while offset < arguments.len() {
        match arguments[offset].as_str() {
            "--yes" if !yes => yes = true,
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
                    "unknown flash-afik option: {option}; pass --recovery to select the recovery-gated form"
                ))
            }
        }
        offset += 1;
    }
    Ok(AfikApplicationArguments {
        image: PathBuf::from(image),
        version,
        yes,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{option} was provided more than once"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Command, ControlRequest, Parsed, Purpose};

    fn strings(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn help_inspect_probe_and_backup_are_strict() {
        assert_eq!(parse(&strings(&["--help"])).unwrap(), Parsed::Help);
        assert!(matches!(
            parse(&strings(&["inspect", "app.raw"])).unwrap(),
            Parsed::Inspect { .. }
        ));
        assert!(matches!(
            parse(&strings(&["--device", "/dev/ttyUSB0", "probe"])).unwrap(),
            Parsed::Hardware {
                command: Command::Probe,
                ..
            }
        ));
        assert!(parse(&strings(&["probe"])).is_err());
        assert!(parse(&strings(&["--device", "/dev/x", "backup-eeprom"])).is_err());
        assert!(matches!(
            parse(&strings(&["--device", "/dev/x", "control", "key", "AUDIO"])).unwrap(),
            Parsed::Hardware {
                command: Command::Control(ControlRequest::ToggleAudio),
                ..
            }
        ));
        assert!(parse(&strings(&["--device", "/dev/x", "control", "key", "PTT"])).is_err());
    }

    #[test]
    fn recovery_and_afik_flash_options_cannot_be_confused() {
        let common = [
            "--backup",
            "eeprom.raw",
            "--version",
            "F4HWN v4.3",
            "--confirm-target",
            "UV-K5-V1-DP32G030",
            "--confirm-image-crc32",
            "12345678",
        ];
        let mut recovery = vec!["--device", "/dev/x", "flash-recovery", "stock.raw"];
        recovery.extend(common);
        assert!(matches!(
            parse(&strings(&recovery)).unwrap(),
            Parsed::Hardware {
                command: Command::Flash(ref flash),
                ..
            } if flash.purpose == Purpose::Recovery
        ));

        let mut afik = vec!["--device", "/dev/x", "flash-afik", "afik.raw"];
        afik.extend(common);
        afik.extend([
            "--recovery",
            "stock.raw",
            "--confirm-recovery-rehearsed",
            "RECOVERY-REHEARSED-ON-THIS-UNIT",
        ]);
        assert!(matches!(
            parse(&strings(&afik)).unwrap(),
            Parsed::Hardware {
                command: Command::Flash(ref flash),
                ..
            } if flash.purpose == Purpose::Afik
        ));

        recovery.extend(["--recovery", "stock.raw"]);
        assert!(parse(&strings(&recovery)).is_err());
    }

    #[test]
    fn plain_flash_afik_is_the_default_and_recovery_opts_into_the_strict_gate() {
        assert!(matches!(
            parse(&strings(&["--device", "/dev/x", "flash-afik", "afik.raw"])).unwrap(),
            Parsed::Hardware {
                command: Command::FlashAfikApplication(ref flash),
                ..
            } if !flash.yes && flash.version.is_none()
        ));
        assert!(matches!(
            parse(&strings(&[
                "--device", "/dev/x", "flash-afik", "afik.raw", "--yes", "--version", "2.01.26",
            ]))
            .unwrap(),
            Parsed::Hardware {
                command: Command::FlashAfikApplication(ref flash),
                ..
            } if flash.yes && flash.version.as_deref() == Some("2.01.26")
        ));
        // Any strict-gate option without --recovery is refused, not ignored.
        assert!(parse(&strings(&[
            "--device",
            "/dev/x",
            "flash-afik",
            "afik.raw",
            "--backup",
            "eeprom.raw",
        ]))
        .is_err());
        // --recovery selects the strict form, which still needs every gate.
        assert!(parse(&strings(&[
            "--device",
            "/dev/x",
            "flash-afik",
            "afik.raw",
            "--recovery",
            "stock.raw",
        ]))
        .is_err());
    }

    #[test]
    fn restore_requires_all_confirmation_gates() {
        let parsed = parse(&strings(&[
            "--device",
            "/dev/x",
            "restore-eeprom",
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
        .unwrap();
        assert!(matches!(
            parsed,
            Parsed::Hardware {
                command: Command::Restore(_),
                ..
            }
        ));
        assert!(parse(&strings(&[
            "--device",
            "/dev/x",
            "restore-eeprom",
            "restore.raw"
        ]))
        .is_err());
    }
}
