//! Persistent programmer session used by the native editor.

use std::{convert::Infallible, fmt, io, path::Path};

use radio_programmer::{
    CompileError, ConfigurationBackup, DeviceCapabilities, ObjectListing, Programmer,
    ProgrammerError, ProtocolTransport, RadioProject, RestoreError, VerifiedConfigurationReceipt,
};
use radio_programmer_serial::{LinuxSerialTransport, SerialOpenError};
use radio_sim::{SimDevice, SimTransport};

/// Transport I/O failure after a backend is connected.
#[derive(Debug)]
pub enum TransportError {
    /// Explicit serial byte I/O failed.
    Serial(io::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serial(error) => write!(formatter, "serial I/O failed: {error}"),
        }
    }
}

/// Session operation failure.
#[derive(Debug)]
pub enum SessionError {
    /// Explicit serial setup failed before protocol negotiation.
    SerialOpen(SerialOpenError),
    /// Project compilation or target capability validation failed.
    Compile(CompileError),
    /// Programmer transport, protocol, device, storage, or verification failed.
    Programmer(ProgrammerError<TransportError>),
    /// Restore image validation or verified mutation failed.
    Restore(RestoreError<TransportError>),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerialOpen(error) => write!(formatter, "serial setup failed: {error}"),
            Self::Compile(error) => write!(formatter, "compile failed: {error}"),
            Self::Programmer(error) => write!(formatter, "programmer failed: {error}"),
            Self::Restore(error) => write!(formatter, "{error}"),
        }
    }
}

enum Transport {
    Simulator(Box<SimTransport>),
    Serial(LinuxSerialTransport),
}

impl ProtocolTransport for Transport {
    type Error = TransportError;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        match self {
            Self::Simulator(transport) => transport.send(frame).map_err(infallible),
            Self::Serial(transport) => transport.send(frame).map_err(TransportError::Serial),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Simulator(transport) => transport.receive(buffer).map_err(infallible),
            Self::Serial(transport) => transport.receive(buffer).map_err(TransportError::Serial),
        }
    }
}

fn infallible(error: Infallible) -> TransportError {
    match error {}
}

/// One connected programmer backend.
pub struct DeviceSession {
    programmer: Programmer<Transport>,
    description: String,
}

impl DeviceSession {
    /// Connects one fresh deterministic simulator session.
    pub fn connect_simulator() -> Result<Self, SessionError> {
        Self::connect(
            Transport::Simulator(Box::new(SimTransport::new(SimDevice::new()))),
            "simulator".to_owned(),
        )
    }

    /// Connects one explicit serial path and baud.
    pub fn connect_serial(path: &Path, baud: u32) -> Result<Self, SessionError> {
        let transport = LinuxSerialTransport::open(path, baud).map_err(SessionError::SerialOpen)?;
        Self::connect(
            Transport::Serial(transport),
            format!("{} at {baud} baud", path.display()),
        )
    }

    /// Returns a short description of the connected backend.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the negotiated device capabilities.
    pub fn capabilities(&self) -> DeviceCapabilities {
        self.programmer.capabilities()
    }

    /// Reads the current generation-tagged object listing.
    pub fn listing(&mut self) -> Result<ObjectListing, SessionError> {
        self.programmer
            .list_objects()
            .map_err(SessionError::Programmer)
    }

    /// Compiles, transactionally writes, and verifies one project.
    pub fn write_project(
        &mut self,
        project: &RadioProject,
    ) -> Result<VerifiedConfigurationReceipt, SessionError> {
        let configuration = self
            .programmer
            .compiler()
            .compile(project)
            .map_err(SessionError::Compile)?;
        self.programmer
            .write_configuration_verified(&configuration)
            .map_err(SessionError::Programmer)
    }

    /// Returns one validated canonical backup image.
    pub fn backup(&mut self) -> Result<ConfigurationBackup, SessionError> {
        self.programmer
            .backup_configuration()
            .map_err(SessionError::Programmer)
    }

    /// Validates, transactionally restores, and verifies one canonical image.
    pub fn restore(&mut self, image: &[u8]) -> Result<VerifiedConfigurationReceipt, SessionError> {
        self.programmer
            .restore_configuration_image(image)
            .map_err(SessionError::Restore)
    }

    fn connect(transport: Transport, description: String) -> Result<Self, SessionError> {
        let programmer = Programmer::connect(transport).map_err(SessionError::Programmer)?;
        Ok(Self {
            programmer,
            description,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DeviceSession;
    use crate::model::ProjectModel;

    #[test]
    fn a_simulator_session_writes_verifies_and_backs_up_a_banked_project() {
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel();
        model.channels[0].banks[0] = true;
        let project = model.validate().unwrap();

        let mut session = DeviceSession::connect_simulator().unwrap();
        assert_eq!(session.description(), "simulator");
        assert_eq!(session.listing().unwrap().objects.len(), 0);

        let receipt = session.write_project(&project).unwrap();
        assert_eq!(receipt.report.explicit_channels, 1);
        assert_eq!(receipt.report.banks, 1);
        assert!(receipt.report.has_radio_config);
        assert_eq!(session.listing().unwrap().objects.len(), 3);

        let backup = session.backup().unwrap();
        assert_eq!(backup.generation, receipt.generation);
        let restored = ProjectModel::from_image(&backup.image).unwrap();
        assert_eq!(restored.channels.len(), 1);
        assert_eq!(restored.banks.len(), 1);

        let receipt = session.restore(&backup.image).unwrap();
        assert!(receipt.generation > backup.generation);
    }
}
