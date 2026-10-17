//! Airport air traffic frequencies near a Maidenhead locator, synced from
//! `OurAirports` (public domain) and cached.
//!
//! As with the repeaters, the studio fetches only when the operator presses
//! Sync; otherwise it reads the raw CSVs cached in
//! `$XDG_CACHE_HOME/afik/ourairports`. The locator is the repeaters' own; the
//! radius and how many channels to take are kept in
//! `$XDG_STATE_HOME/afik/airport-area`, and which services to take and
//! whether each airport gets its own bank in `airport-options`.
//!
//! What a listener hears is mostly the aircraft, so how far a frequency
//! carries depends on its service: an aircraft working approach or radar is
//! high and heard from far off, one in the circuit on tower is low, and one
//! taxiing on ground is heard only nearby. Each [`Service`] has a rough
//! hearing range, approach cut by the [`AirportSize`] (airliners into a
//! large airport are high, a small field's traffic low), and the bank keeps
//! the frequencies inside theirs, the most hearable (distance over range)
//! first.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::SystemTime,
};

use radio_channel_plan::{SharedSettings, MAX_BANKS, MAX_BANK_NAME_LEN, MAX_CHANNEL_NAME_LEN};
use radio_domain::{Bandwidth, Modulation, PowerLevel};

use crate::{
    model::{format_mhz, BankDraft, BankKind, ChannelDraft},
    paths,
    receiver::Receiver,
    repeaters::{distance_km, fetch, locator_centre, sanitise, MAX_RADIUS_KM},
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
    /// How big the airport is, which decides how high its approach traffic
    /// flies.
    pub size: AirportSize,
    /// The use as listed, such as `TWR`, cut to a channel name.
    pub kind: String,
    /// What the frequency is for, read from its use.
    pub service: Service,
}

impl AirportFrequency {
    /// Whether it is a small field's.
    pub fn is_small(&self) -> bool {
        self.size == AirportSize::Small
    }
}

/// How big an airport is, as `OurAirports` types it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirportSize {
    /// A major airport taking airliners, long haul among them, which
    /// approach at height and are heard from far off.
    Large,
    /// A regional airport or military airfield, whose traffic approaches
    /// lower.
    Medium,
    /// An airstrip, gliding site, heliport, balloon port or seaplane base,
    /// whose traffic stays low.
    Small,
}

impl AirportSize {
    /// Every size, largest first, as the studio shows them.
    pub const ALL: [Self; 3] = [Self::Large, Self::Medium, Self::Small];

    /// The size of an `OurAirports` airport type.
    pub fn of(kind: &str) -> Self {
        match kind {
            "large_airport" => Self::Large,
            kind if SMALL_TYPES.contains(&kind) => Self::Small,
            _ => Self::Medium,
        }
    }

    /// Roughly how far off a handheld hears the airport's approach and
    /// radar before the operator chooses, in kilometres: an estimate from
    /// how high its traffic flies, not a measurement.
    pub const fn default_approach_km(self) -> u16 {
        match self {
            Self::Large => Service::Approach.range_km(),
            Self::Medium => 50,
            Self::Small => 30,
        }
    }

    /// What the studio calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Large => "Large",
            Self::Medium => "Medium",
            Self::Small => "Small",
        }
    }

    /// What it covers, for the studio's hover text.
    pub const fn help(self) -> &'static str {
        match self {
            Self::Large => "a large airport, taking airliners",
            Self::Medium => "a regional airport or military field",
            Self::Small => "a small field, heliport or airstrip",
        }
    }

    /// The word the options file keeps its approach range under.
    const fn key(self) -> &'static str {
        match self {
            Self::Large => "approach-large",
            Self::Medium => "approach-medium",
            Self::Small => "approach-small",
        }
    }

    /// Its place in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// What an airport frequency is for, which decides how far off it is heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    /// Approach, departure, radar, director and area control: aircraft at
    /// height, heard from far off.
    Approach,
    /// Tower, AFIS, air/ground, CTAF and UNICOM: aircraft in the circuit.
    Tower,
    /// Ground, delivery, apron and operations: aircraft and vehicles on the
    /// ground, heard only nearby.
    Ground,
    /// ATIS and other recorded broadcasts: a ground transmitter that never
    /// stops, which holds a scan.
    Broadcast,
    /// Anything the use does not say.
    Other,
}

impl Service {
    /// Every service, in the order the studio shows them.
    pub const ALL: [Self; 5] = [
        Self::Approach,
        Self::Tower,
        Self::Ground,
        Self::Broadcast,
        Self::Other,
    ];

