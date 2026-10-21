//! Shared cross-platform serial transport and USB candidate discovery for
//! front ends. In the browser only the baud set and read window are shared:
//! the page opens its port through Web Serial (`ADR-119`).

#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
use radio_programmer::ProtocolTransport;
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::{ffi::OsStr, fs};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fmt,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Baud values accepted by the shared serial adapter.
pub const SUPPORTED_BAUDS: [u32; 8] = [1_200, 2_400, 4_800, 9_600, 19_200, 38_400, 57_600, 115_200];

/// How long one read waits for a byte before reporting none.
///
/// The protocol layers read an empty result as "nothing yet" and keep their own
/// deadlines, so a read must return rather than block indefinitely.
pub const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Reports whether a baud is in the explicit supported set.
pub const fn is_supported_baud(baud: u32) -> bool {
    matches!(
        baud,
        1_200 | 2_400 | 4_800 | 9_600 | 19_200 | 38_400 | 57_600 | 115_200
    )
}

#[cfg(not(target_arch = "wasm32"))]
/// Finds USB serial device paths suitable for protocol-level identification.
///
/// On Linux, stable `/dev/serial/by-id` USB symlinks are preferred. If that
/// directory is unavailable, `ttyUSB*` and `ttyACM*` device nodes are returned.
/// Elsewhere the operating system's USB serial ports are listed: `COMn` on
/// Windows and the `/dev/cu.*` call-out devices on macOS. The returned paths
/// are only transport candidates; the radio protocol must still classify the
/// bootloader before any operation is selected.
pub fn discover_usb_serial_devices() -> io::Result<Vec<PathBuf>> {
    #[cfg(target_os = "linux")]
    {
        discover_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut devices: Vec<PathBuf> = serialport::available_ports()?
            .into_iter()
            .filter(|port| matches!(port.port_type, serialport::SerialPortType::UsbPort(_)))
            .map(|port| port.port_name)
            .filter(|name| is_candidate_port_name(name))
            .map(PathBuf::from)
            .collect();
        devices.sort();
        devices.dedup();
        Ok(devices)
    }
}

/// Keeps one name per port where the system lists two.
///
/// macOS lists every port as a `tty.*` dial-in device, whose open waits for
/// carrier, and a `cu.*` call-out device, which is the one a host opens.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn is_candidate_port_name(name: &str) -> bool {
    !name.starts_with("/dev/tty.")
}

#[cfg(target_os = "linux")]
fn discover_linux() -> io::Result<Vec<PathBuf>> {
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

#[cfg(target_os = "linux")]
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

#[cfg(not(target_arch = "wasm32"))]
/// Serial setup failure before protocol negotiation.
#[derive(Debug)]
pub enum SerialOpenError {
    /// Baud is outside the shared explicit set.
    UnsupportedBaud(u32),
    /// The path could not be opened or configured as a serial port.
    Open(serialport::Error),
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Display for SerialOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBaud(baud) => write!(formatter, "unsupported baud: {baud}"),
            Self::Open(error) => write!(formatter, "could not open serial device: {error}"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Ordered byte stream over one raw 8N1 serial port.
pub struct SerialTransport {
    port: Box<dyn serialport::SerialPort>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SerialTransport {
    /// Opens one explicit path as raw 8N1 without flow control, with reads
    /// returning empty after [`READ_TIMEOUT`].
    pub fn open(path: &Path, baud: u32) -> Result<Self, SerialOpenError> {
        if !is_supported_baud(baud) {
            return Err(SerialOpenError::UnsupportedBaud(baud));
        }
        let port = serialport::new(path.to_string_lossy(), baud)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(SerialOpenError::Open)?;
        // Input buffered before the port was opened is stale, and has been
        // seen to hold a beacon with a gap in it. Readers resynchronise past
        // whatever is left, so a failed clear is not worth refusing the port.
        let _ = port.clear(serialport::ClearBuffer::Input);
        Ok(Self { port })
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Reports a read which timed out as no bytes, as a terminal read does.
fn timeout_as_empty(result: io::Result<usize>) -> io::Result<usize> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::TimedOut => Ok(0),
        other => other,
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Read for SerialTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        timeout_as_empty(self.port.read(buffer))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Write for SerialTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.port.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.port.flush()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ProtocolTransport for SerialTransport {
    type Error = io::Error;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        // A leading delimiter before every frame.
        //
        // Frames are COBS packets terminated by a zero byte, so a receiver
        // which has accumulated anything before the frame starts folds that
        // rubbish into the packet and rejects the whole thing. Opening a USB
        // serial port puts a byte or two on the line as it settles, and a radio
        // counted sixteen bytes arriving for a fourteen-byte hello and refused
        // every one of them.
        //
        // A zero byte flushes whatever the receiver was holding: it decodes and
        // discards that separately, then decodes this frame from a clean start.
        // A receiver holding nothing ignores it, because a delimiter with an
        // empty buffer is not a packet.
        Write::write_all(self, &[0])?;
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
        is_candidate_port_name, is_supported_baud, timeout_as_empty, SerialOpenError,
        SerialTransport, SUPPORTED_BAUDS,
    };
    use std::{io, path::Path};

    #[test]
    fn supported_baud_contract_is_exact_and_open_rechecks_it() {
        for baud in SUPPORTED_BAUDS {
            assert!(is_supported_baud(baud));
        }
        assert!(!is_supported_baud(0));
        assert!(!is_supported_baud(123));
        assert!(matches!(
            SerialTransport::open(Path::new("/definitely/missing"), 123),
            Err(SerialOpenError::UnsupportedBaud(123))
        ));
    }

    #[test]
    fn missing_explicit_path_fails_before_protocol_use() {
        assert!(matches!(
            SerialTransport::open(Path::new("/definitely/missing"), 9_600),
            Err(SerialOpenError::Open(_))
        ));
    }

    #[test]
    fn a_path_that_is_not_a_serial_port_is_refused() {
        assert!(SerialTransport::open(Path::new("Cargo.toml"), 9_600).is_err());
    }

    #[test]
    fn a_read_timeout_is_no_bytes_and_other_failures_stay_failures() {
        let timed_out = io::Error::new(io::ErrorKind::TimedOut, "timed out");
        assert_eq!(timeout_as_empty(Err(timed_out)).unwrap(), 0);
        assert_eq!(timeout_as_empty(Ok(3)).unwrap(), 3);
        let broken = io::Error::new(io::ErrorKind::BrokenPipe, "unplugged");
        assert_eq!(
            timeout_as_empty(Err(broken)).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn macos_dial_in_devices_are_not_candidates() {
        assert!(is_candidate_port_name("/dev/cu.usbserial-110"));
        assert!(is_candidate_port_name("COM3"));
        assert!(!is_candidate_port_name("/dev/tty.usbserial-110"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn usb_candidate_names_are_narrow_and_numeric() {
        use super::is_usb_serial_name;
        use std::ffi::OsStr;

        assert!(is_usb_serial_name(OsStr::new("ttyUSB0")));
        assert!(is_usb_serial_name(OsStr::new("ttyACM12")));
        assert!(!is_usb_serial_name(OsStr::new("ttyUSB")));
        assert!(!is_usb_serial_name(OsStr::new("ttyUSBx")));
        assert!(!is_usb_serial_name(OsStr::new("ttyS0")));
    }
}
