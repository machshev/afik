//! Airport air traffic frequencies near a Maidenhead locator, synced from
//! `OurAirports` (public domain) and cached.
//!
//! As with the repeaters, the studio fetches only when the operator presses
//! Sync; otherwise it reads the raw CSVs cached in
//! `$XDG_CACHE_HOME/afik/ourairports`. The locator is the repeaters' own; the
//! radius and how many channels to take are kept in
//! `$XDG_STATE_HOME/afik/airport-area`.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::SystemTime,
};

use radio_channel_plan::{SharedSettings, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN};
use radio_domain::{Bandwidth, Modulation, PowerLevel};

use crate::{
    model::{format_mhz, BankDraft, BankKind, ChannelDraft},
    paths,
    repeaters::{distance_km, fetch, locator_centre, sanitise, DEFAULT_RADIUS_KM, MAX_RADIUS_KM},
    stations::records,
};

/// Every airport, heliport and seaplane base, with its position.
pub const AIRPORTS_URL: &str = "https://davidmegginson.github.io/ourairports-data/airports.csv";
/// Every frequency `OurAirports` lists for those airports.
pub const FREQUENCIES_URL: &str =
    "https://davidmegginson.github.io/ourairports-data/airport-frequencies.csv";

const CACHE_DIR: &str = "ourairports";
const AIRPORTS_FILE: &str = "airports.csv";
const FREQUENCIES_FILE: &str = "airport-frequencies.csv";
const AREA_FILE: &str = "airport-area";
/// How many frequencies a bank takes before the operator chooses.
pub const DEFAULT_MAX_CHANNELS: u16 = 40;
/// The most the operator can ask for; the storage the studio shows limits a
/// bank first.
pub const MAX_CHANNELS_LIMIT: u16 = 500;
/// The VHF civil airband, AM voice: 118.000 to 136.975 MHz.
const AIRBAND_FIRST_KHZ: u32 = 118_000;
const AIRBAND_LAST_KHZ: u32 = 136_990;
/// The 25 kHz grid the bank's VFO steps on.
const STEP_HZ: u32 = 25_000;

/// One frequency of one airport.
#[derive(Clone, Debug, PartialEq)]
pub struct AirportFrequency {
    /// Channel name: the airport's ident and the frequency's use, such as
    /// `EGSC TWR`.
    pub name: String,
    /// The airport's ident, usually its ICAO code.
    pub ident: String,
    /// The airport's name, such as `Cambridge Airport`.
    pub airport: String,
    /// Carrier in hertz, to the nearest 10 Hz.
    pub frequency_hz: u32,
    /// Airport latitude in degrees north.
    pub latitude: f64,
    /// Airport longitude in degrees east.
    pub longitude: f64,
}

/// The carrier of an airband frequency as listed, in hertz.
///
/// Lists give a channel name, not a carrier. In each 25 kHz block names end
/// in 0 (25 kHz), 5 (8.33 kHz at the block), 10 (8.33 kHz, a third in) and
/// 15 (two thirds in). A name ending in 20 is a 25 kHz frequency cut to two
/// decimals, such as `123.07` for 123.075. Anything else is off both grids
/// and is refused, as is anything outside the VHF airband.
fn carrier_hz(mhz: &str) -> Option<u32> {
    let mhz: f64 = mhz.trim().parse().ok()?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let khz = (mhz * 1_000.0).round() as u32;
    if (f64::from(khz) - mhz * 1_000.0).abs() > 0.01 {
        return None;
    }
    let block = khz - khz % 25;
    let hz = match khz % 25 {
        0 | 5 => block * 1_000,
        10 => block * 1_000 + 8_330,
        15 => block * 1_000 + 16_670,
        20 => (block + 25) * 1_000,
        _ => return None,
    };
    (AIRBAND_FIRST_KHZ * 1_000..=AIRBAND_LAST_KHZ * 1_000)
        .contains(&hz)
        .then_some(hz)
}

