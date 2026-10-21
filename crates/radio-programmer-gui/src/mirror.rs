//! The datasets the Pages build mirrors beside the browser Studio
//! (`ADR-121`).
//!
//! Most of the servers the map's datasets come from do not allow a page on
//! another origin to read them, so the browser cannot sync from them. The
//! Pages build instead syncs every dataset with the desktop code, into
//! `datasets/` beside the page, each in the directory and files of its
//! desktop cache, with a `synced` file holding when, in seconds since 1970.
//! A sync in the browser reads that mirror.

/// The directory beside the page the datasets are mirrored into.
pub const DIR: &str = "datasets";

/// The file in each dataset's directory saying when it was synced.
pub const SYNCED_FILE: &str = "synced";

/// The largest mirrored file the browser takes, in bytes.
#[cfg(target_arch = "wasm32")]
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Reads the [`SYNCED_FILE`] text: seconds since 1970.
pub fn parse_synced(text: &str) -> Result<std::time::SystemTime, String> {
    text.trim()
        .parse()
        .map(|seconds| std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .map_err(|error| format!("{SYNCED_FILE}: {error}"))
}

/// Syncs every dataset into its own directory under `dir`, stamping each
/// that synced; returns each dataset's name and outcome, with how many
/// entries it read.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_all(dir: &std::path::Path) -> Vec<(&'static str, Result<usize, String>)> {
    use crate::{airports, marine, repeaters, satellites, stations, world};

    let into = |name: &str| dir.join(name);
    let outcomes = [
        (
            repeaters::CACHE_DIR,
            repeaters::RepeaterCache::in_dir(into(repeaters::CACHE_DIR))
                .sync()
                .map(|list| list.repeaters.len() + list.gateways.len() + list.beacons.len()),
        ),
        (
            world::CACHE_DIR,
            world::WorldCache::in_dir(into(world::CACHE_DIR))
                .sync()
                .map(|list| list.repeaters.len()),
        ),
        (
            stations::CACHE_DIR,
            stations::StationCache::in_dir(into(stations::CACHE_DIR))
                .sync()
                .map(|list| list.stations.len()),
        ),
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
            (name, stamped)
        })
        .collect()
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

    use super::parse_synced;

    #[test]
    fn the_synced_stamp_is_seconds_since_1970_and_nothing_else() {
        assert_eq!(
            parse_synced("1761072000\n").unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_761_072_000)
        );
        assert!(parse_synced("yesterday").is_err());
        assert!(parse_synced("").is_err());
    }
}
