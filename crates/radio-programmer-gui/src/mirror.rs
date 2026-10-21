//! The datasets the Pages build mirrors beside the browser Studio
//! (`ADR-121`).
//!
//! Most of the servers the map's datasets come from do not allow a page on
//! another origin to read them, so the browser cannot sync from them. The
//! Pages build instead syncs every dataset with the desktop code, into
//! `datasets/` beside the page, each in the directory and files of its
//! desktop cache, with a `synced` file holding when, in seconds since 1970.
//! A sync in the browser reads that mirror.
//!
//! Some sources refuse the build's runners (Ofcom answers 403), so a dataset
//! whose fetch fails is taken from a snapshot kept in the repository under
//! [`SNAPSHOT_DIR`], laid out as the mirror is, when there is one.

/// The directory beside the page the datasets are mirrored into.
pub const DIR: &str = "datasets";

/// The file in each dataset's directory saying when it was synced.
pub const SYNCED_FILE: &str = "synced";

/// The largest mirrored file the browser takes, in bytes.
#[cfg(target_arch = "wasm32")]
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Snapshots of the datasets some build runners cannot fetch, in the
/// mirror's layout.
#[cfg(not(target_arch = "wasm32"))]
pub const SNAPSHOT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/snapshots");

/// How a dataset reached the mirror.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq, Eq)]
pub enum Mirrored {
    /// Fetched just now, with how many entries it read.
    Fetched(usize),
    /// Copied from its snapshot, since the fetch failed for this reason.
    Snapshot(String),
}

/// Reads the [`SYNCED_FILE`] text: seconds since 1970.
pub fn parse_synced(text: &str) -> Result<std::time::SystemTime, String> {
    text.trim()
        .parse()
        .map(|seconds| std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .map_err(|error| format!("{SYNCED_FILE}: {error}"))
}

/// Syncs every dataset into its own directory under `dir`, stamping each
/// that synced, and takes one that does not from its snapshot under
/// `snapshots`; returns each dataset's name and outcome.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_all(
    dir: &std::path::Path,
    snapshots: &std::path::Path,
) -> Vec<(&'static str, Result<Mirrored, String>)> {
    use crate::{airports, marine, repeaters, satellites, stations, world};

    let into = |name: &str| dir.join(name);
    let repeaters_from = |source: world::Source| {
        (
            source.cache_dir(),
            world::WorldCache::in_dir(source, into(source.cache_dir()))
                .sync()
                .map(|list| list.repeaters.len()),
        )
    };
    let stations_from = |source: stations::Source| {
        (
            source.cache_dir(),
            stations::StationCache::in_dir(source, into(source.cache_dir()))
                .sync()
                .map(|list| list.stations.len()),
        )
    };
    let outcomes = [
        (
            repeaters::CACHE_DIR,
            repeaters::RepeaterCache::in_dir(into(repeaters::CACHE_DIR))
                .sync()
                .map(|list| list.repeaters.len() + list.gateways.len() + list.beacons.len()),
        ),
        repeaters_from(world::Source::Hearham),
        repeaters_from(world::Source::Iarc),
        // Each FM source is its own dataset, so one refused takes its own
        // snapshot.
        stations_from(stations::Source::Ofcom),
        stations_from(stations::Source::Fcc),
        stations_from(stations::Source::Ised),
        (
            airports::CACHE_DIR,
            airports::AirportCache::in_dir(into(airports::CACHE_DIR))
                .sync()
                .map(|list| list.frequencies.len()),
        ),
        (
            marine::CACHE_DIR,
            marine::MarineCache::in_dir(into(marine::CACHE_DIR))
                .sync()
                .map(|list| list.places.len()),
        ),
        (
            satellites::CACHE_DIR,
            satellites::SatelliteCache::in_dir(into(satellites::CACHE_DIR))
                .sync()
                .map(|list| list.channels.len()),
        ),
    ];
    outcomes
        .into_iter()
        .map(|(name, outcome)| {
            let stamped = outcome.and_then(|count| stamp(&into(name)).map(|()| count));
            let mirrored = match stamped {
                Ok(count) => Ok(Mirrored::Fetched(count)),
                Err(error) => copy_snapshot(&snapshots.join(name), &into(name))
                    .map(|()| Mirrored::Snapshot(error.clone()))
                    .map_err(|missing| format!("{error}; {missing}")),
            };
            (name, mirrored)
        })
        .collect()
}

