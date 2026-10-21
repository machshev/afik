//! FM repeaters outside the UK, synced from hearham's worldwide repeater
//! list and cached; the Repeaters tab takes them with the ETCC's.
//!
//! As with the ETCC list, the studio fetches only when the operator presses
//! Sync or nothing is cached (`ADR-121`); otherwise it reads the raw response
//! cached in `$XDG_CACHE_HOME/afik/hearham`.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::SystemTime,
};

use radio_channel_plan::MAX_CHANNEL_NAME_LEN;
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use crate::repeaters::fetch_within;
use crate::{
    paths,
    repeaters::{ctcss, receivable_hz, sanitise, Repeater},
};

/// Every repeater hearham lists, worldwide: about 23 000, 9.5 MB.
pub const REPEATERS_URL: &str = "https://hearham.com/api/repeaters/v1";

/// The callsign prefixes the ITU gives the UK, which every ETCC system has.
/// hearham's UK entries are an old copy of ukrepeater.net and IRLP's list;
/// the ETCC list the Repeaters tab reads for the UK is the current one.
const UK_PREFIXES: [char; 5] = ['G', 'M', '2', 'g', 'm'];

pub(crate) const CACHE_DIR: &str = "hearham";
const CACHE_FILE: &str = "repeaters.json";
/// Long enough for the whole list on a slow link.
#[cfg(not(target_arch = "wasm32"))]
const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Picks the analogue FM repeaters out of hearham's list.
///
/// A repeater is taken when it is operational, its mode is FM or a mix with
/// FM (such as `YSF/FM`), it has an offset, and the radio can receive both
/// its output and its input, the output plus the offset. hearham lists
/// gateways, APRS digipeaters and iGates as FM too; they are simplex. UK
/// callsigns are left to the ETCC list. Its `encode` tone, the one a
/// user sends, is the access tone. hearham gives no bandwidth, so every
/// channel is 25 kHz, as most repeaters outside Europe are.
pub fn parse(json: &str) -> Result<Vec<Repeater>, String> {
    let systems = match serde_json::from_str(json) {
        Ok(Value::Array(systems)) => systems,
        Ok(_) => return Err("hearham repeaters: expected a list".to_owned()),
        Err(error) => return Err(format!("hearham repeaters: {error}")),
    };
    let mut repeaters: Vec<Repeater> = systems
        .iter()
        .filter(|system| system.get("operational").and_then(Value::as_i64) == Some(1))
        .filter(|system| {
            !system
                .get("callsign")
                .and_then(Value::as_str)
                .is_some_and(|callsign| callsign.trim_start().starts_with(UK_PREFIXES))
        })
        .filter(|system| {
            system
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode.split('/').any(|part| part.trim() == "FM"))
        })
        .filter_map(|system| {
            let output = system.get("frequency").and_then(Value::as_i64)?;
            // A system with no offset is simplex: a gateway, an APRS
            // digipeater or iGate, or a beacon, not a repeater.
            let offset = system
                .get("offset")
                .and_then(Value::as_i64)
                .filter(|&offset| offset != 0)?;
            let text = |key: &str| system.get(key).and_then(Value::as_str);
            Some(Repeater {
                callsign: sanitise(text("callsign")?, MAX_CHANNEL_NAME_LEN)?,
                town: text("city").unwrap_or_default().trim().to_owned(),
                output_hz: receivable_hz(output)?,
                input_hz: receivable_hz(output.checked_add(offset)?)?,
                ctcss_tenths_hz: text("encode")
                    .and_then(|tone| tone.trim().parse::<f64>().ok())
                    .and_then(ctcss),
                wide: true,
                latitude: system.get("latitude").and_then(Value::as_f64)?,
                longitude: system.get("longitude").and_then(Value::as_f64)?,
                coverage: crate::repeaters::Coverage::Standard,
            })
        })
        .collect();
    if repeaters.is_empty() {
        return Err("hearham: no FM repeaters".to_owned());
    }
    repeaters.sort_by(|a, b| {
        a.callsign
            .cmp(&b.callsign)
            .then(a.output_hz.cmp(&b.output_hz))
    });
    Ok(repeaters)
}

/// The repeaters a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct WorldList {
    /// Every FM repeater, ordered by callsign.
    pub repeaters: Vec<Repeater>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Where the raw hearham response is kept.
