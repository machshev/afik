//! Default file locations, following the XDG base directory convention.
//!
//! Every path field starts filled in with a sensible location under the AFIK
//! data directory, and the operator can always type over it.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Directory name under the XDG data directory.
const APPLICATION_DIR: &str = "afik";
/// Subdirectory holding EEPROM backups.
const EEPROM_DIR: &str = "eeprom";
/// Subdirectory holding projects.
const PROJECTS_DIR: &str = "projects";
/// State file naming the project opened last.
const LAST_PROJECT_FILE: &str = "last-project";

/// Returns the AFIK data directory, `$XDG_DATA_HOME/afik` or `~/.local/share/afik`.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(std::env::var_os("XDG_DATA_HOME"), home())
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
    state_dir_from(std::env::var_os("XDG_STATE_HOME"), home())
}

/// Resolves the state directory from the two environment values, without I/O.
pub fn state_dir_from(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_base(xdg_state_home, home, &[".local", "state"])
}

/// Returns the AFIK cache directory, `$XDG_CACHE_HOME/afik` or `~/.cache/afik`.
///
/// What is kept here can be fetched again, such as the satellite list.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_from(std::env::var_os("XDG_CACHE_HOME"), home())
}

/// Resolves the cache directory from the two environment values, without I/O.
pub fn cache_dir_from(xdg_cache_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_base(xdg_cache_home, home, &[".cache"])
}

/// The home directory. The browser has none, so its files sit under a fixed
/// root of the page's storage (see [`crate::files`]).
fn home() -> Option<OsString> {
    if cfg!(target_arch = "wasm32") {
        Some(OsString::from("/"))
    } else {
        std::env::var_os("HOME")
    }
}

/// Picks an absolute XDG base, or the given fallback under an absolute home.
fn xdg_base(xdg: Option<OsString>, home: Option<OsString>, fallback: &[&str]) -> Option<PathBuf> {
    // `Path::is_absolute` is never true in the browser, where `/` is the
    // root of the page's storage.
    let absolute =
        |path: &PathBuf| path.is_absolute() || cfg!(target_arch = "wasm32") && path.has_root();
    let base = xdg.map(PathBuf::from).filter(absolute).or_else(|| {
        home.map(PathBuf::from)
            .filter(absolute)
            .map(|home| fallback.iter().fold(home, |path, part| path.join(part)))
    })?;
    Some(base.join(APPLICATION_DIR))
}

/// Returns the directory projects are kept in, `$XDG_DATA_HOME/afik/projects`.
pub fn projects_dir() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join(PROJECTS_DIR))
}

/// Returns the file naming the project opened last, under the state directory.
pub fn last_project_file() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join(LAST_PROJECT_FILE))
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

/// Returns the directory one radio family's EEPROM backups are written to.
pub fn family_eeprom_dir(family: &str) -> Option<PathBuf> {
    eeprom_dir().map(|dir| dir.join(family))
}

/// Returns a fresh default backup path in one family's backup directory.
pub fn default_family_eeprom_backup(family: &str) -> PathBuf {
    family_eeprom_dir(family)
        .unwrap_or_default()
        .join(eeprom_backup_name(unix_now()))
}

/// Returns the newest backup kept for one family.
///
/// Backups taken before they were filed by family sit directly in the EEPROM
/// directory; `legacy` says whether those count for this family.
pub fn newest_family_eeprom_backup(family: &str, legacy: bool) -> Option<PathBuf> {
    newest_family_backup_in(&eeprom_dir()?, family, legacy)
}

/// [`newest_family_eeprom_backup`] under an explicit EEPROM directory.
pub fn newest_family_backup_in(eeprom: &Path, family: &str, legacy: bool) -> Option<PathBuf> {
    let filed = newest_eeprom_backup(&eeprom.join(family));
    let unfiled = legacy.then(|| newest_eeprom_backup(eeprom)).flatten();
    // Names carry their time, so the later name is the newer backup.
    match (filed, unfiled) {
        (Some(filed), Some(unfiled)) => Some(if file_name(&unfiled) > file_name(&filed) {
            unfiled
        } else {
            filed
        }),
        (filed, unfiled) => filed.or(unfiled),
    }
}

fn file_name(path: &Path) -> Option<&std::ffi::OsStr> {
    path.file_name()
}

