//! UK amateur FM repeaters near a Maidenhead locator, synced from the RSGB
//! ETCC repeater list and cached.
//!
//! As with the satellites, the studio fetches only when the operator presses
//! Sync; otherwise it reads the raw response cached in
//! `$XDG_CACHE_HOME/afik/etcc`. The operator's locator and radius are kept in
//! `$XDG_STATE_HOME/afik/repeater-area`, so the next start offers them again.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};

use radio_channel_plan::{SharedSettings, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN};
use radio_domain::{
    Bandwidth, Modulation, PowerLevel, Tone, MAX_CTCSS_TENTHS_HZ, MIN_CTCSS_TENTHS_HZ,
};
use serde_json::Value;

use crate::{
    model::{format_mhz, BankDraft, BankKind, ChannelDraft, ToneDraft},
    paths,
};

/// Every repeater, gateway and beacon the ETCC has licensed.
pub const SYSTEMS_URL: &str = "https://api-beta.rsgb.online/all/systems";

const CACHE_DIR: &str = "etcc";
const SYSTEMS_FILE: &str = "systems.json";
const AREA_FILE: &str = "repeater-area";
/// The most repeaters one bank takes, nearest first: about a quarter of the
/// radio's image.
pub const MAX_REPEATER_CHANNELS: usize = 40;
/// The radius offered before the operator chooses one, in kilometres.
pub const DEFAULT_RADIUS_KM: u16 = 50;
/// The radii the studio offers, in kilometres: from a handheld's reach of
/// a hilltop repeater to a mobile's on a good day.
pub const RADII_KM: [u16; 8] = [10, 20, 30, 50, 75, 100, 150, 200];
/// The largest radius accepted; the whole UK is within it of anywhere in it.
pub const MAX_RADIUS_KM: u16 = 1_500;
const TIMEOUT: Duration = Duration::from_secs(30);
/// The response is about half a megabyte; this bounds a misbehaving server.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// Receive coverage the BK4819 datasheet states, in hertz.
const RECEIVE_BANDS: [(u64, u64); 2] = [(18_000_000, 660_000_000), (840_000_000, 1_300_000_000)];
/// ETCC system types which are repeaters: analogue voice, multimode (analogue
/// and digital), and linked. Gateways are single-frequency internet nodes.
const REPEATER_TYPES: [&str; 3] = ["AV", "DM", "RL"];
/// Mean Earth radius in kilometres.
const EARTH_RADIUS_KM: f64 = 6_371.0;

/// One repeater: hear its output, transmit on its input.
#[derive(Clone, Debug, PartialEq)]
pub struct Repeater {
    /// Callsign, such as `GB3WL`.
    pub callsign: String,
    /// Town the ETCC records it at.
    pub town: String,
    /// Output in hertz, which the radio receives.
    pub output_hz: u32,
    /// Input in hertz, which the radio transmits on.
    pub input_hz: u32,
    /// CTCSS tone the input wants, in tenths of a hertz.
    pub ctcss_tenths_hz: Option<u16>,
    /// A 25 kHz channel rather than 12.5 kHz.
    pub wide: bool,
    /// Latitude of its locator square's centre, in degrees north.
    pub latitude: f64,
    /// Longitude of its locator square's centre, in degrees east.
    pub longitude: f64,
}

