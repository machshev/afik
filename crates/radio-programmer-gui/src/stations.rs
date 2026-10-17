//! UK FM broadcast stations near a Maidenhead locator, synced from Ofcom's
//! technical parameters for VHF radio transmitters and cached.
//!
//! As with the repeaters, the studio fetches only when the operator presses
//! Sync; otherwise it reads the raw CSV cached in `$XDG_CACHE_HOME/afik/ofcom`.
//! The locator is the repeaters' own; how many stations to take, and of
//! which licence kinds, is kept in `$XDG_STATE_HOME/afik/station-area`.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::SystemTime,
};

use radio_channel_plan::{SharedSettings, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN};
use radio_domain::{Bandwidth, Modulation, PowerLevel, BROADCAST_FIRST_HZ, BROADCAST_LAST_HZ};

use crate::{
    model::{format_mhz, BankDraft, BankKind, ChannelDraft},
    paths,
    repeaters::{distance_km, fetch, locator_centre, sanitise},
};

/// Every VHF (FM) broadcast transmitter Ofcom licenses, with its frequency,
/// RDS name, power and position.
pub const TRANSMITTERS_URL: &str = "https://www.ofcom.org.uk/siteassets/resources/documents/\
spectrum/tv-transmitter-guidance/tech-parameters/txparamsvhf.csv";

const CACHE_DIR: &str = "ofcom";
const TRANSMITTERS_FILE: &str = "txparamsvhf.csv";
const AREA_FILE: &str = "station-area";
/// The word in an area file from before the kinds that kept the BBC.
const KEEP_BBC: &str = "bbc";
/// What leads the word in the area file naming the kinds taken.
const KINDS_PREFIX: &str = "kinds=";
/// How many stations a bank takes before the operator chooses.
pub const DEFAULT_MAX_CHANNELS: u16 = 30;
/// The most the operator can ask for; a whole region rarely has more.
pub const MAX_CHANNELS_LIMIT: u16 = 200;
/// The raster broadcast channels sit on.
const RASTER_HZ: u32 = 100_000;
/// A handheld's aerial height, for the radio horizon.
const RECEIVER_HEIGHT_M: f64 = 2.0;

/// One transmitter of one station.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    /// Channel name: the RDS name with underscores as spaces, else the
    /// station's name, upper case and at most a channel name long.
    pub name: String,
    /// Station name as Ofcom lists it, such as `BBC Radio 4 (FM)`.
    pub station: String,
    /// Transmitter site, such as `Wrotham`.
    pub site: String,
    /// Carrier in hertz, on the 100 kHz raster.
    pub frequency_hz: u32,
    /// In-use effective radiated power, the larger polarisation, in watts.
    pub erp_watts: f64,
    /// Aerial height above sea level, site plus mast, in metres, where the
    /// list gives it.
    pub height_m: Option<f64>,
    /// Site latitude in degrees north.
    pub latitude: f64,
    /// Site longitude in degrees east.
    pub longitude: f64,
    /// The kind of licence, which the operator filters on.
    pub kind: StationKind,
}

/// The kind of licence a station broadcasts under, from the prefix of its
/// Ofcom licence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationKind {
    /// `BBC...`, or `BBC` anywhere in the station's or channel's name.
    Bbc,
    /// `AN...`, national commercial, such as Classic FM.
    National,
    /// `AL...`, local commercial, such as Capital or Heart.
    Local,
    /// `CR...`, community radio.
    Community,
    /// `LRSL...`, low-power and restricted service: forces bases, hospital
    /// radio, events.
    Restricted,
    /// Anything else: the Isle of Man, Gibraltar, or no licence given.
    Other,
}

impl StationKind {
    /// Every kind, in the order the checkboxes show them.
    pub const ALL: [Self; 6] = [
        Self::Bbc,
        Self::National,
        Self::Local,
        Self::Community,
        Self::Restricted,
        Self::Other,
    ];

