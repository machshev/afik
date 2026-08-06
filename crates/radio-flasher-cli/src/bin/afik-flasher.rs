//! Process entry point for the generic auto-detecting AFIK flasher.

#![forbid(unsafe_code)]

use std::{env, io, process::ExitCode};

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let status = radio_flasher_cli::run_auto_to(&arguments, &mut stdout, &mut stderr);
    u8::try_from(status).map_or(ExitCode::FAILURE, ExitCode::from)
}
