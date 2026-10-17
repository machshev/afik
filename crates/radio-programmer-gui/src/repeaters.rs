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
    receiver::Receiver,
};

/// Every repeater, gateway and beacon the ETCC has licensed.
pub const SYSTEMS_URL: &str = "https://api-beta.rsgb.online/all/systems";

const CACHE_DIR: &str = "etcc";
const SYSTEMS_FILE: &str = "systems.json";
const AREA_FILE: &str = "repeater-area";
const GATEWAY_AREA_FILE: &str = "gateway-area";
const REPEATER_BANDS_FILE: &str = "repeater-bands";
const GATEWAY_BANDS_FILE: &str = "gateway-bands";
const BEACON_BANDS_FILE: &str = "beacon-bands";
const BEACON_AREA_FILE: &str = "beacon-area";
const REACH_FILE: &str = "repeater-reach";
/// How many repeaters a bank takes before the operator chooses: about a
/// tenth of the radio's image.
pub const DEFAULT_MAX_CHANNELS: u16 = 40;
/// The most the operator can ask for: more than the UK list holds, so the
/// storage the studio shows, not this, is what limits a bank. About 380
/// repeaters fit an otherwise empty radio.
pub const MAX_CHANNELS_LIMIT: u16 = 500;
/// The radius repeaters are taken within before the operator chooses one,
/// in kilometres: about how far a hilltop repeater reaches a handheld.
pub const DEFAULT_RADIUS_KM: u16 = 50;
/// The radii the studio offers, in kilometres: from a handheld's reach of
/// a hilltop repeater up to the whole UK. The bank still takes only the
/// nearest [`RepeaterArea::max_channels`].
pub const RADII_KM: [u16; 12] = [
    10,
    20,
    30,
    50,
    75,
    100,
    150,
    200,
    300,
    500,
    1_000,
    MAX_RADIUS_KM,
];
/// The largest radius accepted; the whole UK is within it of anywhere in it.
pub const MAX_RADIUS_KM: u16 = 1_500;
const TIMEOUT: Duration = Duration::from_secs(30);
/// The largest list fetched is `OurAirports`' airports, about 13 MB; this
/// bounds a misbehaving server.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
/// Receive coverage the BK4819 datasheet states, in hertz.
const RECEIVE_BANDS: [(u64, u64); 2] = [(18_000_000, 660_000_000), (840_000_000, 1_300_000_000)];
/// ETCC system types which are repeaters: analogue voice, multimode (analogue
/// and digital), and linked. Gateways are single-frequency internet nodes.
const REPEATER_TYPES: [&str; 3] = ["AV", "DM", "RL"];
/// The ETCC system type of an analogue gateway.
const GATEWAY_TYPE: &str = "AG";
/// The ETCC system type of a beacon.
const BEACON_TYPE: &str = "BN";
/// How far below a beacon's carrier its USB channel is tuned, in hertz, so
/// its keyed carrier is heard as a tone of this pitch.
pub const BEACON_TONE_HZ: u32 = 700;
/// The tuning step of a beacon channel, fine enough to tune in a tone.
const BEACON_STEP_HZ: u32 = 100;
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
    /// Latitude of its grid square's centre, in degrees north.
    pub latitude: f64,
    /// Longitude of its grid square's centre, in degrees east.
    pub longitude: f64,
    /// How far it covers, from its power and aerial height.
    pub coverage: Coverage,
}

/// How far a repeater covers, judged from the power and aerial height the
/// ETCC lists; the site's own height is not listed, so a hilltop with a
/// short mast reads as standard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Coverage {
    /// Full power, 14 dBW (25 W), on an aerial at least
    /// [`WIDE_AREA_HEIGHT_M`] up: heard across a region.
    WideArea,
    /// Neither wide-area nor local, or not listed.
    #[default]
    Standard,
    /// Under 10 dBW (10 W), or an aerial under [`LOCAL_HEIGHT_M`] up: a
    /// town's repeater.
    Local,
}

/// How high a wide-area repeater's aerial is at least, in metres above
/// ground.
pub const WIDE_AREA_HEIGHT_M: u16 = 30;
/// How high a local repeater's aerial is at most, in metres above ground.
pub const LOCAL_HEIGHT_M: u16 = 10;

impl Coverage {
    /// The sizes whose reach the operator chooses, widest first; a
    /// wide-area repeater is heard across the repeater radius.
    pub const CHOSEN: [Self; 2] = [Self::Standard, Self::Local];

    /// The coverage of a repeater of `erp_dbw` on an aerial `height_m` above
    /// ground; a height of 0 is not listed.
    pub fn of(erp_dbw: Option<f64>, height_m: Option<u16>) -> Self {
        let height_m = height_m.filter(|height| *height > 0);
        let full_power = erp_dbw.is_some_and(|erp| erp >= 14.0);
        if full_power && height_m.is_some_and(|height| height >= WIDE_AREA_HEIGHT_M) {
            Self::WideArea
        } else if erp_dbw.is_some_and(|erp| erp < 10.0)
            || height_m.is_some_and(|height| height < LOCAL_HEIGHT_M)
        {
            Self::Local
        } else {
            Self::Standard
        }
    }

    /// What the studio calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::WideArea => "Wide-area",
            Self::Standard => "Standard",
            Self::Local => "Local",
        }
    }

    /// What it covers, for the studio's hover text.
    pub const fn help(self) -> &'static str {
        match self {
            Self::WideArea => "a full-power repeater on a high aerial",
            Self::Standard => "a repeater neither wide-area nor local",
            Self::Local => "a low-power repeater or one on a low aerial",
        }
    }

    /// Its place in [`Self::CHOSEN`], none for a wide-area repeater.
    pub const fn chosen(self) -> Option<usize> {
        match self {
            Self::WideArea => None,
            Self::Standard => Some(0),
            Self::Local => Some(1),
        }
    }
}

