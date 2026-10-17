//! Connection to a radio programmed as memory (`ADR-100`).
//!
//! The studio compiles and validates the format 5 image, then writes it into
//! the radio's memory view over the stock protocol; the radio validates it
//! again when it loads it.

use std::{
    fmt, io,
    path::Path,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc,
    },
    thread,
};

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
    /// back. `progress` is told each written block and the number to write.
    pub fn write_project(
        &mut self,
        project: &RadioProject,
        progress: impl FnMut(u16, u16),
    ) -> Result<ImageWriteReport, SessionError> {
        let image = project
            .compile_radio_image()
            .map_err(SessionError::Compile)?;
        let report = radio_flasher::write_afik_image(&mut self.transport, &image, progress)
            .map_err(SessionError::Radio)?;
        // Each slot's start place or programmed VFO goes in the same write,
        // as the radio's place, so it starts there; its dual watch is the one the
        // configuration programs, which the place would otherwise override.
        if project.writes_place() {
            let dual_watch = project.config().is_some_and(|config| config.dual_watch);
            let record = radio_platform::operator_state::OperatorState::programmed(
                project.starts(),
                project.vfos(),
                dual_watch,
            )
            .encode();
            radio_flasher::write_afik_place(&mut self.transport, &record)
                .map_err(SessionError::Radio)?;
        }
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

    /// Writes `project` on a worker thread, which owns the session until the
    /// write ends and hands it back with the outcome.
    pub fn start_write(self, project: RadioProject) -> Result<DeviceJob, SessionError> {
        DeviceJob::start(self, JobKind::Write, move |session, progress| {
            DeviceOutcome::Wrote(session.write_project(&project, |done, total| {
                progress.store(u32::from(done) << 16 | u32::from(total), Ordering::Relaxed);
            }))
        })
    }

    /// Reads the radio's project on a worker thread.
    pub fn start_read(self) -> Result<DeviceJob, SessionError> {
        DeviceJob::start(self, JobKind::Read, |session, _| {
            DeviceOutcome::Read(session.read_project())
        })
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

/// What a [`DeviceJob`] is doing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobKind {
    /// Writing the project to the radio.
    Write,
    /// Reading the project from the radio.
    Read,
}

/// How a [`DeviceJob`] ended.
#[derive(Debug)]
pub enum DeviceOutcome {
    /// A write, verified or failed.
    Wrote(Result<ImageWriteReport, SessionError>),
    /// A read, decoded or failed.
    Read(Result<RadioProject, SessionError>),
}

/// A write or read running off the calling thread.
pub struct DeviceJob {
    kind: JobKind,
    description: String,
    progress: Arc<AtomicU32>,
    outcome: mpsc::Receiver<(DeviceSession, DeviceOutcome)>,
}

impl DeviceJob {
    fn start(
        mut session: DeviceSession,
        kind: JobKind,
        work: impl FnOnce(&mut DeviceSession, &AtomicU32) -> DeviceOutcome + Send + 'static,
    ) -> Result<Self, SessionError> {
        let description = session.description.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let shared = Arc::clone(&progress);
        let (sender, outcome) = mpsc::channel();
        thread::Builder::new()
            .name("afik-device".to_owned())
            .spawn(move || {
                let result = work(&mut session, &shared);
                let _ignored = sender.send((session, result));
            })
            .map_err(|error| SessionError::Radio(FlashError::Io(error)))?;
        Ok(Self {
            kind,
            description,
            progress,
            outcome,
        })
    }

    /// What the job is doing.
    pub const fn kind(&self) -> JobKind {
        self.kind
    }

    /// The radio the job is talking to.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Blocks written so far and blocks to write; zero before the first.
    pub fn progress(&self) -> (u16, u16) {
        let packed = self.progress.load(Ordering::Relaxed);
        ((packed >> 16) as u16, (packed & 0xFFFF) as u16)
    }

    /// The session and outcome once the job has ended.
    ///
    /// A worker which died without answering is reported as the session
    /// lost; the radio has to be connected again.
    pub fn poll(&self) -> Option<Result<(DeviceSession, DeviceOutcome), ()>> {
        match self.outcome.try_recv() {
            Ok(done) => Some(Ok(done)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(())),
        }
    }

    /// Waits for the job to end.
    pub fn wait(self) -> Option<(DeviceSession, DeviceOutcome)> {
        self.outcome.recv().ok()
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

        let report = session.write_project(&project, |_, _| {}).unwrap();
        assert!(report.changed_blocks > 0);
        let index = session.simulator().unwrap().index().unwrap();
        assert_eq!(index.len(), 1);

        let read = ProjectModel::from_project(&session.read_project().unwrap());
        assert_eq!(read, model);
    }

    #[test]
    fn a_write_sends_the_programmed_vfos_and_dual_watch_as_the_radios_place() {
        use radio_channel_plan::VfoStart;
        use radio_domain::{RadioConfig, Tone};
        use radio_platform::memory_link::PLACE_ORIGIN;
        use radio_platform::operator_state::{OperatorState, OPERATOR_STATE_BYTES};

        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        let mut project = model.validate().unwrap();
        project.set_config(RadioConfig {
            dual_watch: true,
            ..project.config().unwrap_or_else(RadioConfig::conservative)
        });
        let a = VfoStart {
            hz: 446_068_750,
            step_hz: 6_250,
            rx_tone: Tone::Ctcss(885),
            tx_tone: Tone::Ctcss(885),
            power: None,
        };
        project.set_vfo(0, Some(a));

        let mut session = DeviceSession::connect_simulator().unwrap();
        session.write_project(&project, |_, _| {}).unwrap();
        let at = usize::from(PLACE_ORIGIN);
        let record: [u8; OPERATOR_STATE_BYTES] = session.simulator().unwrap().memory()
            [at..at + OPERATOR_STATE_BYTES]
            .try_into()
            .unwrap();
        let place = OperatorState::decode(&record).expect("a place record");
        assert_eq!(
            place,
            OperatorState::programmed([None, None], [Some(a), None], true)
        );
        assert!(place.dual_watch, "the Radio tab's dual watch, not off");
        assert_eq!(place.vfo.rx_tone, Tone::Ctcss(885));
    }

    #[test]
    fn a_write_and_a_read_run_off_the_calling_thread_and_hand_the_session_back() {
        use super::{DeviceOutcome, JobKind};
        let mut model = ProjectModel::new();
        model.add_bank();
        model.add_channel(0);
        let session = DeviceSession::connect_simulator().unwrap();

        let job = session.start_write(model.validate().unwrap()).unwrap();
        assert_eq!(job.kind(), JobKind::Write);
        assert_eq!(job.description(), "simulator");
        let (session, outcome) = job.wait().unwrap();
        let DeviceOutcome::Wrote(Ok(report)) = outcome else {
            panic!("the write succeeds");
        };
        assert!(report.changed_blocks > 0);

        let job = session.start_read().unwrap();
        let (session, outcome) = job.wait().unwrap();
        let DeviceOutcome::Read(Ok(project)) = outcome else {
            panic!("the read succeeds");
        };
        assert_eq!(ProjectModel::from_project(&project), model);
        assert_eq!(session.simulator().unwrap().index().unwrap().len(), 1);
    }
}
