//! Syncs every map dataset into a directory, for the Pages build to publish
//! beside the browser Studio (`ADR-121`), taking one that cannot be fetched
//! from its snapshot. A snapshot taken is a warning; a dataset left out is
//! an error and the exit status says so.

use std::process::ExitCode;

use radio_programmer_gui::mirror::{write_all, Mirrored, SNAPSHOT_DIR};

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: mirror-datasets DIR");
        return ExitCode::from(2);
    };
    // GitHub Actions shows these as annotations on the run.
    let (warning, error) = if std::env::var_os("GITHUB_ACTIONS").is_some() {
        ("::warning::", "::error::")
    } else {
        ("warning: ", "error: ")
    };
    let mut failed = false;
    for (name, outcome) in write_all(dir.as_ref(), SNAPSHOT_DIR.as_ref()) {
        match outcome {
            Ok(Mirrored::Fetched(count)) => println!("{name}: {count} entries"),
            Ok(Mirrored::Snapshot(why)) => {
                println!("{warning}{name}: took the snapshot, since {why}");
            }
            Err(why) => {
                println!("{error}{name}: left out: {why}");
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
