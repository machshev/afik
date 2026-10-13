//! Thin auto-detecting K1/K5 flasher front end.

#![forbid(unsafe_code)]

mod cli;
mod k5;
pub mod prompt;

use prompt::Confirm;
use radio_flasher::BootloaderInfo;
use radio_programmer_serial::LinuxSerialTransport;
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

pub use cli::{run_to, run_with, HELP};

/// Bootloader negotiation version sent by the default K5 AFIK flash.
pub const DEFAULT_NEGOTIATION_VERSION: &str = "2.01.23";

/// Successful process exit code.
pub const EXIT_SUCCESS: i32 = 0;
/// File, validation, transport, protocol, or device failure exit code.
pub const EXIT_OPERATION: i32 = 1;
/// Command-line usage failure exit code.
pub const EXIT_USAGE: i32 = 2;
/// Exact serial speed observed for the supported legacy protocol.
pub const K5_BAUD: u32 = 38_400;

/// An AFIK application write whose radio has already been classified.
struct DetectedFlash<'a> {
    device: &'a Path,
    image_path: &'a Path,
    image_raw: &'a [u8],
    bootloader: &'a BootloaderInfo,
    transaction_id: u32,
}

/// Fails unless the operator approved the write `summary` describes.
fn confirm_write<C: Confirm>(confirm: &mut C, summary: &str) -> Result<(), CliError> {
    if confirm.confirm(summary).map_err(CliError::operation)? {
        return Ok(());
    }
    Err(CliError::Operation(if confirm.is_interactive() {
        "write declined".to_owned()
    } else {
        "nothing can answer a confirmation prompt here; pass --yes to write unattended".to_owned()
    }))
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("{option} was provided more than once"))
    } else {
        Ok(())
    }
}

fn open_serial(path: &Path) -> Result<LinuxSerialTransport, CliError> {
    LinuxSerialTransport::open(path, K5_BAUD).map_err(CliError::operation)
}

fn read_bounded(path: &Path, maximum: usize, description: &str) -> Result<Vec<u8>, CliError> {
    let file = File::open(path)
        .map_err(|error| CliError::Operation(format!("could not open {description}: {error}")))?;
    let limit = u64::try_from(maximum + 1).expect("bounded host file limit");
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::Operation(format!("could not read {description}: {error}")))?;
    if bytes.len() > maximum {
        return Err(CliError::Operation(format!(
            "{description} exceeds {maximum} bytes"
        )));
    }
    Ok(bytes)
}

fn write_private_file(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let mut options = OpenOptions::new();
    options.write(true).mode(0o600);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(path)
        .map_err(|error| CliError::Operation(format!("could not create backup: {error}")))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| CliError::Operation(format!("could not write backup: {error}")))
}

fn parse_crc32(value: &str) -> Result<u32, CliError> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::Operation(
            "--confirm-image-crc32 requires exactly eight hexadecimal digits".into(),
        ));
    }
    u32::from_str_radix(value, 16).map_err(CliError::operation)
}

fn fresh_transaction_id() -> Result<u32, CliError> {
    let mut random = File::open("/dev/urandom").map_err(CliError::operation)?;
    for _ in 0..4 {
        let mut bytes = [0_u8; 4];
        random.read_exact(&mut bytes).map_err(CliError::operation)?;
        let transaction_id = u32::from_le_bytes(bytes);
        if transaction_id != 0 {
            return Ok(transaction_id);
        }
    }
    Err(CliError::Operation(
        "could not obtain a nonzero transaction identifier".into(),
    ))
}

fn write_success<W: Write>(output: &mut W, text: &str) -> i32 {
    output
        .write_all(text.as_bytes())
        .map_or(EXIT_OPERATION, |()| EXIT_SUCCESS)
}

fn write_failure<E: Write>(output: &mut E, code: i32, error: impl fmt::Display) -> i32 {
    let _ = writeln!(output, "error: {error}");
    code
}

#[derive(Debug)]
enum CliError {
    Operation(String),
}

impl CliError {
    fn operation(error: impl fmt::Display) -> Self {
        Self::Operation(error.to_string())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(message) => formatter.write_str(message),
        }
    }
}
