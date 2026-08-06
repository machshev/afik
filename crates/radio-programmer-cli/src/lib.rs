//! Thin, bounded command-line front end for `radio-programmer`.

#![forbid(unsafe_code)]

use radio_channel_plan::{BankName, GeneratedBank};
use radio_domain::{BankId, Frequency, FrequencyStep, TxClass};
use radio_programmer::{Programmer, ProtocolTransport, RadioProject};
use radio_sim::{SimDevice, SimTransport};
use radio_storage::ObjectKind;
use std::{
    convert::Infallible,
    fmt::{self, Write as _},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

/// Successful process exit code.
pub const EXIT_SUCCESS: i32 = 0;
/// Runtime, transport, device, compiler, or file failure exit code.
pub const EXIT_OPERATION: i32 = 1;
/// Command-line usage failure exit code.
pub const EXIT_USAGE: i32 = 2;
/// Maximum canonical image bytes accepted from one input file.
pub const MAX_CLI_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Stable command help text.
pub const HELP: &str = "AFIK programmer CLI\n\
\n\
Usage:\n\
  afik-programmer (--sim | --device PATH --baud BAUD) info\n\
  afik-programmer (--sim | --device PATH --baud BAUD) list\n\
  afik-programmer (--sim | --device PATH --baud BAUD) compile OUTPUT [--force] --bank SPEC...\n\
  afik-programmer (--sim | --device PATH --baud BAUD) write --bank SPEC...\n\
  afik-programmer (--sim | --device PATH --baud BAUD) backup OUTPUT [--force]\n\
  afik-programmer (--sim | --device PATH --baud BAUD) restore INPUT\n\
  afik-programmer --help\n\
  afik-programmer --version\n\
\n\
Bank SPEC: ID:NAME:BASE_HZ:SPACING_HZ:COUNT:TX_CLASS\n\
TX_CLASS: never, licence-free, amateur, marine, aeronautical, business, experimental\n\
Supported BAUD: 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200\n\
\n\
Exit codes: 0 success, 1 operation failure, 2 usage failure.\n";

const SUPPORTED_BAUDS: [u32; 8] = [1_200, 2_400, 4_800, 9_600, 19_200, 38_400, 57_600, 115_200];

/// Captured process output and exit status from one CLI invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliOutcome {
    /// Stable process exit code.
    pub exit_code: i32,
    /// Complete standard output text.
    pub stdout: String,
    /// Complete standard error text.
    pub stderr: String,
}