/// Picks the FM repeaters out of the ETCC systems response.
///
/// A system is taken when it is a repeater type, operational (or at reduced
/// output), carries analogue FM, and the radio can receive both its output and
/// its input. Whether a channel may transmit is still decided by the
/// programmed TX ranges.
pub fn parse(json: &str) -> Result<Vec<Repeater>, String> {
    let systems = match serde_json::from_str(json) {
        Ok(Value::Object(mut body)) => match body.remove("data") {
            Some(Value::Array(systems)) => systems,
            _ => return Err("ETCC systems: expected a data list".to_owned()),
        },
        Ok(_) => return Err("ETCC systems: expected an object".to_owned()),
        Err(error) => return Err(format!("ETCC systems: {error}")),
    };
    let mut repeaters: Vec<Repeater> = systems
        .iter()
        .filter(|system| REPEATER_TYPES.contains(&text(system, "type").unwrap_or_default()))
        .filter(|system| {
            matches!(
                text(system, "status"),
                Some("OPERATIONAL" | "REDUCED OUTPUT")
            )
        })
        .filter(|system| {
            system
                .get("modeCodes")
                .and_then(Value::as_array)
                .is_some_and(|modes| modes.iter().any(|mode| mode.as_str() == Some("A")))
        })
        .filter_map(|system| {
            let (latitude, longitude) = locator_centre(text(system, "locator")?).ok()?;
            Some(Repeater {
                callsign: sanitise(text(system, "repeater")?, MAX_CHANNEL_NAME_LEN)?,
                town: text(system, "town").unwrap_or_default().to_owned(),
                output_hz: receivable(system, "tx")?,
                input_hz: receivable(system, "rx")?,
                ctcss_tenths_hz: system.get("ctcss").and_then(Value::as_f64).and_then(ctcss),
                wide: system.get("txbw").and_then(Value::as_f64) == Some(25.0),
                latitude,
                longitude,
            })
        })
        .collect();
    repeaters.sort_by(|a, b| a.callsign.cmp(&b.callsign));
    Ok(repeaters)
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

/// A CTCSS tone in hertz as tenths, or none where the ETCC records zero.
// Checked against the CTCSS range first, so the cast is exact.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ctcss(hz: f64) -> Option<u16> {
    let tenths = (hz * 10.0).round();
    (f64::from(MIN_CTCSS_TENTHS_HZ)..=f64::from(MAX_CTCSS_TENTHS_HZ))
        .contains(&tenths)
        .then_some(tenths as u16)
}

/// Upper-case printable ASCII no longer than `max`, or none if nothing is left.
fn sanitise(name: &str, max: usize) -> Option<String> {
    let mut clean: String = name
        .trim()
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    clean.truncate(max);
    let clean = clean.trim_end();
    (!clean.is_empty()).then(|| clean.to_owned())
}

/// The centre of a Maidenhead square of 2, 4, 6 or 8 characters, as latitude
/// and longitude in degrees.
pub fn locator_centre(locator: &str) -> Result<(f64, f64), String> {
    let bytes = locator.trim().as_bytes();
    if !matches!(bytes.len(), 2 | 4 | 6 | 8) {
        return Err(format!(
            "{locator:?} is not a 2, 4, 6 or 8 character locator"
        ));
    }
    // Each pair's base character, number of divisions, and size in degrees
    // of longitude; latitude is half.
    let pairs: [(u8, u8, f64); 4] = [
        (b'A', 18, 20.0),
        (b'0', 10, 2.0),
        (b'A', 24, 2.0 / 24.0),
        (b'0', 10, 2.0 / 240.0),
    ];
    let (mut longitude, mut latitude) = (-180.0, -90.0);
    let mut size = 0.0;
    for (pair, &(base, divisions, degrees)) in bytes.chunks(2).zip(&pairs) {
        let digit = |byte: u8| {
            let value = byte.to_ascii_uppercase().wrapping_sub(base);
            (value < divisions)
                .then_some(f64::from(value))
                .ok_or_else(|| format!("{locator:?} is not a Maidenhead locator"))
        };
        longitude += digit(pair[0])? * degrees;
        latitude += digit(pair[1])? * degrees / 2.0;
        size = degrees;
    }
    Ok((latitude + size / 4.0, longitude + size / 2.0))
}

/// Great-circle distance between two points in degrees, in kilometres.
pub fn distance_km(from: (f64, f64), to: (f64, f64)) -> f64 {
    let (lat1, lon1) = (from.0.to_radians(), from.1.to_radians());
    let (lat2, lon2) = (to.0.to_radians(), to.1.to_radians());
    let a = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

/// The repeaters within `radius_km` of `locator`, nearest first, at most
/// [`MAX_REPEATER_CHANNELS`] of them, each with its distance in kilometres.
pub fn nearby<'a>(
    repeaters: &'a [Repeater],
    locator: &str,
    radius_km: u16,
) -> Result<Vec<(&'a Repeater, f64)>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<_> = repeaters
        .iter()
        .map(|repeater| {
            let km = distance_km(home, (repeater.latitude, repeater.longitude));
            (repeater, km)
        })
        .filter(|&(_, km)| km <= f64::from(radius_km))
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.callsign.cmp(&b.0.callsign)));
    near.truncate(MAX_REPEATER_CHANNELS);
    Ok(near)
}

