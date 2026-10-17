//! Marine VHF channels near a Maidenhead locator: a fixed core every bank
//! carries, and the channels `OpenStreetMap` lists for the harbours,
//! marinas, locks and lookouts nearby.
//!
//! As with the airports, the studio fetches only when the operator presses
//! Sync: one Overpass query for every object in the world tagged with a
//! marine VHF channel (about 3 000, 1.4 MB), cached raw in
//! `$XDG_CACHE_HOME/afik/osm-marine`. The locator is the repeaters' own; the
//! radius and how many local channels to take are kept in
//! `$XDG_STATE_HOME/afik/marine-area`.
//!
//! A duplex channel keeps both its frequencies, the coast station's as
//! receive and the ship's as transmit; a radio whose TX ranges do not cover
//! the ship's listens to both (`ADR-108`).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};

use radio_channel_plan::{SharedSettings, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN};
use radio_domain::{Bandwidth, Modulation, PowerLevel};
use serde_json::Value;

use crate::{
    airports::AirportArea,
    model::{format_mhz, BankDraft, BankKind, ChannelDraft},
    paths,
    presets::{marine_hz, MARINE_DUPLEX_OFFSET_HZ},
    repeaters::{distance_km, fetch_within, locator_centre, sanitise},
};

/// The public Overpass instance the query is sent to.
pub const OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
/// Tags carrying a marine VHF channel number.
const CHANNEL_KEYS: [&str; 7] = [
    "vhf",
    "contact:vhf",
    "seamark:radio_station:channel",
    "seamark:signal_station_traffic:channel",
    "seamark:calling-in_point:channel",
    "seamark:pilot_boarding:channel",
    "waterway:ukw_channel",
];
/// The worldwide query takes about a minute.
const SYNC_TIMEOUT: Duration = Duration::from_secs(300);
const CACHE_DIR: &str = "osm-marine";
const CACHE_FILE: &str = "marine-vhf.json";
const AREA_FILE: &str = "marine-area";
/// The 25 kHz grid the bank's VFO steps on.
const STEP_HZ: u32 = 25_000;
/// The channels every bank starts with, whatever is mapped nearby: distress
/// and calling, the UK coastguard's small-craft safety channel, the
/// intership channels, and the UK marina channels.
const CORE: [(&str, &str); 8] = [
    ("16", "CALL"),
    ("67", "SAR"),
    ("6", "SHIP"),
    ("8", "SHIP"),
    ("72", "SHIP"),
    ("77", "SHIP"),
    ("M", "MARINA"),
    ("M2", "MARINA"),
];

/// One marine channel and its frequencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarineChannel {
    /// Its designator: the channel number, `80A` for a duplex channel used
    /// simplex on the ship's frequency, or `M` and `M2`.
    pub designator: String,
    /// Where the coast station transmits, in hertz.
    pub receive_hz: u32,
    /// Where ships transmit, in hertz; the same as receive on a simplex
    /// channel.
    pub transmit_hz: u32,
}

impl MarineChannel {
    fn simplex(designator: String, hz: u32) -> Self {
        Self {
            designator,
            receive_hz: hz,
            transmit_hz: hz,
        }
    }
}

/// The ITU Radio Regulations Appendix 18 duplex channels: ships on the
/// lower frequency, coast stations 4.6 MHz above.
const fn is_duplex(number: u32) -> bool {
    matches!(number, 1..=5 | 7 | 18..=28 | 60..=66 | 78..=86)
}

/// The simplex voice channels: Appendix 18's, plus 31, the Dutch and
/// Belgian marina channel. 70 (DSC) and 75 and 76 (guard bands around 16)
/// carry no voice.
const fn is_simplex(number: u32) -> bool {
    matches!(number, 6 | 8..=17 | 31 | 67..=69 | 71..=74 | 77 | 87 | 88)
}

/// The channel a designator names, if it is a voice channel.
///
/// `M`, `M1` and `37A` are the UK marina channel on 157.850 MHz and `M2` the
/// one on 161.425 MHz. A duplex channel with an `A` is used simplex on the
/// ship's frequency, as `80A` is at marinas.
pub fn channel(designator: &str) -> Option<MarineChannel> {
    let designator = designator.trim().to_ascii_uppercase();
    match designator.as_str() {
        "M" | "M1" | "37" | "37A" => {
            return Some(MarineChannel::simplex("M".to_owned(), 157_850_000));
        }
        "M2" => return Some(MarineChannel::simplex("M2".to_owned(), 161_425_000)),
        _ => {}
    }
    let (digits, simplex) = match designator.strip_suffix('A') {
        Some(digits) => (digits, true),
        None => (designator.as_str(), false),
    };
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    let ship = marine_hz(number);
    if is_duplex(number) {
        Some(if simplex {
            MarineChannel::simplex(format!("{number}A"), ship)
        } else {
            MarineChannel {
                designator: number.to_string(),
                receive_hz: ship + MARINE_DUPLEX_OFFSET_HZ,
                transmit_hz: ship,
            }
        })
    } else if is_simplex(number) {
        Some(MarineChannel::simplex(number.to_string(), ship))
    } else {
        None
    }
}

