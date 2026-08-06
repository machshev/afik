use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
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
    Flash(FlashArguments),
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
        "flash-recovery" => Command::Flash(parse_flash(Purpose::Recovery, &arguments[3..])?),
        "flash-afik" => Command::Flash(parse_flash(Purpose::Afik, &arguments[3..])?),
        command => return Err(format!("unknown command: {command}")),
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

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{option} was provided more than once"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Command, Parsed, Purpose};

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
    }

    #[test]
    fn recovery_and_afik_flash_options_cannot_be_confused() {
        let common = [
            "--backup",
            "eeprom.raw",
            "--version",
            "2.01.23",
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
}