/// Builds the bank for the repeaters `nearby` chose, in their order: each
/// channel receives the output and transmits on the input with the access
/// tone, and keeps its own bandwidth.
pub fn repeater_bank(locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
    let mut name = format!("RPT {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
    let channels = near
        .iter()
        .map(|(repeater, _)| ChannelDraft {
            name: repeater.callsign.clone(),
            receive_mhz: format_mhz(repeater.output_hz),
            transmit_mhz: format_mhz(repeater.input_hz),
            tx_tone: repeater
                .ctcss_tenths_hz
                .map_or_else(ToneDraft::default, |tenths| {
                    ToneDraft::from_tone(Tone::Ctcss(tenths))
                }),
            modulation: Modulation::Fm,
            bandwidth: if repeater.wide {
                Bandwidth::Wide
            } else {
                Bandwidth::Narrow
            },
            power: PowerLevel::High,
            step_hz: 12_500,
            ..ChannelDraft::default()
        })
        .collect();
    BankDraft {
        name,
        kind: BankKind::Named,
        scan_enabled: true,
        shared: SharedSettings::from_bits(
            SharedSettings::MODULATION | SharedSettings::STEP | SharedSettings::POWER,
        )
        .unwrap_or_default(),
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::High,
        step_hz: 12_500,
        channels,
        ..BankDraft::default()
    }
}

/// The repeaters a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct RepeaterList {
    /// Every FM repeater, ordered by callsign.
    pub repeaters: Vec<Repeater>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Where the raw ETCC response is kept.
#[derive(Clone, Debug)]
pub struct RepeaterCache {
    dir: Option<PathBuf>,
}

impl RepeaterCache {
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
    pub fn load(&self) -> Result<Option<RepeaterList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let path = dir.join(SYSTEMS_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let failed = |error: std::io::Error| format!("{}: {error}", path.display());
        let json = fs::read_to_string(&path).map_err(failed)?;
        let synced = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .map_err(failed)?;
        Ok(Some(RepeaterList {
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
    ) -> Result<RepeaterList, String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        let json = fetch(SYSTEMS_URL)?;
        parse(&json)?;
        let path = dir.join(SYSTEMS_FILE);
        let partial = path.with_extension("json.partial");
        fs::create_dir_all(dir)
            .and_then(|()| fs::write(&partial, &json))
            .and_then(|()| fs::rename(&partial, &path))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        self.load()?
            .ok_or_else(|| "the repeater cache was not written".to_owned())
    }
}

/// The locator and radius the operator last chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepeaterArea {
    /// Maidenhead locator the radius is measured from.
    pub locator: String,
    /// Distance within which repeaters are taken, in kilometres.
    pub radius_km: u16,
}

impl Default for RepeaterArea {
    fn default() -> Self {
        Self {
            locator: String::new(),
            radius_km: DEFAULT_RADIUS_KM,
        }
    }
}

impl RepeaterArea {
    /// The state file the area is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(AREA_FILE))
    }

    /// Reads `LOCATOR RADIUS_KM`; anything unreadable gives the default.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut words = text.split_whitespace();
        let locator = words.next().unwrap_or_default().to_owned();
        let radius_km = words
            .next()
            .and_then(|word| word.parse().ok())
            .filter(|km| (1..=MAX_RADIUS_KM).contains(km))
            .unwrap_or(DEFAULT_RADIUS_KM);
        Self { locator, radius_km }
    }

    /// Writes the area for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                fs::write(
                    file,
                    format!("{} {}\n", self.locator.trim(), self.radius_km),
                )
            })
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// Fetches one URL over HTTPS.
fn fetch(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(concat!("afik-studio/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES)
        .read_to_string(&mut body)
        .map_err(|error| format!("{url}: {error}"))?;
    Ok(body)
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<RepeaterList, String>>,
}

impl SyncJob {
    /// Starts fetching from the ETCC into `cache`.
    pub fn start(cache: RepeaterCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        thread::spawn(move || {
            // The editor may have closed; then nobody wants the outcome.
            let _ = sender.send(cache.sync_with(fetch));
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<RepeaterList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the repeater sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        distance_km, locator_centre, nearby, parse, repeater_bank, RepeaterArea, RepeaterCache,
        SYSTEMS_URL,
    };

    /// Systems in the shape the ETCC returns, trimmed to the fields used.
    const SYSTEMS: &str = r#"{"data": [
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3WL",
         "town": "CENTRAL LONDON", "tx": 433050000, "rx": 434650000, "ctcss": 82.5,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3EL",
         "town": "EAST LONDON", "tx": 145737500, "rx": 145137500, "ctcss": 82.5,
         "txbw": 12.5, "locator": "JO01AN"},
        {"type": "DM", "status": "REDUCED OUTPUT", "modeCodes": ["A", "F"], "repeater": "GB3HW",
         "town": "ROMFORD", "tx": 433125000, "rx": 434725000, "ctcss": 82.5,
         "txbw": 12.5, "locator": "JO01CO"},
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3BB",
         "town": "BRECON", "tx": 145625000, "rx": 145025000, "ctcss": 94.8,
         "txbw": 25, "locator": "IO81HW"},
        {"type": "RL", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3NL",
         "town": "NORTH LONDON", "tx": 145612500, "rx": 145012500, "ctcss": 0,
         "txbw": 12.5, "locator": "IO91WN"},
        {"type": "AV", "status": "NOT OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3OFF",
         "town": "LONDON", "tx": 145700000, "rx": 145100000, "ctcss": 82.5,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "DV", "status": "OPERATIONAL", "modeCodes": ["M:1"], "repeater": "GB7DMR",
         "town": "LONDON", "tx": 439400000, "rx": 430400000, "ctcss": 0,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "AG", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "MB7GATE",
         "town": "LONDON", "tx": 145337500, "rx": 145337500, "ctcss": 77,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3TV",
         "town": "DUNSTABLE", "tx": 2400000000, "rx": 2400000000, "ctcss": 0,
         "txbw": 12.5, "locator": "IO91RV"},
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": null, "repeater": "GB3NUL",
         "town": "LONDON", "tx": 145650000, "rx": 145050000, "ctcss": 0,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3BAD",
         "town": "NOWHERE", "tx": 145675000, "rx": 145075000, "ctcss": 0,
         "txbw": 12.5, "locator": "ZZ99"}
    ]}"#;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-etcc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn fake(url: &str) -> Result<String, String> {
        if url == SYSTEMS_URL {
            Ok(SYSTEMS.to_owned())
        } else {
            Err(format!("unexpected {url}"))
        }
    }

    #[test]
    fn a_locator_gives_the_centre_of_its_square() {
        let close = |(lat, lon): (f64, f64), (want_lat, want_lon): (f64, f64)| {
            assert!(
                (lat - want_lat).abs() < 1e-9 && (lon - want_lon).abs() < 1e-9,
                "({lat}, {lon}) is not ({want_lat}, {want_lon})"
            );
        };
        close(locator_centre("IO").unwrap(), (55.0, -10.0));
        close(locator_centre("IO91").unwrap(), (51.5, -1.0));
        // IO91WM spans 51.5 to 51.5417 N and 0.1667 to 0.0833 W.
        close(
            locator_centre("io91wm").unwrap(),
            (51.5 + 1.0 / 48.0, -2.0 + 22.5 / 12.0),
        );
        // Its extended square 55 is the middle tenth each way.
        close(
            locator_centre("IO91WM55").unwrap(),
            (51.5 + 5.5 / 240.0, -2.0 + 22.0 / 12.0 + 5.5 / 120.0),
        );
        for bad in ["", "I", "IO9", "SA00", "IO9A", "IO91ZZ", "IO91WMAA"] {
            assert!(locator_centre(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn distances_are_great_circle_kilometres() {
        // London to Edinburgh is about 534 km.
        let km = distance_km((51.5074, -0.1278), (55.9533, -3.1883));
        assert!((530.0..540.0).contains(&km), "{km}");
        assert!(distance_km((51.5, 0.0), (51.5, 0.0)).abs() < 1e-9);
    }

    #[test]
    fn the_list_keeps_fm_repeaters_the_radio_can_hear() {
        let repeaters = parse(SYSTEMS).expect("parses");
        let summary: Vec<_> = repeaters
            .iter()
            .map(|r| {
                (
                    r.callsign.as_str(),
                    r.output_hz,
                    r.input_hz,
                    r.ctcss_tenths_hz,
                    r.wide,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("GB3BB", 145_625_000, 145_025_000, Some(948), true),
                ("GB3EL", 145_737_500, 145_137_500, Some(825), false),
                ("GB3HW", 433_125_000, 434_725_000, Some(825), false),
                ("GB3NL", 145_612_500, 145_012_500, None, false),
                ("GB3WL", 433_050_000, 434_650_000, Some(825), false),
            ]
        );
    }

    #[test]
    fn a_response_without_a_data_list_is_refused() {
        assert!(parse("[]").is_err());
        assert!(parse("{\"detail\": \"throttled\"}").is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn nearby_repeaters_come_nearest_first_within_the_radius() {
        let repeaters = parse(SYSTEMS).unwrap();
        let near = nearby(&repeaters, "IO91WM", 50).unwrap();
        let names: Vec<_> = near.iter().map(|(r, _)| r.callsign.as_str()).collect();
        assert_eq!(names, ["GB3WL", "GB3NL", "GB3EL", "GB3HW"]);
        assert!(near.windows(2).all(|pair| pair[0].1 <= pair[1].1));
        // Brecon is about 230 km away.
        let wide = nearby(&repeaters, "IO91WM", 300).unwrap();
        assert_eq!(wide.last().unwrap().0.callsign, "GB3BB");
        assert_eq!(nearby(&repeaters, "IO91WM", 1).unwrap().len(), 1);
        assert!(nearby(&repeaters, "nonsense", 50).is_err());
    }

    #[test]
    fn the_bank_receives_outputs_and_transmits_inputs_with_the_tone() {
        let repeaters = parse(SYSTEMS).unwrap();
        let near = nearby(&repeaters, "io91wm", 300).unwrap();
        let bank = repeater_bank("io91wm", &near);
        assert_eq!(bank.name, "RPT IO91WM");
        assert_eq!(bank.channels[0].name, "GB3WL");
        assert_eq!(bank.channels[0].receive_mhz, "433.050000");
        assert_eq!(bank.channels[0].transmit_mhz, "434.650000");
        assert_eq!(bank.channels[0].tx_tone.value, "82.5");
        let brecon = bank.channels.last().unwrap();
        assert_eq!(brecon.name, "GB3BB");
        assert_eq!(brecon.bandwidth, radio_domain::Bandwidth::Wide);
        let mut project = crate::model::ProjectModel::new();
        project.banks.push(bank);
        assert!(project.validate().is_ok());
    }

    #[test]
    fn a_sync_caches_the_response_and_a_failed_one_keeps_it() {
        let dir = scratch("sync");
        let cache = RepeaterCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none(), "nothing synced yet");
        assert_eq!(cache.sync_with(fake).unwrap().repeaters.len(), 5);
        assert!(cache.sync_with(|_| Err("offline".to_owned())).is_err());
        assert!(cache.sync_with(|_| Ok("<html>".to_owned())).is_err());
        let loaded = RepeaterCache::in_dir(dir.clone()).load().unwrap().unwrap();
        assert_eq!(loaded.repeaters.len(), 5);
        assert!(RepeaterCache::none().sync_with(fake).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_area_is_remembered_and_a_bad_file_gives_the_default() {
        let dir = scratch("area");
        let file = dir.join("repeater-area");
        assert_eq!(RepeaterArea::load(&file), RepeaterArea::default());
        let area = RepeaterArea {
            locator: "IO91WM".to_owned(),
            radius_km: 30,
        };
        area.save(&file).unwrap();
        assert_eq!(RepeaterArea::load(&file), area);
        std::fs::write(&file, "IO91WM lots\n").unwrap();
        assert_eq!(RepeaterArea::load(&file).radius_km, 50);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Fetches from the real ETCC list; run with `--ignored` to check the
    /// live response still parses and a London bank fits a radio.
    #[test]
    #[ignore = "fetches from api-beta.rsgb.online"]
    fn the_live_etcc_list_parses() {
        let dir = scratch("live");
        let list = RepeaterCache::in_dir(dir.clone())
            .sync_with(super::fetch)
            .expect("syncs");
        println!("{} FM repeaters", list.repeaters.len());
        let near = nearby(&list.repeaters, "IO91WM", 50).unwrap();
        for (repeater, km) in &near {
            println!("{km:6.1} km {repeater:?}");
        }
        assert!(near.len() > 10);
        let mut project = crate::model::ProjectModel::new();
        project.banks.push(repeater_bank("IO91WM", &near));
        assert!(project.validate().is_ok(), "the bank validates");
        project.to_radio_image().expect("the bank fits a radio");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