/// The columns of a CSV, found by their headings.
fn columns<const N: usize>(
    list: &str,
    heading: &[String],
    names: [&str; N],
) -> Result<[usize; N], String> {
    let mut found = [0; N];
    for (at, name) in found.iter_mut().zip(names) {
        *at = heading
            .iter()
            .position(|heading| heading.trim() == name)
            .ok_or_else(|| format!("OurAirports {list}: no {name:?} column"))?;
    }
    Ok(found)
}

/// Joins `OurAirports`' frequencies to their airports and keeps those in the
/// VHF airband at an airport that is not closed.
pub fn parse(airports_csv: &str, frequencies_csv: &str) -> Result<Vec<AirportFrequency>, String> {
    let mut rows = records(frequencies_csv).into_iter();
    let heading = rows.next().ok_or("OurAirports frequencies: empty")?;
    let [airport_at, kind_at, frequency_at] = columns(
        "frequencies",
        &heading,
        ["airport_ref", "type", "frequency_mhz"],
    )?;
    let listed: Vec<(String, String, u32)> = rows
        .filter_map(|row| {
            let field = |at: usize| row.get(at).map_or("", String::as_str);
            Some((
                field(airport_at).trim().to_owned(),
                field(kind_at).trim().to_owned(),
                carrier_hz(field(frequency_at))?,
            ))
        })
        .collect();
    let wanted: HashSet<&str> = listed.iter().map(|(id, _, _)| id.as_str()).collect();

    let mut rows = records(airports_csv).into_iter();
    let heading = rows.next().ok_or("OurAirports airports: empty")?;
    let [id_at, ident_at, type_at, name_at, latitude_at, longitude_at] = columns(
        "airports",
        &heading,
        [
            "id",
            "ident",
            "type",
            "name",
            "latitude_deg",
            "longitude_deg",
        ],
    )?;
    let airports: HashMap<String, (String, String, f64, f64)> = rows
        .filter_map(|row| {
            let field = |at: usize| row.get(at).map_or("", String::as_str).trim();
            let id = field(id_at);
            if !wanted.contains(id) || field(type_at) == "closed" {
                return None;
            }
            Some((
                id.to_owned(),
                (
                    field(ident_at).to_owned(),
                    field(name_at).to_owned(),
                    field(latitude_at).parse().ok()?,
                    field(longitude_at).parse().ok()?,
                ),
            ))
        })
        .collect();

    let mut frequencies: Vec<AirportFrequency> = listed
        .into_iter()
        .filter_map(|(id, kind, frequency_hz)| {
            let (ident, airport, latitude, longitude) = airports.get(&id)?;
            Some(AirportFrequency {
                name: sanitise(&format!("{ident} {kind}"), MAX_CHANNEL_NAME_LEN)?,
                ident: ident.clone(),
                airport: airport.clone(),
                frequency_hz,
                latitude: *latitude,
                longitude: *longitude,
            })
        })
        .collect();
    if frequencies.is_empty() {
        return Err("OurAirports: no airband frequencies".to_owned());
    }
    frequencies.sort_by(|a, b| {
        a.ident
            .cmp(&b.ident)
            .then(a.frequency_hz.cmp(&b.frequency_hz))
    });
    Ok(frequencies)
}

/// Every frequency of the airports within `radius_km` of `locator`, nearest
/// airport first and each airport's in frequency order, with its distance in
/// kilometres. A frequency several airports share, such as a common
/// traffic frequency, is kept once, for the nearest.
pub fn nearby<'a>(
    frequencies: &'a [AirportFrequency],
    locator: &str,
    radius_km: u16,
) -> Result<Vec<(&'a AirportFrequency, f64)>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<_> = frequencies
        .iter()
        .map(|frequency| {
            let km = distance_km(home, (frequency.latitude, frequency.longitude));
            (frequency, km)
        })
        .filter(|(_, km)| *km <= f64::from(radius_km))
        .collect();
    near.sort_by(|a, b| {
        a.1.total_cmp(&b.1)
            .then(a.0.ident.cmp(&b.0.ident))
            .then(a.0.frequency_hz.cmp(&b.0.frequency_hz))
    });
    let mut taken = HashSet::new();
    near.retain(|(frequency, _)| taken.insert(frequency.frequency_hz));
    Ok(near)
}

