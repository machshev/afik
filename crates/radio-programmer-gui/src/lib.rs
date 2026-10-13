//! Native cross-platform AFIK channel, configuration, and flashing editor.
//!
//! The library holds every decision the editor makes: the validated project
//! model, the programmer session, and the guarded flashing operations. The
//! `app` module only draws those decisions.

#![forbid(unsafe_code)]

pub mod app;
pub mod device;
pub mod flash;
pub mod model;
pub mod presets;
pub mod session;

/// Command-line help for the native editor binary.
pub const HELP: &str = "AFIK Studio: native channel, configuration, and flashing editor\n\
\n\
Usage:\n\
  afik-studio [--sim | --device PATH|auto] [--baud BAUD] [--project FILE]\n\
  afik-studio --help\n\
\n\
Options:\n\
  --sim              Connect the deterministic simulator at start-up.\n\
  --device PATH      Serial device used for the configuration protocol.\n\
  --device auto      Detect one USB serial device and connect it.\n\
  --baud BAUD        Serial baud rate; defaults to 38400.\n\
  --project FILE     Load a canonical AFIK configuration image at start-up.\n\
  --help             Print this message.\n\
\n\
Supported BAUD: 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200\n\
\n\
Connecting at start-up is optional: the Device tab detects USB serial\n\
devices, offers a single candidate as the selection, leaves an ambiguous\n\
choice to the operator, and always accepts a manually entered path.\n\
Firmware and EEPROM operations live in the Flash tab and keep every\n\
recovery-gated confirmation required by the flasher library.\n";

/// Which serial device the command line asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeviceSelector {
    /// Detect exactly one USB serial device.
    Auto,
    /// Use this explicit path.
    Explicit(std::path::PathBuf),
}

/// Start-up options parsed from the command line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Options {
    /// Connect the simulator at start-up.
    pub simulator: bool,
    /// Serial device selection for the configuration protocol.
    pub device: Option<DeviceSelector>,
    /// Serial baud rate for the configuration protocol.
    pub baud: Option<u32>,
    /// Canonical configuration image to load at start-up.
    pub project: Option<std::path::PathBuf>,
    /// Print help and exit.
    pub help: bool,
}

/// Parses command-line arguments, rejecting unknown or incomplete input.
pub fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--help" | "-h" => options.help = true,
            "--sim" => options.simulator = true,
            "--device" => {
                index += 1;
                let value = arguments.get(index).ok_or("--device requires a path")?;
                options.device = Some(if value == "auto" {
                    DeviceSelector::Auto
                } else {
                    DeviceSelector::Explicit(std::path::PathBuf::from(value))
                });
            }
            "--baud" => {
                index += 1;
                let value = arguments.get(index).ok_or("--baud requires a value")?;
                let baud = value.parse().map_err(|_| "--baud must be a number")?;
                if !radio_programmer_serial::is_supported_baud(baud) {
                    return Err(format!("--baud {baud} is not a supported rate"));
                }
                options.baud = Some(baud);
            }
            "--project" => {
                index += 1;
                let value = arguments.get(index).ok_or("--project requires a path")?;
                options.project = Some(std::path::PathBuf::from(value));
            }
            other => return Err(format!("unknown argument {other}")),
        }
        index += 1;
    }
    if options.simulator && options.device.is_some() {
        return Err("--sim and --device are mutually exclusive".to_owned());
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::{parse_options, DeviceSelector, Options};

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn options_require_complete_and_exclusive_backends() {
        assert_eq!(parse_options(&[]).unwrap(), Options::default());
        assert!(parse_options(&arguments(&["--sim"])).unwrap().simulator);
        assert!(parse_options(&arguments(&["--help"])).unwrap().help);
        let serial =
            parse_options(&arguments(&["--device", "/dev/ttyUSB0", "--baud", "38400"])).unwrap();
        assert_eq!(serial.baud, Some(38_400));
        assert_eq!(
            serial.device,
            Some(DeviceSelector::Explicit(std::path::PathBuf::from(
                "/dev/ttyUSB0"
            )))
        );
        assert_eq!(
            parse_options(&arguments(&["--sim", "--device", "/dev/ttyUSB0"])),
            Err("--sim and --device are mutually exclusive".to_owned())
        );
        // The baud is optional and detection replaces an explicit path.
        assert_eq!(
            parse_options(&arguments(&["--device", "auto"]))
                .unwrap()
                .device,
            Some(DeviceSelector::Auto)
        );
        assert_eq!(
            parse_options(&arguments(&["--device", "/dev/ttyUSB0"]))
                .unwrap()
                .baud,
            None
        );
        assert!(parse_options(&arguments(&["--baud"])).is_err());
        assert!(parse_options(&arguments(&["--baud", "123"])).is_err());
        assert!(parse_options(&arguments(&["--nope"])).is_err());
        assert_eq!(
            parse_options(&arguments(&["--project", "plan.afik"]))
                .unwrap()
                .project,
            Some(std::path::PathBuf::from("plan.afik"))
        );
    }
}