#[derive(Clone, Debug)]
pub struct WorldCache {
    dir: Option<PathBuf>,
}

impl WorldCache {
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
    pub fn load(&self) -> Result<Option<WorldList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let path = dir.join(CACHE_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let failed = |error: std::io::Error| format!("{}: {error}", path.display());
        let json = fs::read_to_string(&path).map_err(failed)?;
        let synced = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .map_err(failed)?;
        Ok(Some(WorldList {
            repeaters: parse(&json)?,
            synced,
        }))
    }

    /// Fetches the list, checks it, caches it, and returns what it holds.
    ///
    /// Nothing is written unless the response parses, so a bad fetch keeps
    /// the cache as it was; the write goes through a temporary file and a
    /// rename.
    pub fn sync_with(
        &self,
        fetch: impl Fn(&str) -> Result<String, String>,
    ) -> Result<WorldList, String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        let json = fetch(REPEATERS_URL)?;
        parse(&json)?;
        let path = dir.join(CACHE_FILE);
        let partial = path.with_extension("json.partial");
        fs::create_dir_all(dir)
            .and_then(|()| fs::write(&partial, &json))
            .and_then(|()| fs::rename(&partial, &path))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        self.load()?
            .ok_or_else(|| "the hearham cache was not written".to_owned())
    }

    /// Fetches from hearham, checks and caches what came, and returns it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync(&self) -> Result<WorldList, String> {
        self.sync_with(|url| fetch_within(url, SYNC_TIMEOUT))
    }

    /// Reads what the Pages build mirrored from hearham beside the browser
    /// Studio (`ADR-121`); the page keeps nothing.
    #[cfg(target_arch = "wasm32")]
    pub async fn sync(&self) -> Result<WorldList, String> {
        let ([json], synced) = crate::mirror::fetch(CACHE_DIR, [CACHE_FILE]).await?;
        Ok(WorldList {
            repeaters: parse(&json)?,
            synced,
        })
    }
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<WorldList, String>>,
}

