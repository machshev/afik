//! The amateur FM satellites, synced from the SatNOGS database and cached.
//!
//! The studio fetches only when the operator presses Sync. Otherwise it reads
//! the cache in `$XDG_CACHE_HOME/afik/satnogs`, and with no cache the built-in
//! satellite bank applies. The raw responses are what is cached, so changing
//! the selection below needs no new fetch.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime},
};

use radio_channel_plan::MAX_CHANNEL_NAME_LEN;
use radio_domain::{MAX_CTCSS_TENTHS_HZ, MIN_CTCSS_TENTHS_HZ};
use serde_json::Value;

use crate::paths;

/// Every transmitter SatNOGS believes alive.
pub const TRANSMITTERS_URL: &str =
    "https://db.satnogs.org/api/transmitters/?format=json&alive=true";
/// Every satellite, for its name and whether it is still in orbit.
pub const SATELLITES_URL: &str = "https://db.satnogs.org/api/satellites/?format=json";

const CACHE_DIR: &str = "satnogs";
const TRANSMITTERS_FILE: &str = "transmitters.json";
const SATELLITES_FILE: &str = "satellites.json";
/// The most synced channels one bank takes; the list is about thirty.
pub const MAX_SATELLITE_CHANNELS: usize = 40;
const TIMEOUT: Duration = Duration::from_secs(30);
/// The responses are a few megabytes; this bounds a misbehaving server.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// Receive coverage the BK4819 datasheet states, in hertz.
const RECEIVE_BANDS: [(u64, u64); 2] = [(18_000_000, 660_000_000), (840_000_000, 1_300_000_000)];

/// One satellite FM channel: hear the downlink, transmit the uplink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SatelliteChannel {
    /// Channel name: the OSCAR designation where there is one.
    pub name: String,
    /// Downlink in hertz.
    pub receive_hz: u32,
    /// Uplink in hertz.
    pub transmit_hz: u32,
    /// CTCSS tone the uplink wants, in tenths of a hertz.
    pub tx_ctcss_tenths_hz: Option<u16>,
}

