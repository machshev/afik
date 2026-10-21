//! FM broadcast stations near a Maidenhead locator, synced from the
//! regulators' transmitter lists and cached: Ofcom's technical parameters for
//! VHF radio transmitters in the UK, the FCC's FM Query in the US and ISED's
//! broadcast database in Canada; Israel's regional stations are built in.
//!
//! As with the repeaters, the studio fetches only when the operator presses
//! Sync or nothing is cached (`ADR-121`); otherwise it reads each raw list
//! cached in `$XDG_CACHE_HOME/afik/<source>`. The locator is the repeaters'
//! own; how many stations to take, and of which licence kinds, is kept in
//! `$XDG_STATE_HOME/afik/station-area`.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::SystemTime,
};

use radio_channel_plan::{SharedSettings, MAX_CHANNEL_NAME_LEN};
use radio_domain::{Bandwidth, Modulation, PowerLevel, BROADCAST_FIRST_HZ, BROADCAST_LAST_HZ};

#[cfg(not(target_arch = "wasm32"))]
use crate::repeaters::fetch_bytes_within;
use crate::{
    model::{format_mhz, BankDraft, BankKind, ChannelDraft},
    paths,
    receiver::Receiver,
    repeaters::{distance_km, locator_centre, sanitise},
};

/// Every VHF (FM) broadcast transmitter Ofcom licenses, with its frequency,
/// RDS name, power and position.
pub const TRANSMITTERS_URL: &str = "https://www.ofcom.org.uk/siteassets/resources/documents/\
spectrum/tv-transmitter-guidance/tech-parameters/txparamsvhf.csv";

/// Every FM record the FCC holds, one line each with `|` between fields
/// (`list=4`), vacant allotments and applications included (`vac=3`).
pub const FCC_URL: &str =
    "https://transition.fcc.gov/fcc-bin/fmq?freq=87.5&fre2=108.0&vac=3&list=4";

/// ISED's broadcast database: a zip of dBASE tables, about 27 MB, of which
/// `fmstatio.dbf` holds every FM station and allotment.
pub const ISED_URL: &str = "https://www.ic.gc.ca/engineering/BC_DBF_FILES/baserad.zip";
/// The table of FM stations in ISED's zip.
const ISED_FM_TABLE: &str = "fmstatio.dbf";
/// The provinces and territories, which ISED's list mixes with US states.
const CANADA: [&str; 13] = [
    "AB", "BC", "MB", "NB", "NL", "NS", "NT", "NU", "ON", "PE", "QC", "SK", "YT",
];

const AREA_FILE: &str = "station-area";
/// The word in an area file from before the kinds that kept the BBC.
const KEEP_BBC: &str = "bbc";
/// What leads the word in an area file from before the US kinds naming the
/// kinds taken.
const KINDS_PREFIX: &str = "kinds=";
/// What leads the word in the area file naming the kinds left out.
const OFF_PREFIX: &str = "off=";
/// How many stations a bank takes before the operator chooses.
pub const DEFAULT_MAX_CHANNELS: u16 = 30;
/// The most the operator can ask for; a whole region rarely has more.
pub const MAX_CHANNELS_LIMIT: u16 = 200;
/// The raster broadcast channels sit on.
const RASTER_HZ: u32 = 100_000;
/// No transmitter further than this is offered. It is far past any FM
/// station's horizon, and keeps one country's list out of another's bank.
const MAX_KM: f64 = 1_000.0;
/// How long the FCC's list, about 18 MB, or ISED's zip, about 27 MB, may
/// take to arrive.
#[cfg(not(target_arch = "wasm32"))]
const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Where a list of transmitters comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Ofcom, the UK.
    Ofcom,
    /// The FCC, the US and its territories.
    Fcc,
    /// ISED, Canada.
    Ised,
}

impl Source {
    /// Every source, each cached in its own directory.
    pub const ALL: [Self; 3] = [Self::Ofcom, Self::Fcc, Self::Ised];

    /// The directory the source is cached and mirrored in.
    pub const fn cache_dir(self) -> &'static str {
        match self {
            Self::Ofcom => "ofcom",
            Self::Fcc => "fcc",
            Self::Ised => "ised",
        }
    }

    /// The file the list is kept in: as fetched, except ISED's FM table,
    /// kept as CSV out of its zip.
    pub const fn file(self) -> &'static str {
        match self {
            Self::Ofcom => "txparamsvhf.csv",
            Self::Fcc => "fmq.txt",
            Self::Ised => "fmstatio.csv",
        }
    }

    /// Where the list is fetched from.
    pub const fn url(self) -> &'static str {
        match self {
            Self::Ofcom => TRANSMITTERS_URL,
            Self::Fcc => FCC_URL,
            Self::Ised => ISED_URL,
        }
    }

    /// What is kept of a fetched `body`: the text, except that ISED's FM
    /// table is taken out of its zip as CSV. Text that is not UTF-8 is
    /// replaced rather than refused.
    pub fn prepare(self, body: &[u8]) -> Result<String, String> {
        match self {
            Self::Ofcom | Self::Fcc => Ok(String::from_utf8_lossy(body).into_owned()),
            Self::Ised => crate::archive::dbf_to_csv(&crate::archive::unzip(body, ISED_FM_TABLE)?),
        }
    }

    /// Reads the list as kept.
    pub fn parse(self, text: &str) -> Result<Vec<Station>, String> {
        match self {
            Self::Ofcom => parse(text),
            Self::Fcc => parse_fcc(text),
            Self::Ised => parse_ised(text),
        }
    }
}

/// One transmitter of one station.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    /// Channel name: the RDS name with underscores as spaces, else the
    /// station's name, upper case and at most a channel name long; in the US
    /// the call sign.
    pub name: String,
    /// The programme: the station name as Ofcom lists it, such as `BBC Radio
    /// 4 (FM)`, or a US call sign without its `-FM` suffix, which a station's
    /// boosters share.
    pub station: String,
    /// Transmitter site, such as `Wrotham`, or a US licence's city and
    /// state.
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