    /// The listed uses (split into words, `A/G` read as `AG`) of each
    /// service; a use naming several takes the one heard furthest.
    const WORDS: [(Self, &'static [&'static str]); 4] = [
        (
            Self::Approach,
            &[
                "APP", "APPR", "APPROACH", "DEP", "ARR", "RDR", "RAD", "RADAR", "DIR", "DIRECTOR",
                "CNTR", "CTR", "CENTRE", "CENTER", "ACC", "ARTC", "CONTROL", "LARS", "ZONE", "TCA",
                "TMA", "CTRL", "ARSA", "STAR", "ACP", "GCA", "PAR", "FIS", "FIA",
            ],
        ),
        (
            Self::Broadcast,
            &["ATIS", "DATIS", "AWOS", "ASOS", "AWIS", "AWIB", "VOLMET"],
        ),
        (
            Self::Tower,
            &[
                "TWR",
                "TOWER",
                "AFIS",
                "AG",
                "AD",
                "AA",
                "CTAF",
                "UNIC",
                "UNICOM",
                "RDO",
                "RADIO",
                "ATF",
                "MF",
                "MULTICOM",
                "FSS",
                "RCO",
                "INFO",
                "INFORMATION",
                "SAFETY",
                "SAFETYCOM",
            ],
        ),
        (
            Self::Ground,
            &[
                "GND", "GNDC", "GROUND", "GRN", "GMC", "SMC", "DLV", "START", "DEL", "DELIVERY",
                "CLD", "CLNC", "RMP", "RAMP", "APRON", "OPS", "PTD", "FIRE",
            ],
        ),
    ];

    /// The service a listed use, such as `TWR` or `RAD/APP`, names.
    pub fn of(kind: &str) -> Self {
        let kind = kind
            .to_ascii_uppercase()
            .replace("A/G", "AG")
            .replace("A/D", "AD")
            .replace("A/A", "AA");
        let words: Vec<&str> = kind
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        Self::WORDS
            .iter()
            .filter(|(_, listed)| words.iter().any(|word| listed.contains(word)))
            .map(|(service, _)| *service)
            .max_by_key(|service| service.range_km())
            .unwrap_or(Self::Other)
    }

    /// Roughly how far off a handheld at ground level hears it at most, in
    /// kilometres: an estimate from the height of what transmits, not a
    /// measurement. Approach is heard as far as [`AirportOptions`] says for
    /// the airport's size.
    pub const fn range_km(self) -> u16 {
        match self {
            Self::Approach => 150,
            Self::Broadcast => 60,
            Self::Other => 50,
            Self::Tower => 30,
            Self::Ground => 10,
        }
    }

    /// How far off `receiver` hears it, in kilometres.
    pub const fn reach_km(self, receiver: Receiver) -> u16 {
        receiver.reach_km(self.range_km())
    }

    /// What the studio calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Approach => "Approach/radar",
            Self::Tower => "Tower",
            Self::Ground => "Ground",
            Self::Broadcast => "ATIS",
            Self::Other => "Other",
        }
    }

    /// The word the options file keeps it as.
    const fn key(self) -> &'static str {
        match self {
            Self::Approach => "approach",
            Self::Tower => "tower",
            Self::Ground => "ground",
            Self::Broadcast => "atis",
            Self::Other => "other",
        }
    }

    /// Whether it belongs to one airport, so goes in that airport's bank
    /// when each has its own.
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Tower | Self::Ground | Self::Broadcast)
    }

    /// Its place in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// `OurAirports` types of a small field.
const SMALL_TYPES: [&str; 4] = ["small_airport", "heliport", "balloonport", "seaplane_base"];