/// How far off a handheld hears each [`Coverage::CHOSEN`] repeater before
/// the operator chooses, in kilometres: an estimate, not a measurement.
pub const DEFAULT_REACH_KM: [u16; Coverage::CHOSEN.len()] = [30, 15];
/// The words the reach file keeps each chosen coverage's reach under.
const REACH_KEYS: [&str; Coverage::CHOSEN.len()] = ["standard", "local"];

/// How far off a handheld hears a standard and a local repeater; a
/// wide-area one is heard across the radius.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepeaterReach {
    /// In kilometres, by [`Coverage::chosen`].
    pub reach_km: [u16; Coverage::CHOSEN.len()],
}

impl Default for RepeaterReach {
    fn default() -> Self {
        Self {
            reach_km: DEFAULT_REACH_KM,
        }
    }
}

impl RepeaterReach {
    /// How far off `receiver` hears a repeater of `coverage`, in
    /// kilometres: as far as chosen for it, and a wide-area one across
    /// `radius_km`, whatever the receiver.
    pub const fn reach_km(&self, coverage: Coverage, receiver: Receiver, radius_km: u16) -> u16 {
        match coverage.chosen() {
            Some(at) => receiver.reach_km(self.reach_km[at]),
            None => radius_km,
        }
    }

    /// The state file the reach is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(REACH_FILE))
    }

    /// Reads `standard=KM local=KM`; anything missing or unreadable gives
    /// its default.
    pub fn load(file: &Path) -> Self {
        let mut reach = Self::default();
        let text = fs::read_to_string(file).unwrap_or_default();
        for word in text.split_whitespace() {
            for (at, key) in REACH_KEYS.iter().enumerate() {
                let km = word
                    .strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .and_then(|km| km.parse().ok())
                    .filter(|km| (1..=MAX_RADIUS_KM).contains(km));
                if let Some(km) = km {
                    reach.reach_km[at] = km;
                }
            }
        }
        reach
    }

    /// Writes the reach for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let words: Vec<String> = REACH_KEYS
            .iter()
            .zip(self.reach_km)
            .map(|(key, km)| format!("{key}={km}"))
            .collect();
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, words.join(" ") + "\n"))
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// The systems a studio bank can be built from, each ordered by callsign.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Systems {
    /// FM repeaters: hear the output, transmit on the input.
    pub repeaters: Vec<Repeater>,
    /// Analogue gateways: single-frequency internet nodes, so the output and
    /// input are the same.
    pub gateways: Vec<Repeater>,
    /// Beacons, which only send: the output and input are both the carrier.
    pub beacons: Vec<Repeater>,
}

/// Picks the FM repeaters out of the ETCC systems response.
///
/// A system is taken when it is a repeater type, operational (or at reduced
/// output), carries analogue FM, and the radio can receive both its output and
/// its input. Whether a channel may transmit is still decided by the
/// programmed TX ranges.
///
/// A repeater is placed at its national grid reference, a 1 km square, where
/// the ETCC gives one. Its locator is the fallback: about a quarter of them
/// are only a 4-character square, up to 80 km from the repeater.
pub fn parse(json: &str) -> Result<Vec<Repeater>, String> {
    Ok(parse_systems(json)?.repeaters)
}

/// Picks the repeaters, analogue gateways and beacons out of the ETCC
/// systems response, each as [`parse`] picks the repeaters. A gateway must
/// carry analogue FM as a repeater does; a beacon is taken whatever it sends,
/// on its carrier alone.
pub fn parse_systems(json: &str) -> Result<Systems, String> {
    let systems = match serde_json::from_str(json) {
        Ok(Value::Object(mut body)) => match body.remove("data") {
            Some(Value::Array(systems)) => systems,
            _ => return Err("ETCC systems: expected a data list".to_owned()),
        },
        Ok(_) => return Err("ETCC systems: expected an object".to_owned()),
        Err(error) => return Err(format!("ETCC systems: {error}")),
    };
    let take = |types: &[&str], analogue: bool| {
        let mut found: Vec<Repeater> = systems
            .iter()
            .filter(|system| types.contains(&text(system, "type").unwrap_or_default()))
            .filter(|system| {
                matches!(
                    text(system, "status"),
                    Some("OPERATIONAL" | "REDUCED OUTPUT")
                )
            })
            .filter(|system| {
                !analogue
                    || system
                        .get("modeCodes")
                        .and_then(Value::as_array)
                        .is_some_and(|modes| modes.iter().any(|mode| mode.as_str() == Some("A")))
            })
            .filter_map(|system| located(system, analogue))
            .collect();
        found.sort_by(|a, b| a.callsign.cmp(&b.callsign));
        found
    };
    Ok(Systems {
        repeaters: take(&REPEATER_TYPES, true),
        gateways: take(&[GATEWAY_TYPE], true),
        beacons: take(&[BEACON_TYPE], false),
    })
}

