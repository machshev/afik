//! Serial device discovery and operator selection for the editor.
//!
//! Detection only proposes candidates. It never opens a port and never picks a
//! device for a write on its own: a single candidate is offered as the selection
//! and anything else leaves the choice to the operator, exactly as the
//! auto-detecting flasher CLI fails closed on more than one candidate. A manual
//! path always overrides detection, so an unusual port is never unreachable.

use std::path::{Path, PathBuf};

use radio_programmer_serial::{discover_usb_serial_devices, is_supported_baud, SUPPORTED_BAUDS};

/// Baud the shared configuration protocol uses unless told otherwise.
pub const DEFAULT_BAUD: u32 = 38_400;

/// One discovered serial device path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceCandidate {
    /// The device path to open.
    pub path: PathBuf,
}

impl DeviceCandidate {
    /// Wraps one discovered path.
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Returns the shortest label which still identifies the device.
    ///
    /// A `/dev/serial/by-id` symlink names the adapter, which is what an
    /// operator with two radios on the bench needs to tell them apart.
    pub fn label(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

/// Which device the operator has chosen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeviceChoice {
    /// The detected candidate at this index.
    Detected(usize),
    /// The manually entered path.
    #[default]
    Manual,
}

/// Detected candidates plus the operator's current selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceChooser {
    /// Candidates from the last detection run.
    pub candidates: Vec<DeviceCandidate>,
    /// The current selection.
    pub choice: DeviceChoice,
    /// A manually entered device path which overrides detection.
    pub manual_path: String,
    /// Baud used for the configuration protocol.
    pub baud: u32,
    /// Whether detection has run at least once.
    pub detected: bool,
}

impl Default for DeviceChooser {
    fn default() -> Self {
        Self {
            candidates: Vec::new(),
            choice: DeviceChoice::default(),
            manual_path: String::new(),
            baud: DEFAULT_BAUD,
            detected: false,
        }
    }
}

impl DeviceChooser {
    /// Returns a chooser which has not detected anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs discovery and returns the status line describing the outcome.
    pub fn detect(&mut self) -> String {
        match discover_usb_serial_devices() {
            Ok(paths) => self.accept(paths.into_iter().map(DeviceCandidate::new).collect()),
            Err(error) => format!("Could not scan for serial devices: {error}"),
        }
    }

    /// Records one candidate list and selects from it, without any I/O.
    ///
    /// Exactly one candidate is selected for the operator. Several are listed
    /// without choosing, so the wrong radio is never programmed by default.
    pub fn accept(&mut self, candidates: Vec<DeviceCandidate>) -> String {
        self.candidates = candidates;
        self.detected = true;
        match self.candidates.len() {
            0 => {
                self.choice = DeviceChoice::Manual;
                "No USB serial device detected. Enter a path to connect one.".to_owned()
            }
            1 => {
                self.choice = DeviceChoice::Detected(0);
                format!("Detected {}.", self.candidates[0].label())
            }
            count => {
                // More than one candidate is ambiguous: the operator chooses.
                if !matches!(self.choice, DeviceChoice::Detected(index) if index < count) {
                    self.choice = DeviceChoice::Manual;
                }
                format!("Detected {count} serial devices. Choose the radio to connect.")
            }
        }
    }

    /// Returns the chosen path when the current selection names one.
    pub fn chosen_path(&self) -> Option<&Path> {
        match self.choice {
            DeviceChoice::Detected(index) => {
                self.candidates.get(index).map(|entry| entry.path.as_path())
            }
            DeviceChoice::Manual => {
                let trimmed = self.manual_path.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(Path::new(trimmed))
                }
            }
        }
    }

    /// Resolves the selection into one path and baud, or explains what is missing.
    pub fn resolve(&self) -> Result<(PathBuf, u32), String> {
        if !is_supported_baud(self.baud) {
            return Err(format!(
                "Baud {} is not one of {SUPPORTED_BAUDS:?}.",
                self.baud
            ));
        }
        match self.chosen_path() {
            Some(path) => Ok((path.to_path_buf(), self.baud)),
            None => Err(match self.candidates.len() {
                0 if self.detected => {
                    "No USB serial device was detected. Enter a device path.".to_owned()
                }
                0 => "Detect a device or enter a device path.".to_owned(),
                count => format!("Choose one of the {count} detected devices."),
            }),
        }
    }
}