impl CliOutcome {
    fn success(stdout: String) -> Self {
        Self {
            exit_code: EXIT_SUCCESS,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(exit_code: i32, error: impl fmt::Display) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr: format!("error: {error}\n"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Parsed {
    Help,
    Version,
    Run { backend: Backend, command: Command },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Backend {
    Simulator,
    Serial { path: PathBuf, baud: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Command {
    Info,
    List,
    Compile {
        output: PathBuf,
        force: bool,
        project: RadioProject,
    },
    Write {
        project: RadioProject,
    },
    Backup {
        output: PathBuf,
        force: bool,
    },
    Restore {
        input: PathBuf,
    },
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    Operation(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Operation(message) => formatter.write_str(message),
        }
    }
}

impl CliError {
    fn operation(error: impl fmt::Display) -> Self {
        Self::Operation(error.to_string())
    }
}

/// Parses and executes one invocation without terminating the process.
pub fn run(arguments: &[String]) -> CliOutcome {
    match parse(arguments) {
        Ok(Parsed::Help) => CliOutcome::success(HELP.to_owned()),
        Ok(Parsed::Version) => {
            CliOutcome::success(format!("afik-programmer {}\n", env!("CARGO_PKG_VERSION")))
        }
        Ok(Parsed::Run { backend, command }) => match execute(backend, command) {
            Ok(output) => CliOutcome::success(output),
            Err(CliError::Usage(message)) => CliOutcome::failure(EXIT_USAGE, message),
            Err(CliError::Operation(message)) => CliOutcome::failure(EXIT_OPERATION, message),
        },
        Err(CliError::Usage(message)) => CliOutcome::failure(EXIT_USAGE, message),
        Err(CliError::Operation(message)) => CliOutcome::failure(EXIT_OPERATION, message),
    }
}

fn parse(arguments: &[String]) -> Result<Parsed, CliError> {
    if arguments == ["--help"] || arguments == ["-h"] || arguments == ["help"] {
        return Ok(Parsed::Help);
    }
    if arguments == ["--version"] {
        return Ok(Parsed::Version);
    }
    if arguments.is_empty() {
        return Err(CliError::Usage(
            "missing backend and command; use --help".into(),
        ));
    }

    let mut simulator = false;
    let mut device = None;
    let mut baud = None;
    let mut offset = 0;
    while let Some(argument) = arguments.get(offset) {
        match argument.as_str() {
            "--sim" => {
                if simulator {
                    return Err(CliError::Usage("--sim was provided more than once".into()));
                }
                simulator = true;
                offset += 1;
            }
            "--device" => {
                if device.is_some() {
                    return Err(CliError::Usage(
                        "--device was provided more than once".into(),
                    ));
                }
                device = Some(PathBuf::from(require_value(arguments, offset, "--device")?));
                offset += 2;
            }
            "--baud" => {
                if baud.is_some() {
                    return Err(CliError::Usage("--baud was provided more than once".into()));
                }
                let value = require_value(arguments, offset, "--baud")?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| CliError::Usage(format!("invalid baud: {value}")))?;
                if !SUPPORTED_BAUDS.contains(&parsed) {
                    return Err(CliError::Usage(format!("unsupported baud: {parsed}")));
                }
                baud = Some(parsed);
                offset += 2;
            }
            _ => break,
        }
    }

    let backend = match (simulator, device, baud) {
        (true, None, None) => Backend::Simulator,
        (false, Some(path), Some(baud)) => Backend::Serial { path, baud },
        (false, None, None) => {
            return Err(CliError::Usage(
                "select exactly one backend: --sim or --device PATH --baud BAUD".into(),
            ));
        }
        (true, _, _) => {
            return Err(CliError::Usage(
                "--sim conflicts with --device and --baud".into(),
            ));
        }
        (false, Some(_), None) => {
            return Err(CliError::Usage("--device requires --baud".into()));
        }
        (false, None, Some(_)) => {
            return Err(CliError::Usage("--baud requires --device".into()));
        }
    };

    let command_name = arguments
        .get(offset)
        .ok_or_else(|| CliError::Usage("missing command; use --help".into()))?;
    let command_arguments = &arguments[offset + 1..];
    let command = match command_name.as_str() {
        "info" => {
            require_no_arguments(command_arguments, "info")?;
            Command::Info
        }
        "list" => {
            require_no_arguments(command_arguments, "list")?;
            Command::List
        }
        "compile" => parse_compile(command_arguments)?,
        "write" => Command::Write {
            project: parse_banks(command_arguments)?,
        },
        "backup" => parse_backup(command_arguments)?,
        "restore" => {
            if command_arguments.len() != 1 {
                return Err(CliError::Usage("restore requires exactly one INPUT".into()));
            }
            Command::Restore {
                input: PathBuf::from(&command_arguments[0]),
            }
        }
        unknown => return Err(CliError::Usage(format!("unknown command: {unknown}"))),
    };
    Ok(Parsed::Run { backend, command })
}

fn require_value<'a>(
    arguments: &'a [String],
    option_offset: usize,
    option: &str,
) -> Result<&'a str, CliError> {
    arguments
        .get(option_offset + 1)
        .map(String::as_str)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| CliError::Usage(format!("{option} requires a value")))
}

fn require_no_arguments(arguments: &[String], command: &str) -> Result<(), CliError> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "{command} does not accept arguments"
        )))
    }
}

fn parse_compile(arguments: &[String]) -> Result<Command, CliError> {
    let output = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| CliError::Usage("compile requires OUTPUT".into()))?;
    let (force, project) = parse_force_and_banks(&arguments[1..], true)?;
    Ok(Command::Compile {
        output: PathBuf::from(output),
        force,
        project,
    })
}