    /// The position of this kind in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The checkbox label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bbc => "BBC",
            Self::National => "National",
            Self::Local => "Local",
            Self::Community => "Community",
            Self::Restricted => "Restricted",
            Self::Other => "Other",
        }
    }

    /// What the checkbox takes, for its hover text.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Bbc => "BBC stations, and any station with BBC in its name",
            Self::National => "national commercial stations, such as Classic FM",
            Self::Local => "local commercial stations, such as Capital and Heart",
            Self::Community => "community radio stations",
            Self::Restricted => {
                "low-power and restricted service stations: forces bases, hospital radio, events"
            }
            Self::Other => "stations licensed elsewhere: the Isle of Man and Gibraltar",
        }
    }

    /// The word the area file keeps the kind under.
    const fn word(self) -> &'static str {
        match self {
            Self::Bbc => "bbc",
            Self::National => "national",
            Self::Local => "local",
            Self::Community => "community",
            Self::Restricted => "restricted",
            Self::Other => "other",
        }
    }

    /// The kind for an Ofcom licence number and the station's names. The
    /// BBC is matched broadly on purpose: the operator asked for anything
    /// named BBC to go with it.
    fn of(licence: &str, names: &[&str]) -> Self {
        let licence = licence.trim().to_ascii_uppercase();
        if licence.starts_with("BBC")
            || names
                .iter()
                .any(|name| name.to_ascii_uppercase().contains("BBC"))
        {
            return Self::Bbc;
        }
        let prefix: String = licence
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        match prefix.as_str() {
            "AN" => Self::National,
            "AL" => Self::Local,
            "CR" => Self::Community,
            "LRSL" => Self::Restricted,
            _ => Self::Other,
        }
    }
}

/// Splits CSV text into records, honouring quoted fields with doubled
/// quotes and line breaks inside them.
pub(crate) fn records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (_, c) => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// A number as Ofcom writes it; power is in kilowatts with a comma before
/// the last three decimals, such as `124.999,996`.
fn number(field: &str) -> Option<f64> {
    field.replace(',', "").trim().parse().ok()
}

/// Picks the transmitters the radio can play out of Ofcom's VHF CSV.
///
/// A row is taken when its frequency is inside the broadcast receiver's band
/// and on the 100 kHz raster, and it radiates some power. Columns are found
/// by their headings, so a reordered file still reads.
pub fn parse(csv: &str) -> Result<Vec<Station>, String> {
    let mut rows = records(csv).into_iter();
    let heading = rows.next().ok_or("Ofcom VHF list: empty")?;
    let column = |name: &str| {
        heading
            .iter()
            .position(|heading| heading.trim().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("Ofcom VHF list: no {name:?} column"))
    };
    let station_at = column("Station")?;
    let site_at = column("Site")?;
    let frequency_at = column("Frequency")?;
    let rds_at = column("RDS PS")?;
    let horizontal_at = column("In-UseERP/HP")?;
    let vertical_at = column("In-Use ERP/VP")?;
    let latitude_at = column("Lat")?;
    let longitude_at = column("Long")?;
    // Only the kind filter reads it; without it the BBC still matches by
    // name and the rest are Other.
    let licence_at = column("Licence").ok();
    // Heights only sharpen the ranking; a list without them still reads.
    let site_height_at = column("Site Ht").ok();
    let aerial_height_at = column("In-Use Aerial Ht").ok();
    let mut stations: Vec<Station> = rows
        .filter_map(|row| {
            let field = |at: usize| row.get(at).map_or("", String::as_str);
            // Rounded to the raster, then checked to have been on it.
            let mhz = number(field(frequency_at))?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let frequency_hz = ((mhz * 10.0).round() as u32).checked_mul(RASTER_HZ)?;
            if (f64::from(frequency_hz) - mhz * 1e6).abs() > 1.0
                || !(BROADCAST_FIRST_HZ..=BROADCAST_LAST_HZ).contains(&frequency_hz)
            {
                return None;
            }
            let kilowatts = number(field(horizontal_at))
                .unwrap_or(0.0)
                .max(number(field(vertical_at)).unwrap_or(0.0));
            if kilowatts <= 0.0 {
                return None;
            }
            let station = field(station_at).trim().to_owned();
            // RDS pads with underscores; runs of them read as one space.
            let rds = field(rds_at).replace('_', " ");
            let rds = rds.split_whitespace().collect::<Vec<_>>().join(" ");
            let name = sanitise(&rds, MAX_CHANNEL_NAME_LEN)
                .or_else(|| sanitise(&station, MAX_CHANNEL_NAME_LEN))?;
            let kind = StationKind::of(licence_at.map_or("", field), &[&station, &name]);
            Some(Station {
                name,
                station,
                site: field(site_at).trim().to_owned(),
                frequency_hz,
                erp_watts: kilowatts * 1_000.0,
                height_m: site_height_at
                    .zip(aerial_height_at)
                    .and_then(|(site, aerial)| Some(number(field(site))? + number(field(aerial))?)),
                latitude: number(field(latitude_at))?,
                longitude: number(field(longitude_at))?,
                kind,
            })
        })
        .collect();
    if stations.is_empty() {
        return Err("Ofcom VHF list: no transmitters the radio can play".to_owned());
    }
    stations.sort_by(|a, b| {
        a.station
            .cmp(&b.station)
            .then(a.frequency_hz.cmp(&b.frequency_hz))
    });
    Ok(stations)
}