/// Lists plausible serial device paths without opening any of them.
pub fn discover_candidates() -> Vec<DeviceCandidate> {
    discover_usb_serial_devices()
        .unwrap_or_default()
        .into_iter()
        .map(DeviceCandidate::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DeviceCandidate, DeviceChoice, DeviceChooser, DEFAULT_BAUD};
    use std::path::PathBuf;

    fn candidates(paths: &[&str]) -> Vec<DeviceCandidate> {
        paths
            .iter()
            .map(|path| DeviceCandidate::new(PathBuf::from(path)))
            .collect()
    }

    #[test]
    fn one_candidate_is_selected_and_resolves() {
        let mut chooser = DeviceChooser::new();
        assert!(chooser.resolve().unwrap_err().contains("Detect a device"));
        let status = chooser.accept(candidates(&["/dev/ttyUSB0"]));
        assert_eq!(status, "Detected ttyUSB0.");
        assert_eq!(chooser.choice, DeviceChoice::Detected(0));
        assert_eq!(
            chooser.resolve().unwrap(),
            (PathBuf::from("/dev/ttyUSB0"), DEFAULT_BAUD)
        );
    }

    #[test]
    fn several_candidates_are_listed_without_choosing_one() {
        let mut chooser = DeviceChooser::new();
        let status = chooser.accept(candidates(&["/dev/ttyUSB0", "/dev/ttyACM1"]));
        assert!(status.contains("Detected 2 serial devices"));
        assert_eq!(chooser.choice, DeviceChoice::Manual);
        assert!(chooser
            .resolve()
            .unwrap_err()
            .contains("Choose one of the 2"));

        // An explicit choice survives a second detection which still offers it.
        chooser.choice = DeviceChoice::Detected(1);
        chooser.accept(candidates(&["/dev/ttyUSB0", "/dev/ttyACM1"]));
        assert_eq!(chooser.choice, DeviceChoice::Detected(1));
        assert_eq!(chooser.resolve().unwrap().0, PathBuf::from("/dev/ttyACM1"));

        // A choice which no longer exists is dropped rather than reused.
        chooser.choice = DeviceChoice::Detected(5);
        chooser.accept(candidates(&["/dev/ttyUSB0", "/dev/ttyACM1"]));
        assert_eq!(chooser.choice, DeviceChoice::Manual);
    }

    #[test]
    fn no_candidate_leaves_the_manual_path_in_charge() {
        let mut chooser = DeviceChooser::new();
        let status = chooser.accept(Vec::new());
        assert!(status.contains("No USB serial device detected"));
        assert!(chooser
            .resolve()
            .unwrap_err()
            .contains("Enter a device path"));
        chooser.manual_path = "  /dev/ttyS3  ".to_owned();
        assert_eq!(chooser.resolve().unwrap().0, PathBuf::from("/dev/ttyS3"));
    }

    #[test]
    fn a_manual_path_overrides_a_detected_candidate() {
        let mut chooser = DeviceChooser::new();
        chooser.accept(candidates(&["/dev/ttyUSB0"]));
        chooser.manual_path = "/dev/ttyUSB9".to_owned();
        chooser.choice = DeviceChoice::Manual;
        assert_eq!(chooser.resolve().unwrap().0, PathBuf::from("/dev/ttyUSB9"));
    }

    #[test]
    fn an_unsupported_baud_is_refused_before_any_device_is_opened() {
        let mut chooser = DeviceChooser::new();
        chooser.accept(candidates(&["/dev/ttyUSB0"]));
        chooser.baud = 123;
        assert!(chooser.resolve().unwrap_err().contains("is not one of"));
    }

    #[test]
    fn candidate_labels_fall_back_to_the_whole_path() {
        assert_eq!(
            DeviceCandidate::new(PathBuf::from("/dev/serial/by-id/usb-radio-if00")).label(),
            "usb-radio-if00"
        );
        assert_eq!(DeviceCandidate::new(PathBuf::from("/")).label(), "/");
    }
}
