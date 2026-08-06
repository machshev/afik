//! Process entry point for the thin AFIK programmer CLI.

#![forbid(unsafe_code)]

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let outcome = radio_programmer_cli::run(&arguments);
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    u8::try_from(outcome.exit_code).map_or(ExitCode::FAILURE, ExitCode::from)
}