/// The channels a sync selected, and when the cache was written.
#[derive(Clone, Debug)]
pub struct SatelliteList {
    /// The selected channels, ordered by name.
    pub channels: Vec<SatelliteChannel>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Picks the FM voice satellites from the two SatNOGS responses.
///
/// A transmitter is taken when it is active, FM both ways, both frequencies
/// fall in the radio's receive coverage, and its satellite is in orbit. The
/// uplink tone is read from the description, where SatNOGS records it as
/// `CTCSS 67.0 Hz` or `PL 88.5Hz`. Whether a channel may transmit is still
/// decided by the programmed TX ranges.
pub fn select(transmitters: &str, satellites: &str) -> Result<Vec<SatelliteChannel>, String> {
    let transmitters = parse_array(transmitters, "transmitters")?;
    let satellites = parse_array(satellites, "satellites")?;
    let names: HashMap<&str, String> = satellites
        .iter()
        .filter(|satellite| text(satellite, "status") == Some("in orbit"))
        .filter_map(|satellite| {
            let id = text(satellite, "sat_id")?;
            let name = text(satellite, "names")
                .and_then(designation)
                .or_else(|| text(satellite, "name"))?;
            Some((id, sanitise(name)))
        })
        .collect();

    let mut seen = HashSet::new();
    let mut channels: Vec<SatelliteChannel> = transmitters
        .iter()
        .filter(|tx| tx.get("alive").and_then(Value::as_bool) == Some(true))
        .filter(|tx| text(tx, "status") == Some("active"))
        .filter(|tx| text(tx, "mode") == Some("FM"))
        .filter(|tx| {
            matches!(tx.get("uplink_mode"), None | Some(Value::Null))
                || text(tx, "uplink_mode") == Some("FM")
        })
        .filter_map(|tx| {
            let name = names.get(text(tx, "sat_id")?)?;
            let channel = SatelliteChannel {
                name: name.clone(),
                receive_hz: receivable(tx, "downlink_low")?,
                transmit_hz: receivable(tx, "uplink_low")?,
                tx_ctcss_tenths_hz: text(tx, "description").and_then(ctcss),
            };
            // Several satellites of one constellation share every frequency.
            seen.insert((
                channel.receive_hz,
                channel.transmit_hz,
                channel.tx_ctcss_tenths_hz,
            ))
            .then_some(channel)
        })
        .collect();
    channels.sort_by(|a, b| {
        (&a.name, a.receive_hz, a.transmit_hz).cmp(&(&b.name, b.receive_hz, b.transmit_hz))
    });
    channels.truncate(MAX_SATELLITE_CHANNELS);
    number_repeats(&mut channels);
    Ok(channels)
}

fn parse_array(json: &str, what: &str) -> Result<Vec<Value>, String> {
    match serde_json::from_str(json) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(format!("SatNOGS {what}: expected a list")),
        Err(error) => Err(format!("SatNOGS {what}: {error}")),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A frequency in hertz which the radio can receive.
fn receivable(value: &Value, key: &str) -> Option<u32> {
    let hz = value.get(key).and_then(Value::as_u64)?;
    RECEIVE_BANDS
        .iter()
        .any(|&(low, high)| (low..=high).contains(&hz))
        .then(|| u32::try_from(hz).ok())
        .flatten()
}

/// The OSCAR designation among a satellite's other names, such as `SO-50`,
/// which is what operators call it.
fn designation(names: &str) -> Option<&str> {
    names.split(',').map(str::trim).find(|name| {
        let Some((letters, digits)) = name.split_once('-') else {
            return false;
        };
        (1..=3).contains(&letters.len())
            && letters.bytes().all(|b| b.is_ascii_uppercase())
            && (1..=3).contains(&digits.len())
            && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Upper-case printable ASCII, cut to what a channel name holds.
fn sanitise(name: &str) -> String {
    let mut clean: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    clean = clean.trim().to_owned();
    clean.truncate(MAX_CHANNEL_NAME_LEN);
    let clean = clean.trim_end().to_owned();
    if clean.is_empty() {
        "SAT".to_owned()
    } else {
        clean
    }
}

/// Numbers the second and later channels of one satellite: `ISS`, `ISS 2`.
fn number_repeats(channels: &mut [SatelliteChannel]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for channel in channels {
        let count = counts.entry(channel.name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            let suffix = format!(" {count}");
            let mut base = channel.name.clone();
            base.truncate(MAX_CHANNEL_NAME_LEN - suffix.len());
            channel.name = format!("{}{suffix}", base.trim_end());
        }
    }
}

/// Reads a CTCSS tone from a transmitter description, in tenths of a hertz.
///
/// The description must name the tone (`CTCSS` or `PL`), and one that says
/// there is no CTCSS any longer has none.
fn ctcss(description: &str) -> Option<u16> {
    let lower = description.to_ascii_lowercase();
    if lower.contains("no ctcss") {
        return None;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .filter(|token| !token.is_empty())
        .collect();
    if !tokens
        .iter()
        .any(|&token| token == "ctcss" || token == "pl")
    {
        return None;
    }
    tokens.iter().find_map(|token| {
        let number = token.strip_suffix("hz").unwrap_or(token);
        let (whole, fraction) = number.split_once('.')?;
        let whole: u16 = whole.parse().ok()?;
        let tenth = u16::from(fraction.bytes().next().filter(u8::is_ascii_digit)? - b'0');
        let tenths = whole.checked_mul(10)?.checked_add(tenth)?;
        (MIN_CTCSS_TENTHS_HZ..=MAX_CTCSS_TENTHS_HZ)
            .contains(&tenths)
            .then_some(tenths)
    })
}

/// Where the raw SatNOGS responses are kept.
#[derive(Clone, Debug)]
pub struct SatelliteCache {
    dir: Option<PathBuf>,
}

impl SatelliteCache {
    /// The cache under the XDG cache directory.
    pub fn open() -> Self {
        Self {
            dir: paths::cache_dir().map(|dir| dir.join(CACHE_DIR)),
        }
    }

    /// A cache in an explicit directory.
    pub fn in_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    /// No cache: nothing is read, and a sync is refused.
    pub const fn none() -> Self {
        Self { dir: None }
    }

    /// Returns the cache directory, if there is one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Reads the cached list; `Ok(None)` when nothing has been synced.
    pub fn load(&self) -> Result<Option<SatelliteList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let transmitters_path = dir.join(TRANSMITTERS_FILE);
        if !transmitters_path.exists() {
            return Ok(None);
        }
        let read = |path: &Path| {
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
        };
        let transmitters = read(&transmitters_path)?;
        let satellites = read(&dir.join(SATELLITES_FILE))?;
        let synced = fs::metadata(&transmitters_path)
            .and_then(|meta| meta.modified())
            .map_err(|error| format!("{}: {error}", transmitters_path.display()))?;
        Ok(Some(SatelliteList {
            channels: select(&transmitters, &satellites)?,
            synced,
        }))
    }

    /// Writes both responses, each through a temporary file and a rename, so a
    /// failed write leaves the previous cache whole.
    fn store(&self, transmitters: &str, satellites: &str) -> Result<(), String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        // The satellites go first: the transmitters file's time is the sync time.
        for (name, body) in [
            (SATELLITES_FILE, satellites),
            (TRANSMITTERS_FILE, transmitters),
        ] {
            let path = dir.join(name);
            let partial = path.with_extension("json.partial");
            fs::write(&partial, body)
                .and_then(|()| fs::rename(&partial, &path))
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(())
    }

    /// Fetches both lists, checks them, caches them, and returns the selection.
    ///
    /// Nothing is written unless both responses parse, so a bad fetch keeps
    /// the cache as it was.
    pub fn sync_with(
        &self,
        fetch: impl Fn(&str) -> Result<String, String>,
    ) -> Result<SatelliteList, String> {
        let transmitters = fetch(TRANSMITTERS_URL)?;
        let satellites = fetch(SATELLITES_URL)?;
        select(&transmitters, &satellites)?;
        self.store(&transmitters, &satellites)?;
        self.load()?
            .ok_or_else(|| "the satellite cache was not written".to_owned())
    }
}

/// Fetches one URL over HTTPS.
fn fetch(url: &str) -> Result<String, String> {
    log::debug!("fetching {url}");
    let started = web_time::Instant::now();
    let body = crate::net::get(
        url,
        TIMEOUT,
        MAX_RESPONSE_BYTES,
        concat!("afik-studio/", env!("CARGO_PKG_VERSION")),
    )?;
    let body = String::from_utf8(body).map_err(|error| format!("{url}: {error}"))?;
    log::debug!(
        "fetched {} bytes from {url} in {} ms",
        body.len(),
        started.elapsed().as_millis()
    );
    Ok(body)
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<SatelliteList, String>>,
}

impl SyncJob {
    /// Starts fetching from SatNOGS into `cache`.
    pub fn start(cache: SatelliteCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        // A worker that cannot start drops `sender`, which `poll` reports.
        let _started = crate::platform::spawn("afik-sync", move || async move {
            // The editor may have closed; then nobody wants the outcome.
            let _ = sender.send(cache.sync_with(fetch));
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<SatelliteList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the satellite sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ctcss, select, SatelliteCache, SATELLITES_URL, TRANSMITTERS_URL};

    /// Entries in the shape SatNOGS returns, trimmed to the fields used.
    const TRANSMITTERS: &str = r#"[
        {"sat_id": "ISS", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "Mode V/U FM - Voice Repeater CTCSS 67.0 Hz",
         "downlink_low": 437800000, "uplink_low": 145990000},
        {"sat_id": "ISS", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "Mode V/V FM (crew R1)",
         "downlink_low": 145800000, "uplink_low": 145200000},
        {"sat_id": "SO50", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": null, "description": "Mode V/U FM Voice CTCSS 67.0 Hz",
         "downlink_low": 436795000, "uplink_low": 145850000},
        {"sat_id": "IO86", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "V/U FM Voice (PL 88.5Hz)",
         "downlink_low": 435880000, "uplink_low": 145880000},
        {"sat_id": "AO91", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "Mode U/V FM Voice (no CTCSS any longer) 67.0",
         "downlink_low": 145960000, "uplink_low": 435250000},
        {"sat_id": "TEVEL1", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "Mode V/U - FM Transponder",
         "downlink_low": 436400000, "uplink_low": 145970000},
        {"sat_id": "TEVEL2", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "Mode V/U - FM Transponder",
         "downlink_low": 436400000, "uplink_low": 145970000},
        {"sat_id": "QO100", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "digimodes",
         "downlink_low": 10489540000, "uplink_low": 2400040000},
        {"sat_id": "ISS", "alive": true, "status": "active", "mode": "AFSK",
         "uplink_mode": "AFSK", "description": "APRS",
         "downlink_low": 145825000, "uplink_low": 145825000},
        {"sat_id": "HADES", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "USB", "description": "USB/FM repeater",
         "downlink_low": 436665000, "uplink_low": 29472000},
        {"sat_id": "FUTURE", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "FM",
         "downlink_low": 435596000, "uplink_low": 145921000},
        {"sat_id": "SO50", "alive": true, "status": "active", "mode": "FM",
         "uplink_mode": "FM", "description": "beacon only",
         "downlink_low": 436795000, "uplink_low": null}
    ]"#;

    const SATELLITES: &str = r#"[
        {"sat_id": "ISS", "name": "ISS", "names": "ZARYA, RS0ISS, NA1SS", "status": "in orbit"},
        {"sat_id": "SO50", "name": "SAUDISAT 1C", "names": "SO-50", "status": "in orbit"},
        {"sat_id": "IO86", "name": "LAPAN-A2", "names": "ORARI, IO-86, YB0X", "status": "in orbit"},
        {"sat_id": "AO91", "name": "FOX-1B", "names": "AO-91, RADFXSAT", "status": "in orbit"},
        {"sat_id": "TEVEL1", "name": "TEVEL2-1", "names": "", "status": "in orbit"},
        {"sat_id": "TEVEL2", "name": "TEVEL2-2", "names": "", "status": "in orbit"},
        {"sat_id": "QO100", "name": "Es'hail 2", "names": "QO-100", "status": "in orbit"},
        {"sat_id": "HADES", "name": "HADES-L", "names": "", "status": "in orbit"},
        {"sat_id": "FUTURE", "name": "CAS-11", "names": "", "status": "future"}
    ]"#;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-satnogs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn fake(url: &str) -> Result<String, String> {
        match url {
            TRANSMITTERS_URL => Ok(TRANSMITTERS.to_owned()),
            SATELLITES_URL => Ok(SATELLITES.to_owned()),
            _ => Err(format!("unexpected {url}")),
        }
    }

    #[test]
    fn the_selection_keeps_fm_voice_satellites_the_radio_can_hear() {
        let channels = select(TRANSMITTERS, SATELLITES).expect("parses");
        let summary: Vec<_> = channels
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.receive_hz,
                    c.transmit_hz,
                    c.tx_ctcss_tenths_hz,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("AO-91", 145_960_000, 435_250_000, None),
                ("IO-86", 435_880_000, 145_880_000, Some(885)),
                ("ISS", 145_800_000, 145_200_000, None),
                ("ISS 2", 437_800_000, 145_990_000, Some(670)),
                ("SO-50", 436_795_000, 145_850_000, Some(670)),
                ("TEVEL2-1", 436_400_000, 145_970_000, None),
            ]
        );
    }

    #[test]
    fn a_tone_is_read_only_where_the_description_names_one() {
        assert_eq!(ctcss("Mode V/U FM 67.0 PL"), Some(670));
        assert_eq!(ctcss("U/V FM CTCSS 67.0Hz"), Some(670));
        assert_eq!(ctcss("FM VOICE 145.900"), None);
        assert_eq!(ctcss("CTCSS 300.0 Hz"), None, "outside the CTCSS range");
        assert_eq!(ctcss("(no CTCSS any longer) 67.0"), None);
    }

    #[test]
    fn a_response_that_is_not_a_list_is_refused() {
        assert!(select("{\"detail\": \"throttled\"}", SATELLITES).is_err());
        assert!(select(TRANSMITTERS, "not json").is_err());
    }

    #[test]
    fn a_sync_caches_both_responses_and_a_later_load_reads_them_offline() {
        let dir = scratch("sync");
        let cache = SatelliteCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none(), "nothing synced yet");

        let synced = cache.sync_with(fake).expect("syncs");
        assert_eq!(synced.channels.len(), 6);
        assert!(dir.join("transmitters.json").exists());
        assert!(dir.join("satellites.json").exists());

        // A fresh cache over the same directory reads without fetching.
        let loaded = SatelliteCache::in_dir(dir.clone()).load().unwrap().unwrap();
        assert_eq!(loaded.channels, synced.channels);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_or_malformed_fetch_keeps_the_previous_cache() {
        let dir = scratch("keep");
        let cache = SatelliteCache::in_dir(dir.clone());
        cache.sync_with(fake).expect("first sync");

        let offline = cache.sync_with(|_| Err("offline".to_owned()));
        assert_eq!(offline.unwrap_err(), "offline");
        let garbage = cache.sync_with(|url| {
            if url == SATELLITES_URL {
                Ok("<html>".to_owned())
            } else {
                fake(url)
            }
        });
        assert!(garbage.is_err());
        assert_eq!(cache.load().unwrap().unwrap().channels.len(), 6);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Fetches from the real SatNOGS; run with `--ignored` to check the live
    /// responses still parse.
    #[test]
    #[ignore = "fetches from db.satnogs.org"]
    fn the_live_satnogs_lists_parse() {
        let dir = scratch("live");
        let list = SatelliteCache::in_dir(dir.clone())
            .sync_with(super::fetch)
            .expect("syncs");
        for channel in &list.channels {
            println!("{channel:?}");
        }
        assert!(list.channels.iter().any(|c| c.name.starts_with("ISS")));
        let mut project = crate::model::ProjectModel::new();
        project
            .banks
            .push(crate::presets::satellite_bank(&list.channels));
        assert!(project.validate().is_ok(), "the synced bank validates");
        project
            .to_radio_image()
            .expect("the synced bank fits a radio");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_a_cache_directory_nothing_loads_and_a_sync_is_refused() {
        let cache = SatelliteCache::none();
        assert!(cache.load().unwrap().is_none());
        assert!(cache.sync_with(fake).is_err());
    }
}