/// Every marine channel a tag value names, in order, each once.
///
/// Values are free text: `13;16`, `VHF 72/16`, `UKW 18`, `Kanal 20`,
/// `Ch9 to Ch73`, `74 (Humber Cruising)`. A part giving a frequency, such as
/// an airfield's `123.425 MHz` or a CB `27MHz 88`, is skipped.
pub fn channels_in(value: &str) -> Vec<MarineChannel> {
    let mut found: Vec<MarineChannel> = Vec::new();
    for part in value.split(';') {
        let part = part.to_ascii_uppercase();
        // A point between digits, or a comma after three (`107,3`); a
        // comma after fewer separates channels (`16,67`).
        let bytes = part.as_bytes();
        let decimal = (1..bytes.len().saturating_sub(1)).any(|at| {
            let digit_after = bytes[at + 1].is_ascii_digit();
            match bytes[at] {
                b'.' => bytes[at - 1].is_ascii_digit() && digit_after,
                b',' => at >= 3 && bytes[at - 3..at].iter().all(u8::is_ascii_digit) && digit_after,
                _ => false,
            }
        });
        if part.contains("MHZ") || decimal {
            continue;
        }
        let words = part
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty());
        for word in words {
            let word = ["CH", "VHF", "UKW"]
                .iter()
                .find_map(|prefix| {
                    word.strip_prefix(prefix)
                        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
                })
                .unwrap_or(word);
            if let Some(channel) = channel(word) {
                if !found.contains(&channel) {
                    found.push(channel);
                }
            }
        }
    }
    found
}

/// One mapped place and the channels it listens on.
#[derive(Clone, Debug, PartialEq)]
pub struct MarinePlace {
    /// Its name, or what it is when it has none, such as `LOCK`.
    pub name: String,
    /// Latitude in degrees north.
    pub latitude: f64,
    /// Longitude in degrees east.
    pub longitude: f64,
    /// Its channels, in the order the map gives them.
    pub channels: Vec<MarineChannel>,
}

/// What an unnamed place is, from its tags.
fn kind(tags: &serde_json::Map<String, Value>) -> &'static str {
    let tag = |key: &str| tags.get(key).and_then(Value::as_str);
    if tag("leisure") == Some("marina") {
        "MARINA"
    } else if tag("lock").is_some() || tag("waterway") == Some("lock_gate") {
        "LOCK"
    } else if tag("bridge:movable").is_some() {
        "BRIDGE"
    } else if tags.contains_key("seamark:pilot_boarding:channel") {
        "PILOTS"
    } else if tags.contains_key("seamark:signal_station_traffic:channel") {
        "TRAFFIC"
    } else if tag("harbour").is_some() || tag("landuse") == Some("harbour") {
        "HARBOUR"
    } else {
        "MARINE"
    }
}

/// Reads an Overpass answer: every place with a position and at least one
/// marine voice channel.
pub fn parse(json: &str) -> Result<Vec<MarinePlace>, String> {
    let answer: Value =
        serde_json::from_str(json).map_err(|error| format!("Overpass answer: {error}"))?;
    let elements = answer
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("Overpass answer: no elements")?;
    let places: Vec<MarinePlace> = elements
        .iter()
        .filter_map(|element| {
            let tags = element.get("tags")?.as_object()?;
            let at = element.get("center").unwrap_or(element);
            let latitude = at.get("lat")?.as_f64()?;
            let longitude = at.get("lon")?.as_f64()?;
            let mut channels = Vec::new();
            for key in CHANNEL_KEYS {
                let value = tags.get(key).and_then(Value::as_str).unwrap_or("");
                for channel in channels_in(value) {
                    if !channels.contains(&channel) {
                        channels.push(channel);
                    }
                }
            }
            if channels.is_empty() {
                return None;
            }
            let named = ["name", "seamark:name", "operator"]
                .iter()
                .find_map(|key| tags.get(*key).and_then(Value::as_str))
                .and_then(|name| sanitise(name, MAX_CHANNEL_NAME_LEN));
            Some(MarinePlace {
                name: named.unwrap_or_else(|| kind(tags).to_owned()),
                latitude,
                longitude,
                channels,
            })
        })
        .collect();
    if places.is_empty() {
        return Err("Overpass answer: no marine VHF channels".to_owned());
    }
    Ok(places)
}

