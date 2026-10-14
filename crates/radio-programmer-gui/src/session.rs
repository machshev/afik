//! Connection to a radio programmed as memory (`ADR-100`).
//!
//! The studio compiles and validates the format 5 image, then writes it into
//! the radio's memory view over the stock protocol; the radio validates it
//! again when it loads it.

use std::{fmt, io, path::Path};

use radio_flasher::{FlashError, ImageWriteReport};
use radio_programmer::{RadioImageError, RadioProject};
use radio_programmer_serial::{LinuxSerialTransport, SerialOpenError};
use radio_sim::memory_radio::MemoryRadio;
use radio_storage::StorageError;

/// Session operation failure.
#[derive(Debug)]
pub enum SessionError {
    /// The serial device could not be opened.
    SerialOpen(SerialOpenError),
    /// The project does not compile into a radio image.
    Compile(RadioImageError),
    /// The radio refused, did not answer, or read back differently.
    Radio(FlashError),
    /// The radio's memory holds no valid image.
    Image(StorageError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerialOpen(error) => write!(formatter, "serial setup failed: {error}"),
            Self::Compile(error) => write!(formatter, "compile failed: {error}"),
            Self::Radio(error) => write!(formatter, "{error}"),
            Self::Image(error) => write!(formatter, "the radio holds no valid image: {error}"),
        }
    }
}

enum Transport {
    Simulator(Box<MemoryRadio>),
    Serial(LinuxSerialTransport),
}

impl io::Read for Transport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Simulator(radio) => radio.read(buffer),
            Self::Serial(serial) => serial.read(buffer),
        }
    }
}

impl io::Write for Transport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Simulator(radio) => radio.write(buffer),
            Self::Serial(serial) => serial.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Simulator(radio) => radio.flush(),
            Self::Serial(serial) => serial.flush(),
        }
    }
}

/// One connected radio.
pub struct DeviceSession {
    transport: Transport,
    description: String,
    firmware: String,
}

impl DeviceSession {
    /// Connects a simulated radio with erased memory.
    pub fn connect_simulator() -> Result<Self, SessionError> {
        Self::connect(Transport::Simulator(Box::default()), "simulator".to_owned())
    }

    /// Connects one explicit serial path and baud.
    pub fn connect_serial(path: &Path, baud: u32) -> Result<Self, SessionError> {
        let serial = LinuxSerialTransport::open(path, baud).map_err(SessionError::SerialOpen)?;
        Self::connect(
            Transport::Serial(serial),
            format!("{} at {baud} baud", path.display()),
        )
    }

    /// Returns a short description of the connected backend.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the firmware the radio named when it connected.
    pub fn firmware(&self) -> &str {
        &self.firmware
    }

    /// Compiles the project, writes it into the radio's memory and reads it
    /// back.
    pub fn write_project(
        &mut self,
        project: &RadioProject,
    ) -> Result<ImageWriteReport, SessionError> {
        let image = project
            .compile_radio_image()
            .map_err(SessionError::Compile)?;
        let report = radio_flasher::write_afik_image(&mut self.transport, &image, |_, _| {})
            .map_err(SessionError::Radio)?;
        report.firmware.version().clone_into(&mut self.firmware);
        Ok(report)
    }

    /// Reads the radio's image and decodes it as the radio would.
    pub fn read_project(&mut self) -> Result<RadioProject, SessionError> {
        let (firmware, region) =
            radio_flasher::read_afik_image(&mut self.transport).map_err(SessionError::Radio)?;
        firmware.version().clone_into(&mut self.firmware);
        RadioProject::from_radio_image(&region).map_err(SessionError::Image)
    }

    /// The simulated radio, when that is what is connected.
    pub fn simulator(&self) -> Option<&MemoryRadio> {
        match &self.transport {
            Transport::Simulator(radio) => Some(radio),
            Transport::Serial(_) => None,
        }
    }

    fn connect(mut transport: Transport, description: String) -> Result<Self, SessionError> {
        let firmware = radio_flasher::probe_normal_firmware(&mut transport)
            .map_err(SessionError::Radio)?
            .version()
            .to_owned();
        Ok(Self {
            transport,
            description,
            firmware,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceSession, SessionError};
    use crate::model::ProjectModel;

    #[test]
    fn a_simulated_radio_loads_what_is_written_and_reads_it_back() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        let project = model.validate().unwrap();

        let mut session = DeviceSession::connect_simulator().unwrap();
        assert_eq!(session.description(), "simulator");
        assert!(session.firmware().starts_with("AFIK-K5-"));
        assert!(matches!(
            session.read_project(),
            Err(SessionError::Image(_))
        ));

        let report = session.write_project(&project).unwrap();
        assert!(report.changed_blocks > 0);
        let index = session.simulator().unwrap().index().unwrap();
        assert_eq!(index.len(), 1);

        let read = ProjectModel::from_project(&session.read_project().unwrap());
        assert_eq!(read, model);
    }
}