/// The carrier of an airband frequency as listed, in hertz.
///
/// Lists give a channel name, not a carrier. In each 25 kHz block names end
/// in 0 (25 kHz), 5 (8.33 kHz at the block), 10 (8.33 kHz, a third in) and
/// 15 (two thirds in). A name ending in 20 is a 25 kHz frequency cut to two
/// decimals, such as `123.07` for 123.075. Anything else is off both grids
/// and is refused, as is anything outside the VHF airband.
pub(crate) fn carrier_hz(mhz: &str) -> Option<u32> {
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
    let airports: HashMap<String, (String, String, f64, f64, AirportSize)> = rows
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
                    AirportSize::of(field(type_at)),
                ),
            ))
        })
        .collect();

    let mut frequencies: Vec<AirportFrequency> = listed
        .into_iter()
        .filter_map(|(id, kind, frequency_hz)| {
            let (ident, airport, latitude, longitude, size) = airports.get(&id)?;
            Some(AirportFrequency {
                name: sanitise(&format!("{ident} {kind}"), MAX_CHANNEL_NAME_LEN)?,
                service: Service::of(&kind),
                kind: sanitise(&kind, MAX_CHANNEL_NAME_LEN).unwrap_or_default(),
                ident: ident.clone(),
                airport: airport.clone(),
                frequency_hz,
                latitude: *latitude,
                longitude: *longitude,
                size: *size,
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

/// The frequencies `keep` accepts of the airports within `radius_km` of
/// `locator`, nearest airport first and each airport's in frequency order,
/// with its distance in kilometres. A frequency several airports share,
/// such as a common traffic frequency, is kept once, for the nearest, and
/// every carrier already in `taken` is left out.
pub(crate) fn nearby_matching<'a>(
    frequencies: &'a [AirportFrequency],
    locator: &str,
    radius_km: u16,
    keep: impl Fn(&AirportFrequency) -> bool,
    mut taken: HashSet<u32>,
) -> Result<Vec<(&'a AirportFrequency, f64)>, String> {
    let home = locator_centre(locator)?;
    let mut near: Vec<_> = frequencies
        .iter()
        .filter(|frequency| keep(frequency))
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
    near.retain(|(frequency, _)| taken.insert(frequency.frequency_hz));
    Ok(near)
}

/// One airport given its own bank, with its frequencies.
#[derive(Clone, Debug)]
pub struct AirportGroup<'a> {
    /// Its ident, usually its ICAO code.
    pub ident: &'a str,
    /// Its name.
    pub airport: &'a str,
    /// How far it is, in kilometres.
    pub km: f64,
    /// Its local frequencies: tower, then ground, then ATIS, each in
    /// frequency order.
    pub frequencies: Vec<&'a AirportFrequency>,
}

/// What the airport banks take.
#[derive(Clone, Debug)]
pub struct AirportPlan<'a> {
    /// The area bank's frequencies, most hearable first, with their
    /// distance in kilometres.
    pub area: Vec<(&'a AirportFrequency, f64)>,
    /// How many hearable frequencies there were before the area bank's
    /// limit.
    pub hearable: usize,
    /// The airports given their own bank, nearest first.
    pub airports: Vec<AirportGroup<'a>>,
    /// The large airports' approach and radar heard here, most hearable
    /// first, for the one [`MAJOR_BANK`] every location shares.
    pub major: Vec<(&'a AirportFrequency, f64)>,
}

/// The one bank of the large airports' approach and radar, which every
/// location shares since it is heard from far off.
pub const MAJOR_BANK: &str = "AIR INTL";

impl AirportFrequency {
    /// Whether it goes in [`MAJOR_BANK`]: a large airport's approach.
    pub fn is_major(&self) -> bool {
        self.size == AirportSize::Large && self.service == Service::Approach
    }
}

impl AirportPlan<'_> {
    /// How many channels the banks hold.
    pub fn channels(&self) -> usize {
        self.area.len()
            + self.major.len()
            + self
                .airports
                .iter()
                .map(|group| group.frequencies.len())
                .sum::<usize>()
    }
}

/// How near an airport must be to get a bank of its own: two thirds of
/// where `receiver` hears its tower, 20 km on a handheld, as each bank is
/// costly.
pub const fn own_bank_km(receiver: Receiver) -> u16 {
    Service::Tower.reach_km(receiver) * 2 / 3
}

/// Chooses the frequencies of the services `options` takes at the airports
/// within the area's radius of `locator`.
///
/// When each airport gets its own bank, the nearest `options.airports`
/// airports within the radius and [`own_bank_km`] with a local (tower,
/// ground or ATIS) frequency each take all of their local ones, even those
/// too weak to hear, since that bank is for when the operator is there.
/// Every other frequency inside its service's hearing range from `receiver`
/// goes to the area
/// bank, most hearable first (distance over range, so approach 60 km off
/// comes before a tower 20 km off), a carrier several airports share kept
/// for the most hearable, up to `area.max_channels`. A large airport's
/// approach goes to [`AirportPlan::major`] instead, without a limit.
pub fn plan<'a>(
    frequencies: &'a [AirportFrequency],
    locator: &str,
    area: &AirportArea,
    options: &AirportOptions,
    receiver: Receiver,
) -> Result<AirportPlan<'a>, String> {
    let home = locator_centre(locator)?;
    let in_radius: Vec<_> = frequencies
        .iter()
        .filter(|frequency| options.takes(frequency.service))
        .map(|frequency| {
            let km = distance_km(home, (frequency.latitude, frequency.longitude));
            (frequency, km)
        })
        .filter(|(_, km)| *km <= f64::from(area.radius_km))
        .collect();

    let heard = |(frequency, km): &(&AirportFrequency, f64)| {
        *km <= f64::from(options.reach_km(frequency, receiver))
    };
    let mut airports: Vec<AirportGroup<'a>> = Vec::new();
    if options.per_airport {
        let mut local: Vec<_> = in_radius
            .iter()
            .filter(|(frequency, _)| frequency.service.is_local())
            .collect();
        // Only an airport near enough gets a bank.
        let own_km = f64::from(own_bank_km(receiver));
        local.retain(|(_, km)| *km <= own_km);
        local.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.ident.cmp(&b.0.ident)));
        for (frequency, km) in local {
            if let Some(group) = airports.iter_mut().find(|g| g.ident == frequency.ident) {
                group.frequencies.push(frequency);
            } else if airports.len() < usize::from(options.airports) {
                airports.push(AirportGroup {
                    ident: &frequency.ident,
                    airport: &frequency.airport,
                    km: *km,
                    frequencies: vec![frequency],
                });
            }
        }
        for group in &mut airports {
            group.frequencies.sort_by_key(|frequency| {
                (
                    [Service::Tower, Service::Ground, Service::Broadcast]
                        .iter()
                        .position(|service| *service == frequency.service),
                    frequency.frequency_hz,
                )
            });
            let mut seen = HashSet::new();
            group
                .frequencies
                .retain(|frequency| seen.insert(frequency.frequency_hz));
        }
    }

    let own_bank = |frequency: &AirportFrequency| {
        frequency.service.is_local() && airports.iter().any(|g| g.ident == frequency.ident)
    };
    let hearability = |(frequency, km): &(&AirportFrequency, f64)| {
        km / f64::from(options.reach_km(frequency, receiver))
    };
    let mut near: Vec<_> = in_radius
        .iter()
        .filter(|candidate| heard(candidate) && !own_bank(candidate.0))
        .copied()
        .collect();
    near.sort_by(|a, b| {
        hearability(a)
            .total_cmp(&hearability(b))
            .then(a.0.ident.cmp(&b.0.ident))
            .then(a.0.frequency_hz.cmp(&b.0.frequency_hz))
    });
    let mut taken = HashSet::new();
    near.retain(|(frequency, _)| taken.insert(frequency.frequency_hz));
    let (major, mut near): (Vec<_>, Vec<_>) = near
        .into_iter()
        .partition(|(frequency, _)| frequency.is_major());
    let hearable = near.len();
    near.truncate(usize::from(area.max_channels));
    Ok(AirportPlan {
        area: near,
        hearable,
        airports,
        major,
    })
}