/// A channel used near the locator, and the nearest place using it.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalChannel {
    /// The channel.
    pub channel: MarineChannel,
    /// The nearest place that lists it.
    pub place: String,
    /// How far away that place is, in kilometres.
    pub km: f64,
    /// That place's latitude and longitude, in degrees.
    pub position: (f64, f64),
}

/// The channels the places within `radius_km` of `locator` list, nearest
/// place first, each channel once for its nearest place. Channels the core
/// already has are left out.
pub fn nearby(
    places: &[MarinePlace],
    locator: &str,
    radius_km: u16,
) -> Result<Vec<LocalChannel>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<(&MarinePlace, f64)> = places
        .iter()
        .map(|place| (place, distance_km(home, (place.latitude, place.longitude))))
        .filter(|(_, km)| *km <= f64::from(radius_km))
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.name.cmp(&b.0.name)));
    let mut taken: HashSet<String> = CORE.iter().map(|(core, _)| (*core).to_owned()).collect();
    let mut local = Vec::new();
    for (place, km) in near {
        for channel in &place.channels {
            if taken.insert(channel.designator.clone()) {
                local.push(LocalChannel {
                    channel: channel.clone(),
                    place: place.name.clone(),
                    km,
                    position: (place.latitude, place.longitude),
                });
            }
        }
    }
    Ok(local)
}

fn draft(name: &str, channel: &MarineChannel) -> ChannelDraft {
    ChannelDraft {
        name: sanitise(name, MAX_CHANNEL_NAME_LEN).unwrap_or_else(|| channel.designator.clone()),
        receive_mhz: format_mhz(channel.receive_hz),
        transmit_mhz: format_mhz(channel.transmit_hz),
        modulation: Modulation::Fm,
        // Marine VHF is 25 kHz FM.
        bandwidth: Bandwidth::Wide,
        power: PowerLevel::Low1,
        step_hz: STEP_HZ,
        ..ChannelDraft::default()
    }
}

/// Builds the marine bank: the core channels, then the local ones in the
/// order `nearby` chose, each named by its designator and place.
pub fn marine_bank(locator: &str, local: &[LocalChannel]) -> BankDraft {
    let mut name = format!("SEA {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
    let core = CORE.iter().filter_map(|(designator, use_)| {
        channel(designator).map(|channel| draft(&format!("{designator} {use_}"), &channel))
    });
    let local = local.iter().map(|local| {
        draft(
            &format!("{} {}", local.channel.designator, local.place),
            &local.channel,
        )
    });
    BankDraft {
        name,
        kind: BankKind::Named,
        scan_enabled: true,
        shared: SharedSettings::from_bits(
            SharedSettings::MODULATION
                | SharedSettings::BANDWIDTH
                | SharedSettings::STEP
                | SharedSettings::POWER,
        )
        .unwrap_or_default(),
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Wide,
        power: PowerLevel::Low1,
        step_hz: STEP_HZ,
        channels: core.chain(local).collect(),
        ..BankDraft::default()
    }
}

/// How many channels a bank holds before any local one.
pub const CORE_CHANNELS: usize = CORE.len();

/// The Overpass query for every place in the world tagged with a channel.
fn query() -> String {
    let parts = CHANNEL_KEYS
        .iter()
        .fold(String::new(), |parts, key| parts + "nwr[\"" + key + "\"];");
    format!("[out:json][timeout:240];({parts});out tags center;")
}

/// The URL that asks Overpass the query.
pub fn query_url() -> String {
    let encoded: String = query()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("{OVERPASS_URL}?data={encoded}")
}

/// The places a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct MarineList {
    /// Every place with a marine voice channel.
    pub places: Vec<MarinePlace>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Where the raw Overpass answer is kept.
#[derive(Clone, Debug)]
pub struct MarineCache {
    dir: Option<PathBuf>,
}

impl MarineCache {
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

    /// Reads the cached answer; `Ok(None)` when nothing has been synced.
    pub fn load(&self) -> Result<Option<MarineList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let file = dir.join(CACHE_FILE);
        if !file.exists() {
            return Ok(None);
        }
        let text = fs::read(&file)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| format!("{}: {error}", file.display()))?;
        let synced = fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .map_err(|error| format!("{}: {error}", file.display()))?;
        Ok(Some(MarineList {
            places: parse(&text)?,
            synced,
        }))
    }

    /// Asks Overpass, checks the answer, caches it, and returns what it
    /// holds. Nothing is written unless the answer parses, and the write
    /// goes through a temporary file and a rename.
    pub fn sync_with(
        &self,
        fetch: impl Fn(&str) -> Result<String, String>,
    ) -> Result<MarineList, String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        let answer = fetch(&query_url())?;
        parse(&answer)?;
        fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let path = dir.join(CACHE_FILE);
        let partial = path.with_extension("json.partial");
        fs::write(&partial, &answer)
            .and_then(|()| fs::rename(&partial, &path))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        self.load()?
            .ok_or_else(|| "the marine cache was not written".to_owned())
    }
}