/// One system with its position, or none if the radio cannot receive it or
/// it has no usable callsign or place. One without an input, a beacon, is
/// given its output as its input.
fn located(system: &Value, has_input: bool) -> Option<Repeater> {
    let (latitude, longitude) = system
        .get("extraDetails")
        .and_then(|details| text(details, "ngr"))
        .and_then(|reference| grid_reference_centre(reference).ok())
        .or_else(|| locator_centre(text(system, "locator")?).ok())?;
    let output_hz = receivable(system, "tx")?;
    Some(Repeater {
        callsign: sanitise(text(system, "repeater")?, MAX_CHANNEL_NAME_LEN)?,
        town: text(system, "town").unwrap_or_default().to_owned(),
        output_hz,
        input_hz: if has_input {
            receivable(system, "rx")?
        } else {
            output_hz
        },
        ctcss_tenths_hz: system
            .get("ctcss")
            .and_then(Value::as_f64)
            .and_then(ctcss)
            .filter(|_| has_input),
        wide: system.get("txbw").and_then(Value::as_f64) == Some(25.0),
        latitude,
        longitude,
        coverage: Coverage::of(
            system.get("dbwErp").and_then(Value::as_f64),
            system
                .get("extraDetails")
                .and_then(|details| details.get("antennaHeight"))
                .and_then(Value::as_u64)
                .and_then(|height| u16::try_from(height).ok()),
        ),
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A frequency in hertz which the radio can receive.
fn receivable(value: &Value, key: &str) -> Option<u32> {
    receivable_hz(value.get(key).and_then(Value::as_i64)?)
}

/// `hz` if the radio can receive it.
pub(crate) fn receivable_hz(hz: i64) -> Option<u32> {
    let hz = u64::try_from(hz).ok()?;
    RECEIVE_BANDS
        .iter()
        .any(|&(low, high)| (low..=high).contains(&hz))
        .then(|| u32::try_from(hz).ok())
        .flatten()
}

/// A CTCSS tone in hertz as tenths, or none where the ETCC records zero.
// Checked against the CTCSS range first, so the cast is exact.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn ctcss(hz: f64) -> Option<u16> {
    let tenths = (hz * 10.0).round();
    (f64::from(MIN_CTCSS_TENTHS_HZ)..=f64::from(MAX_CTCSS_TENTHS_HZ))
        .contains(&tenths)
        .then_some(tenths as u16)
}

/// Upper-case printable ASCII no longer than `max`, or none if nothing is left.
pub(crate) fn sanitise(name: &str, max: usize) -> Option<String> {
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

/// A transverse Mercator national grid: its ellipsoid and projection.
struct NationalGrid {
    /// Semi-major and semi-minor axes in metres.
    a: f64,
    b: f64,
    /// Scale factor on the central meridian.
    f0: f64,
    /// True origin in degrees.
    latitude0: f64,
    longitude0: f64,
    /// False origin in metres.
    easting0: f64,
    northing0: f64,
}

/// The Ordnance Survey National Grid (OSGB36, Airy 1830).
const OS_GRID: NationalGrid = NationalGrid {
    a: 6_377_563.396,
    b: 6_356_256.909,
    f0: 0.999_601_271_7,
    latitude0: 49.0,
    longitude0: -2.0,
    easting0: 400_000.0,
    northing0: -100_000.0,
};

/// The Irish Grid (Ireland 1965, Airy modified), which Northern Ireland uses.
const IRISH_GRID: NationalGrid = NationalGrid {
    a: 6_377_340.189,
    b: 6_356_034.447,
    f0: 1.000_035,
    latitude0: 53.5,
    longitude0: -8.0,
    easting0: 200_000.0,
    northing0: 250_000.0,
};

/// The centre of a national grid reference square, as latitude and longitude
/// in degrees: Ordnance Survey letters and digits such as `TL5480`, or the
/// Irish grid as the ETCC writes it, `I` then its letter, such as `IJ4076`.
///
/// The datum is left as the grid's own; its difference from WGS84, about
/// 120 m, is far inside any square the ETCC gives.
pub fn grid_reference_centre(reference: &str) -> Result<(f64, f64), String> {
    let refused = || format!("{reference:?} is not a national grid reference");
    let clean: String = reference.chars().filter(|c| *c != ' ').collect();
    let clean = clean.to_ascii_uppercase();
    let letters = clean.bytes().take_while(u8::is_ascii_uppercase).count();
    let digits = &clean[letters..];
    if digits.is_empty()
        || digits.len() % 2 != 0
        || digits.len() > 10
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(refused());
    }
    // A letter's place in the 5 by 5 grid of squares, which leaves out I.
    let place = |letter: u8| match letter {
        b'A'..=b'H' => Some(i32::from(letter - b'A')),
        b'J'..=b'Z' => Some(i32::from(letter - b'A') - 1),
        _ => None,
    };
    let bytes = clean.as_bytes();
    let (grid, east_100km, north_100km) = match letters {
        2 if bytes[0] == b'I' => {
            let square = place(bytes[1]).ok_or_else(refused)?;
            (&IRISH_GRID, square % 5, 4 - square / 5)
        }
        2 => {
            let (major, minor) = (
                place(bytes[0]).ok_or_else(refused)?,
                place(bytes[1]).ok_or_else(refused)?,
            );
            (
                &OS_GRID,
                (major - 2).rem_euclid(5) * 5 + minor % 5,
                19 - (major / 5) * 5 - minor / 5,
            )
        }
        _ => return Err(refused()),
    };
    let half = digits.len() / 2;
    let size = 10_f64.powi(5 - i32::try_from(half).map_err(|_| refused())?);
    let offset = |text: &str| text.parse::<u32>().map(f64::from).map_err(|_| refused());
    let easting = f64::from(east_100km) * 100_000.0 + offset(&digits[..half])? * size + size / 2.0;
    let northing =
        f64::from(north_100km) * 100_000.0 + offset(&digits[half..])? * size + size / 2.0;
    Ok(grid.to_latitude_longitude(easting, northing))
}

impl NationalGrid {
    /// Inverse transverse Mercator, as in the Ordnance Survey's guide to
    /// coordinate systems in Great Britain, annex C.
    fn to_latitude_longitude(&self, easting: f64, northing: f64) -> (f64, f64) {
        let Self { a, b, f0, .. } = *self;
        let (latitude0, longitude0) = (self.latitude0.to_radians(), self.longitude0.to_radians());
        let e2 = 1.0 - (b * b) / (a * a);
        let n = (a - b) / (a + b);
        let meridional = |latitude: f64| {
            let (d, s) = (latitude - latitude0, latitude + latitude0);
            b * f0
                * ((1.0 + n + 1.25 * n * n + 1.25 * n.powi(3)) * d
                    - (3.0 * n + 3.0 * n * n + 21.0 / 8.0 * n.powi(3)) * d.sin() * s.cos()
                    + (15.0 / 8.0) * (n * n + n.powi(3)) * (2.0 * d).sin() * (2.0 * s).cos()
                    - (35.0 / 24.0) * n.powi(3) * (3.0 * d).sin() * (3.0 * s).cos())
        };
        let mut latitude = latitude0;
        let mut m = 0.0;
        for _ in 0..16 {
            latitude += (northing - self.northing0 - m) / (a * f0);
            m = meridional(latitude);
            if (northing - self.northing0 - m).abs() < 0.01 {
                break;
            }
        }
        let sin2 = latitude.sin().powi(2);
        let nu = a * f0 / (1.0 - e2 * sin2).sqrt();
        let rho = a * f0 * (1.0 - e2) / (1.0 - e2 * sin2).powf(1.5);
        let eta2 = nu / rho - 1.0;
        let (t, sec) = (latitude.tan(), 1.0 / latitude.cos());
        let (t2, t4) = (t * t, t.powi(4));
        let vii = t / (2.0 * rho * nu);
        let viii = t / (24.0 * rho * nu.powi(3)) * (5.0 + 3.0 * t2 + eta2 - 9.0 * t2 * eta2);
        let ix = t / (720.0 * rho * nu.powi(5)) * (61.0 + 90.0 * t2 + 45.0 * t4);
        let x = sec / nu;
        let xi = sec / (6.0 * nu.powi(3)) * (nu / rho + 2.0 * t2);
        let xii = sec / (120.0 * nu.powi(5)) * (5.0 + 28.0 * t2 + 24.0 * t4);
        let xiia =
            sec / (5040.0 * nu.powi(7)) * (61.0 + 662.0 * t2 + 1320.0 * t4 + 720.0 * t.powi(6));
        let de = easting - self.easting0;
        let latitude = latitude - vii * de.powi(2) + viii * de.powi(4) - ix * de.powi(6);
        let longitude =
            longitude0 + x * de - xi * de.powi(3) + xii * de.powi(5) - xiia * de.powi(7);
        (latitude.to_degrees(), longitude.to_degrees())
    }
}

/// Great-circle distance between two points in degrees, in kilometres.
pub fn distance_km(from: (f64, f64), to: (f64, f64)) -> f64 {
    let (lat1, lon1) = (from.0.to_radians(), from.1.to_radians());
    let (lat2, lon2) = (to.0.to_radians(), to.1.to_radians());
    let a = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

/// Every repeater within `radius_km` of `locator`, nearest first, each with
/// its distance in kilometres.
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
    Ok(near)
}

/// Every repeater within `radius_km` of `locator` that `receiver` hears by
/// its coverage, the most hearable (distance over reach) first, each with
/// its distance in kilometres.
pub fn hearable<'a>(
    repeaters: &'a [Repeater],
    locator: &str,
    radius_km: u16,
    reach: &RepeaterReach,
    receiver: Receiver,
) -> Result<Vec<(&'a Repeater, f64)>, String> {
    let reach_km =
        |repeater: &Repeater| f64::from(reach.reach_km(repeater.coverage, receiver, radius_km));
    let mut heard = nearby(repeaters, locator, radius_km)?;
    heard.retain(|(repeater, km)| *km <= reach_km(repeater));
    heard.sort_by(|a, b| {
        (a.1 / reach_km(a.0))
            .total_cmp(&(b.1 / reach_km(b.0)))
            .then(a.0.callsign.cmp(&b.0.callsign))
    });
    Ok(heard)
}

/// Builds the bank for the repeaters `nearby` chose, in their order: each
/// channel receives the output and transmits on the input with the access
/// tone, and keeps its own bandwidth.
pub fn repeater_bank(locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
    fm_bank("RPT", locator, near)
}

/// Builds the bank for the gateways `nearby` chose, in their order: each
/// channel is simplex on the node's frequency with its access tone.
pub fn gateway_bank(locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
    fm_bank("GW", locator, near)
}

/// The bank name for `prefix` and the locator, cut to what a radio holds.
fn bank_name(prefix: &str, locator: &str) -> String {
    let mut name = format!("{prefix} {}", locator.trim().to_ascii_uppercase());
    name.truncate(MAX_BANK_NAME_LEN);
    name
}

/// Builds the bank for the beacons `nearby` chose, in their order: each
/// channel receives USB [`BEACON_TONE_HZ`] below the carrier, so a keyed
/// carrier is heard as a tone, and never transmits (`ADR-107`).
pub fn beacon_bank(locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
    let channels = near
        .iter()
        .map(|(beacon, _)| ChannelDraft {
            name: beacon.callsign.clone(),
            receive_mhz: format_mhz(beacon.output_hz - BEACON_TONE_HZ),
            transmit_mhz: String::new(),
            modulation: Modulation::Usb,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low1,
            step_hz: BEACON_STEP_HZ,
            ..ChannelDraft::default()
        })
        .collect();
    BankDraft {
        name: bank_name("BCN", locator),
        kind: BankKind::Named,
        scan_enabled: true,
        shared: SharedSettings::from_bits(
            SharedSettings::MODULATION | SharedSettings::STEP | SharedSettings::POWER,
        )
        .unwrap_or_default(),
        modulation: Modulation::Usb,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low1,
        step_hz: BEACON_STEP_HZ,
        channels,
        ..BankDraft::default()
    }
}

/// An FM bank named `prefix` and the locator: each channel receives the
/// output and transmits on the input with the access tone, and keeps its own
/// bandwidth.
pub(crate) fn fm_bank(prefix: &str, locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
    let name = bank_name(prefix, locator);
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
    /// Every analogue gateway, ordered by callsign.
    pub gateways: Vec<Repeater>,
    /// Every beacon the radio can hear, ordered by callsign.
    pub beacons: Vec<Repeater>,
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
        let systems = parse_systems(&json)?;
        Ok(Some(RepeaterList {
            repeaters: systems.repeaters,
            gateways: systems.gateways,
            beacons: systems.beacons,
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
    /// How many of them the bank takes: nearest first, or for repeaters
    /// the most hearable first.
    pub max_channels: u16,
}

impl Default for RepeaterArea {
    fn default() -> Self {
        Self {
            locator: String::new(),
            radius_km: DEFAULT_RADIUS_KM,
            max_channels: DEFAULT_MAX_CHANNELS,
        }
    }
}

impl RepeaterArea {
    /// The state file the area is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(AREA_FILE))
    }

    /// Reads `LOCATOR RADIUS_KM MAX_CHANNELS`; anything missing or
    /// unreadable gives its default, so older files still load.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut words = text.split_whitespace();
        let locator = words.next().unwrap_or_default().to_owned();
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
            locator,
            radius_km,
            max_channels,
        }
    }

    /// Writes the area for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                fs::write(
                    file,
                    format!(
                        "{} {} {}\n",
                        self.locator.trim(),
                        self.radius_km,
                        self.max_channels
                    ),
                )
            })
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// An amateur band an ETCC system is on, which the gateway filter takes or
/// drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmateurBand {
    /// 10 m, 28-29.7 MHz.
    TenMetres,
    /// 6 m, 50-54 MHz.
    SixMetres,
    /// 4 m, 70-71 MHz.
    FourMetres,
    /// 2 m, 144-148 MHz.
    TwoMetres,
    /// 70 cm, 420-450 MHz.
    SeventyCentimetres,
    /// 23 cm, 1240-1325 MHz.
    TwentyThreeCentimetres,
}