/// Builds the receive-only AM bank for the frequencies `nearby` chose, in
/// its order; no TX range covers the airband, so the radio never transmits.
pub fn airport_bank(locator: &str, near: &[(&AirportFrequency, f64)]) -> BankDraft {
    let mut name = format!("AIR {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
    let channels = near
        .iter()
        .map(|(frequency, _)| ChannelDraft {
            name: frequency.name.clone(),
            receive_mhz: format_mhz(frequency.frequency_hz),
            // Simplex; no TX range covers the airband, so it is never used.
            transmit_mhz: format_mhz(frequency.frequency_hz),
            modulation: Modulation::Am,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low1,
            step_hz: STEP_HZ,
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
        modulation: Modulation::Am,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low1,
        step_hz: STEP_HZ,
        channels,
        ..BankDraft::default()
    }
}

/// The frequencies a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct AirportList {
    /// Every airband frequency, ordered by airport.
    pub frequencies: Vec<AirportFrequency>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Where the raw `OurAirports` CSVs are kept.
#[derive(Clone, Debug)]
pub struct AirportCache {
    dir: Option<PathBuf>,
}

impl AirportCache {
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

    /// Reads the cached lists; `Ok(None)` when nothing has been synced.
    pub fn load(&self) -> Result<Option<AirportList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let airports = dir.join(AIRPORTS_FILE);
        let frequencies = dir.join(FREQUENCIES_FILE);
        if !airports.exists() || !frequencies.exists() {
            return Ok(None);
        }
        let read = |path: &Path| {
            fs::read(path)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("{}: {error}", path.display()))
        };
        let synced = fs::metadata(&frequencies)
            .and_then(|meta| meta.modified())
            .map_err(|error| format!("{}: {error}", frequencies.display()))?;
        Ok(Some(AirportList {
            frequencies: parse(&read(&airports)?, &read(&frequencies)?)?,
            synced,
        }))
    }

    /// Fetches both lists, checks them, caches them, and returns what they
    /// hold.
    ///
    /// Nothing is written unless the pair parses, so a bad fetch keeps the
    /// cache as it was; each write goes through a temporary file and a
    /// rename, the frequencies last, since their time is the sync time.
    pub fn sync_with(
        &self,
        fetch: impl Fn(&str) -> Result<String, String>,
    ) -> Result<AirportList, String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        let airports = fetch(AIRPORTS_URL)?;
        let frequencies = fetch(FREQUENCIES_URL)?;
        parse(&airports, &frequencies)?;
        fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for (file, text) in [(AIRPORTS_FILE, &airports), (FREQUENCIES_FILE, &frequencies)] {
            let path = dir.join(file);
            let partial = path.with_extension("csv.partial");
            fs::write(&partial, text)
                .and_then(|()| fs::rename(&partial, &path))
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        self.load()?
            .ok_or_else(|| "the airport cache was not written".to_owned())
    }
}

/// The radius and channel count the operator last chose; the locator is the
/// repeaters' own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirportArea {
    /// Distance within which airports are taken, in kilometres.
    pub radius_km: u16,
    /// How many frequencies, nearest airport first, the bank takes.
    pub max_channels: u16,
}

impl Default for AirportArea {
    fn default() -> Self {
        Self {
            radius_km: DEFAULT_RADIUS_KM,
            max_channels: DEFAULT_MAX_CHANNELS,
        }
    }
}

