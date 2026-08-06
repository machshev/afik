//! Process entry point for the thin AFIK programmer GUI launcher.

#![forbid(unsafe_code)]

use std::{env, process::ExitCode};

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    u8::try_from(radio_programmer_gui::main_entry(&arguments))
        .map_or(ExitCode::FAILURE, ExitCode::from)
}
