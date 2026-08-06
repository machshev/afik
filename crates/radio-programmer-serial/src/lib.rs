//! Shared explicit-path Linux serial transport for programmer front ends.

#![forbid(unsafe_code)]

use radio_programmer::ProtocolTransport;
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
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
    /// Configures one explicit path as raw/no-echo with a 0.1-second read timer.
    pub fn open(path: &Path, baud: u32) -> Result<Self, SerialOpenError> {
        if !is_supported_baud(baud) {
            return Err(SerialOpenError::UnsupportedBaud(baud));
        }
        let baud_text = baud.to_string();
        let configured = Command::new("stty")
            .arg("-F")
            .arg(path)
            .args(["raw", "-echo", "min", "0", "time", "1"])
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

impl ProtocolTransport for LinuxSerialTransport {
    type Error = io::Error;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.file.write_all(frame)?;
        self.file.flush()
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        self.file.read(buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_supported_baud, LinuxSerialTransport, SerialOpenError, SUPPORTED_BAUDS};
    use std::path::Path;

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
}