fn parse_backup(arguments: &[String]) -> Result<Command, CliError> {
    let output = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .ok_or_else(|| CliError::Usage("backup requires OUTPUT".into()))?;
    let mut force = false;
    for argument in &arguments[1..] {
        if argument == "--force" && !force {
            force = true;
        } else {
            return Err(CliError::Usage(format!(
                "unexpected backup argument: {argument}"
            )));
        }
    }
    Ok(Command::Backup {
        output: PathBuf::from(output),
        force,
    })
}

fn parse_banks(arguments: &[String]) -> Result<RadioProject, CliError> {
    parse_force_and_banks(arguments, false).map(|(_, project)| project)
}

fn parse_force_and_banks(
    arguments: &[String],
    allow_force: bool,
) -> Result<(bool, RadioProject), CliError> {
    let mut force = false;
    let mut project = RadioProject::new();
    let mut bank_count = 0_usize;
    let mut offset = 0;
    while offset < arguments.len() {
        match arguments[offset].as_str() {
            "--force" if allow_force && !force => {
                force = true;
                offset += 1;
            }
            "--bank" => {
                let spec = require_value(arguments, offset, "--bank")?;
                project.add_generated_bank(parse_bank(spec)?);
                bank_count += 1;
                offset += 2;
            }
            argument => {
                return Err(CliError::Usage(format!(
                    "unexpected project argument: {argument}"
                )));
            }
        }
    }
    if bank_count == 0 {
        return Err(CliError::Usage(
            "compile and write require at least one --bank SPEC".into(),
        ));
    }
    Ok((force, project))
}

fn parse_bank(spec: &str) -> Result<GeneratedBank, CliError> {
    let fields = spec.split(':').collect::<Vec<_>>();
    if fields.len() != 6 {
        return Err(CliError::Usage(format!(
            "bank spec requires six colon-separated fields: {spec}"
        )));
    }
    let id = parse_integer::<u16>(fields[0], "bank ID")?;
    let name = BankName::new(fields[1])
        .map_err(|error| CliError::Usage(format!("invalid bank name: {error}")))?;
    let base = Frequency::from_hz(parse_integer::<u32>(fields[2], "base frequency")?)
        .map_err(|error| CliError::Usage(format!("invalid base frequency: {error}")))?;
    let spacing = FrequencyStep::from_hz(parse_integer::<u32>(fields[3], "spacing")?)
        .map_err(|error| CliError::Usage(format!("invalid spacing: {error}")))?;
    let count = parse_integer::<u16>(fields[4], "channel count")?;
    let class = parse_class(fields[5])?;
    GeneratedBank::linear_simplex(BankId::new(id), name, base, spacing, count, class)
        .map_err(|error| CliError::Usage(format!("invalid generated bank: {error}")))
}

fn parse_integer<T>(value: &str, label: &str) -> Result<T, CliError>
where
    T: core::str::FromStr,
{
    value
        .parse::<T>()
        .map_err(|_| CliError::Usage(format!("invalid {label}: {value}")))
}

fn parse_class(value: &str) -> Result<TxClass, CliError> {
    match value {
        "never" => Ok(TxClass::Never),
        "licence-free" => Ok(TxClass::LicenceFreePlan),
        "amateur" => Ok(TxClass::Amateur),
        "marine" => Ok(TxClass::Marine),
        "aeronautical" => Ok(TxClass::Aeronautical),
        "business" => Ok(TxClass::Business),
        "experimental" => Ok(TxClass::Experimental),
        _ => Err(CliError::Usage(format!("unknown TX class: {value}"))),
    }
}

fn execute(backend: Backend, command: Command) -> Result<String, CliError> {
    let transport = match backend {
        Backend::Simulator => {
            CliTransport::Simulator(Box::new(SimTransport::new(SimDevice::new())))
        }
        Backend::Serial { path, baud } => {
            CliTransport::Serial(SerialTransport::open(&path, baud).map_err(CliError::operation)?)
        }
    };
    let mut programmer = Programmer::connect(transport).map_err(CliError::operation)?;
    match command {
        Command::Info => Ok(render_info(programmer.capabilities())),
        Command::List => execute_list(&mut programmer),
        Command::Compile {
            output,
            force,
            project,
        } => execute_compile(&programmer, &output, force, &project),
        Command::Write { project } => execute_write(&mut programmer, &project, "written"),
        Command::Backup { output, force } => execute_backup(&mut programmer, &output, force),
        Command::Restore { input } => execute_restore(&mut programmer, &input),
    }
}