/// Copies the files of the snapshot in `from` into `to`; there must be one,
/// stamped with when it was taken.
#[cfg(not(target_arch = "wasm32"))]
fn copy_snapshot(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    let stamp = from.join(SYNCED_FILE);
    let synced = std::fs::read_to_string(&stamp)
        .map_err(|error| format!("no snapshot ({}: {error})", stamp.display()))?;
    parse_synced(&synced)?;
    let failed = |error: std::io::Error| format!("{}: {error}", from.display());
    std::fs::create_dir_all(to).map_err(failed)?;
    for entry in std::fs::read_dir(from).map_err(failed)? {
        let path = entry.map_err(failed)?.path();
        if let Some(name) = path.file_name() {
            std::fs::copy(&path, to.join(name)).map_err(failed)?;
        }
    }
    Ok(())
}

/// Writes the [`SYNCED_FILE`] for a dataset synced into `dir` just now.
#[cfg(not(target_arch = "wasm32"))]
fn stamp(dir: &std::path::Path) -> Result<(), String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(SYNCED_FILE);
    std::fs::write(&path, format!("{seconds}\n"))
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Fetches the mirrored `files` of the dataset `name` from beside the page,
/// in order, and when they were synced.
#[cfg(target_arch = "wasm32")]
pub async fn fetch<const N: usize>(
    name: &str,
    files: [&str; N],
) -> Result<([String; N], std::time::SystemTime), String> {
    let text = |file: &str| {
        let url = format!("{DIR}/{name}/{file}");
        async move {
            crate::net::fetch(&url, MAX_FILE_BYTES)
                .await
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("the Studio's mirror has no {name} list ({error})"))
        }
    };
    let synced = parse_synced(&text(SYNCED_FILE).await?)?;
    let mut texts = [const { String::new() }; N];
    for (slot, file) in texts.iter_mut().zip(files) {
        *slot = text(file).await?;
    }
    Ok((texts, synced))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{copy_snapshot, parse_synced, SNAPSHOT_DIR, SYNCED_FILE};

    #[test]
    fn the_synced_stamp_is_seconds_since_1970_and_nothing_else() {
        assert_eq!(
            parse_synced("1761072000\n").unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_761_072_000)
        );
        assert!(parse_synced("yesterday").is_err());
        assert!(parse_synced("").is_err());
    }

    #[test]
    fn the_ofcom_snapshot_is_a_list_the_studio_reads() {
        let ofcom = crate::stations::Source::Ofcom;
        let dir = std::path::Path::new(SNAPSHOT_DIR).join(ofcom.cache_dir());
        parse_synced(&std::fs::read_to_string(dir.join(SYNCED_FILE)).unwrap()).unwrap();
        let list = crate::stations::StationCache::in_dir(ofcom, dir)
            .load()
            .unwrap()
            .unwrap();
        assert!(list.stations.len() > 1_000, "{}", list.stations.len());
    }

    #[test]
    fn a_failed_dataset_takes_its_snapshot_and_one_without_fails() {
        let root = std::env::temp_dir().join(format!("afik-mirror-{}", std::process::id()));
        let (snapshot, out) = (root.join("snapshot"), root.join("out"));
        std::fs::create_dir_all(&snapshot).unwrap();
        let unstamped = copy_snapshot(&snapshot, &out);
        std::fs::write(snapshot.join(SYNCED_FILE), "1761072000\n").unwrap();
        std::fs::write(snapshot.join("list.csv"), "a,b\n").unwrap();
        let copied = copy_snapshot(&snapshot, &out);
        let list = std::fs::read_to_string(out.join("list.csv"));
        std::fs::remove_dir_all(&root).unwrap();
        assert!(unstamped.unwrap_err().starts_with("no snapshot"));
        copied.unwrap();
        assert_eq!(list.unwrap(), "a,b\n");
    }
}
