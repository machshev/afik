//! Default file locations, following the XDG base directory convention.
//!
//! Every path field starts filled in with a sensible location under the AFIK
//! data directory, and the operator can always type over it.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Directory name under the XDG data directory.
const APPLICATION_DIR: &str = "afik";
/// Subdirectory holding EEPROM backups.
const EEPROM_DIR: &str = "eeprom";

/// Returns the AFIK data directory, `$XDG_DATA_HOME/afik` or `~/.local/share/afik`.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

/// Resolves the data directory from the two environment values, without I/O.
///
/// The XDG specification ignores a relative `XDG_DATA_HOME`, and so does this.
pub fn data_dir_from(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_base(xdg_data_home, home, &[".local", "share"])
}

/// Returns the AFIK state directory, `$XDG_STATE_HOME/afik` or `~/.local/state/afik`.
///
/// Logs are state rather than data: worth keeping, but not something an
/// operator would copy to another machine.
pub fn state_dir() -> Option<PathBuf> {
    state_dir_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// Resolves the state directory from the two environment values, without I/O.
pub fn state_dir_from(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_base(xdg_state_home, home, &[".local", "state"])
}

/// Picks an absolute XDG base, or the given fallback under an absolute home.
fn xdg_base(xdg: Option<OsString>, home: Option<OsString>, fallback: &[&str]) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| fallback.iter().fold(home, |path, part| path.join(part)))
        })?;
    Some(base.join(APPLICATION_DIR))
}

/// Returns the directory EEPROM backups are written to by default.
pub fn eeprom_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join(EEPROM_DIR))
}

/// Returns a fresh default EEPROM backup path named for the current time.
///
/// A backup never overwrites a file, so each one gets its own name.
pub fn default_eeprom_backup() -> PathBuf {
    eeprom_dir()
        .unwrap_or_default()
        .join(eeprom_backup_name(unix_now()))
}

/// Returns the newest EEPROM backup in `directory`, if there is one.
///
/// Backup names sort by time, so the newest is the last by name.
pub fn newest_eeprom_backup(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| extension == "bin")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("eeprom-"))
        })
        .max()
}

/// Names one backup by its UTC time, as `eeprom-YYYYMMDDTHHMMSSZ.bin`.
pub fn eeprom_backup_name(unix_seconds: u64) -> String {
    let [year, month, day, hour, minute, second] = utc_fields(unix_seconds);
    format!("eeprom-{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z.bin")
}

/// Formats a UTC time as ISO 8601, `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc_timestamp(unix_seconds: u64) -> String {
    let [year, month, day, hour, minute, second] = utc_fields(unix_seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Returns the current time in whole seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Splits a Unix time into year, month, day, hour, minute, and second.
const fn utc_fields(unix_seconds: u64) -> [u64; 6] {
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(unix_seconds / 86_400);
    [
        year,
        month,
        day,
        seconds_of_day / 3_600,
        seconds_of_day % 3_600 / 60,
        seconds_of_day % 60,
    ]
}

/// Converts days since 1970-01-01 to a proleptic Gregorian date.
///
/// This is Howard Hinnant's `civil_from_days`, restricted to dates after 1970.
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{
        data_dir_from, eeprom_backup_name, newest_eeprom_backup, state_dir_from, utc_timestamp,
    };
    use std::{ffi::OsString, path::PathBuf};

    fn os(value: &str) -> OsString {
        OsString::from(value)
    }

    #[test]
    fn the_data_directory_follows_xdg_and_falls_back_to_home() {
        assert_eq!(
            data_dir_from(Some(os("/data")), Some(os("/home/op"))),
            Some(PathBuf::from("/data/afik"))
        );
        assert_eq!(
            data_dir_from(None, Some(os("/home/op"))),
            Some(PathBuf::from("/home/op/.local/share/afik"))
        );
        // A relative XDG_DATA_HOME is ignored, as the specification says.
        assert_eq!(
            data_dir_from(Some(os("relative")), Some(os("/home/op"))),
            Some(PathBuf::from("/home/op/.local/share/afik"))
        );
        assert_eq!(data_dir_from(None, None), None);
        assert_eq!(
            state_dir_from(None, Some(os("/home/op"))),
            Some(PathBuf::from("/home/op/.local/state/afik"))
        );
        assert_eq!(
            state_dir_from(Some(os("/state")), Some(os("/home/op"))),
            Some(PathBuf::from("/state/afik"))
        );
        assert_eq!(data_dir_from(Some(os("")), Some(os(""))), None);
    }

    #[test]
    fn backup_names_carry_their_utc_time_and_sort_by_it() {
        assert_eq!(eeprom_backup_name(0), "eeprom-19700101T000000Z.bin");
        // 2024-02-29 23:59:59 UTC, a leap day.
        assert_eq!(
            eeprom_backup_name(1_709_251_199),
            "eeprom-20240229T235959Z.bin"
        );
        assert_eq!(
            eeprom_backup_name(1_709_251_200),
            "eeprom-20240301T000000Z.bin"
        );
        assert!(eeprom_backup_name(1_709_251_199) < eeprom_backup_name(1_709_251_200));
        assert_eq!(utc_timestamp(1_709_251_199), "2024-02-29T23:59:59Z");
    }

    #[test]
    fn the_newest_backup_is_found_by_name() {
        let directory =
            std::env::temp_dir().join(format!("afik-eeprom-dir-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(newest_eeprom_backup(&directory), None);
        for name in [
            "eeprom-20240101T000000Z.bin",
            "eeprom-20250101T000000Z.bin",
            "notes.txt",
        ] {
            std::fs::write(directory.join(name), [0_u8]).unwrap();
        }
        assert_eq!(
            newest_eeprom_backup(&directory),
            Some(directory.join("eeprom-20250101T000000Z.bin"))
        );
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(newest_eeprom_backup(&directory), None);
    }
}