/// The state file the marine radius and channel count are kept in; they
/// read and write as the airports' do.
pub fn area_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(AREA_FILE))
}

/// The radius marine places are taken within before the operator chooses
/// one, in kilometres: about how far a coast station or ship reaches a
/// handheld; a marina or lock carries less.
pub const DEFAULT_RADIUS_KM: u16 = 30;

/// The radius and how many local channels to take, as for the airports.
pub type MarineArea = AirportArea;

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<MarineList, String>>,
}

impl SyncJob {
    /// Starts asking Overpass into `cache`.
    pub fn start(cache: MarineCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        thread::spawn(move || {
            // The editor may have closed; then nobody wants the outcome.
            let _ = sender.send(cache.sync_with(|url| fetch_within(url, SYNC_TIMEOUT)));
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<MarineList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the marine sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        channel, channels_in, marine_bank, nearby, parse, query_url, MarineCache, CORE_CHANNELS,
    };
    use crate::model::ProjectModel;
    use radio_domain::{Bandwidth, Modulation};

    /// Elements in the shape Overpass answers with, trimmed.
    const ANSWER: &str = r#"{"version":0.6,"elements":[
{"type":"way","id":1,"center":{"lat":51.627,"lon":0.806},
 "tags":{"leisure":"marina","name":"Burnham Yacht Harbour","vhf":"80"}},
{"type":"node","id":2,"lat":51.95,"lon":1.28,
 "tags":{"seamark:type":"harbour","seamark:name":"Harwich Haven","contact:vhf":"71;14"}},
{"type":"node","id":3,"lat":51.96,"lon":1.35,
 "tags":{"seamark:radio_station:channel":"65","name":"Coastwatch Lookout Station"}},
{"type":"node","id":4,"lat":51.70,"lon":0.95,"tags":{"lock":"yes","vhf":"VHF 80/16"}},
{"type":"node","id":5,"lat":51.7,"lon":0.9,"tags":{"vhf":"123.425 MHz","name":"Airfield"}},
{"type":"node","id":6,"lat":53.48,"lon":-2.24,"tags":{"vhf":"none","name":"Nowhere"}}
]}"#;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-sea-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn designators_become_channels() {
        let sea = |d| channel(d).map(|c| (c.designator, c.receive_hz, c.transmit_hz));
        assert_eq!(sea("16"), Some(("16".into(), 156_800_000, 156_800_000)));
        assert_eq!(sea("09"), Some(("9".into(), 156_450_000, 156_450_000)));
        // Duplex: the coast station 4.6 MHz above the ship.
        assert_eq!(sea("24"), Some(("24".into(), 161_800_000, 157_200_000)));
        assert_eq!(sea("80"), Some(("80".into(), 161_625_000, 157_025_000)));
        // A duplex channel used simplex on the ship's frequency.
        assert_eq!(sea("80A"), Some(("80A".into(), 157_025_000, 157_025_000)));
        assert_eq!(sea("71A"), Some(("71".into(), 156_575_000, 156_575_000)));
        assert_eq!(sea("37A"), Some(("M".into(), 157_850_000, 157_850_000)));
        assert_eq!(sea("m2"), Some(("M2".into(), 161_425_000, 161_425_000)));
        assert_eq!(sea("31"), Some(("31".into(), 157_550_000, 157_550_000)));
        for refused in ["70", "75", "0", "89", "150", "03Q", "", "A", "yes"] {
            assert_eq!(sea(refused), None, "{refused}");
        }
    }

    #[test]
    fn free_text_values_name_their_channels() {
        let names = |value| {
            channels_in(value)
                .into_iter()
                .map(|c| c.designator)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("13;16"), ["13", "16"]);
        assert_eq!(names("VHF 72/16"), ["72", "16"]);
        assert_eq!(names("UKW 18"), ["18"]);
        assert_eq!(names("Kanal 20"), ["20"]);
        assert_eq!(names("Ch9 to Ch73"), ["9", "73"]);
        assert_eq!(names("VHF73"), ["73"]);
        assert_eq!(names("74 (Humber Cruising)"), ["74"]);
        assert_eq!(names("09 & 16; 9"), ["9", "16"]);
        assert_eq!(names("VHF 16,67,73;27MHz 86,88,91"), ["16", "67", "73"]);
        assert!(names("123.425 MHz").is_empty());
        assert!(names("107,3 MHZ").is_empty());
        assert!(names("none").is_empty());
        assert!(names("[03Q]").is_empty());
    }

    #[test]
    fn the_answer_gives_each_place_with_a_voice_channel() {
        let places = parse(ANSWER).unwrap();
        let summary: Vec<_> = places
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.channels
                        .iter()
                        .map(|c| c.designator.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("BURNHAM YACH", vec!["80"]),
                ("HARWICH HAVE", vec!["71", "14"]),
                ("COASTWATCH L", vec!["65"]),
                ("LOCK", vec!["80", "16"]),
            ]
        );
        assert!((places[0].latitude - 51.627).abs() < 1e-9, "a way's centre");
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"elements":[]}"#).is_err());
    }

    #[test]
    fn nearby_names_each_channel_after_its_nearest_place_beyond_the_core() {
        let places = parse(ANSWER).unwrap();
        // JO01HP holds Burnham-on-Crouch; Harwich is about 40 km away.
        let near = nearby(&places, "JO01HP", 75).unwrap();
        let chosen: Vec<_> = near
            .iter()
            .map(|l| (l.channel.designator.as_str(), l.place.as_str()))
            .collect();
        // The lock's 80 is the nearer marina's, and 16 is in the core.
        assert_eq!(chosen[0], ("80", "BURNHAM YACH"));
        assert_eq!(
            &chosen[1..],
            [
                ("71", "HARWICH HAVE"),
                ("14", "HARWICH HAVE"),
                ("65", "COASTWATCH L")
            ]
        );
        assert!(near.windows(2).all(|pair| pair[0].km <= pair[1].km));
        assert_eq!(nearby(&places, "JO01HP", 20).unwrap().len(), 1);
        assert!(nearby(&places, "nonsense", 50).is_err());
    }

    #[test]
    fn the_bank_starts_with_the_core_and_compiles() {
        let places = parse(ANSWER).unwrap();
        let near = nearby(&places, "jo01hp", 75).unwrap();
        let bank = marine_bank("jo01hp", &near);
        assert_eq!(bank.name, "SEA JO01HP");
        assert_eq!(bank.channels.len(), CORE_CHANNELS + 4);
        assert_eq!(bank.channels[0].name, "16 CALL");
        assert_eq!(bank.channels[0].receive_mhz, "156.800000");
        let burnham = &bank.channels[CORE_CHANNELS];
        assert_eq!(burnham.name, "80 BURNHAM Y");
        assert_eq!(burnham.receive_mhz, "161.625000");
        assert_eq!(burnham.transmit_mhz, "157.025000");
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Fm && c.bandwidth == Bandwidth::Wide));
        let project = ProjectModel {
            banks: vec![bank],
            ..ProjectModel::new()
        };
        project.to_radio_image().expect("a valid marine bank");
    }

    #[test]
    fn the_query_is_one_encoded_url() {
        let url = query_url();
        assert!(url.starts_with("https://overpass-api.de/api/interpreter?data=%5Bout%3Ajson%5D"));
        assert!(url.contains("contact%3Avhf"));
        assert!(!url.contains(' ') && !url.contains('"'));
    }

    #[test]
    fn a_sync_caches_only_an_answer_that_parses() {
        let dir = scratch("sync");
        let cache = MarineCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none());
        let good = |_: &str| Ok(ANSWER.to_owned());
        assert_eq!(cache.sync_with(good).unwrap().places.len(), 4);
        assert!(cache
            .sync_with(|_| Ok(r#"{"elements":[]}"#.to_owned()))
            .is_err());
        assert!(cache.sync_with(|_| Err("offline".to_owned())).is_err());
        assert_eq!(cache.load().unwrap().unwrap().places.len(), 4);
        assert!(MarineCache::none().sync_with(good).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