fn execute_list(programmer: &mut Programmer<CliTransport>) -> Result<String, CliError> {
    let listing = programmer.list_objects().map_err(CliError::operation)?;
    let mut output = format!(
        "generation={}\nobject_count={}\n",
        listing.generation,
        listing.objects.len()
    );
    for object in listing.objects {
        writeln!(
            output,
            "object={}:{}:{}",
            object_kind_name(object.key.kind),
            object.key.id,
            object.encoded_len
        )
        .map_err(CliError::operation)?;
    }
    Ok(output)
}

fn execute_compile(
    programmer: &Programmer<CliTransport>,
    output: &Path,
    force: bool,
    project: &RadioProject,
) -> Result<String, CliError> {
    let configuration = programmer
        .compiler()
        .compile(project)
        .map_err(CliError::operation)?;
    let mut image = vec![0; configuration.image_len().map_err(CliError::operation)?];
    let image_bytes = configuration
        .encode_image(&mut image)
        .map_err(CliError::operation)?;
    write_output(output, &image, force)?;
    Ok(render_capacity(
        "compiled",
        configuration.report(),
        Some(image_bytes),
        Some(output),
    ))
}

fn execute_write(
    programmer: &mut Programmer<CliTransport>,
    project: &RadioProject,
    action: &str,
) -> Result<String, CliError> {
    let configuration = programmer
        .compiler()
        .compile(project)
        .map_err(CliError::operation)?;
    write_and_verify(programmer, &configuration, action)
}

fn execute_backup(
    programmer: &mut Programmer<CliTransport>,
    output: &Path,
    force: bool,
) -> Result<String, CliError> {
    let backup = programmer
        .backup_configuration()
        .map_err(CliError::operation)?;
    write_output(output, &backup.image, force)?;
    let mut rendered = format!("generation={}\n", backup.generation);
    rendered.push_str(&render_capacity(
        "backed_up",
        backup.report,
        Some(backup.image.len()),
        Some(output),
    ));
    Ok(rendered)
}

fn execute_restore(
    programmer: &mut Programmer<CliTransport>,
    input: &Path,
) -> Result<String, CliError> {
    let image = read_bounded(input)?;
    let receipt = programmer
        .restore_configuration_image(&image)
        .map_err(CliError::operation)?;
    Ok(render_verified_receipt(receipt, "restored"))
}

fn write_and_verify(
    programmer: &mut Programmer<CliTransport>,
    configuration: &radio_programmer::CompiledConfiguration,
    action: &str,
) -> Result<String, CliError> {
    let receipt = programmer
        .write_configuration_verified(configuration)
        .map_err(CliError::operation)?;
    Ok(render_verified_receipt(receipt, action))
}

fn render_verified_receipt(
    receipt: radio_programmer::VerifiedConfigurationReceipt,
    action: &str,
) -> String {
    let mut output = format!("generation={}\nverified=true\n", receipt.generation);
    output.push_str(&render_capacity(action, receipt.report, None, None));
    output
}

fn render_info(capabilities: radio_programmer::DeviceCapabilities) -> String {
    format!(
        "protocol_version={}\nstorage_version={}\nmax_frame_payload={}\nmax_objects={}\nmax_object_size={}\nplan_encodings=0x{:04x}\n",
        capabilities.protocol_version,
        capabilities.storage_version,
        capabilities.max_frame_payload,
        capabilities.max_objects,
        capabilities.max_object_size,
        capabilities.plan_encodings,
    )
}

fn render_capacity(
    action: &str,
    report: radio_programmer::CapacityReport,
    image_bytes: Option<usize>,
    output_path: Option<&Path>,
) -> String {
    let mut output = format!(
        "action={action}\nobject_count={}\nstorage_bytes={}\ngenerated_channels={}\n",
        report.object_count, report.storage_bytes, report.generated_channels
    );
    if let Some(image_bytes) = image_bytes {
        let _ignored = writeln!(output, "image_bytes={image_bytes}");
    }
    if let Some(path) = output_path {
        let _ignored = writeln!(output, "output={}", path.display());
    }
    output
}