/// The kind of licence a station broadcasts under: in the UK from the prefix
/// of its Ofcom licence number, in the US from the FCC's service code.
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
    /// US `FM`: a full-power station.
    FullPower,
    /// US `FX`: a translator, relaying another station on its own
    /// frequency.
    Translator,
    /// US `FL`: a low-power (LPFM) community station.
    LowPower,
    /// US `FB`: a booster, filling in its station's area on its frequency.
    Booster,
    /// An Israeli regional station, franchised by the Second Authority.
    Regional,
    /// Canada's public broadcaster, CBC/Radio-Canada: a `CB` call sign.
    Cbc,
}

impl StationKind {
    /// Every kind, in the order the checkboxes show them.
    pub const ALL: [Self; 12] = [
        Self::Bbc,
        Self::National,
        Self::Local,
        Self::Community,
        Self::Restricted,
        Self::Other,
        Self::FullPower,
        Self::Translator,
        Self::LowPower,
        Self::Booster,
        Self::Regional,
        Self::Cbc,
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
            Self::FullPower => "Full power",
            Self::Translator => "Translator",
            Self::LowPower => "Low power",
            Self::Booster => "Booster",
            Self::Regional => "Regional",
            Self::Cbc => "CBC",
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
            Self::FullPower => "full-power US and Canadian stations",
            Self::Translator => {
                "US translators, each relaying another station on its own frequency"
            }
            Self::LowPower => {
                "low-power stations: US LPFM community stations, Canadian LP and VLP classes"
            }
            Self::Booster => "US boosters, each filling in its station's area on its frequency",
            Self::Regional => {
                "Israeli regional stations the Second Authority franchises, placed at their \
                 area's main town"
            }
            Self::Cbc => "CBC/Radio-Canada stations and their rebroadcasters",
        }
    }

    /// Whether the kind is one of Ofcom's, the only ones before the US.
    const fn is_uk(self) -> bool {
        matches!(
            self,
            Self::Bbc
                | Self::National
                | Self::Local
                | Self::Community
                | Self::Restricted
                | Self::Other
        )
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
            Self::FullPower => "full-power",
            Self::Translator => "translator",
            Self::LowPower => "low-power",
            Self::Booster => "booster",
            Self::Regional => "il-regional",
            Self::Cbc => "cbc",
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
            let frequency_hz = playable(number(field(frequency_at))?)?;
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

/// The carrier for `mhz` when the broadcast receiver can play it: inside its
/// band and on the 100 kHz raster.
fn playable(mhz: f64) -> Option<u32> {
    // Rounded to the raster, then checked to have been on it.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frequency_hz = ((mhz * 10.0).round() as u32).checked_mul(RASTER_HZ)?;
    ((f64::from(frequency_hz) - mhz * 1e6).abs() <= 1.0
        && (BROADCAST_FIRST_HZ..=BROADCAST_LAST_HZ).contains(&frequency_hz))
    .then_some(frequency_hz)
}

/// Picks the transmitters on air in the US out of the FCC's FM Query list.
///
/// The list has no heading; each line is `|`, then each field padded and
/// followed by `|`, in the order of the FCC's key to its text lists. A line
/// is taken when it is licensed (`LIC`) in the US, for a full-power station,
/// translator, low-power station or booster, and it radiates some power on a
/// frequency the radio can play. Allotments, applications, permits not yet
/// built, auxiliary transmitters and Canadian and Mexican records are left
/// out.
pub fn parse_fcc(text: &str) -> Result<Vec<Station>, String> {
    // Field positions, counting the empty one before the leading `|`.
    const CALL: usize = 1;
    const FREQUENCY: usize = 2;
    const SERVICE: usize = 3;
    const STATUS: usize = 9;
    const CITY: usize = 10;
    const STATE: usize = 11;
    const COUNTRY: usize = 12;
    const ERP_HORIZONTAL: usize = 14;
    const ERP_VERTICAL: usize = 15;
    const LATITUDE: usize = 19;
    const LONGITUDE: usize = 23;
    const HEIGHT_HORIZONTAL: usize = 31;
    const HEIGHT_VERTICAL: usize = 32;
    // A number followed by its unit, such as `88.5  MHz` or `226.   m`.
    let number = |field: &str| {
        field
            .trim()
            .trim_end_matches(char::is_alphabetic)
            .trim()
            .parse::<f64>()
            .ok()
    };
    let larger = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let mut stations: Vec<Station> = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('|').map(str::trim).collect();
            let field = |at: usize| fields.get(at).copied().unwrap_or("");
            let kind = match field(SERVICE) {
                "FM" => StationKind::FullPower,
                "FX" => StationKind::Translator,
                "FL" => StationKind::LowPower,
                "FB" => StationKind::Booster,
                _ => return None,
            };
            if field(STATUS) != "LIC" || field(COUNTRY) != "US" {
                return None;
            }
            let frequency_hz = playable(number(field(FREQUENCY))?)?;
            let kilowatts = larger(number(field(ERP_HORIZONTAL)), number(field(ERP_VERTICAL)))?;
            if kilowatts <= 0.0 {
                return None;
            }
            // Degrees, minutes and seconds after the hemisphere.
            let angle = |at: usize, negative: &str| {
                let degrees = number(field(at + 1))?
                    + number(field(at + 2))? / 60.0
                    + number(field(at + 3))? / 3_600.0;
                Some(if field(at) == negative {
                    -degrees
                } else {
                    degrees
                })
            };
            let call = field(CALL);
            // A booster is its station's call with `-FM1` and so on, and
            // shares the station's programme.
            let station = match call.find("-FM") {
                Some(at) => &call[..at],
                None => call,
            };
            Some(Station {
                name: sanitise(station, MAX_CHANNEL_NAME_LEN)?,
                station: station.to_owned(),
                site: format!("{}, {}", field(CITY), field(STATE)),
                frequency_hz,
                erp_watts: kilowatts * 1_000.0,
                height_m: larger(
                    number(field(HEIGHT_HORIZONTAL)),
                    number(field(HEIGHT_VERTICAL)),
                ),
                latitude: angle(LATITUDE, "S")?,
                longitude: angle(LONGITUDE, "W")?,
                kind,
            })
        })
        .collect();
    if stations.is_empty() {
        return Err("FCC FM list: no transmitters the radio can play".to_owned());
    }
    stations.sort_by(|a, b| {
        a.station
            .cmp(&b.station)
            .then(a.frequency_hz.cmp(&b.frequency_hz))
    });
    Ok(stations)
}