/// Returns the places a packaged image named `name` is looked for, in order.
///
/// The workspace build output comes first, because that is where
/// `tool/build-*.sh --release` leaves a fresh image; `$XDG_DATA_HOME/afik/images`
/// holds images kept outside a checkout. The browser has no checkout: its
/// packaged images are fetched from beside the page into the data directory.
pub fn packaged_image_candidates(name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if !cfg!(target_arch = "wasm32") {
        candidates.push(normalise(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/thumbv6m-none-eabi/release")
                .join(name),
        ));
    }
    if let Some(data) = data_dir() {
        candidates.push(data.join("images").join(name));
    }
    candidates
}

/// Removes `.` and `..` components without touching the filesystem.
///
/// The path is shown to the operator and logged, and a workspace path reads
/// better as `afik/target/...` than as `afik/crates/x/../../target/...`.
pub fn normalise(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normal.pop() {
                    normal.push(component);
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// Returns the first packaged image which exists, or the first place to put one.
pub fn packaged_image(name: &str) -> PathBuf {
    let candidates = packaged_image_candidates(name);
    candidates
        .iter()
        .find(|path| crate::files::is_file(path))
        .or_else(|| candidates.first())
        .cloned()
        .unwrap_or_default()
}

/// Returns the newest EEPROM backup in `directory`, if there is one.
///
/// Backup names sort by time, so the newest is the last by name.
pub fn newest_eeprom_backup(directory: &Path) -> Option<PathBuf> {
    crate::files::list(directory)
        .into_iter()
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "bin")
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
    // `std`'s clock panics in the browser.
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
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
        cache_dir_from, data_dir_from, eeprom_backup_name, newest_eeprom_backup, state_dir_from,
        utc_timestamp,
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
        assert_eq!(
            cache_dir_from(None, Some(os("/home/op"))),
            Some(PathBuf::from("/home/op/.cache/afik"))
        );
        assert_eq!(
            cache_dir_from(Some(os("/cache")), Some(os("/home/op"))),
            Some(PathBuf::from("/cache/afik"))
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
    fn family_backups_include_unfiled_ones_only_when_asked() {
        let eeprom = std::env::temp_dir().join(format!("afik-family-{}", std::process::id()));
        std::fs::create_dir_all(eeprom.join("k5")).unwrap();
        std::fs::write(eeprom.join("eeprom-20260101T000000Z.bin"), [0_u8]).unwrap();
        assert_eq!(
            super::newest_family_backup_in(&eeprom, "k5", true),
            Some(eeprom.join("eeprom-20260101T000000Z.bin"))
        );
        assert_eq!(super::newest_family_backup_in(&eeprom, "k1", false), None);
        std::fs::write(
            eeprom.join("k5").join("eeprom-20250101T000000Z.bin"),
            [0_u8],
        )
        .unwrap();
        // The unfiled one is newer, so it still wins.
        assert_eq!(
            super::newest_family_backup_in(&eeprom, "k5", true),
            Some(eeprom.join("eeprom-20260101T000000Z.bin"))
        );
        std::fs::write(
            eeprom.join("k5").join("eeprom-20270101T000000Z.bin"),
            [0_u8],
        )
        .unwrap();
        assert_eq!(
            super::newest_family_backup_in(&eeprom, "k5", true),
            Some(eeprom.join("k5").join("eeprom-20270101T000000Z.bin"))
        );
        std::fs::remove_dir_all(&eeprom).unwrap();
    }

    #[test]
    fn paths_are_normalised_for_display() {
        assert_eq!(
            super::normalise(std::path::Path::new("/a/crates/gui/../../target/./x.raw")),
            PathBuf::from("/a/target/x.raw")
        );
        assert_eq!(
            super::normalise(std::path::Path::new("../x")),
            PathBuf::from("../x")
        );
    }

    #[test]
    fn packaged_images_are_looked_for_in_the_build_output_first() {
        let candidates = super::packaged_image_candidates("afik-k5-v1.raw");
        assert!(candidates[0].ends_with("target/thumbv6m-none-eabi/release/afik-k5-v1.raw"));
        assert!(!candidates[0].to_string_lossy().contains(".."));
        if let Some(data) = super::data_dir() {
            assert_eq!(candidates[1], data.join("images").join("afik-k5-v1.raw"));
        }
        // With nothing on disk the first candidate is where one would go.
        assert_eq!(
            super::packaged_image("definitely-not-built.raw"),
            super::packaged_image_candidates("definitely-not-built.raw")[0]
        );
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