/// The estimated strength of `station` at `km`: power over distance squared,
/// falling a further fourth power of distance beyond the radio horizon, so a
/// big transmitter far past it ranks below a small local one.
fn strength(station: &Station, km: f64) -> f64 {
    let near = station.erp_watts / km.max(1.0).powi(2);
    let Some(height_m) = station.height_m else {
        return near;
    };
    // The 4/3-earth radio horizon, in kilometres, for heights in metres.
    let horizon_km = 4.12 * (height_m.max(0.0).sqrt() + RECEIVER_HEIGHT_M.sqrt());
    if km <= horizon_km {
        near
    } else {
        near * (horizon_km / km).powi(4)
    }
}

/// Every station as heard at `locator`, strongest first, each with its
/// transmitter's distance in kilometres.
///
/// Strength is [`strength`], with no distance cut-off: a cut-off dropped a
/// 122 kW main transmitter at 57 km and kept 130 W relays at 25 km. Each
/// station keeps only its strongest transmitter, and each frequency only its
/// strongest station, so a bank holds one channel per programme.
pub fn strongest<'a>(
    stations: impl IntoIterator<Item = &'a Station>,
    locator: &str,
) -> Result<Vec<(&'a Station, f64)>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<_> = stations
        .into_iter()
        .map(|station| {
            let km = distance_km(home, (station.latitude, station.longitude));
            (station, km)
        })
        .collect();
    near.sort_by(|a, b| {
        strength(b.0, b.1)
            .total_cmp(&strength(a.0, a.1))
            .then(a.0.station.cmp(&b.0.station))
    });
    let mut programmes = HashSet::new();
    let mut frequencies = HashSet::new();
    near.retain(|(station, _)| {
        programmes.insert(station.station.as_str()) && frequencies.insert(station.frequency_hz)
    });
    Ok(near)
}