/// Builds the receive-only AM banks `plan` chose: the area bank `AIR
/// <locator>` when it holds anything, then one per airport named by its
/// ident and name, its channels by use alone.
pub fn airport_banks(locator: &str, plan: &AirportPlan) -> Vec<BankDraft> {
    let mut banks = Vec::new();
    if !plan.area.is_empty() {
        let channels = plan
            .area
            .iter()
            .map(|(frequency, _)| am_channel(&frequency.name, frequency.frequency_hz))
            .collect();
        banks.push(am_bank("AIR", locator, channels));
    }
    for group in &plan.airports {
        let channels = group
            .frequencies
            .iter()
            .map(|frequency| {
                let name = if frequency.kind.is_empty() {
                    &frequency.name
                } else {
                    &frequency.kind
                };
                am_channel(name, frequency.frequency_hz)
            })
            .collect();
        let mut bank = am_bank(group.ident, "", channels);
        bank.name = sanitise(
            &format!("{} {}", group.ident, group.airport),
            MAX_BANK_NAME_LEN,
        )
        .unwrap_or(bank.name);
        banks.push(bank);
    }
    banks
}

/// Builds the receive-only AM bank of the large airports' approach in
/// `major`, in its order, which every location shares.
pub fn major_bank(major: &[(&AirportFrequency, f64)]) -> BankDraft {
    let channels = major
        .iter()
        .map(|(frequency, _)| am_channel(&frequency.name, frequency.frequency_hz))
        .collect();
    let mut bank = am_bank("", "", channels);
    MAJOR_BANK.clone_into(&mut bank.name);
    bank
}

/// One receive-only airband channel.
pub(crate) fn am_channel(name: &str, frequency_hz: u32) -> ChannelDraft {
    ChannelDraft {
        name: name.to_owned(),
        receive_mhz: format_mhz(frequency_hz),
        // AM only receives (`ADR-107`).
        transmit_mhz: String::new(),
        modulation: Modulation::Am,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low1,
        step_hz: STEP_HZ,
        ..ChannelDraft::default()
    }
}

