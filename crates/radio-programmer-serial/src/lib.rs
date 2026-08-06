//! Shared Linux serial transport and USB candidate discovery for front ends.

#![forbid(unsafe_code)]

use radio_programmer::ProtocolTransport;
use std::{
    ffi::OsStr,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

/// Baud values accepted by the shared Linux serial adapter.
pub const SUPPORTED_BAUDS: [u32; 8] = [1_200, 2_400, 4_800, 9_600, 19_200, 38_400, 57_600, 115_200];

/// Reports whether a baud is in the explicit supported set.
pub const fn is_supported_baud(baud: u32) -> bool {
    matches!(
        baud,
        1_200 | 2_400 | 4_800 | 9_600 | 19_200 | 38_400 | 57_600 | 115_200
    )
}

/// Finds USB serial device paths suitable for protocol-level identification.
///
/// Stable `/dev/serial/by-id` USB symlinks are preferred. If that directory is
/// unavailable, Linux `ttyUSB*` and `ttyACM*` device nodes are returned. The
/// returned paths are only transport candidates; the radio protocol must still
/// classify the bootloader before any operation is selected.
pub fn discover_usb_serial_devices() -> io::Result<Vec<PathBuf>> {
    let by_id = Path::new("/dev/serial/by-id");
    match fs::read_dir(by_id) {
        Ok(entries) => {
            let mut devices = Vec::new();
            for entry in entries {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with("usb-") {
                    devices.push(entry.path());
                }
            }
            devices.sort();
            if !devices.is_empty() {
                return Ok(devices);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut devices = Vec::new();
    for entry in fs::read_dir("/dev")? {
        let entry = entry?;
        if is_usb_serial_name(&entry.file_name()) {
            devices.push(entry.path());
        }
    }
    devices.sort();
    Ok(devices)
}

fn is_usb_serial_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    ["ttyUSB", "ttyACM"].into_iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

/// Explicit Linux serial setup failure before protocol negotiation.
#[derive(Debug)]
pub enum SerialOpenError {
    /// Baud is outside the shared explicit set.
    UnsupportedBaud(u32),
    /// The host could not execute `stty`.
    Stty(io::Error),
    /// `stty` rejected the path or requested terminal configuration.
    Configure {
        /// Child process exit status, if available.
        status: Option<i32>,
        /// Bounded-by-process captured diagnostic text.
        detail: String,
    },
    /// The configured path could not be opened for ordered reads and writes.
    Open(io::Error),
}

impl fmt::Display for SerialOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBaud(baud) => write!(formatter, "unsupported baud: {baud}"),
            Self::Stty(error) => write!(formatter, "could not execute stty: {error}"),
            Self::Configure { status, detail } => {
                write!(formatter, "stty configuration failed with {status:?}")?;
                if !detail.is_empty() {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            Self::Open(error) => write!(formatter, "could not open serial device: {error}"),
        }
    }
}

/// Ordered file byte stream configured through Linux `stty`.
pub struct LinuxSerialTransport {
    file: File,
}

impl LinuxSerialTransport {
    /// Configures one explicit path as raw/no-echo with a 0.2-second read timer.
    pub fn open(path: &Path, baud: u32) -> Result<Self, SerialOpenError> {
        if !is_supported_baud(baud) {
            return Err(SerialOpenError::UnsupportedBaud(baud));
        }
        let baud_text = baud.to_string();
        let configured = Command::new("stty")
            .arg("-F")
            .arg(path)
            .args(["raw", "-echo", "min", "0", "time", "2"])
            .arg(&baud_text)
            .output()
            .map_err(SerialOpenError::Stty)?;
        if !configured.status.success() {
            let detail = String::from_utf8_lossy(&configured.stderr)
                .trim()
                .to_owned();
            return Err(SerialOpenError::Configure {
                status: configured.status.code(),
                detail,
            });
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(SerialOpenError::Open)?;
        Ok(Self { file })
    }
}

impl Read for LinuxSerialTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }
}

impl Write for LinuxSerialTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl ProtocolTransport for LinuxSerialTransport {
    type Error = io::Error;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        Write::write_all(self, frame)?;
        Write::flush(self)
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        Read::read(self, buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_supported_baud, is_usb_serial_name, LinuxSerialTransport, SerialOpenError,
        SUPPORTED_BAUDS,
    };
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn supported_baud_contract_is_exact_and_open_rechecks_it() {
        for baud in SUPPORTED_BAUDS {
            assert!(is_supported_baud(baud));
        }
        assert!(!is_supported_baud(0));
        assert!(!is_supported_baud(123));
        assert!(matches!(
            LinuxSerialTransport::open(Path::new("/definitely/missing"), 123),
            Err(SerialOpenError::UnsupportedBaud(123))
        ));
    }

    #[test]
    fn missing_explicit_path_fails_before_protocol_use() {
        assert!(LinuxSerialTransport::open(Path::new("/definitely/missing"), 9_600).is_err());
    }

    #[test]
    fn usb_candidate_names_are_narrow_and_numeric() {
        assert!(is_usb_serial_name(OsStr::new("ttyUSB0")));
        assert!(is_usb_serial_name(OsStr::new("ttyACM12")));
        assert!(!is_usb_serial_name(OsStr::new("ttyUSB")));
        assert!(!is_usb_serial_name(OsStr::new("ttyUSBx")));
        assert!(!is_usb_serial_name(OsStr::new("ttyS0")));
    }
}