/// Builds the bank for the stations `strongest` chose, in frequency order, each
/// played by the broadcast receiver and never transmitted (`ADR-105`).
pub fn station_bank(locator: &str, near: &[(&Station, f64)]) -> BankDraft {
    let mut name = format!("FM {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
    let mut chosen: Vec<&Station> = near.iter().map(|(station, _)| *station).collect();
    chosen.sort_by_key(|station| station.frequency_hz);
    let channels = chosen
        .iter()
        .map(|station| ChannelDraft {
            name: station.name.clone(),
            receive_mhz: format_mhz(station.frequency_hz),
            // Receive only: a station has no transmit side.
            transmit_mhz: String::new(),
            modulation: Modulation::Broadcast,
            bandwidth: Bandwidth::Wide,
            power: PowerLevel::Low1,
            step_hz: RASTER_HZ,
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
        modulation: Modulation::Broadcast,
        bandwidth: Bandwidth::Wide,
        power: PowerLevel::Low1,
        step_hz: RASTER_HZ,
        channels,
        ..BankDraft::default()
    }
}

/// The transmitters a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct StationList {
    /// Every transmitter the radio can play, ordered by station.
    pub stations: Vec<Station>,
    /// When the cache was last written.
    pub synced: SystemTime,
}

/// Where the raw Ofcom CSV is kept.
#[derive(Clone, Debug)]
pub struct StationCache {
    dir: Option<PathBuf>,
}

impl StationCache {
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
    pub fn load(&self) -> Result<Option<StationList>, String> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let path = dir.join(TRANSMITTERS_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let failed = |error: std::io::Error| format!("{}: {error}", path.display());
        let bytes = fs::read(&path).map_err(failed)?;
        let synced = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .map_err(failed)?;
        Ok(Some(StationList {
            stations: parse(&String::from_utf8_lossy(&bytes))?,
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
    ) -> Result<StationList, String> {
        let dir = self
            .dir
            .as_ref()
            .ok_or("no XDG cache directory or HOME is set")?;
        let csv = fetch(TRANSMITTERS_URL)?;
        parse(&csv)?;
        let path = dir.join(TRANSMITTERS_FILE);
        let partial = path.with_extension("csv.partial");
        fs::create_dir_all(dir)
            .and_then(|()| fs::write(&partial, &csv))
            .and_then(|()| fs::rename(&partial, &path))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        self.load()?
            .ok_or_else(|| "the station cache was not written".to_owned())
    }
}

/// The station count the operator last chose, and the licence kinds taken;
/// the locator is the repeaters' own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationArea {
    /// How many stations, strongest first, the bank takes.
    pub max_channels: u16,
    /// Whether each [`StationKind`] is taken, by [`StationKind::index`].
    pub kinds: [bool; StationKind::ALL.len()],
}

impl Default for StationArea {
    fn default() -> Self {
        Self {
            max_channels: DEFAULT_MAX_CHANNELS,
            kinds: Self::NO_BBC,
        }
    }
}

impl StationArea {
    /// Every kind but the BBC, the default.
    pub const NO_BBC: [bool; StationKind::ALL.len()] = [false, true, true, true, true, true];

    /// Whether `station` is of a kind taken.
    pub const fn takes(&self, station: &Station) -> bool {
        self.kinds[station.kind.index()]
    }

    /// The state file the area is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(AREA_FILE))
    }

    /// Reads `MAX_CHANNELS`, the last number, so a file from when a radius
    /// came first still reads, and the kinds from a `kinds=` word. A file
    /// from before the kinds takes every kind for a `bbc` word, else all but
    /// the BBC; anything missing or unreadable gives the default.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let max_channels = text
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .next_back()
            .filter(|value| (1..=MAX_CHANNELS_LIMIT).contains(value))
            .unwrap_or(DEFAULT_MAX_CHANNELS);
        let words = || text.split_whitespace();
        let kinds = if let Some(list) = words().find_map(|word| word.strip_prefix(KINDS_PREFIX)) {
            let taken: Vec<_> = list.split(',').collect();
            StationKind::ALL.map(|kind| taken.contains(&kind.word()))
        } else if words().any(|word| word == KEEP_BBC) {
            [true; StationKind::ALL.len()]
        } else {
            Self::NO_BBC
        };
        Self {
            max_channels,
            kinds,
        }
    }

    /// Writes the area for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                let kinds: Vec<_> = StationKind::ALL
                    .into_iter()
                    .filter(|kind| self.kinds[kind.index()])
                    .map(StationKind::word)
                    .collect();
                fs::write(
                    file,
                    format!("{KINDS_PREFIX}{}\n{}\n", kinds.join(","), self.max_channels),
                )
            })
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<StationList, String>>,
}

impl SyncJob {
    /// Starts fetching from Ofcom into `cache`.
    pub fn start(cache: StationCache) -> Self {
        let (sender, outcome) = mpsc::channel();
        thread::spawn(move || {
            // The editor may have closed; then nobody wants the outcome.
            let _ = sender.send(cache.sync_with(fetch));
        });
        Self { outcome }
    }