/// Picks the stations on air in Canada out of ISED's FM table, as
/// [`Source::prepare`] keeps it.
///
/// A row is taken when its province is Canadian, its banner (status) is
/// `OP`, operating, and it radiates some power on a frequency the radio can
/// play; allotments, applications, the US and French records ISED keeps
/// for coordination, and the HD Radio records (a `-HD` call sign, the
/// digital service of a station whose `-FM` record is there too) are left
/// out. Positions are `DDMMSS` north and
/// `DDDMMSS` west; `RAD_CENTER` is the aerial's height above sea level. A
/// `CB` call sign is CBC/Radio-Canada's, `LP` and `VLP` classes low power.
pub fn parse_ised(csv: &str) -> Result<Vec<Station>, String> {
    let mut rows = records(csv).into_iter();
    let heading = rows.next().ok_or("ISED FM table: empty")?;
    let column = |name: &str| {
        heading
            .iter()
            .position(|heading| heading.trim().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("ISED FM table: no {name:?} column"))
    };
    let province_at = column("PROVINCE")?;
    let city_at = column("CITY")?;
    let call_at = column("CALL_SIGN")?;
    let frequency_at = column("FREQUENCY")?;
    let class_at = column("CLASS")?;
    let latitude_at = column("LATITUDE")?;
    let longitude_at = column("LONGITUDE")?;
    let banner_at = column("BANNER")?;
    let power_at = [column("ERPHAV")?, column("ERPVAV")?];
    let height_at = column("RAD_CENTER").ok();
    // Degrees, minutes and seconds run together, as `DDMMSS`.
    let angle = |field: &str| {
        let packed: u32 = field.trim().parse().ok()?;
        let (degrees, minutes, seconds) = (packed / 10_000, packed / 100 % 100, packed % 100);
        (minutes < 60 && seconds < 60)
            .then(|| f64::from(degrees) + f64::from(minutes) / 60.0 + f64::from(seconds) / 3_600.0)
    };
    let mut stations: Vec<Station> = rows
        .filter_map(|row| {
            let field = |at: usize| row.get(at).map_or("", |field| field.trim());
            if !CANADA.contains(&field(province_at)) || field(banner_at) != "OP" {
                return None;
            }
            let frequency_hz = playable(field(frequency_at).parse().ok()?)?;
            let watts = power_at
                .iter()
                .filter_map(|&at| field(at).parse::<f64>().ok())
                .fold(0.0, f64::max);
            if watts <= 0.0 {
                return None;
            }
            let call = field(call_at);
            if call.contains("-HD") {
                return None;
            }
            // A rebroadcaster is its station's call with `-FM-2` and so on,
            // and shares the station's programme.
            let station = call.find("-FM").map_or(call, |at| &call[..at]);
            let kind = if station.starts_with("CB") {
                StationKind::Cbc
            } else if matches!(field(class_at), "LP" | "VLP") {
                StationKind::LowPower
            } else {
                StationKind::FullPower
            };
            Some(Station {
                name: sanitise(station, MAX_CHANNEL_NAME_LEN)?,
                station: station.to_owned(),
                site: format!("{}, {}", field(city_at), field(province_at)),
                frequency_hz,
                erp_watts: watts,
                height_m: height_at
                    .and_then(|at| field(at).parse::<f64>().ok())
                    .filter(|&height| height > 0.0),
                latitude: angle(field(latitude_at))?,
                longitude: -angle(field(longitude_at))?,
                kind,
            })
        })
        .collect();
    if stations.is_empty() {
        return Err("ISED FM table: no stations the radio can play".to_owned());
    }
    stations.sort_by(|a, b| {
        a.station
            .cmp(&b.station)
            .then(a.frequency_hz.cmp(&b.frequency_hz))
    });
    Ok(stations)
}

/// The power every built-in Israeli station is taken to radiate: the list
/// gives none, so they rank by distance alone.
const ISRAEL_NOMINAL_WATTS: f64 = 1_000.0;