/// A receive-only AM bank named `prefix` and the locator.
pub(crate) fn am_bank(prefix: &str, locator: &str, channels: Vec<ChannelDraft>) -> BankDraft {
    let mut name = format!("{prefix} {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
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

/// The radius airports are taken within before the operator chooses one, in
/// kilometres: the reach of approach, aircraft at height, the furthest
/// service; each service's own range trims the rest.
pub const DEFAULT_AIRPORT_RADIUS_KM: u16 = 150;

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
            radius_km: DEFAULT_AIRPORT_RADIUS_KM,
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
        Self::load_or(file, &Self::default())
    }

    /// Reads the area as [`Self::load`] does, with `defaults` for anything
    /// missing or unreadable.
    pub fn load_or(file: &Path, defaults: &Self) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut words = text.split_whitespace();
        let mut number = |limit: u16, default: u16| {
            words
                .next()
                .and_then(|word| word.parse().ok())
                .filter(|value| (1..=limit).contains(value))
                .unwrap_or(default)
        };
        let radius_km = number(MAX_RADIUS_KM, defaults.radius_km);
        let max_channels = number(MAX_CHANNELS_LIMIT, defaults.max_channels);
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

/// Which services the airport banks take, and whether each airport near
/// enough gets a bank of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirportOptions {
    /// Whether each [`Service`] is taken, by [`Service::index`].
    pub services: [bool; Service::ALL.len()],
    /// Whether the nearest airports each get a bank of their local
    /// frequencies.
    pub per_airport: bool,
    /// How many airports get a bank of their own.
    pub airports: u16,
    /// How far off a handheld hears approach and radar, in kilometres, by
    /// [`AirportSize::index`].
    pub approach_km: [u16; AirportSize::ALL.len()],
}

/// The furthest approach range the operator can choose, in kilometres.
pub const MAX_APPROACH_KM: u16 = 500;

/// How many airports get a bank of their own before the operator chooses.
pub const DEFAULT_AIRPORT_BANKS: u16 = 5;
const OPTIONS_FILE: &str = "airport-options";

impl Default for AirportOptions {
    /// Every service but ATIS, which holds a scan; one area bank.
    fn default() -> Self {
        let mut services = [true; Service::ALL.len()];
        services[Service::Broadcast.index()] = false;
        Self {
            services,
            per_airport: false,
            airports: DEFAULT_AIRPORT_BANKS,
            approach_km: AirportSize::ALL.map(AirportSize::default_approach_km),
        }
    }
}

impl AirportOptions {
    /// Whether `service` is taken.
    pub const fn takes(&self, service: Service) -> bool {
        self.services[service.index()]
    }

    /// Roughly how far off a handheld at ground level hears `frequency`, in
    /// kilometres: its service's range, approach and radar as far as chosen
    /// for the airport's size, since that is how high its traffic flies.
    pub const fn range_km(&self, frequency: &AirportFrequency) -> u16 {
        match frequency.service {
            Service::Approach => self.approach_km[frequency.size.index()],
            service => service.range_km(),
        }
    }

    /// How far off `receiver` hears `frequency`, in kilometres.
    pub const fn reach_km(&self, frequency: &AirportFrequency, receiver: Receiver) -> u16 {
        receiver.reach_km(self.range_km(frequency))
    }