    /// Returns the outcome once the worker has finished.
    pub fn poll(&self) -> Option<Result<StationList, String>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the station sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse, records, station_bank, strongest, StationArea, StationCache, StationKind,
        TRANSMITTERS_URL,
    };
    use crate::model::ProjectModel;
    use radio_domain::Modulation;

    /// Rows in the shape Ofcom publishes, trimmed to the columns used plus
    /// a few it has around them.
    const VHF: &str = "Date,Station,Area,Site,Licence ,Frequency,NGR,RDS PS,RDS PI,\
Site Ht,In-Use Aerial Ht,In-UseERP/HP,In-Use ERP/VP,Lat,Long,\r
01/01/2020,BBC Radio 4 (FM),London,Wrotham,BBC000004,93.5,TQ,_BBC_R4_,C204,219,150,\"124.999,996\",\"124.999,996\",51.320705,0.287703,\r
01/01/2020,BBC Radio 4 (FM),London,Crystal Palace,BBC000004,93.2,TQ,_BBC_R4_,C204,110,170,\"0.000,000\",\"4.000,000\",51.424171,-0.074944,\r
01/01/2020,Classic FM,London,Wrotham,AN000001,100.9,TQ,Classic_,C2,219,150,\"124.999,996\",\"124.999,996\",51.320705,0.287703,\r
01/01/2020,\"Rainbow Radio, London\",London,Crystal Palace,AL000261,92.4,TQ,RAINBOW,C3,110,40,\"0.030,000\",\"0.030,000\",51.424171,-0.074944,\r
01/01/2020,Local Two,Kent,Wrotham,CR3,92.4,TQ,,C4,219,20,\"0.010,000\",\"0.010,000\",51.320705,0.287703,\r
01/01/2020,Off Air,London,Crystal Palace,CR4,95.0,TQ,OFF,C5,110,40,\"0.000,000\",\"0.000,000\",51.424171,-0.074944,\r
01/01/2020,Radio Scotland,Glasgow,Black Hill,BBC000041,94.3,NS,_BBC__SCOT_,C6,275,200,\"125.000,000\",\"125.000,000\",55.861,-3.873,\r
01/01/2020,Odd Raster,London,Crystal Palace,CR5,95.05,TQ,ODD,C7,110,40,\"1.000,000\",\"1.000,000\",51.424171,-0.074944,\r
";

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-ofcom-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn csv_fields_may_be_quoted_with_commas_and_quotes_inside() {
        assert_eq!(
            records("a,\"b,c\",\"say \"\"hi\"\"\"\r\nd,e,\n"),
            [vec!["a", "b,c", "say \"hi\""], vec!["d", "e", ""],]
        );
    }

    #[test]
    fn the_list_keeps_transmitters_the_broadcast_receiver_can_play() {
        let stations = parse(VHF).unwrap();
        let summary: Vec<_> = stations
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.site.as_str(),
                    s.frequency_hz,
                    s.erp_watts,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("BBC R4", "Crystal Palace", 93_200_000, 4_000.0),
                ("BBC R4", "Wrotham", 93_500_000, 124_999.996),
                ("CLASSIC", "Wrotham", 100_900_000, 124_999.996),
                ("LOCAL TWO", "Wrotham", 92_400_000, 10.0),
                ("BBC SCOT", "Black Hill", 94_300_000, 125_000.0),
                ("RAINBOW", "Crystal Palace", 92_400_000, 30.0),
            ]
        );
        let kinds: Vec<_> = stations.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("BBC R4", StationKind::Bbc),
                ("BBC R4", StationKind::Bbc),
                ("CLASSIC", StationKind::National),
                ("LOCAL TWO", StationKind::Community),
                ("BBC SCOT", StationKind::Bbc),
                ("RAINBOW", StationKind::Local),
            ],
            "by the licence column"
        );
        assert!(parse("Station\n").is_err(), "no usable columns");
        assert!(parse(VHF.lines().next().unwrap()).is_err(), "no rows");
    }

    #[test]
    fn the_kind_is_the_licence_prefix_and_bbc_also_by_name() {
        let csv = "Station,Site,Licence,Frequency,RDS PS,In-UseERP/HP,In-Use ERP/VP,Lat,Long\n\
Plain,A,AL000001,90.0,PLAIN,1,1,51,0\n\
Licensed,A,BBC000002,90.1,LICENSED,1,1,51,0\n\
Not the BBC,A,CR000003,90.2,NOT,1,1,51,0\n\
Quiet,A,CR000004,90.3,_bbcfan_,1,1,51,0\n\
Forces,A,LRSL000005,90.4,FORCES,1,1,51,0\n\
Manx,A,IOM000006,90.5,MANX,1,1,51,0\n\
Rock,A,,90.6,ROCK,1,1,51,0\n\
Classic,A,AN000001,90.7,CLASSIC,1,1,51,0\n";
        let kinds: Vec<_> = parse(csv)
            .unwrap()
            .iter()
            .map(|s| (s.station.clone(), s.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Classic".to_owned(), StationKind::National),
                ("Forces".to_owned(), StationKind::Restricted),
                ("Licensed".to_owned(), StationKind::Bbc),
                ("Manx".to_owned(), StationKind::Other),
                ("Not the BBC".to_owned(), StationKind::Bbc),
                ("Plain".to_owned(), StationKind::Local),
                ("Quiet".to_owned(), StationKind::Bbc),
                ("Rock".to_owned(), StationKind::Other),
            ]
        );
    }

    #[test]
    fn each_programme_and_frequency_keeps_its_strongest_transmitter() {
        let stations = parse(VHF).unwrap();
        // IO91WM, central London: Crystal Palace about 11 km, Wrotham 36 km,
        // Black Hill in Scotland about 550 km.
        let near = strongest(&stations, "IO91WM").unwrap();
        let chosen: Vec<_> = near
            .iter()
            .map(|(s, _)| (s.name.as_str(), s.frequency_hz))
            .collect();
        assert_eq!(
            chosen,
            [
                // 125 kW at 36 km beats 4 kW at 11 km.
                ("BBC R4", 93_500_000),
                ("CLASSIC", 100_900_000),
                // 92.4 MHz goes to the stronger of the two stations on it.
                ("RAINBOW", 92_400_000),
                // No distance cut-off: a far station still ranks, but past
                // the radio horizon it falls below even 30 W nearby.
                ("BBC SCOT", 94_300_000),
            ]
        );
        // Without heights the ranking is power over distance squared, and
        // 125 kW at 550 km would beat 30 W at 11 km.
        let flat: Vec<_> = stations
            .iter()
            .map(|s| super::Station {
                height_m: None,
                ..s.clone()
            })
            .collect();
        let names: Vec<_> = strongest(&flat, "IO91WM")
            .unwrap()
            .iter()
            .map(|(s, _)| s.name.clone())
            .collect();
        assert_eq!(names, ["BBC R4", "CLASSIC", "BBC SCOT", "RAINBOW"]);
        assert!(strongest(&stations, "nonsense").is_err());
    }

    #[test]
    fn the_bank_plays_each_station_in_frequency_order_and_never_transmits() {
        let stations = parse(VHF).unwrap();
        let mut near = strongest(&stations, "io91wm").unwrap();
        near.truncate(3);
        let bank = station_bank("io91wm", &near);
        assert_eq!(bank.name, "FM IO91WM");
        let channels: Vec<_> = bank
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.receive_mhz.as_str()))
            .collect();
        assert_eq!(
            channels,
            [
                ("RAINBOW", "92.400000"),
                ("BBC R4", "93.500000"),
                ("CLASSIC", "100.900000"),
            ]
        );
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Broadcast && c.transmit_mhz.is_empty()));
        // The radio accepts it.
        let project = ProjectModel {
            banks: vec![bank],
            ..ProjectModel::new()
        };
        project.to_radio_image().expect("a valid broadcast bank");
    }

    #[test]
    fn a_sync_caches_only_a_list_that_parses() {
        let dir = scratch("sync");
        let cache = StationCache::in_dir(dir.clone());
        assert!(cache.load().unwrap().is_none());
        let list = cache
            .sync_with(|url| {
                assert_eq!(url, TRANSMITTERS_URL);
                Ok(VHF.to_owned())
            })
            .unwrap();
        assert_eq!(list.stations.len(), 6);
        assert!(cache.sync_with(|_| Ok("not,a,list\n".to_owned())).is_err());
        assert_eq!(cache.load().unwrap().unwrap().stations.len(), 6);
        assert!(StationCache::none()
            .sync_with(|_| Ok(VHF.to_owned()))
            .is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_area_comes_back_as_saved_and_defaults_otherwise() {
        let dir = scratch("area");
        let file = dir.join("station-area");
        assert_eq!(StationArea::load(&file), StationArea::default());
        for kinds in [
            [true; 6],
            [false; 6],
            [true, false, true, false, false, true],
        ] {
            let area = StationArea {
                max_channels: 12,
                kinds,
            };
            area.save(&file).unwrap();
            assert_eq!(StationArea::load(&file), area);
        }
        // A file from when the radius came first keeps its count, and one
        // from before the kinds leaves the BBC out unless it kept it.
        std::fs::write(&file, "50 30\n").unwrap();
        assert_eq!(StationArea::load(&file).max_channels, 30);
        assert_eq!(StationArea::load(&file).kinds, StationArea::NO_BBC);
        std::fs::write(&file, "bbc\n30\n").unwrap();
        assert_eq!(StationArea::load(&file).kinds, [true; 6]);
        std::fs::write(&file, "0 9999\n").unwrap();
        assert_eq!(StationArea::load(&file), StationArea::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