impl AirportArea {
    /// The state file the area is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(AREA_FILE))
    }

    /// Reads `RADIUS_KM MAX_CHANNELS`; anything missing or unreadable gives
    /// its default.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut words = text.split_whitespace();
        let mut number = |limit: u16, default: u16| {
            words
                .next()
                .and_then(|word| word.parse().ok())
                .filter(|value| (1..=limit).contains(value))
                .unwrap_or(default)
        };
        let radius_km = number(MAX_RADIUS_KM, DEFAULT_RADIUS_KM);
        let max_channels = number(MAX_CHANNELS_LIMIT, DEFAULT_MAX_CHANNELS);
        Self {
            radius_km,
            max_channels,
        }
    }

    /// Writes the area for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, format!("{} {}\n", self.radius_km, self.max_channels)))
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<AirportList, String>>,
}

impl SyncJob {
    /// Starts fetching from `OurAirports` into `cache`.
    pub fn start(cache: AirportCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        thread::spawn(move || {
            // The editor may have closed; then nobody wants the outcome.
            let _ = sender.send(cache.sync_with(fetch));
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<AirportList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the airport sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        airport_bank, carrier_hz, nearby, parse, AirportArea, AirportCache, AIRPORTS_URL,
        FREQUENCIES_URL,
    };
    use crate::model::ProjectModel;
    use radio_domain::Modulation;

    /// Rows in the shape `OurAirports` publishes, trimmed.
    const AIRPORTS: &str =
        "\"id\",\"ident\",\"type\",\"name\",\"latitude_deg\",\"longitude_deg\",\"elevation_ft\"
2473,\"EGSC\",\"medium_airport\",\"Cambridge City Airport\",52.205002,0.175,47
2398,\"EGCC\",\"large_airport\",\"Manchester Airport\",53.349375,-2.279521,257
9001,\"EGXX\",\"closed\",\"Old Field\",52.21,0.18,40
9002,\"EGYY\",\"small_airport\",\"Nearby Strip, Fen\",52.25,0.2,10
";
    const FREQUENCIES: &str =
        "\"id\",\"airport_ref\",\"airport_ident\",\"type\",\"description\",\"frequency_mhz\"
1,2473,\"EGSC\",\"TWR\",\"Cambridge Tower\",122.2
2,2473,\"EGSC\",\"ATIS\",\"Cambridge Information\",134.6
3,2473,\"EGSC\",\"RDR\",\"Cambridge Radar\",124.975
4,2473,\"EGSC\",\"NDB\",,332
5,2398,\"EGCC\",\"ATIS\",\"ATIS\",128.175
6,2398,\"EGCC\",\"GND\",\"GND\",121.935
7,9001,\"EGXX\",\"TWR\",,119.1
8,9002,\"EGYY\",\"A/G\",,122.2
9,9002,\"EGYY\",\"SAFETYCOM\",,135.48
10,9002,\"EGYY\",\"UNIC\",,123.07
";

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-air-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn listed_channel_names_become_carriers() {
        assert_eq!(carrier_hz("118.625"), Some(118_625_000));
        assert_eq!(carrier_hz("122.9"), Some(122_900_000));
        // 8.33 kHz channel names.
        assert_eq!(carrier_hz("121.905"), Some(121_900_000));
        assert_eq!(carrier_hz("121.935"), Some(121_933_330));
        assert_eq!(carrier_hz("121.940"), Some(121_941_670));
        // A 25 kHz frequency cut to two decimals.
        assert_eq!(carrier_hz("123.07"), Some(123_075_000));
        // Off both grids, outside the airband, or not a number.
        assert_eq!(carrier_hz("130.449"), None);
        assert_eq!(carrier_hz("120.0925"), None);
        assert_eq!(carrier_hz("113.75"), None);
        assert_eq!(carrier_hz("137.0"), None);
        assert_eq!(carrier_hz("243"), None);
        assert_eq!(carrier_hz(""), None);
    }

    #[test]
    fn the_list_keeps_airband_frequencies_at_open_airports() {
        let list = parse(AIRPORTS, FREQUENCIES).unwrap();
        let summary: Vec<_> = list
            .iter()
            .map(|f| (f.name.as_str(), f.frequency_hz))
            .collect();
        assert_eq!(
            summary,
            [
                ("EGCC GND", 121_933_330),
                ("EGCC ATIS", 128_175_000),
                ("EGSC TWR", 122_200_000),
                ("EGSC RDR", 124_975_000),
                ("EGSC ATIS", 134_600_000),
                ("EGYY A/G", 122_200_000),
                ("EGYY UNIC", 123_075_000),
                // A two-decimal cut that is also an 8.33 kHz name.
                ("EGYY SAFETYC", 135_475_000),
            ]
        );
        assert_eq!(list[2].airport, "Cambridge City Airport");
        assert!(parse(AIRPORTS, "\"id\"\n").is_err(), "no usable columns");
        assert!(
            parse("\"id\"\n", FREQUENCIES).is_err(),
            "no usable airport columns"
        );
        assert!(
            parse(AIRPORTS, FREQUENCIES.lines().next().unwrap()).is_err(),
            "no rows"
        );
    }

    #[test]
    fn nearby_takes_the_nearest_airport_for_each_frequency_in_range() {
        let list = parse(AIRPORTS, FREQUENCIES).unwrap();
        // JO02BF is the square holding Cambridge; Manchester is about 230 km.
        let near = nearby(&list, "JO02BF", 50).unwrap();
        let chosen: Vec<_> = near.iter().map(|(f, _)| f.name.as_str()).collect();
        // The strip's 122.2 is Cambridge Tower's, which is nearer.
        assert_eq!(
            chosen,
            [
                "EGSC TWR",
                "EGSC RDR",
                "EGSC ATIS",
                "EGYY UNIC",
                "EGYY SAFETYC"
            ]
        );
        assert!(near.windows(2).all(|pair| pair[0].1 <= pair[1].1));
        let far = nearby(&list, "JO02BF", 300).unwrap();
        assert_eq!(far.len(), 7);
        assert!(nearby(&list, "nonsense", 50).is_err());
    }

    #[test]
    fn the_bank_receives_am_and_never_transmits() {
        let list = parse(AIRPORTS, FREQUENCIES).unwrap();
        let near = nearby(&list, "jo02bf", 300).unwrap();
        let bank = airport_bank("jo02bf", &near);
        assert_eq!(bank.name, "AIR JO02BF");
        assert_eq!(bank.channels.len(), 7);
        assert_eq!(bank.channels[0].receive_mhz, "122.200000");
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Am && c.transmit_mhz == c.receive_mhz));
        assert!(bank.channels.iter().any(|c| c.receive_mhz == "121.933330"));
        // The radio accepts it.
        let project = ProjectModel {
            banks: vec![bank],
            ..ProjectModel::new()
        };
        project.to_radio_image().expect("a valid airband bank");
    }