    /// The state file the options are kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(OPTIONS_FILE))
    }

    /// Reads the services taken by name, `per-airport`, `airports=N` and
    /// each size's `approach-<size>=KM`; with no file, the defaults.
    pub fn load(file: &Path) -> Self {
        let Ok(text) = fs::read_to_string(file) else {
            return Self::default();
        };
        let mut options = Self {
            services: [false; Service::ALL.len()],
            ..Self::default()
        };
        for word in text.split_whitespace() {
            let approach = AirportSize::ALL.iter().find_map(|size| {
                let km = word.strip_prefix(size.key())?.strip_prefix('=')?;
                Some((size, km.parse().ok()))
            });
            if let Some((size, km)) = approach {
                if let Some(km) = km.filter(|km| (1..=MAX_APPROACH_KM).contains(km)) {
                    options.approach_km[size.index()] = km;
                }
                continue;
            }
            if let Some(service) = Service::ALL.iter().find(|s| s.key() == word) {
                options.services[service.index()] = true;
            } else if word == "per-airport" {
                options.per_airport = true;
            } else if let Some(count) = word.strip_prefix("airports=") {
                if let Some(count) = count
                    .parse()
                    .ok()
                    .filter(|count| (1..=MAX_BANKS).contains(count))
                {
                    options.airports = count;
                }
            }
        }
        options
    }

    /// Writes the options for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let mut words: Vec<String> = Service::ALL
            .iter()
            .filter(|service| self.takes(**service))
            .map(|service| service.key().to_owned())
            .collect();
        if self.per_airport {
            words.push("per-airport".to_owned());
        }
        words.push(format!("airports={}", self.airports));
        for size in AirportSize::ALL {
            words.push(format!("{}={}", size.key(), self.approach_km[size.index()]));
        }
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, words.join(" ") + "\n"))
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
        airport_banks, carrier_hz, parse, plan, AirportArea, AirportCache, AirportFrequency,
        AirportOptions, AirportSize, Service, AIRPORTS_URL, FREQUENCIES_URL,
    };
    use crate::model::ProjectModel;
    use crate::receiver::Receiver;
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
        let sizes: Vec<_> = [0, 2, 5].map(|at| list[at].size).into();
        assert_eq!(
            sizes,
            [AirportSize::Large, AirportSize::Medium, AirportSize::Small]
        );
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

    /// The centre of IO91WM.
    const HOME: (f64, f64) = (51.520_833, -0.125);

    /// A frequency `km` due north of IO91WM's centre.
    fn heard(ident: &str, kind: &str, hz: u32, km: f64) -> AirportFrequency {
        AirportFrequency {
            name: format!("{ident} {kind}"),
            ident: ident.to_owned(),
            airport: format!("{ident} Field"),
            frequency_hz: hz,
            latitude: HOME.0 + km / 111.2,
            longitude: HOME.1,
            size: AirportSize::Medium,
            kind: kind.to_owned(),
            service: Service::of(kind),
        }
    }

    /// The default options with a medium airport's approach heard as far
    /// as a large one's, so [`heard`]'s stay in the area bank.
    fn wide() -> AirportOptions {
        let mut options = AirportOptions::default();
        options.approach_km[AirportSize::Medium.index()] = Service::Approach.range_km();
        options
    }

    fn area(radius_km: u16, max_channels: u16) -> AirportArea {
        AirportArea {
            radius_km,
            max_channels,
        }
    }

    #[test]
    fn a_listed_use_names_its_service() {
        assert_eq!(Service::of("TWR"), Service::Tower);
        assert_eq!(Service::of("A/G"), Service::Tower);
        assert_eq!(Service::of("A/G / AFIS"), Service::Tower);
        assert_eq!(Service::of("Brimpton Radio"), Service::Tower);
        assert_eq!(Service::of("GND"), Service::Ground);
        assert_eq!(Service::of("CLD"), Service::Ground);
        assert_eq!(Service::of("ATIS"), Service::Broadcast);
        assert_eq!(Service::of("RAD/APP"), Service::Approach);
        assert_eq!(Service::of("RDR"), Service::Approach);
        // A use naming several takes the one heard furthest.
        assert_eq!(Service::of("TWR/APP/DEP"), Service::Approach);
        assert_eq!(Service::of("Safety Comm"), Service::Tower);
        assert_eq!(Service::of("MISC"), Service::Other);
        assert_eq!(Service::of(""), Service::Other);
        assert!(Service::Approach.range_km() > Service::Tower.range_km());
        assert!(Service::Tower.range_km() > Service::Ground.range_km());
    }

    #[test]
    fn the_area_bank_keeps_what_is_hearable_most_hearable_first() {
        let list = [
            heard("EGLL", "APP", 119_725_000, 20.0),
            heard("EGLL", "TWR", 118_500_000, 20.0),
            heard("EGLL", "GND", 121_900_000, 20.0),
            heard("EGLL", "ATIS", 128_075_000, 20.0),
            heard("EGLC", "GND", 121_825_000, 5.0),
            heard("EGKK", "APP", 126_825_000, 90.0),
            heard("EGKK", "TWR", 124_225_000, 90.0),
        ];
        let names = |options: &AirportOptions, area: &AirportArea| -> Vec<String> {
            plan(&list, "IO91WM", area, options, Receiver::Handheld)
                .unwrap()
                .area
                .iter()
                .map(|(f, _)| f.name.clone())
                .collect()
        };
        // Ground 20 km off and tower 90 km off are out of hearing; ATIS is
        // left out unless asked for.
        let options = wide();
        assert_eq!(
            names(&options, &area(300, 40)),
            ["EGLL APP", "EGLC GND", "EGKK APP", "EGLL TWR"]
        );
        let chosen = plan(&list, "IO91WM", &area(300, 2), &options, Receiver::Handheld).unwrap();
        assert_eq!(chosen.area.len(), 2);
        assert_eq!(chosen.hearable, 4);
        // The radius still bounds everything.
        assert_eq!(
            names(&options, &area(50, 40)),
            ["EGLL APP", "EGLC GND", "EGLL TWR"]
        );
        let mut atis = options.clone();
        atis.services[Service::Broadcast.index()] = true;
        atis.services[Service::Approach.index()] = false;
        assert_eq!(
            names(&atis, &area(300, 40)),
            ["EGLL ATIS", "EGLC GND", "EGLL TWR"]
        );
    }

    #[test]
    fn each_airport_can_have_a_bank_of_its_local_frequencies() {
        let list = [
            heard("EGLL", "APP", 119_725_000, 20.0),
            heard("EGLL", "TWR", 118_500_000, 20.0),
            heard("EGLL", "GND", 121_900_000, 20.0),
            heard("EGLL", "ATIS", 128_075_000, 20.0),
            heard("EGLC", "GND", 121_825_000, 5.0),
            heard("EGKK", "APP", 126_825_000, 90.0),
            heard("EGKK", "TWR", 124_225_000, 90.0),
        ];
        let options = AirportOptions {
            per_airport: true,
            airports: 2,
            ..wide()
        };
        let chosen = plan(
            &list,
            "IO91WM",
            &area(300, 40),
            &options,
            Receiver::Handheld,
        )
        .unwrap();
        let area_names: Vec<_> = chosen.area.iter().map(|(f, _)| f.name.as_str()).collect();
        assert_eq!(area_names, ["EGLL APP", "EGKK APP"]);
        let idents: Vec<_> = chosen.airports.iter().map(|g| g.ident).collect();
        assert_eq!(idents, ["EGLC", "EGLL"]);
        // Ground too far to hear from home still goes in its airport's bank.
        let heathrow: Vec<_> = chosen.airports[1]
            .frequencies
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(heathrow, ["EGLL TWR", "EGLL GND"]);
        assert_eq!(chosen.channels(), 5);

        let banks = airport_banks("io91wm", &chosen);
        let summary: Vec<_> = banks
            .iter()
            .map(|b| {
                (
                    b.name.as_str(),
                    b.channels
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("AIR IO91WM", vec!["EGLL APP", "EGKK APP"]),
                ("EGLC EGLC FIELD", vec!["GND"]),
                ("EGLL EGLL FIELD", vec!["TWR", "GND"]),
            ]
        );
        let project = ProjectModel {
            banks,
            ..ProjectModel::new()
        };
        project.to_radio_image().expect("valid airband banks");

        // With approach left out nothing is left for the area bank.
        let mut local = options;
        local.services[Service::Approach.index()] = false;
        let chosen = plan(&list, "IO91WM", &area(300, 40), &local, Receiver::Handheld).unwrap();
        assert!(chosen.area.is_empty());
        assert_eq!(airport_banks("IO91WM", &chosen).len(), 2);

        // Only airports inside the radius get a bank, however many are asked
        // for: Heathrow at 20 km is outside 10 km.
        let many = AirportOptions {
            per_airport: true,
            airports: 5,
            ..wide()
        };
        let chosen = plan(&list, "IO91WM", &area(10, 40), &many, Receiver::Handheld).unwrap();
        let idents: Vec<_> = chosen.airports.iter().map(|g| g.ident).collect();
        assert_eq!(idents, ["EGLC"]);
        // Nor does one whose tower and ground are too far to hear: Gatwick's
        // tower at 90 km is past a handheld's 30 km, so it stays in the area
        // bank's approach alone.
        let chosen = plan(&list, "IO91WM", &area(300, 40), &many, Receiver::Handheld).unwrap();
        let idents: Vec<_> = chosen.airports.iter().map(|g| g.ident).collect();
        assert_eq!(idents, ["EGLC", "EGLL"]);
        // A tower heard at 25 km is past a handheld's 20 km for a bank of
        // its own, so it goes in the area bank; a base station's is 40 km.
        let heard_far = [heard("EGWU", "TWR", 124_475_000, 25.0)];
        let chosen = plan(
            &heard_far,
            "IO91WM",
            &area(300, 40),
            &many,
            Receiver::Handheld,
        )
        .unwrap();
        assert!(chosen.airports.is_empty());
        assert_eq!(chosen.area.len(), 1);
        assert_eq!(super::own_bank_km(Receiver::Base), 40);
        let chosen = plan(&heard_far, "IO91WM", &area(300, 40), &many, Receiver::Base).unwrap();
        assert_eq!(chosen.airports.len(), 1);
    }

    #[test]
    fn a_smaller_airports_approach_is_heard_only_nearer() {
        let sized = |ident: &str, size, km: u16| AirportFrequency {
            size,
            ..heard(
                ident,
                "APP",
                120_000_000 + u32::from(km) * 25_000,
                f64::from(km),
            )
        };
        // Airliners into a large airport are heard 140 km off; a regional
        // airport's or airfield's approach only nearer.
        let list = [
            sized("EGSS", AirportSize::Large, 140),
            sized("EGSC", AirportSize::Medium, 45),
            sized("EGSH", AirportSize::Medium, 60),
            sized("EGSO", AirportSize::Small, 25),
            sized("GB-0001", AirportSize::Small, 35),
        ];
        let names = |options: &AirportOptions, receiver| -> (Vec<String>, Vec<String>) {
            let chosen = plan(&list, "IO91WM", &area(300, 40), options, receiver).unwrap();
            let idents = |taken: &[(&AirportFrequency, f64)]| {
                taken.iter().map(|(f, _)| f.ident.clone()).collect()
            };
            (idents(&chosen.area), idents(&chosen.major))
        };
        let options = AirportOptions::default();
        // The large airport's approach goes to the shared bank.
        assert_eq!(
            names(&options, Receiver::Handheld),
            (vec!["EGSO".into(), "EGSC".into()], vec!["EGSS".into()])
        );
        // A better aerial hears every size further.
        assert_eq!(
            names(&options, Receiver::Base).0,
            ["EGSO", "EGSC", "GB-0001", "EGSH"]
        );
        // The tower is heard as far whatever the size.
        let tower = AirportFrequency {
            size: AirportSize::Small,
            ..heard("EGSO", "TWR", 118_000_000, 20.0)
        };
        assert_eq!(options.range_km(&tower), Service::Tower.range_km());
        // The operator can hear a medium airport further.
        let mut further = AirportOptions::default();
        further.approach_km[AirportSize::Medium.index()] = 70;
        assert_eq!(
            names(&further, Receiver::Handheld).0,
            ["EGSC", "EGSO", "EGSH"]
        );
        assert_eq!(AirportSize::of("large_airport"), AirportSize::Large);
        assert_eq!(AirportSize::of("medium_airport"), AirportSize::Medium);
        assert_eq!(AirportSize::of("heliport"), AirportSize::Small);
    }

    #[test]
    fn a_better_receiver_hears_further() {
        // Ground at 15 km is past a handheld's 10 km, inside a base
        // station's 20 km.
        let list = [heard("EGLC", "GND", 121_825_000, 15.0)];
        let options = AirportOptions::default();
        let names = |receiver| -> Vec<String> {
            plan(&list, "IO91WM", &area(50, 40), &options, receiver)
                .unwrap()
                .area
                .iter()
                .map(|(f, _)| f.name.clone())
                .collect()
        };
        assert!(names(Receiver::Handheld).is_empty());
        assert_eq!(names(Receiver::Base), ["EGLC GND"]);
        assert_eq!(Service::Ground.reach_km(Receiver::Base), 20);
    }

    #[test]
    fn the_listed_fixture_plans_one_receive_only_bank() {
        let list = parse(AIRPORTS, FREQUENCIES).unwrap();
        let options = AirportOptions::default();
        // JO02BF is the square holding Cambridge; Manchester is about 200
        // km, too far to hear its ground, and ATIS is left out.
        let chosen = plan(
            &list,
            "jo02bf",
            &area(300, 40),
            &options,
            Receiver::Handheld,
        )
        .unwrap();
        let names: Vec<_> = chosen.area.iter().map(|(f, _)| f.name.as_str()).collect();
        // The strip's 122.2 is Cambridge Tower's, which is more hearable.
        assert_eq!(names, ["EGSC RDR", "EGSC TWR", "EGYY UNIC", "EGYY SAFETYC"]);
        let banks = airport_banks("jo02bf", &chosen);
        assert_eq!(banks.len(), 1);
        assert_eq!(banks[0].name, "AIR JO02BF");
        assert!(banks[0]
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Am && c.transmit_mhz.is_empty()));
        assert!(plan(
            &list,
            "nonsense",
            &area(50, 40),
            &options,
            Receiver::Handheld
        )
        .is_err());
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

    #[test]
    fn the_options_come_back_as_saved_and_default_without_atis() {
        let dir = scratch("options");
        let file = dir.join("airport-options");
        let defaults = AirportOptions::load(&file);
        assert_eq!(defaults, AirportOptions::default());
        assert!(!defaults.takes(Service::Broadcast));
        assert!(defaults.takes(Service::Tower));
        let options = AirportOptions {
            services: [false, true, true, true, false],
            per_airport: true,
            airports: 5,
            approach_km: [200, 60, 20],
        };
        options.save(&file).unwrap();
        assert_eq!(AirportOptions::load(&file), options);
        // Nothing taken stays nothing taken; a bad count is the default.
        std::fs::write(&file, "airports=999\n").unwrap();
        let none = AirportOptions::load(&file);
        assert!(Service::ALL.iter().all(|s| !none.takes(*s)));
        assert_eq!(none.airports, super::DEFAULT_AIRPORT_BANKS);
        // A file from before the ranges, or a bad range, keeps the default.
        std::fs::write(&file, "approach per-airport approach-medium=0\n").unwrap();
        let older = AirportOptions::load(&file);
        assert!(older.takes(Service::Approach) && older.per_airport);
        assert_eq!(older.approach_km, AirportOptions::default().approach_km);
        let _ = std::fs::remove_dir_all(dir);
    }
}
