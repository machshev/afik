//! Persistent programmer session and embedded local web interface.

#![forbid(unsafe_code)]

use radio_channel_plan::{BankName, GeneratedBank};
use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};
use radio_programmer::{
    CapacityReport, CompileError, DeviceCapabilities, ObjectListing, Programmer, ProgrammerError,
    ProtocolTransport, RadioProject, RestoreError, VerifiedConfigurationReceipt,
};
use radio_programmer_serial::{LinuxSerialTransport, SerialOpenError};
use radio_sim::{SimDevice, SimTransport};
use std::{convert::Infallible, fmt, io, path::Path};

/// Maximum UTF-8 project form bytes accepted by the GUI model.
pub const MAX_PROJECT_TEXT_BYTES: usize = 64 * 1024;
/// Embedded accessible application document.
pub const INDEX_HTML: &str = include_str!("../assets/index.html");
/// Embedded responsive application stylesheet.
pub const APP_CSS: &str = include_str!("../assets/app.css");
/// Embedded browser interaction layer.
pub const APP_JS: &str = include_str!("../assets/app.js");

/// Current negotiated device state displayed by the GUI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuiState {
    /// Negotiated programmer capabilities.
    pub capabilities: DeviceCapabilities,
    /// Complete generation-tagged object listing.
    pub listing: ObjectListing,
}

/// Canonical compiled image download plus exact capacity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledDownload {
    /// Exact compilation capacity report.
    pub report: CapacityReport,
    /// Canonical configuration image bytes.
    pub image: Vec<u8>,
}

/// GUI project text failed strict bounded parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectTextError {
    line: Option<usize>,
    detail: String,
}

impl ProjectTextError {
    fn whole(detail: impl Into<String>) -> Self {
        Self {
            line: None,
            detail: detail.into(),
        }
    }

    fn line(line: usize, detail: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ProjectTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "project line {line}: {}", self.detail),
            None => formatter.write_str(&self.detail),
        }
    }
}

/// GUI transport I/O failure after a backend is connected.
#[derive(Debug)]
pub enum GuiTransportError {
    /// Explicit serial byte I/O failed.
    Serial(io::Error),
}

impl fmt::Display for GuiTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serial(error) => write!(formatter, "serial I/O failed: {error}"),
        }
    }
}

/// Persistent GUI session operation failure.
#[derive(Debug)]
pub enum GuiError {
    /// Explicit serial setup failed before protocol negotiation.
    SerialOpen(SerialOpenError),
    /// Project form text was malformed or outside bounds.
    Project(ProjectTextError),
    /// Project compilation or target capability validation failed.
    Compile(CompileError),
    /// Programmer transport, protocol, device, storage, or verification failed.
    Programmer(ProgrammerError<GuiTransportError>),
    /// Restore image validation or verified mutation failed.
    Restore(RestoreError<GuiTransportError>),
}

impl fmt::Display for GuiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerialOpen(error) => write!(formatter, "serial setup failed: {error}"),
            Self::Project(error) => write!(formatter, "project rejected: {error}"),
            Self::Compile(error) => write!(formatter, "compile failed: {error}"),
            Self::Programmer(error) => write!(formatter, "programmer failed: {error}"),
            Self::Restore(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<ProjectTextError> for GuiError {
    fn from(error: ProjectTextError) -> Self {
        Self::Project(error)
    }
}

impl From<CompileError> for GuiError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

impl From<ProgrammerError<GuiTransportError>> for GuiError {
    fn from(error: ProgrammerError<GuiTransportError>) -> Self {
        Self::Programmer(error)
    }
}

enum GuiTransport {
    Simulator(Box<SimTransport>),
    Serial(LinuxSerialTransport),
}

impl ProtocolTransport for GuiTransport {
    type Error = GuiTransportError;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        match self {
            Self::Simulator(transport) => transport.send(frame).map_err(infallible),
            Self::Serial(transport) => transport.send(frame).map_err(GuiTransportError::Serial),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Simulator(transport) => transport.receive(buffer).map_err(infallible),
            Self::Serial(transport) => transport.receive(buffer).map_err(GuiTransportError::Serial),
        }
    }
}

fn infallible(error: Infallible) -> GuiTransportError {
    match error {}
}