/// Israel's regional stations, from the Second Authority's list of
/// franchises (rashut2.org.il, "רדיו אזורי", read 2026-10-21): the channel
/// name, the frequency in tenths of a megahertz, and where it is placed, in
/// degrees north and east. Israel publishes no transmitter sites, so each
/// frequency is at the town the list names for it, else its franchise
/// area's main town, to about a kilometre. Each frequency is its own
/// programme, so a station's second frequency is kept too.
const ISRAEL_REGIONAL: [(&str, u16, &str, f64, f64); 40] = [
    // Radio Tzafon: Galilee and Golan; 104.5 for the Upper Galilee, the
    // Galilee Panhandle and the Golan, 101.5 its second station.
    ("RADIO TZAFON", 1_045, "Upper Galilee", 33.00, 35.58),
    ("HATACHANA", 1_015, "Upper Galilee", 33.00, 35.58),
    // A-Shams: the north; 101 for Nazareth and the north, 98.1 for the
    // Triangle and the coastal plain.
    ("A-SHAMS", 1_010, "Nazareth", 32.70, 35.30),
    ("A-SHAMS", 981, "Tayibe", 32.27, 35.01),
    // Radio Haifa: the Carmel and the bay; 99.5 its second station.
    ("RADIO HAIFA", 1_075, "Haifa", 32.79, 34.99),
    ("HAM ESH", 995, "Haifa", 32.79, 34.99),
    // Radio Nas: the north.
    ("RADIO NAS", 1_033, "Nazareth", 32.70, 35.30),
    ("RADIO NAS", 1_078, "Nazareth", 32.70, 35.30),
    // Kol Rega: the valleys; 96 for Nazareth, 91.5 for Tiberias and the
    // Jordan Valley, 106.4 its Russian station.
    ("KOL REGA", 960, "Nazareth", 32.70, 35.30),
    ("KOL REGA", 915, "Tiberias", 32.79, 35.53),
    ("LUCHSHAYA", 1_064, "Afula", 32.61, 35.29),
    // Radio 90: Emek Hefer.
    ("RADIO 90", 900, "Emek Hefer", 32.37, 34.90),
    ("RADIO 90", 947, "Emek Hefer", 32.37, 34.90),
    // Radius and Eco 99: the Sharon.
    ("RADIUS", 1_000, "Kfar Saba", 32.18, 34.91),
    ("RADIUS", 963, "Kfar Saba", 32.18, 34.91),
    ("ECO 99", 990, "Kfar Saba", 32.18, 34.91),
    // 103FM and Radio Tel Aviv: Gush Dan.
    ("103FM", 1_030, "Tel Aviv", 32.08, 34.78),
    ("103FM", 902, "Tel Aviv", 32.08, 34.78),
    ("RADIO TLV", 1_020, "Tel Aviv", 32.08, 34.78),
    ("RADIO TLV", 996, "Tel Aviv", 32.08, 34.78),
    // Kol Chai: the centre, religious; 102.5 its Haredi music station.
    ("KOL CHAI", 930, "Bnei Brak", 32.09, 34.83),
    ("KOL CHAI", 928, "Bnei Brak", 32.09, 34.83),
    ("KOL CHAI MUS", 1_025, "Bnei Brak", 32.09, 34.83),
    // Lev HaMedina: the Judean lowlands; 89.1 its Russian station.
    ("LEV HAMEDINA", 910, "Rehovot", 31.89, 34.81),
    ("LEV HAMEDINA", 933, "Rehovot", 31.89, 34.81),
    ("PERVAYA", 891, "Rehovot", 31.89, 34.81),
    // Kol Barama: the centre; 107.6 its youth station.
    ("KOL BARAMA", 921, "Bnei Brak", 32.09, 34.83),
    ("KOL BARAMA", 1_057, "Bnei Brak", 32.09, 34.83),
    ("KOL BARAMA", 1_043, "Bnei Brak", 32.09, 34.83),
    ("KOL PLAY", 1_076, "Bnei Brak", 32.09, 34.83),
    // Radio Jerusalem: greater Jerusalem.
    ("RADIO JLM", 1_010, "Jerusalem", 31.78, 35.22),
    ("RADIO JLM", 895, "Jerusalem", 31.78, 35.22),
    // Galei Israel: Judea and Samaria.
    ("GALEI ISRAEL", 940, "Ariel", 32.10, 35.17),
    ("GALEI ISRAEL", 893, "Ariel", 32.10, 35.17),
    ("GALEI ISRAEL", 1_065, "Ariel", 32.10, 35.17),
    // Radio Darom: the Negev; 101.5 for Ashdod, Ashkelon, Lachish and the
    // Gaza envelope.
    ("RADIO DAROM", 970, "Be'er Sheva", 31.25, 34.79),
    ("RADIO DAROM", 958, "Be'er Sheva", 31.25, 34.79),
    ("RADIO DAROM", 1_015, "Ashkelon", 31.67, 34.57),
    // Kol HaYam HaAdom: Eilat and Hevel Eilot.
    ("RED SEA", 1_020, "Eilat", 29.56, 34.95),
    ("RED SEA", 1_011, "Eilat", 29.56, 34.95),
];

/// The Second Authority's list, as one station per frequency.
fn israel() -> impl Iterator<Item = Station> {
    ISRAEL_REGIONAL
        .iter()
        .map(|&(name, tenths, site, latitude, longitude)| Station {
            name: name.to_owned(),
            station: format!("{name} {tenths}"),
            site: site.to_owned(),
            frequency_hz: u32::from(tenths) * RASTER_HZ,
            erp_watts: ISRAEL_NOMINAL_WATTS,
            height_m: None,
            latitude,
            longitude,
            kind: StationKind::Regional,
        })
}

/// The estimated strength of `station` at `km`: power over distance squared,
/// falling a further fourth power of distance beyond the radio horizon, so a
/// big transmitter far past it ranks below a small local one.
fn strength(station: &Station, km: f64, receiver: Receiver) -> f64 {
    let near = station.erp_watts / km.max(1.0).powi(2);
    let Some(height_m) = station.height_m else {
        return near;
    };
    // The 4/3-earth radio horizon, in kilometres, for heights in metres.
    let horizon_km = 4.12 * (height_m.max(0.0).sqrt() + receiver.aerial_height_m().sqrt());
    if km <= horizon_km {
        near
    } else {
        near * (horizon_km / km).powi(4)
    }
}

/// Every station as heard at `locator`, strongest first, each with its
/// transmitter's distance in kilometres.
///
/// Strength is [`strength`], with no distance cut-off short of [`MAX_KM`]: a
/// cut-off dropped a 122 kW main transmitter at 57 km and kept 130 W relays
/// at 25 km. Each
/// station keeps only its strongest transmitter, and each frequency only its
/// strongest station, so a bank holds one channel per programme.
pub fn strongest<'a>(
    stations: impl IntoIterator<Item = &'a Station>,
    locator: &str,
    receiver: Receiver,
) -> Result<Vec<(&'a Station, f64)>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<_> = stations
        .into_iter()
        .map(|station| {
            let km = distance_km(home, (station.latitude, station.longitude));
            (station, km)
        })
        .filter(|&(_, km)| km <= MAX_KM)
        .collect();
    near.sort_by(|a, b| {
        strength(b.0, b.1, receiver)
            .total_cmp(&strength(a.0, a.1, receiver))
            .then(a.0.station.cmp(&b.0.station))
    });
    let mut programmes = HashSet::new();
    let mut frequencies = HashSet::new();
    near.retain(|(station, _)| {
        programmes.insert(station.station.as_str()) && frequencies.insert(station.frequency_hz)
    });
    Ok(near)
}

/// The one bank every location's FM stations share, so a station heard at
/// several is kept once.
pub const STATION_BANK: &str = "FM STATIONS";

/// Builds the bank for the stations `strongest` chose, in frequency order, each
/// played by the broadcast receiver and never transmitted (`ADR-105`).
pub fn station_bank(near: &[(&Station, f64)]) -> BankDraft {
    let name = STATION_BANK.to_owned();
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
    /// Every transmitter the radio can play, of every source read, ordered
    /// by station within each source.
    pub stations: Vec<Station>,
    /// When the oldest cache read was last written.
    pub synced: SystemTime,
    /// Whether every source was read; a cache from before a source was
    /// added lacks it until the next sync.
    pub complete: bool,
}

/// Where each source's raw list is kept.
#[derive(Clone, Debug)]
pub struct StationCache {
    /// Each source and its cache directory, if it has one.
    sources: Vec<(Source, Option<PathBuf>)>,
}