impl AmateurBand {
    /// Every band, lowest first, as the studio shows them.
    pub const ALL: [Self; 6] = [
        Self::TenMetres,
        Self::SixMetres,
        Self::FourMetres,
        Self::TwoMetres,
        Self::SeventyCentimetres,
        Self::TwentyThreeCentimetres,
    ];

    /// The band holding `hz`, if any.
    pub fn of(hz: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|band| {
            let (low, high) = band.span_hz();
            (low..=high).contains(&hz)
        })
    }

    /// The band's edges in hertz, wide enough for every ITU region.
    const fn span_hz(self) -> (u32, u32) {
        match self {
            Self::TenMetres => (28_000_000, 29_700_000),
            Self::SixMetres => (50_000_000, 54_000_000),
            Self::FourMetres => (70_000_000, 71_000_000),
            Self::TwoMetres => (144_000_000, 148_000_000),
            Self::SeventyCentimetres => (420_000_000, 450_000_000),
            Self::TwentyThreeCentimetres => (1_240_000_000, 1_325_000_000),
        }
    }

    /// The label the studio shows.
    pub const fn label(self) -> &'static str {
        match self {
            Self::TenMetres => "10 m",
            Self::SixMetres => "6 m",
            Self::FourMetres => "4 m",
            Self::TwoMetres => "2 m",
            Self::SeventyCentimetres => "70 cm",
            Self::TwentyThreeCentimetres => "23 cm",
        }
    }

    /// The word the state file keeps it as.
    const fn key(self) -> &'static str {
        match self {
            Self::TenMetres => "10m",
            Self::SixMetres => "6m",
            Self::FourMetres => "4m",
            Self::TwoMetres => "2m",
            Self::SeventyCentimetres => "70cm",
            Self::TwentyThreeCentimetres => "23cm",
        }
    }

    /// Its position in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Which amateur bands a bank takes systems from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandFilter {
    /// Whether each [`AmateurBand`] is taken, by [`AmateurBand::index`].
    pub bands: [bool; AmateurBand::ALL.len()],
}