const fn object_kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::GeneratedBank => "generated-bank",
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let file = File::open(path).map_err(CliError::operation)?;
    let metadata = file.metadata().map_err(CliError::operation)?;
    if metadata.len() > MAX_CLI_IMAGE_BYTES {
        return Err(CliError::Operation(format!(
            "input image exceeds {MAX_CLI_IMAGE_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CLI_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(CliError::operation)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CLI_IMAGE_BYTES {
        return Err(CliError::Operation(format!(
            "input image exceeds {MAX_CLI_IMAGE_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let mut options = OpenOptions::new();
    options.write(true);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(CliError::operation)?;
    file.write_all(bytes).map_err(CliError::operation)?;
    file.flush().map_err(CliError::operation)
}

enum CliTransport {
    Simulator(Box<SimTransport>),
    Serial(SerialTransport),
}

#[derive(Debug)]
enum CliTransportError {
    Serial(io::Error),
}

impl fmt::Display for CliTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serial(error) => write!(formatter, "serial I/O failed: {error}"),
        }
    }
}

impl ProtocolTransport for CliTransport {
    type Error = CliTransportError;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        match self {
            Self::Simulator(transport) => transport.send(frame).map_err(infallible),
            Self::Serial(transport) => transport.send(frame).map_err(CliTransportError::Serial),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        match self {
            Self::Simulator(transport) => transport.receive(buffer).map_err(infallible),
            Self::Serial(transport) => transport.receive(buffer).map_err(CliTransportError::Serial),
        }
    }
}

fn infallible(error: Infallible) -> CliTransportError {
    match error {}
}

struct SerialTransport {
    file: File,
}

impl SerialTransport {
    fn open(path: &Path, baud: u32) -> Result<Self, SerialOpenError> {
        let baud_text = baud.to_string();
        let configured = ProcessCommand::new("stty")
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

impl ProtocolTransport for SerialTransport {
    type Error = io::Error;

    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error> {
        self.file.write_all(frame)?;
        self.file.flush()
    }

    fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        self.file.read(buffer)
    }
}

#[derive(Debug)]
enum SerialOpenError {
    Stty(io::Error),
    Configure { status: Option<i32>, detail: String },
    Open(io::Error),
}

impl fmt::Display for SerialOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

#[cfg(test)]
mod tests {
    use super::{run, CliOutcome, EXIT_OPERATION, EXIT_SUCCESS, EXIT_USAGE, HELP};
    use radio_storage::decode_configuration_image;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
    };

    static NEXT_FILE: AtomicU32 = AtomicU32::new(1);

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn invoke(values: &[&str]) -> CliOutcome {
        run(&arguments(values))
    }

    fn temp_file(label: &str) -> PathBuf {
        let suffix = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "afik-cli-{}-{label}-{suffix}.afik",
            std::process::id()
        ))
    }

    fn bank(id: u16) -> String {
        format!("{id}:PMR446:446006250:12500:16:licence-free")
    }

    #[test]
    fn help_version_and_usage_exit_codes_are_stable() {
        assert_eq!(
            invoke(&["--help"]),
            CliOutcome {
                exit_code: EXIT_SUCCESS,
                stdout: HELP.to_owned(),
                stderr: String::new(),
            }
        );
        let version = invoke(&["--version"]);
        assert_eq!(version.exit_code, EXIT_SUCCESS);
        assert_eq!(version.stdout, "afik-programmer 0.1.0\n");
        for values in [
            &[][..],
            &["info"][..],
            &["--sim", "--device", "/dev/null", "--baud", "9600", "info"][..],
            &["--device", "/dev/null", "info"][..],
            &["--sim", "unknown"][..],
            &["--sim", "write"][..],
            &["--sim", "write", "--bank", "1:x:1:1:1:unknown"][..],
            &["--sim", "compile", "out", "--bank", "broken"][..],
        ] {
            let outcome = invoke(values);
            assert_eq!(outcome.exit_code, EXIT_USAGE, "{values:?}");
            assert!(outcome.stdout.is_empty());
            assert!(outcome.stderr.starts_with("error: "));
        }
    }

    #[test]
    fn simulator_info_and_empty_listing_are_exact() {
        assert_eq!(
            invoke(&["--sim", "info"]),
            CliOutcome {
                exit_code: EXIT_SUCCESS,
                stdout: "protocol_version=1\nstorage_version=1\nmax_frame_payload=128\nmax_objects=8\nmax_object_size=64\nplan_encodings=0x0001\n".into(),
                stderr: String::new(),
            }
        );
        assert_eq!(
            invoke(&["--sim", "list"]).stdout,
            "generation=0\nobject_count=0\n"
        );
    }

    #[test]
    fn compile_is_deterministic_and_requires_explicit_overwrite() {
        let first = temp_file("first");
        let second = temp_file("second");
        let first_text = first.to_string_lossy().into_owned();
        let second_text = second.to_string_lossy().into_owned();
        let first_bank = bank(1);
        let second_bank = bank(2);
        let first_run = run(&arguments(&[
            "--sim",
            "compile",
            &first_text,
            "--bank",
            &second_bank,
            "--bank",
            &first_bank,
        ]));
        assert_eq!(first_run.exit_code, EXIT_SUCCESS);
        let second_run = run(&arguments(&[
            "--sim",
            "compile",
            &second_text,
            "--bank",
            &first_bank,
            "--bank",
            &second_bank,
        ]));
        assert_eq!(second_run.exit_code, EXIT_SUCCESS);
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());

        let refused = run(&arguments(&[
            "--sim",
            "compile",
            &first_text,
            "--bank",
            &first_bank,
        ]));
        assert_eq!(refused.exit_code, EXIT_OPERATION);
        let forced = run(&arguments(&[
            "--sim",
            "compile",
            &first_text,
            "--force",
            "--bank",
            &first_bank,
        ]));
        assert_eq!(forced.exit_code, EXIT_SUCCESS);

        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn simulator_write_backup_and_restore_use_programmer_logic() {
        let spec = bank(1);
        let written = run(&arguments(&["--sim", "write", "--bank", &spec]));
        assert_eq!(written.exit_code, EXIT_SUCCESS);
        assert!(written.stdout.contains("generation=1\nverified=true\n"));

        let backup = temp_file("backup");
        let backup_text = backup.to_string_lossy().into_owned();
        let backed_up = run(&arguments(&["--sim", "backup", &backup_text]));
        assert_eq!(backed_up.exit_code, EXIT_SUCCESS);
        let bytes = fs::read(&backup).unwrap();
        assert_eq!(
            decode_configuration_image(&bytes).unwrap().object_count(),
            0
        );

        let restored = run(&arguments(&["--sim", "restore", &backup_text]));
        assert_eq!(restored.exit_code, EXIT_SUCCESS);
        assert!(restored.stdout.contains("generation=1\nverified=true\n"));
        fs::remove_file(backup).unwrap();
    }

    #[test]
    fn duplicate_bank_and_oversized_input_are_operation_failures() {
        let duplicate = bank(1);
        let outcome = run(&arguments(&[
            "--sim", "write", "--bank", &duplicate, "--bank", &duplicate,
        ]));
        assert_eq!(outcome.exit_code, EXIT_OPERATION);
        assert!(outcome.stderr.contains("duplicate object"));

        let oversized = temp_file("oversized");
        let file = fs::File::create(&oversized).unwrap();
        file.set_len(super::MAX_CLI_IMAGE_BYTES + 1).unwrap();
        let path = oversized.to_string_lossy().into_owned();
        let outcome = run(&arguments(&["--sim", "restore", &path]));
        assert_eq!(outcome.exit_code, EXIT_OPERATION);
        assert!(outcome.stderr.contains("exceeds"));
        fs::remove_file(oversized).unwrap();
    }

    #[test]
    fn unsupported_baud_is_usage_and_missing_serial_device_is_operation() {
        assert_eq!(
            invoke(&["--device", "/definitely/missing", "--baud", "123", "info"]).exit_code,
            EXIT_USAGE
        );
        assert_eq!(
            invoke(&["--device", "/definitely/missing", "--baud", "9600", "info"]).exit_code,
            EXIT_OPERATION
        );
    }
}