impl StationCache {
    /// Every source, cached under the XDG cache directory.
    pub fn open() -> Self {
        let root = paths::cache_dir();
        Self {
            sources: Source::ALL
                .into_iter()
                .map(|source| {
                    (
                        source,
                        root.as_ref().map(|dir| dir.join(source.cache_dir())),
                    )
                })
                .collect(),
        }
    }

    /// One source, cached in an explicit directory.
    pub fn in_dir(source: Source, dir: PathBuf) -> Self {
        Self {
            sources: vec![(source, Some(dir))],
        }
    }

    /// No cache: nothing is read, and a sync is refused.
    pub fn none() -> Self {
        Self {
            sources: Source::ALL
                .into_iter()
                .map(|source| (source, None))
                .collect(),
        }
    }

    /// Reads each cached list; `Ok(None)` when nothing has been synced.
    pub fn load(&self) -> Result<Option<StationList>, String> {
        let mut stations = Vec::new();
        let mut synced: Option<SystemTime> = None;
        let mut complete = true;
        for (source, dir) in &self.sources {
            let Some(path) = dir.as_ref().map(|dir| dir.join(source.file())) else {
                complete = false;
                continue;
            };
            if !path.exists() {
                complete = false;
                continue;
            }
            let failed = |error: std::io::Error| format!("{}: {error}", path.display());
            let bytes = fs::read(&path).map_err(failed)?;
            let written = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map_err(failed)?;
            stations.extend(source.parse(&String::from_utf8_lossy(&bytes))?);
            synced = Some(synced.map_or(written, |oldest| oldest.min(written)));
        }
        // The built-in stations come with any list read, so they go with
        // the sources' state on screen.
        stations.extend(israel());
        Ok(synced.map(|synced| StationList {
            stations,
            synced,
            complete,
        }))
    }

    /// Fetches each source's list, checks it, caches it, and returns what
    /// every cache then holds.
    ///
    /// Nothing is written for a source unless its response parses, so a bad
    /// fetch keeps its cache as it was; the write goes through a temporary
    /// file and a rename. Any source that failed makes the sync fail, after
    /// the others are cached.
    pub fn sync_with(
        &self,
        fetch: impl Fn(&str) -> Result<Vec<u8>, String>,
    ) -> Result<StationList, String> {
        let mut failures = Vec::new();
        for (source, dir) in &self.sources {
            let synced = (|| {
                let dir = dir
                    .as_ref()
                    .ok_or("no XDG cache directory or HOME is set")?;
                let text = source.prepare(&fetch(source.url())?)?;
                source.parse(&text)?;
                let path = dir.join(source.file());
                let partial = path.with_extension("partial");
                fs::create_dir_all(dir)
                    .and_then(|()| fs::write(&partial, &text))
                    .and_then(|()| fs::rename(&partial, &path))
                    .map_err(|error| format!("{}: {error}", path.display()))
            })();
            if let Err(error) = synced {
                failures.push(error);
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("; "));
        }
        self.load()?
            .ok_or_else(|| "the station cache was not written".to_owned())
    }