impl Default for BandFilter {
    /// 2 m and 70 cm, the radio's amateur bands; every generator's band
    /// filter starts here.
    fn default() -> Self {
        let mut bands = [false; AmateurBand::ALL.len()];
        bands[AmateurBand::TwoMetres.index()] = true;
        bands[AmateurBand::SeventyCentimetres.index()] = true;
        Self { bands }
    }
}

impl BandFilter {
    /// Every band.
    pub const ALL: Self = Self {
        bands: [true; AmateurBand::ALL.len()],
    };

    /// Whether a system on `hz` is taken; one outside every band is kept,
    /// since no box could drop it.
    pub fn takes(&self, hz: u32) -> bool {
        AmateurBand::of(hz).is_none_or(|band| self.bands[band.index()])
    }

    /// Reads the bands taken by name; with no file, the default.
    pub fn load(file: &Path) -> Self {
        Self::load_or(file, &Self::default())
    }

    /// Reads the bands as [`Self::load`] does, with `defaults` when there is
    /// no file.
    pub fn load_or(file: &Path, defaults: &Self) -> Self {
        let Ok(text) = fs::read_to_string(file) else {
            return defaults.clone();
        };
        let mut filter = Self {
            bands: [false; AmateurBand::ALL.len()],
        };
        for word in text.split_whitespace() {
            if let Some(band) = AmateurBand::ALL.iter().find(|band| band.key() == word) {
                filter.bands[band.index()] = true;
            }
        }
        filter
    }