/// One persistent selected-backend programmer session for the local GUI.
pub struct GuiSession {
    programmer: Programmer<GuiTransport>,
}

impl GuiSession {
    /// Connects one fresh deterministic simulator session.
    pub fn connect_simulator() -> Result<Self, GuiError> {
        Self::connect(GuiTransport::Simulator(Box::new(SimTransport::new(
            SimDevice::new(),
        ))))
    }

    /// Configures and connects one explicit Linux serial path and baud.
    pub fn connect_serial(path: &Path, baud: u32) -> Result<Self, GuiError> {
        let transport = LinuxSerialTransport::open(path, baud).map_err(GuiError::SerialOpen)?;
        Self::connect(GuiTransport::Serial(transport))
    }

    /// Reads the current capability and stable object-listing view.
    pub fn state(&mut self) -> Result<GuiState, GuiError> {
        Ok(GuiState {
            capabilities: self.programmer.capabilities(),
            listing: self.programmer.list_objects()?,
        })
    }

    /// Strictly parses and compiles newline-separated generated-bank specs.
    pub fn compile_project(&self, text: &str) -> Result<CompiledDownload, GuiError> {
        let project = parse_project(text)?;
        let configuration = self.programmer.compiler().compile(&project)?;
        let mut image = vec![0; configuration.image_len().map_err(CompileError::Storage)?];
        configuration
            .encode_image(&mut image)
            .map_err(CompileError::Storage)?;
        Ok(CompiledDownload {
            report: configuration.report(),
            image,
        })
    }

    /// Strictly parses, compiles, transactionally writes, and verifies a project.
    pub fn write_project(&mut self, text: &str) -> Result<VerifiedConfigurationReceipt, GuiError> {
        let project = parse_project(text)?;
        let configuration = self.programmer.compiler().compile(&project)?;
        self.programmer
            .write_configuration_verified(&configuration)
            .map_err(GuiError::Programmer)
    }

    /// Returns one validated canonical backup image from the persistent session.
    pub fn backup(&mut self) -> Result<radio_programmer::ConfigurationBackup, GuiError> {
        self.programmer
            .backup_configuration()
            .map_err(GuiError::Programmer)
    }

    /// Validates, transactionally restores, and verifies one canonical image.
    pub fn restore(&mut self, image: &[u8]) -> Result<VerifiedConfigurationReceipt, GuiError> {
        self.programmer
            .restore_configuration_image(image)
            .map_err(GuiError::Restore)
    }

    fn connect(transport: GuiTransport) -> Result<Self, GuiError> {
        let programmer = Programmer::connect(transport).map_err(GuiError::Programmer)?;
        Ok(Self { programmer })
    }
}

fn parse_project(text: &str) -> Result<RadioProject, ProjectTextError> {
    if text.len() > MAX_PROJECT_TEXT_BYTES {
        return Err(ProjectTextError::whole(format!(
            "project text exceeds {MAX_PROJECT_TEXT_BYTES} bytes"
        )));
    }
    let mut project = RadioProject::new();
    let mut bank_count = 0_usize;
    for (offset, line) in text.lines().enumerate() {
        let line_number = offset + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        project.add_generated_bank(parse_bank(line_number, line)?);
        bank_count += 1;
    }
    if bank_count == 0 {
        return Err(ProjectTextError::whole(
            "enter at least one generated-bank specification",
        ));
    }
    Ok(project)
}

fn parse_bank(line_number: usize, spec: &str) -> Result<GeneratedBank, ProjectTextError> {
    let fields = spec.split(':').collect::<Vec<_>>();
    if fields.len() != 6 {
        return Err(ProjectTextError::line(
            line_number,
            "expected ID:NAME:BASE_HZ:SPACING_HZ:COUNT:TX_CLASS",
        ));
    }
    let id = parse_integer::<u16>(line_number, fields[0], "bank ID")?;
    let name = BankName::new(fields[1])
        .map_err(|error| ProjectTextError::line(line_number, error.to_string()))?;
    let base = Frequency::from_hz(parse_integer::<u32>(
        line_number,
        fields[2],
        "base frequency",
    )?)
    .map_err(|error| ProjectTextError::line(line_number, error.to_string()))?;
    let spacing = FrequencyStep::from_hz(parse_integer::<u32>(line_number, fields[3], "spacing")?)
        .map_err(|error| ProjectTextError::line(line_number, error.to_string()))?;
    let count = parse_integer::<u16>(line_number, fields[4], "channel count")?;
    let class = parse_class(line_number, fields[5])?;
    GeneratedBank::linear_simplex(BankId::new(id), name, base, spacing, count, class)
        .map_err(|error| ProjectTextError::line(line_number, error.to_string()))
}

