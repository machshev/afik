//! Syncs every map dataset into a directory, for the Pages build to publish
//! beside the browser Studio (`ADR-121`). Each dataset that fails is named,
//! and the exit status says whether any did.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: mirror-datasets DIR");
        return ExitCode::from(2);
    };
    let mut failed = false;
    for (name, outcome) in radio_programmer_gui::mirror::write_all(dir.as_ref()) {
        match outcome {
            Ok(count) => println!("{name}: {count} entries"),
            Err(error) => {
                eprintln!("{name}: {error}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