impl SyncJob {
    /// Starts fetching from hearham into `cache`.
    pub fn start(cache: WorldCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        // A worker that cannot start drops `sender`, which `poll` reports.
        let _started = crate::platform::spawn("afik-sync", move || async move {
            // The editor may have closed; then nobody wants the outcome.
            #[cfg(not(target_arch = "wasm32"))]
            let outcome = cache.sync();
            #[cfg(target_arch = "wasm32")]
            let outcome = cache.sync().await;
            let _ = sender.send(outcome);
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<WorldList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the hearham sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use radio_domain::{Bandwidth, Tone};

    use super::{parse, WorldCache, REPEATERS_URL};
    use crate::{
        model::ToneDraft,
        repeaters::{nearby, repeater_bank},
    };

    /// Repeaters in the shape hearham returns, trimmed to the fields used.
    const REPEATERS: &str = r#"[
        {"callsign": "VE7RHS", "latitude": 49.26973, "longitude": -123.24992,
         "city": "Vancouver, BC Canada", "mode": "FM", "encode": "100.0",
         "frequency": 145270000, "offset": -600000, "operational": 1},
        {"callsign": "F1ZXY", "latitude": 48.85, "longitude": 2.35, "city": "Paris, France",
         "mode": "YSF/FM ", "encode": "", "frequency": 430075000, "offset": 9400000,
         "operational": 1},
        {"callsign": "W1DMR", "latitude": 42.36, "longitude": -71.06, "city": "Boston",
         "mode": "DMR", "encode": "CC1", "frequency": 446500000, "offset": -5000000,
         "operational": 1},
        {"callsign": "W1OFF", "latitude": 42.36, "longitude": -71.06, "city": "Boston",
         "mode": "FM", "encode": "88.5", "frequency": 146820000, "offset": -600000,
         "operational": 0},
        {"callsign": "W1TEN", "latitude": 42.36, "longitude": -71.06, "city": "Boston",
         "mode": "FM", "encode": "0", "frequency": 29620000, "offset": -100000,
         "operational": 1},
        {"callsign": "W1TV", "latitude": 42.36, "longitude": -71.06, "city": "Boston",
         "mode": "FM", "encode": "0", "frequency": 1252000000, "offset": -552000000,
         "operational": 1},
        {"callsign": "", "latitude": 42.36, "longitude": -71.06, "city": "Boston",
         "mode": "FM", "encode": "0", "frequency": 146940000, "offset": -600000,
         "operational": 1},
        {"callsign": "MB7UJM", "latitude": 52.3125, "longitude": 0.375,
         "city": "Fordham  (UK)", "mode": "FM", "encode": "0", "frequency": 144800000,
         "offset": 0, "operational": 1},
        {"callsign": "MB7IPH", "latitude": 51.17, "longitude": 0.67, "city": "Headcorn (UK)",
         "mode": "FM", "encode": "0", "frequency": 145237500, "operational": 1},
        {"callsign": "G6OHM", "latitude": 52.48, "longitude": 0.04, "city": "Chatteris (UK)",
         "mode": "FM", "encode": "94.8", "frequency": 438537500, "offset": -7600000,
         "operational": 1},
        {"callsign": "GB3OA", "latitude": 53.65, "longitude": -3.01,
         "city": "Southport, LANCS England", "mode": "FM", "encode": "82.5",
         "frequency": 145612500, "offset": -600000, "operational": 1}
    ]"#;

    #[test]
    fn the_list_keeps_operational_fm_repeaters_the_radio_can_hear() {
        let repeaters = parse(REPEATERS).expect("parses");
        let summary: Vec<_> = repeaters
            .iter()
            .map(|r| {
                (
                    r.callsign.as_str(),
                    r.output_hz,
                    r.input_hz,
                    r.ctcss_tenths_hz,
                )
            })
            .collect();
        // Digital only, not operational, an input the radio cannot hear, no
        // callsign, simplex (an APRS iGate, a gateway) and UK callsigns, left
        // to the ETCC list, are left out.
        assert_eq!(
            summary,
            [
                ("F1ZXY", 430_075_000, 439_475_000, None),
                ("VE7RHS", 145_270_000, 144_670_000, Some(1000)),
                ("W1TEN", 29_620_000, 29_520_000, None),
            ]
        );
        assert!(parse("[]").is_err());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn a_repeater_bank_takes_them_with_their_access_tones() {
        let repeaters = parse(REPEATERS).unwrap();
        // CN89 covers Vancouver.
        let near = nearby(&repeaters, "CN89", 100).unwrap();
        let bank = repeater_bank("cn89", &near);
        assert_eq!(bank.name, "RPT CN89");
        assert_eq!(bank.channels.len(), 1);
        let channel = &bank.channels[0];
        assert_eq!(channel.name, "VE7RHS");
        assert_eq!(channel.receive_mhz, "145.270000");
        assert_eq!(channel.transmit_mhz, "144.670000");
        assert_eq!(channel.bandwidth, Bandwidth::Wide);
        assert_eq!(channel.tx_tone, ToneDraft::from_tone(Tone::Ctcss(1000)));
    }

    #[test]
    fn a_sync_caches_only_a_list_that_parses() {
        let dir = std::env::temp_dir().join(format!("afik-hearham-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = WorldCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none());
        assert!(cache.sync_with(|_| Ok("[]".to_owned())).is_err());
        assert!(cache.load().unwrap().is_none(), "nothing written");
        let list = cache
            .sync_with(|url| {
                assert_eq!(url, REPEATERS_URL);
                Ok(REPEATERS.to_owned())
            })
            .unwrap();
        assert_eq!(list.repeaters.len(), 3);
        assert_eq!(cache.load().unwrap().unwrap().repeaters.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fetches the real hearham list; run with `--ignored` to check the live
    /// response still parses and a Paris bank fits a radio.
    #[test]
    #[ignore = "fetches from hearham.com"]
    fn the_live_hearham_list_parses() {
        let dir = std::env::temp_dir().join(format!("afik-hearham-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let list = WorldCache::in_dir(dir.clone())
            .sync_with(|url| super::fetch_within(url, super::SYNC_TIMEOUT))
            .expect("syncs");
        println!("{} FM repeaters", list.repeaters.len());
        let near = nearby(&list.repeaters, "JN18EU", 50).unwrap();
        for (repeater, km) in &near {
            println!("{km:6.1} km {repeater:?}");
        }
        assert!(near.len() > 5);
        let mut project = crate::model::ProjectModel::new();
        project.banks.push(repeater_bank("JN18EU", &near));
        assert!(project.to_radio_image().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