    /// Writes the bands taken for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let words: Vec<_> = AmateurBand::ALL
            .iter()
            .filter(|band| self.bands[band.index()])
            .map(|band| band.key())
            .collect();
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, words.join(" ") + "\n"))
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

/// The state file the repeater bands are kept in.
pub fn repeater_bands_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(REPEATER_BANDS_FILE))
}

/// The state file the gateway bands are kept in.
pub fn gateway_bands_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(GATEWAY_BANDS_FILE))
}

/// The state file the beacon bands are kept in.
pub fn beacon_bands_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(BEACON_BANDS_FILE))
}

/// The state file the gateway radius and count are kept in; they read and
/// write as the airports' do.
pub fn gateway_area_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(GATEWAY_AREA_FILE))
}

/// The state file the beacon radius and count are kept in.
pub fn beacon_area_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(BEACON_AREA_FILE))
}

/// Fetches one URL over HTTPS. Text that is not UTF-8 is replaced rather
/// than refused, so a stray byte in a list does not stop a sync.
pub(crate) fn fetch(url: &str) -> Result<String, String> {
    fetch_within(url, TIMEOUT)
}

/// Fetches one URL as [`fetch`] does, allowing `timeout` for a slow server.
pub(crate) fn fetch_within(url: &str, timeout: Duration) -> Result<String, String> {
    log::debug!("fetching {url}");
    let started = std::time::Instant::now();
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .user_agent(concat!("afik-studio/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("{url}: {error}"))?;
    // A cut-off list could still parse, missing whatever came last.
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!("{url}: over {MAX_RESPONSE_BYTES} bytes"));
    }
    log::debug!(
        "fetched {} bytes from {url} in {} ms",
        body.len(),
        started.elapsed().as_millis()
    );
    Ok(String::from_utf8_lossy(&body).into_owned())
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
    use radio_domain::{Modulation, Tone};

    use super::{
        beacon_bank, distance_km, gateway_bank, grid_reference_centre, hearable, locator_centre,
        nearby, parse, parse_systems, repeater_bank, AmateurBand, BandFilter, Coverage,
        RepeaterArea, RepeaterCache, RepeaterReach, MAX_RADIUS_KM, SYSTEMS_URL,
    };
    use crate::{model::ToneDraft, receiver::Receiver};

    #[test]
    fn power_and_aerial_height_say_how_far_a_repeater_covers() {
        assert_eq!(Coverage::of(Some(14.0), Some(58)), Coverage::WideArea);
        assert_eq!(Coverage::of(Some(14.0), Some(20)), Coverage::Standard);
        assert_eq!(Coverage::of(Some(11.5), Some(50)), Coverage::Standard);
        assert_eq!(Coverage::of(Some(7.0), Some(60)), Coverage::Local);
        assert_eq!(Coverage::of(Some(14.0), Some(6)), Coverage::Local);
        // A height of 0 is not listed, so says nothing.
        assert_eq!(Coverage::of(Some(14.0), Some(0)), Coverage::Standard);
        assert_eq!(Coverage::of(None, None), Coverage::Standard);

        let json = r#"{"data": [
            {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3PI",
             "town": "ROYSTON", "tx": 145750000, "rx": 145150000, "ctcss": 77,
             "txbw": 12.5, "locator": "IO92XA", "dbwErp": 14,
             "extraDetails": {"ngr": "TL3736", "antennaHeight": 58, "polarisation": "V"}},
            {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3DE-L",
             "town": "IPSWICH", "tx": 433250000, "rx": 434850000, "ctcss": 110.9,
             "txbw": 12.5, "locator": "JO02QF", "dbwErp": 7,
             "extraDetails": {"ngr": "TM1844", "antennaHeight": 6, "polarisation": "V"}},
            {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"], "repeater": "GB3EL",
             "town": "ELY", "tx": 433100000, "rx": 434700000, "ctcss": 77,
             "txbw": 12.5, "locator": "JO02", "dbwErp": 11.8, "extraDetails": null}
        ]}"#;
        let coverage: Vec<_> = parse(json)
            .unwrap()
            .iter()
            .map(|r| (r.callsign.clone(), r.coverage))
            .collect();
        assert_eq!(
            coverage,
            [
                ("GB3DE-L".to_owned(), Coverage::Local),
                ("GB3EL".to_owned(), Coverage::Standard),
                ("GB3PI".to_owned(), Coverage::WideArea),
            ]
        );
    }

    #[test]
    fn a_repeater_is_taken_where_its_coverage_reaches_most_hearable_first() {
        let list = parse(SYSTEMS).unwrap();
        let at = |callsign: &str, coverage| super::Repeater {
            coverage,
            ..list
                .iter()
                .find(|r| r.callsign == callsign)
                .unwrap()
                .clone()
        };
        // From IO91WM: GB3WL is in the square, GB3EL about 12 km, GB3HW
        // about 25 km and GB3BB in Brecon about 220 km off.
        let repeaters = [
            at("GB3WL", Coverage::Standard),
            at("GB3EL", Coverage::Local),
            at("GB3HW", Coverage::Local),
            at("GB3BB", Coverage::WideArea),
        ];
        let reach = RepeaterReach::default();
        let names = |receiver, radius_km| -> Vec<String> {
            hearable(&repeaters, "IO91WM", radius_km, &reach, receiver)
                .unwrap()
                .iter()
                .map(|(r, _)| r.callsign.clone())
                .collect()
        };
        // The local one 25 km off is past a handheld's 15 km.
        assert_eq!(names(Receiver::Handheld, 50), ["GB3WL", "GB3EL"]);
        // The wide-area one is heard across the radius, most hearable by its
        // share of it.
        assert_eq!(names(Receiver::Handheld, 300), ["GB3WL", "GB3BB", "GB3EL"]);
        assert_eq!(names(Receiver::Base, 50), ["GB3WL", "GB3EL", "GB3HW"]);
    }

    #[test]
    fn the_reach_comes_back_as_saved_and_defaults_otherwise() {
        let dir = std::env::temp_dir().join(format!("afik-reach-{}", std::process::id()));
        let file = dir.join("repeater-reach");
        assert_eq!(RepeaterReach::load(&file), RepeaterReach::default());
        let chosen = RepeaterReach { reach_km: [40, 10] };
        chosen.save(&file).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "standard=40 local=10\n"
        );
        assert_eq!(RepeaterReach::load(&file), chosen);
        std::fs::write(&file, "standard=0 local=12\n").unwrap();
        assert_eq!(RepeaterReach::load(&file).reach_km, [30, 12]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gateway_bands_default_to_two_metres_and_seventy_centimetres() {
        assert_eq!(AmateurBand::of(51_810_000), Some(AmateurBand::SixMetres));
        assert_eq!(
            AmateurBand::of(1_297_000_000),
            Some(AmateurBand::TwentyThreeCentimetres)
        );
        assert_eq!(AmateurBand::of(100_000_000), None);
        let filter = BandFilter::default();
        assert!(filter.takes(145_337_500) && filter.takes(433_100_000));
        assert!(!filter.takes(28_500_000) && !filter.takes(70_400_000));
        assert!(filter.takes(100_000_000), "outside every band is kept");

        let dir = std::env::temp_dir().join(format!("afik-bands-{}", std::process::id()));
        let file = dir.join("gateway-bands");
        assert_eq!(BandFilter::load(&file), filter, "no file: the default");
        let mut chosen = BandFilter::default();
        chosen.bands[AmateurBand::FourMetres.index()] = true;
        chosen.bands[AmateurBand::SeventyCentimetres.index()] = false;
        chosen.save(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "4m 2m\n");
        assert_eq!(BandFilter::load(&file), chosen);
        std::fs::remove_dir_all(dir).unwrap();
    }

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
         "txbw": 12.5, "locator": "ZZ99"},
        {"type": "AG", "status": "OPERATIONAL", "modeCodes": ["D"], "repeater": "MB7DIG",
         "town": "LONDON", "tx": 430100000, "rx": 430100000, "ctcss": 0,
         "txbw": 12.5, "locator": "IO91WM"},
        {"type": "BN", "status": "OPERATIONAL", "modeCodes": ["B"], "repeater": "GB3VHF",
         "town": "WROTHAM", "tx": 144430000, "rx": 0, "ctcss": 0, "txbw": 1,
         "locator": "JO01EH"},
        {"type": "BN", "status": "OPERATIONAL", "modeCodes": ["B"], "repeater": "GB3UHF",
         "town": "WROTHAM", "tx": 432430000, "rx": 0, "ctcss": 0, "txbw": 1,
         "locator": "JO01EH"},
        {"type": "BN", "status": "NOT OPERATIONAL", "modeCodes": ["B"], "repeater": "GB3OLD",
         "town": "WROTHAM", "tx": 50050000, "rx": 0, "ctcss": 0, "txbw": 1,
         "locator": "JO01EH"},
        {"type": "BN", "status": "OPERATIONAL", "modeCodes": ["B"], "repeater": "G0HF",
         "town": "KENT", "tx": 10139930, "rx": 0, "ctcss": 0, "txbw": 0.5,
         "locator": "JO01NT"}
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
    fn a_grid_reference_gives_the_centre_of_its_square() {
        let near = |(lat, lon): (f64, f64), (want_lat, want_lon): (f64, f64), degrees: f64| {
            assert!(
                (lat - want_lat).abs() < degrees && (lon - want_lon).abs() < degrees,
                "({lat}, {lon}) is not ({want_lat}, {want_lon})"
            );
        };
        // The Ordnance Survey's worked example, TG 51409 13177, is
        // 52 39 27.2531 N, 1 43 4.5177 E on OSGB36; this is its 1 m centre.
        near(
            grid_reference_centre("TG 51409 13177").unwrap(),
            (52.657_570_3, 1.717_921_6),
            1e-5,
        );
        // GB3EL's square at Ely, which the ETCC locates only as JO02.
        near(
            grid_reference_centre("tl5480").unwrap(),
            (52.40, 0.27),
            0.01,
        );
        // Shetland's and Scilly's corners of the lettering.
        near(
            grid_reference_centre("HU4741").unwrap(),
            (60.15, -1.15),
            0.02,
        );
        near(
            grid_reference_centre("SV9010").unwrap(),
            (49.91, -6.31),
            0.02,
        );
        // Belfast City Hall on the Irish grid, J 338 742.
        near(
            grid_reference_centre("IJ338742").unwrap(),
            (54.597, -5.930),
            0.01,
        );
        for bad in [
            "", "TL", "TL548", "TL54A0", "II5480", "XI5480", "T5480", "TLL5480",
        ] {
            assert!(grid_reference_centre(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_repeater_is_placed_by_its_grid_reference_before_its_locator() {
        let systems = r#"{"data": [
            {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"],
             "repeater": "GB3EL", "town": "ELY", "tx": 430837500, "rx": 438437500,
             "ctcss": 156.7, "txbw": 12.5, "locator": "JO02",
             "extraDetails": {"ngr": "TL5480"}},
            {"type": "AV", "status": "OPERATIONAL", "modeCodes": ["A"],
             "repeater": "GB3PY", "town": "CAMBRIDGE", "tx": 145762500, "rx": 145162500,
             "ctcss": 77.0, "txbw": 12.5, "locator": "JO02BF",
             "extraDetails": {"ngr": ""}}
        ]}"#;
        let repeaters = parse(systems).unwrap();
        let ely = &repeaters[0];
        assert!((ely.latitude - 52.40).abs() < 0.01 && (ely.longitude - 0.27).abs() < 0.01);
        let near = nearby(&repeaters, "JO02EH", 50).unwrap();
        let names: Vec<_> = near.iter().map(|(r, _)| r.callsign.as_str()).collect();
        assert_eq!(names, ["GB3EL", "GB3PY"]);
        assert!(near[0].1 < 15.0, "{}", near[0].1);
    }

    #[test]
    fn gateways_are_analogue_nodes_on_one_frequency() {
        let gateways = parse_systems(SYSTEMS).expect("parses").gateways;
        let summary: Vec<_> = gateways
            .iter()
            .map(|g| {
                (
                    g.callsign.as_str(),
                    g.output_hz,
                    g.input_hz,
                    g.ctcss_tenths_hz,
                )
            })
            .collect();
        // The digital-only node is left out.
        assert_eq!(summary, [("MB7GATE", 145_337_500, 145_337_500, Some(770))]);
        let near = nearby(&gateways, "IO91WM", 10).unwrap();
        let bank = gateway_bank("io91wm", &near);
        assert_eq!(bank.name, "GW IO91WM");
        let channel = &bank.channels[0];
        assert_eq!(channel.name, "MB7GATE");
        assert_eq!(channel.receive_mhz, "145.337500");
        assert_eq!(channel.transmit_mhz, "145.337500");
        assert_eq!(channel.modulation, Modulation::Fm);
        assert_eq!(channel.tx_tone, ToneDraft::from_tone(Tone::Ctcss(770)));
    }

    #[test]
    fn beacons_the_radio_can_hear_are_tuned_below_their_carrier() {
        let beacons = parse_systems(SYSTEMS).expect("parses").beacons;
        let summary: Vec<_> = beacons
            .iter()
            .map(|b| {
                (
                    b.callsign.as_str(),
                    b.output_hz,
                    b.input_hz,
                    b.ctcss_tenths_hz,
                )
            })
            .collect();
        // Not operational, and below the radio's coverage, are left out.
        assert_eq!(
            summary,
            [
                ("GB3UHF", 432_430_000, 432_430_000, None),
                ("GB3VHF", 144_430_000, 144_430_000, None),
            ]
        );
        let near = nearby(&beacons, "JO01EH", 300).unwrap();
        let bank = beacon_bank("JO01EH", &near);
        assert_eq!(bank.name, "BCN JO01EH");
        assert_eq!(bank.modulation, Modulation::Usb);
        let tuned: Vec<_> = bank
            .channels
            .iter()
            .map(|c| {
                (
                    c.receive_mhz.as_str(),
                    c.transmit_mhz.as_str(),
                    c.modulation,
                )
            })
            .collect();
        assert_eq!(
            tuned,
            [
                ("432.429300", "", Modulation::Usb),
                ("144.429300", "", Modulation::Usb),
            ]
        );
        // A receive-only bank compiles without a transmit side.
        let project = crate::model::ProjectModel {
            banks: vec![bank],
            ..crate::model::ProjectModel::new()
        };
        assert!(project.to_radio_image().is_ok());
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
        assert_eq!(wide.len(), 5, "no limit here; the bank's is the caller's");
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
            max_channels: 12,
        };
        area.save(&file).unwrap();
        assert_eq!(RepeaterArea::load(&file), area);
        // Every radius offered, up to the whole UK, comes back as saved.
        for radius_km in super::RADII_KM {
            let area = RepeaterArea {
                locator: "IO91WM".to_owned(),
                radius_km,
                max_channels: 150,
            };
            area.save(&file).unwrap();
            assert_eq!(RepeaterArea::load(&file), area);
        }
        std::fs::write(&file, "IO91WM lots\n").unwrap();
        assert_eq!(RepeaterArea::load(&file).radius_km, 50);
        // A file from before the channel limit keeps its radius.
        std::fs::write(&file, "IO91WM 75\n").unwrap();
        let old = RepeaterArea::load(&file);
        assert_eq!((old.radius_km, old.max_channels), (75, 40));
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
        // How many of the nearest repeaters a radio holds, for the limit.
        let all = nearby(&list.repeaters, "IO91WM", MAX_RADIUS_KM).unwrap();
        let fits = |count: usize| {
            let mut whole = crate::model::ProjectModel::new();
            whole.banks.push(repeater_bank("IO91WM", &all[..count]));
            whole.to_radio_image().ok().map(|image| image.len())
        };
        let most = (1..=all.len()).rev().find(|&count| fits(count).is_some());
        println!("{most:?} of {} repeaters fit a radio", all.len());
        assert!(most.is_some_and(|most| most >= 100), "{most:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