fn parse_integer<T>(line: usize, value: &str, label: &str) -> Result<T, ProjectTextError>
where
    T: core::str::FromStr,
{
    value
        .parse::<T>()
        .map_err(|_| ProjectTextError::line(line, format!("invalid {label}: {value}")))
}

fn parse_class(line: usize, value: &str) -> Result<TxClass, ProjectTextError> {
    match value {
        "never" => Ok(TxClass::Never),
        "licence-free" => Ok(TxClass::LicenceFreePlan),
        "amateur" => Ok(TxClass::Amateur),
        "marine" => Ok(TxClass::Marine),
        "aeronautical" => Ok(TxClass::Aeronautical),
        "business" => Ok(TxClass::Business),
        "experimental" => Ok(TxClass::Experimental),
        _ => Err(ProjectTextError::line(
            line,
            format!("unknown TX class: {value}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{GuiSession, APP_CSS, APP_JS, INDEX_HTML, MAX_PROJECT_TEXT_BYTES};
    use radio_storage::decode_configuration_image;

    const PROJECT: &str = "2:TWO:446018750:12500:2:licence-free\n\
1:ONE:446006250:12500:1:licence-free\n";

    fn run_session() -> (super::GuiState, Vec<u8>) {
        let mut session = GuiSession::connect_simulator().unwrap();
        assert_eq!(session.state().unwrap().listing.generation, 0);
        let compiled = session.compile_project(PROJECT).unwrap();
        assert_eq!(compiled.report.object_count, 2);
        assert_eq!(compiled.report.generated_channels, 3);
        let receipt = session.write_project(PROJECT).unwrap();
        assert_eq!(receipt.generation, 1);
        let state = session.state().unwrap();
        let backup = session.backup().unwrap();
        assert_eq!(backup.generation, 1);
        assert_eq!(backup.image, compiled.image);
        (state, backup.image)
    }

    #[test]
    fn persistent_simulator_compile_write_backup_and_restore_are_repeatable() {
        let first = run_session();
        let second = run_session();
        assert_eq!(first, second);
        assert_eq!(first.0.listing.generation, 1);
        assert_eq!(first.0.listing.objects.len(), 2);
        assert_eq!(
            decode_configuration_image(&first.1).unwrap().object_count(),
            2
        );

        let mut restored = GuiSession::connect_simulator().unwrap();
        assert_eq!(restored.restore(&first.1).unwrap().generation, 1);
        assert_eq!(restored.backup().unwrap().image, first.1);
    }

    #[test]
    fn project_text_is_bounded_strict_and_leaves_compiler_rules_central() {
        let session = GuiSession::connect_simulator().unwrap();
        assert!(session.compile_project("").is_err());
        assert!(session.compile_project("broken").is_err());
        assert!(session.compile_project("1:X:1:1:1:unknown").is_err());
        assert!(session
            .compile_project("1:X:1:1:1:never\n1:Y:2:1:1:never")
            .is_err());
        assert!(session
            .compile_project(&"x".repeat(MAX_PROJECT_TEXT_BYTES + 1))
            .is_err());
    }

    #[test]
    fn embedded_assets_expose_readable_confirmed_responsive_workflow() {
        for required in [
            "<main",
            "aria-live=\"polite\"",
            "Device capabilities",
            "Installed objects",
            "Compile image",
            "Write to radio",
            "Download backup",
            "Restore image",
            "confirm-write",
            "confirm-restore",
            "__AFIK_SESSION_TOKEN__",
        ] {
            assert!(INDEX_HTML.contains(required), "missing {required}");
        }
        assert!(APP_CSS.contains("@media"));
        assert!(APP_CSS.contains(":focus-visible"));
        assert!(APP_JS.contains("X-Afik-Session"));
        assert!(APP_JS.contains("confirm-write"));
        assert!(APP_JS.contains("confirm-restore"));
    }
}