    /// Fetches from each source, checks and caches what came, and returns
    /// it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync(&self) -> Result<StationList, String> {
        self.sync_with(|url| fetch_bytes_within(url, SYNC_TIMEOUT))
    }

    /// Reads what the Pages build mirrored from each source beside the
    /// browser Studio (`ADR-121`); the page keeps nothing. A source the
    /// mirror lacks leaves the list incomplete rather than empty.
    #[cfg(target_arch = "wasm32")]
    pub async fn sync(&self) -> Result<StationList, String> {
        let mut list: Option<StationList> = None;
        let mut failures = Vec::new();
        for (source, _) in &self.sources {
            let read = crate::mirror::fetch(source.cache_dir(), [source.file()])
                .await
                .and_then(|([text], synced)| Ok((source.parse(&text)?, synced)));
            let (stations, synced) = match read {
                Ok(read) => read,
                Err(error) => {
                    log::warn!("FM stations from {}: {error}", source.cache_dir());
                    failures.push(error);
                    continue;
                }
            };
            match &mut list {
                Some(list) => {
                    list.stations.extend(stations);
                    list.synced = list.synced.min(synced);
                }
                None => {
                    list = Some(StationList {
                        stations,
                        synced,
                        complete: true,
                    });
                }
            }
        }
        let mut list = list.ok_or_else(|| failures.join("; "))?;
        list.complete = failures.is_empty();
        list.stations.extend(israel());
        Ok(list)
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
    pub const NO_BBC: [bool; StationKind::ALL.len()] = {
        let mut kinds = [true; StationKind::ALL.len()];
        kinds[StationKind::Bbc.index()] = false;
        kinds
    };

    /// Whether `station` is of a kind taken.
    pub const fn takes(&self, station: &Station) -> bool {
        self.kinds[station.kind.index()]
    }

    /// The state file the area is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(AREA_FILE))
    }

    /// Reads `MAX_CHANNELS`, the last number, so a file from when a radius
    /// came first still reads, and the kinds left out from an `off=` word. A
    /// file from before the US kinds names the UK kinds taken in a `kinds=`
    /// word and takes every US kind; one from before the kinds takes every
    /// kind for a `bbc` word, else all but the BBC; anything missing or
    /// unreadable gives the default.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let max_channels = text
            .split_whitespace()
            .filter_map(|word| word.parse().ok())
            .next_back()
            .filter(|value| (1..=MAX_CHANNELS_LIMIT).contains(value))
            .unwrap_or(DEFAULT_MAX_CHANNELS);
        let words = || text.split_whitespace();
        let listed = |prefix: &str| {
            words()
                .find_map(|word| word.strip_prefix(prefix))
                .map(|list| list.split(',').collect::<Vec<_>>())
        };
        let kinds = if let Some(off) = listed(OFF_PREFIX) {
            StationKind::ALL.map(|kind| !off.contains(&kind.word()))
        } else if let Some(taken) = listed(KINDS_PREFIX) {
            StationKind::ALL.map(|kind| taken.contains(&kind.word()) || !kind.is_uk())
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
                let off: Vec<_> = StationKind::ALL
                    .into_iter()
                    .filter(|kind| !self.kinds[kind.index()])
                    .map(StationKind::word)
                    .collect();
                fs::write(
                    file,
                    format!("{OFF_PREFIX}{}\n{}\n", off.join(","), self.max_channels),
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
        parse, parse_fcc, parse_ised, records, station_bank, strongest, Source, StationArea,
        StationCache, StationKind, FCC_URL, ISED_URL, ISRAEL_REGIONAL, TRANSMITTERS_URL,
    };
    use crate::model::ProjectModel;
    use crate::receiver::Receiver;
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

    /// Lines in the shape the FCC's FM Query publishes, trimmed after the
    /// heights: WAMU and its booster, a translator, a low-power station, and
    /// records left out.
    const FMQ: &str = "\
|WAMU        |88.5  MHz |FM |203 |ND  |H  |B  |-  |LIC    |WASHINGTON   |DC |US |0000106853  |47.    kW |47.    kW |156.0   |156.0   |65399  |N |38 |56 |10.4  |W |77  |5  |31.9  |AMERICAN UNIVERSITY |   0.00 km |   0.00 mi |  0.00 deg |226.   m|226.0  m|-  |\n\
|WAMU-FM1    |88.5  MHz |FB |203 |DA  |   |D  |-  |LIC    |SILVER SPRING|MD |US |BLFTB-1     |0.5    kW |-      kW |40.0    |-       |65400  |N |39 |0  |0     |W |77  |0  |0     |AMERICAN UNIVERSITY |   0.00 km |   0.00 mi |  0.00 deg |100.   m|       m|-  |\n\
|W232AB      |94.3  MHz |FX |232 |ND  |   |D  |-  |LIC    |ARLINGTON    |VA |US |BLFT-2      |0.25   kW |0.25   kW |50.0    |50.0    |1      |N |38 |52 |0     |W |77  |6  |0     |SOMEONE             |   0.00 km |   0.00 mi |  0.00 deg |       m|       m|-  |\n\
|WOOF-LP     |99.1  MHz |FL |256 |ND  |   |L1 |-  |LIC    |BETHESDA     |MD |US |BLL-3       |0.1    kW |0.1    kW |30.0    |30.0    |2      |N |38 |59 |0     |W |77  |6  |0     |COMMUNITY           |   0.00 km |   0.00 mi |  0.00 deg |120.   m|120.0  m|-  |\n\
|WAMU        |88.5  MHz |FS |203 |ND  |H  |B  |-  |LIC    |WASHINGTON   |DC |US |BXMLED-4    |13.    kW |13.    kW |122.0   |122.0   |65399  |N |38 |56 |10.6  |W |77  |5  |31.9  |AMERICAN UNIVERSITY |   0.00 km |   0.00 mi |  0.00 deg |191.   m|191.0  m|-  |\n\
|WNEW        |90.1  MHz |FM |211 |ND  |   |A  |-  |CP     |ROCKVILLE    |MD |US |0000000005  |6.     kW |6.     kW |100.0   |100.0   |3      |N |39 |5  |0     |W |77  |9  |0     |NEWCOMER            |   0.00 km |   0.00 mi |  0.00 deg |200.   m|200.0  m|-  |\n\
|-           |91.1  MHz |FA |216 |ND  |   |A  |-  |VAC    |LAUREL       |MD |US |-           |-      kW |-      kW |-       |-       |4      |N |39 |6  |0     |W |76  |51 |0     |-                   |   0.00 km |   0.00 mi |  0.00 deg |       m|       m|-  |\n\
|CBU-2-FM    |88.1  MHz |FM |201 |DA  |   |C  |-  |LIC    |VANCOUVER    |BC |CA |-           |97.6   kW |97.6   kW |605.8   |605.8   |5      |N |49 |21 |13.4  |W |122 |57 |22.7  |-                   |   0.00 km |   0.00 mi |  0.00 deg |700.   m|700.0  m|-  |\n\
|KOFF        |96.1  MHz |FM |241 |ND  |   |A  |-  |LIC    |NOWHERE      |KS |US |0000000006  |-      kW |-      kW |-       |-       |6      |N |38 |0  |0     |W |98  |0  |0     |SILENT              |   0.00 km |   0.00 mi |  0.00 deg |       m|       m|-  |\n";

    /// ISED's broadcast database in the shape it publishes it, trimmed: a
    /// zip holding the FM table with the columns read, rows of each kind
    /// and records left out.
    fn ised_zip() -> Vec<u8> {
        use crate::archive::tests::{dbf, zip};
        let columns = [
            ("PROVINCE", 2),
            ("CITY", 20),
            ("CALL_SIGN", 12),
            ("FREQUENCY", 7),
            ("CLASS", 3),
            ("LATITUDE", 7),
            ("LONGITUDE", 8),
            ("BANNER", 2),
            ("ERPHAV", 7),
            ("ERPVAV", 7),
            ("RAD_CENTER", 6),
        ];
        let row = |fields: [&'static str; 11]| fields.map(str::as_bytes);
        let rows = [
            row([
                "ON", "Toronto", "CBLA-FM", "99.10", "C1", "433856", "792254", "OP", "55109",
                "55109", "421.0",
            ]),
            row([
                "ON", "Toronto", "CHFI-FM", "98.10", "C1", "433833", "792314", "OP", "36180",
                "36180", "553.7",
            ]),
            row([
                "ON", "Toronto", "CJLP-FM", "88.10", "LP", "434000", "792500", "OP", "50", "50",
                "120.0",
            ]),
            row([
                "ON",
                "Paris",
                "CBLA-FM-2",
                "93.50",
                "B",
                "431200",
                "802300",
                "OP",
                "4000",
                "4000",
                "0.0",
            ]),
            // An HD Radio record, a US record, an allotment and a silent
            // station are left out.
            row([
                "ON", "Toronto", "CBLA-HD", "99.10", "C1", "433856", "792254", "OP", "98000",
                "98000", "421.0",
            ]),
            row([
                "MI", "Detroit", "WXYZ-FM", "101.10", "B", "422000", "830300", "OP", "50000",
                "50000", "300.0",
            ]),
            row([
                "ON", "Barrie", "", "95.90", "A", "442300", "794100", "AL", "", "", "",
            ]),
            row([
                "ON", "Nowhere", "CQQQ-FM", "96.10", "A", "440000", "790000", "OP", "0", "0", "",
            ]),
            row([
                "QC", "Gone", "CGON-FM", "97.70", "A", "453000", "733500", "OP", "1000", "1000", "",
            ]),
        ];
        let records: Vec<(bool, &[&[u8]])> = rows
            .iter()
            .enumerate()
            .map(|(index, fields)| (index == 8, fields.as_slice()))
            .collect();
        let table = dbf(&columns, &records);
        zip(&[("allcode.txt", b"codes"), ("fmstatio.dbf", &table)], true)
    }

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
    fn the_fcc_list_keeps_licensed_us_transmitters_the_radio_can_play() {
        let stations = parse_fcc(FMQ).unwrap();
        let summary: Vec<_> = stations
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.site.as_str(),
                    s.frequency_hz,
                    s.erp_watts,
                    s.height_m,
                    s.kind,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "W232AB",
                    "ARLINGTON, VA",
                    94_300_000,
                    250.0,
                    None,
                    StationKind::Translator
                ),
                (
                    "WAMU",
                    "WASHINGTON, DC",
                    88_500_000,
                    47_000.0,
                    Some(226.0),
                    StationKind::FullPower
                ),
                (
                    "WAMU",
                    "SILVER SPRING, MD",
                    88_500_000,
                    500.0,
                    Some(100.0),
                    StationKind::Booster
                ),
                (
                    "WOOF-LP",
                    "BETHESDA, MD",
                    99_100_000,
                    100.0,
                    Some(120.0),
                    StationKind::LowPower
                ),
            ]
        );
        // Degrees, minutes and seconds, west negative.
        let wamu = &stations[1];
        assert!((wamu.latitude - (38.0 + 56.0 / 60.0 + 10.4 / 3_600.0)).abs() < 1e-9);
        assert!((wamu.longitude + (77.0 + 5.0 / 60.0 + 31.9 / 3_600.0)).abs() < 1e-9);
        assert!(parse_fcc("").is_err());
        assert!(
            parse_fcc(FMQ.lines().nth(6).unwrap()).is_err(),
            "a vacancy only"
        );
    }

    #[test]
    fn iseds_table_keeps_canadian_stations_on_air() {
        let csv = Source::Ised.prepare(&ised_zip()).unwrap();
        let stations = Source::Ised.parse(&csv).unwrap();
        let summary: Vec<_> = stations
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.site.as_str(),
                    s.frequency_hz,
                    s.erp_watts,
                    s.height_m,
                    s.kind,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "CBLA",
                    "Paris, ON",
                    93_500_000,
                    4_000.0,
                    None,
                    StationKind::Cbc
                ),
                (
                    "CBLA",
                    "Toronto, ON",
                    99_100_000,
                    55_109.0,
                    Some(421.0),
                    StationKind::Cbc
                ),
                (
                    "CHFI",
                    "Toronto, ON",
                    98_100_000,
                    36_180.0,
                    Some(553.7),
                    StationKind::FullPower
                ),
                (
                    "CJLP",
                    "Toronto, ON",
                    88_100_000,
                    50.0,
                    Some(120.0),
                    StationKind::LowPower
                ),
            ]
        );
        // `DDMMSS` north and `DDDMMSS` west.
        let toronto = &stations[1];
        assert!((toronto.latitude - (43.0 + 38.0 / 60.0 + 56.0 / 3_600.0)).abs() < 1e-9);
        assert!((toronto.longitude + (79.0 + 22.0 / 60.0 + 54.0 / 3_600.0)).abs() < 1e-9);
        // FN03HP, Toronto: the CBC once, at its stronger transmitter.
        let near = strongest(&stations, "FN03HP", Receiver::Handheld).unwrap();
        let chosen: Vec<_> = near
            .iter()
            .map(|(s, _)| (s.name.as_str(), s.frequency_hz))
            .collect();
        assert_eq!(
            chosen,
            [
                ("CBLA", 99_100_000),
                ("CHFI", 98_100_000),
                ("CJLP", 88_100_000)
            ]
        );
        assert!(Source::Ised.prepare(b"not a zip").is_err());
        assert!(parse_ised("PROVINCE\n").is_err());
    }

    #[test]
    fn a_station_and_its_booster_are_one_programme() {
        let stations = parse_fcc(FMQ).unwrap();
        // FN18, Washington DC.
        let near = strongest(&stations, "FM18LV", Receiver::Handheld).unwrap();
        let chosen: Vec<_> = near.iter().map(|(s, _)| s.name.as_str()).collect();
        assert_eq!(chosen, ["WAMU", "W232AB", "WOOF-LP"]);
    }

    #[test]
    fn one_countrys_stations_stay_out_of_anothers() {
        let mut stations = parse(VHF).unwrap();
        stations.extend(parse_fcc(FMQ).unwrap());
        let names = |locator| {
            strongest(&stations, locator, Receiver::Base)
                .unwrap()
                .iter()
                .map(|(s, _)| s.name.clone())
                .collect::<Vec<_>>()
        };
        assert!(names("IO91WM").iter().all(|name| !name.starts_with('W')));
        assert_eq!(names("FM18LV"), ["WAMU", "W232AB", "WOOF-LP"]);
    }

    #[test]
    fn israels_regional_stations_are_heard_nearest_first() {
        let stations: Vec<_> = super::israel().collect();
        // Every frequency the receiver can play, each its own programme.
        assert!(stations
            .iter()
            .all(
                |s| super::playable(f64::from(s.frequency_hz) / 1e6) == Some(s.frequency_hz)
                    && s.kind == StationKind::Regional
                    && s.name.len() <= radio_channel_plan::MAX_CHANNEL_NAME_LEN
            ));
        let names = |locator| {
            let mut near = strongest(&stations, locator, Receiver::Handheld).unwrap();
            near.truncate(4);
            near.iter()
                .map(|(s, _)| (s.name.clone(), s.frequency_hz))
                .collect::<Vec<_>>()
        };
        // KM72JB, Tel Aviv: Gush Dan's stations, then Bnei Brak's.
        let tel_aviv = names("KM72JB");
        assert!(tel_aviv[..2]
            .iter()
            .all(|(name, _)| name == "103FM" || name == "RADIO TLV"));
        // KM72LS, Haifa: Radio Haifa and Ham Esh first.
        let haifa = names("KM72LS");
        assert!(haifa[..2]
            .iter()
            .all(|(name, _)| name == "RADIO HAIFA" || name == "HAM ESH"));
        // KL79LN, Eilat: the Red Sea's two frequencies.
        let eilat = names("KL79LN");
        assert_eq!(eilat[0].0, "RED SEA");
        assert_eq!(eilat[1].0, "RED SEA");
        // 101.5 is Radio Tzafon's second station in the north and Radio
        // Darom's in Ashkelon; each locator gets its own.
        let ashkelon = strongest(&stations, "KM71GQ", Receiver::Handheld).unwrap();
        let on_1015 = ashkelon
            .iter()
            .find(|(s, _)| s.frequency_hz == 101_500_000)
            .unwrap();
        assert_eq!(on_1015.0.name, "RADIO DAROM");
    }

    #[test]
    fn a_roof_aerial_sees_further_past_the_horizon() {
        let station = super::Station {
            name: "FAR".to_owned(),
            station: "Far FM".to_owned(),
            site: String::new(),
            frequency_hz: 100_000_000,
            erp_watts: 1_000.0,
            height_m: Some(100.0),
            latitude: 0.0,
            longitude: 0.0,
            kind: StationKind::Local,
        };
        // A handheld's horizon with a 100 m mast is about 47 km, a roof
        // aerial's about 54 km; at 30 km both see it the same.
        let at = |km, receiver| super::strength(&station, km, receiver);
        assert!(at(100.0, Receiver::Base) > at(100.0, Receiver::Handheld));
        assert!((at(30.0, Receiver::Base) - at(30.0, Receiver::Handheld)).abs() < 1e-12);
    }

    #[test]
    fn each_programme_and_frequency_keeps_its_strongest_transmitter() {
        let stations = parse(VHF).unwrap();
        // IO91WM, central London: Crystal Palace about 11 km, Wrotham 36 km,
        // Black Hill in Scotland about 550 km.
        let near = strongest(&stations, "IO91WM", Receiver::Handheld).unwrap();
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
        let names: Vec<_> = strongest(&flat, "IO91WM", Receiver::Handheld)
            .unwrap()
            .iter()
            .map(|(s, _)| s.name.clone())
            .collect();
        assert_eq!(names, ["BBC R4", "CLASSIC", "BBC SCOT", "RAINBOW"]);
        assert!(strongest(&stations, "nonsense", Receiver::Handheld).is_err());
    }

    #[test]
    fn the_bank_plays_each_station_in_frequency_order_and_never_transmits() {
        let stations = parse(VHF).unwrap();
        let mut near = strongest(&stations, "io91wm", Receiver::Handheld).unwrap();
        near.truncate(3);
        let bank = station_bank(&near);
        assert_eq!(bank.name, "FM STATIONS");
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
        let cache = StationCache::in_dir(Source::Ofcom, dir.clone());
        assert!(cache.load().unwrap().is_none());
        let list = cache
            .sync_with(|url| {
                assert_eq!(url, TRANSMITTERS_URL);
                Ok(VHF.into())
            })
            .unwrap();
        assert_eq!(list.stations.len(), 6 + ISRAEL_REGIONAL.len());
        assert!(cache.sync_with(|_| Ok("not,a,list\n".into())).is_err());
        assert_eq!(
            cache.load().unwrap().unwrap().stations.len(),
            6 + ISRAEL_REGIONAL.len()
        );
        assert!(StationCache::none().sync_with(|_| Ok(VHF.into())).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_source_is_synced_and_read_as_one_list() {
        let dir = scratch("sources");
        let both = |dir: &std::path::Path| StationCache {
            sources: Source::ALL
                .into_iter()
                .map(|source| (source, Some(dir.join(source.cache_dir()))))
                .collect(),
        };
        let cache = both(&dir);
        let list = cache
            .sync_with(|url| match url {
                TRANSMITTERS_URL => Ok(VHF.into()),
                FCC_URL => Ok(FMQ.into()),
                ISED_URL => Ok(ised_zip()),
                _ => Err(format!("unexpected {url}")),
            })
            .unwrap();
        assert_eq!(list.stations.len(), 6 + 4 + 4 + ISRAEL_REGIONAL.len());
        assert!(list.complete);
        // A failed source fails the sync, but the other is still cached
        // and the failed one keeps what it had.
        let error = cache
            .sync_with(|url| match url {
                FCC_URL => Err("HTTP 403".to_owned()),
                ISED_URL => Ok(ised_zip()),
                _ => Ok(VHF.lines().take(2).collect::<Vec<_>>().join("\n").into()),
            })
            .unwrap_err();
        assert_eq!(error, "HTTP 403");
        assert_eq!(
            cache.load().unwrap().unwrap().stations.len(),
            1 + 4 + 4 + ISRAEL_REGIONAL.len()
        );
        // A cache from before the FCC was a source is read, but incomplete.
        std::fs::remove_dir_all(dir.join(Source::Fcc.cache_dir())).unwrap();
        let list = cache.load().unwrap().unwrap();
        assert_eq!(
            (list.stations.len(), list.complete),
            (1 + 4 + ISRAEL_REGIONAL.len(), false)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_area_comes_back_as_saved_and_defaults_otherwise() {
        let dir = scratch("area");
        let file = dir.join("station-area");
        assert_eq!(StationArea::load(&file), StationArea::default());
        for kinds in [
            [true; StationKind::ALL.len()],
            [false; StationKind::ALL.len()],
            StationKind::ALL.map(|kind| kind.index() % 3 != 1),
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
        assert_eq!(
            StationArea::load(&file).kinds,
            [true; StationKind::ALL.len()]
        );
        // One from before the US kinds takes them, and every later kind.
        std::fs::write(&file, "kinds=bbc,local\n30\n").unwrap();
        assert_eq!(
            StationArea::load(&file).kinds,
            StationKind::ALL.map(|kind| !matches!(
                kind,
                StationKind::National
                    | StationKind::Community
                    | StationKind::Restricted
                    | StationKind::Other
            ))
        );
        std::fs::write(&file, "0 9999\n").unwrap();
        assert_eq!(StationArea::load(&file), StationArea::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
