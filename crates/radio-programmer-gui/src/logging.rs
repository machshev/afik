//! Error and operation logging for the editor.
//!
//! Every failure the editor shows on its status lines is also logged, with the
//! context the status line leaves out, so a problem seen at the bench can be
//! read back afterwards. Lines go to stderr and are appended to
//! `$XDG_STATE_HOME/afik/studio.log` (`~/.local/state/afik/studio.log`).
//!
//! `AFIK_LOG` sets the level: `off`, `error`, `warn`, `info` (the default),
//! `debug`, or `trace`. Other crates, such as the windowing stack, log only
//! their warnings and errors unless the level is `debug` or finer.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use log::{LevelFilter, Log, Metadata, Record};

use crate::paths;

/// Log file name inside the AFIK state directory.
const LOG_FILE: &str = "studio.log";
/// Size past which the log is moved aside to `studio.log.1` at start-up.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Target prefixes of the editor's own records: the library and the binary.
const OWN_TARGETS: [&str; 2] = ["radio_programmer_gui", "afik_studio"];

/// Parses an `AFIK_LOG` value; anything unrecognised keeps the default.
pub fn parse_level(value: Option<&str>) -> LevelFilter {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("off") => LevelFilter::Off,
        Some("error") => LevelFilter::Error,
        Some("warn" | "warning") => LevelFilter::Warn,
        Some("debug") => LevelFilter::Debug,
        Some("trace") => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// Reports whether a record at this level and target should be written.
///
/// Other crates are held to warnings at the default level: the windowing
/// stack's informational chatter would bury the editor's own lines.
pub fn admits(level: LevelFilter, record_level: log::Level, target: &str) -> bool {
    let limit =
        if OWN_TARGETS.iter().any(|own| target.starts_with(own)) || level >= LevelFilter::Debug {
            level
        } else {
            level.min(LevelFilter::Warn)
        };
    record_level <= limit
}

/// Formats one line: UTC time, level, target, and message.
pub fn format_line(unix_seconds: u64, level: log::Level, target: &str, message: &str) -> String {
    format!(
        "{} {level:<5} {target}: {message}",
        paths::utc_timestamp(unix_seconds)
    )
}

struct StudioLogger {
    level: LevelFilter,
    file: Option<Mutex<File>>,
}

impl Log for StudioLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        admits(self.level, metadata.level(), metadata.target())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            paths::unix_now(),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        // A logger has nowhere to report its own failure, so a lost line is
        // dropped rather than allowed to disturb the editor.
        let _ignored = writeln!(std::io::stderr(), "{line}");
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ignored = writeln!(file, "{line}");
            }
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ignored = file.flush();
            }
        }
    }
}

/// Opens the log file for appending, moving an oversized one aside first.
fn open_log_file(directory: &Path) -> std::io::Result<(File, PathBuf)> {
    fs::create_dir_all(directory)?;
    let path = directory.join(LOG_FILE);
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        fs::rename(&path, directory.join(format!("{LOG_FILE}.1")))?;
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    Ok((file, path))
}

/// Installs the logger and returns the log file in use, if one could be opened.
///
/// Logging never stops the editor: without a usable state directory it logs to
/// stderr alone and says so.
pub fn init() -> Option<PathBuf> {
    let level = parse_level(std::env::var("AFIK_LOG").ok().as_deref());
    let opened = paths::state_dir().map(|directory| open_log_file(&directory));
    let (file, path, problem) = match opened {
        Some(Ok((file, path))) => (Some(Mutex::new(file)), Some(path), None),
        Some(Err(error)) => (
            None,
            None,
            Some(format!("could not open the log file: {error}")),
        ),
        None => (
            None,
            None,
            Some("no XDG state directory or HOME is set".to_owned()),
        ),
    };
    let logger = StudioLogger { level, file };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    if let Some(problem) = problem {
        log::warn!("logging to stderr only: {problem}");
    }
    path
}

#[cfg(test)]
mod tests {
    use super::{admits, format_line, open_log_file, parse_level, LOG_FILE, MAX_LOG_BYTES};
    use log::{Level, LevelFilter};

    #[test]
    fn the_level_comes_from_afik_log_and_defaults_to_info() {
        assert_eq!(parse_level(None), LevelFilter::Info);
        assert_eq!(parse_level(Some("DEBUG")), LevelFilter::Debug);
        assert_eq!(parse_level(Some(" warn ")), LevelFilter::Warn);
        assert_eq!(parse_level(Some("off")), LevelFilter::Off);
        assert_eq!(parse_level(Some("nonsense")), LevelFilter::Info);
    }

    #[test]
    fn other_crates_log_only_warnings_until_debug() {
        let own = "radio_programmer_gui::app";
        assert!(admits(LevelFilter::Info, Level::Info, own));
        assert!(!admits(LevelFilter::Info, Level::Debug, own));
        assert!(!admits(LevelFilter::Info, Level::Info, "winit::platform"));
        assert!(admits(LevelFilter::Info, Level::Warn, "winit::platform"));
        assert!(admits(LevelFilter::Debug, Level::Debug, "winit::platform"));
        assert!(!admits(LevelFilter::Off, Level::Error, own));
        assert!(admits(LevelFilter::Info, Level::Info, "afik_studio"));
    }

    #[test]
    fn lines_carry_time_level_target_and_message() {
        assert_eq!(
            format_line(0, Level::Error, "radio_programmer_gui::app", "boom"),
            "1970-01-01T00:00:00Z ERROR radio_programmer_gui::app: boom"
        );
    }

    #[test]
    fn an_oversized_log_is_moved_aside_and_a_fresh_one_appended() {
        let directory = std::env::temp_dir().join(format!("afik-log-{}", std::process::id()));
        let (_file, path) = open_log_file(&directory).unwrap();
        assert_eq!(path, directory.join(LOG_FILE));
        let oversized = usize::try_from(MAX_LOG_BYTES).unwrap() + 1;
        std::fs::write(&path, vec![b'x'; oversized]).unwrap();
        let (_file, path) = open_log_file(&directory).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert!(directory.join(format!("{LOG_FILE}.1")).is_file());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