    #[test]
    fn a_sync_caches_only_lists_that_parse() {
        let dir = scratch("sync");
        let cache = AirportCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none());
        let good = |url: &str| match url {
            AIRPORTS_URL => Ok(AIRPORTS.to_owned()),
            FREQUENCIES_URL => Ok(FREQUENCIES.to_owned()),
            other => Err(format!("unexpected {other}")),
        };
        assert_eq!(cache.sync_with(good).unwrap().frequencies.len(), 8);
        let bad = |url: &str| match url {
            AIRPORTS_URL => Ok(AIRPORTS.to_owned()),
            _ => Ok("not,a,list\n".to_owned()),
        };
        assert!(cache.sync_with(bad).is_err());
        assert_eq!(cache.load().unwrap().unwrap().frequencies.len(), 8);
        assert!(AirportCache::none().sync_with(good).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_area_comes_back_as_saved_and_defaults_otherwise() {
        let dir = scratch("area");
        let file = dir.join("airport-area");
        assert_eq!(AirportArea::load(&file), AirportArea::default());
        let area = AirportArea {
            radius_km: 75,
            max_channels: 12,
        };
        area.save(&file).unwrap();
        assert_eq!(AirportArea::load(&file), area);
        std::fs::write(&file, "0 9999\n").unwrap();
        assert_eq!(AirportArea::load(&file), AirportArea::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
