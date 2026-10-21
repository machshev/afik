//! FM repeaters outside the UK, synced from hearham's worldwide repeater
//! list and the Israel Amateur Radio Club's network page, and cached; the
//! Repeaters tab takes them with the ETCC's.
//!
//! As with the ETCC list, the studio fetches only when the operator presses
//! Sync or nothing is cached (`ADR-121`); otherwise it reads each raw
//! response cached in `$XDG_CACHE_HOME/afik/<source>`.

use std::{fs, path::PathBuf, sync::mpsc, time::SystemTime};

use radio_channel_plan::MAX_CHANNEL_NAME_LEN;
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use crate::repeaters::fetch_within;
use crate::{
    paths,
    repeaters::{ctcss, receivable_hz, sanitise, Coverage, Repeater},
};

/// Every repeater hearham lists, worldwide: about 23 000, 9.5 MB.
pub const REPEATERS_URL: &str = "https://hearham.com/api/repeaters/v1";

/// The callsign prefixes the ITU gives the UK, which every ETCC system has.
/// hearham's UK entries are an old copy of ukrepeater.net and IRLP's list;
/// the ETCC list the Repeaters tab reads for the UK is the current one.
const UK_PREFIXES: [char; 5] = ['G', 'M', '2', 'g', 'm'];

/// The Israel Amateur Radio Club's repeater network: a page of tables, one
/// per band and mode.
pub const IARC_URL: &str = "https://www.iarc.org/iarc-repeaters-network/";

/// The callsign prefixes Israel's repeaters have. hearham's few are stale
/// (the 2 m network is missing, and its UHF entries are digital by default);
/// the IARC's page is the current list.
const ISRAEL_PREFIXES: [&str; 2] = ["4X", "4Z"];

/// Long enough for the whole list on a slow link.
#[cfg(not(target_arch = "wasm32"))]
const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Where a list of repeaters comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// hearham's worldwide list.
    Hearham,
    /// The Israel Amateur Radio Club.
    Iarc,
}

impl Source {
    /// Every source, each cached in its own directory.
    pub const ALL: [Self; 2] = [Self::Hearham, Self::Iarc];

    /// The directory the source is cached and mirrored in.
    pub const fn cache_dir(self) -> &'static str {
        match self {
            Self::Hearham => "hearham",
            Self::Iarc => "iarc",
        }
    }

    /// The file the raw response is kept in.
    pub const fn file(self) -> &'static str {
        match self {
            Self::Hearham => "repeaters.json",
            Self::Iarc => "repeaters.html",
        }
    }

    /// Where the list is fetched from.
    pub const fn url(self) -> &'static str {
        match self {
            Self::Hearham => REPEATERS_URL,
            Self::Iarc => IARC_URL,
        }
    }

    /// Reads the raw response.
    pub fn parse(self, text: &str) -> Result<Vec<Repeater>, String> {
        match self {
            Self::Hearham => parse(text),
            Self::Iarc => parse_iarc(text),
        }
    }
}

/// Picks the analogue FM repeaters out of hearham's list.
///
/// A repeater is taken when it is operational, its mode is FM or a mix with
/// FM (such as `YSF/FM`), it has an offset, and the radio can receive both
/// its output and its input, the output plus the offset. hearham lists
/// gateways, APRS digipeaters and iGates as FM too; they are simplex. UK
/// callsigns are left to the ETCC list, and Israeli ones to the IARC's. Its `encode` tone, the one a
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
                .map(|callsign| callsign.trim_start().to_ascii_uppercase())
                .is_some_and(|callsign| {
                    callsign.starts_with(UK_PREFIXES)
                        || ISRAEL_PREFIXES
                            .iter()
                            .any(|prefix| callsign.starts_with(prefix))
                })
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

/// Picks the analogue repeaters out of the IARC's network page.
///
/// The page holds a table per band and mode. The analogue tables have
/// Repeater, Frequency (the output, MHz), Shift (such as `– 600KHz`), PL(tx),
/// PL(rx) and Location columns; the DMR tables have none of Frequency and
/// Shift and are passed over. A row the comments call digital by default is
/// left out. PL(rx), the tone the repeater receives, is the access tone, else
/// PL(tx), since the network keeps one tone on both. The page names only a
/// place, so each is placed at its town in [`site`]; a place not there is
/// left out until it is added.
pub fn parse_iarc(html: &str) -> Result<Vec<Repeater>, String> {
    let mut repeaters = Vec::new();
    for table in html.split("<table").skip(1) {
        let table = table.split("</table>").next().unwrap_or_default();
        let mut rows = table.split("<tr").skip(1).map(cells);
        let Some(heading) = rows.next() else {
            continue;
        };
        let column = |name: &str| {
            heading
                .iter()
                .position(|cell| cell.eq_ignore_ascii_case(name))
        };
        let (Some(name_at), Some(output_at), Some(shift_at), Some(place_at)) = (
            column("Repeater"),
            column("Frequency"),
            column("Shift"),
            column("Location"),
        ) else {
            continue;
        };
        let tones_at = [column("PL(rx)"), column("PL(tx)")];
        let comments_at = column("Comments");
        for row in rows {
            let field =
                |at: Option<usize>| at.and_then(|at| row.get(at)).map_or("", String::as_str);
            if field(comments_at)
                .to_ascii_lowercase()
                .contains("digital repeater by default")
            {
                continue;
            }
            let place = field(Some(place_at));
            let Some((latitude, longitude)) = site(place) else {
                log::debug!("IARC repeater at {place:?}: no site known");
                continue;
            };
            let Some(output) = hertz(field(Some(output_at)), 1e6) else {
                continue;
            };
            let Some(shift) = shift_hz(field(Some(shift_at))) else {
                continue;
            };
            let (Some(output_hz), Some(input_hz)) = (
                receivable_hz(output),
                output.checked_add(shift).and_then(receivable_hz),
            ) else {
                continue;
            };
            let name = format!("{} {place}", field(Some(name_at)).replace('-', ""));
            let Some(callsign) = sanitise(&name, MAX_CHANNEL_NAME_LEN) else {
                continue;
            };
            repeaters.push(Repeater {
                callsign,
                town: place.to_owned(),
                output_hz,
                input_hz,
                ctcss_tenths_hz: tones_at
                    .iter()
                    .find_map(|&at| field(at).parse::<f64>().ok())
                    .and_then(ctcss),
                wide: true,
                latitude,
                longitude,
                coverage: Coverage::Standard,
            });
        }
    }
    if repeaters.is_empty() {
        return Err("IARC repeaters: no analogue repeaters".to_owned());
    }
    repeaters.sort_by(|a, b| {
        a.callsign
            .cmp(&b.callsign)
            .then(a.output_hz.cmp(&b.output_hz))
    });
    Ok(repeaters)
}

/// The text of each `<td>` or `<th>` cell in an HTML table row.
fn cells(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut rest = row;
    while let Some(at) = ["<td", "<th"].iter().filter_map(|tag| rest.find(tag)).min() {
        let Some(open) = rest[at..].find('>') else {
            break;
        };
        let body = &rest[at + open + 1..];
        let end = body.find("</t").unwrap_or(body.len());
        cells.push(text(&body[..end]));
        rest = &body[end..];
    }
    cells
}

/// HTML as plain text: tags dropped, character references decoded, spaces
/// collapsed.
fn text(html: &str) -> String {
    let mut plain = String::new();
    let mut rest = html;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
            plain.push(' ');
            continue;
        }
        if c == '&' {
            if let Some(end) = rest.find(';').filter(|&end| end <= 10) {
                let name = &rest[1..end];
                let decoded = match name {
                    "amp" => Some('&'),
                    "nbsp" => Some(' '),
                    "quot" => Some('"'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    _ => name
                        .strip_prefix('#')
                        .and_then(|number| number.parse().ok())
                        .and_then(char::from_u32),
                };
                if let Some(decoded) = decoded {
                    plain.push(decoded);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A number of `unit` hertz as whole hertz.
// Rounded first, and a frequency is far below `i64::MAX`.
#[allow(clippy::cast_possible_truncation)]
fn hertz(number: &str, unit: f64) -> Option<i64> {
    let value: f64 = number.trim().parse().ok()?;
    let hz = (value * unit).round();
    (hz.is_finite() && hz.abs() < 1e12).then_some(hz as i64)
}

/// A shift such as `– 600KHz`, `+ 600KHz` or `-7.6MHz`, in hertz.
fn shift_hz(shift: &str) -> Option<i64> {
    let shift = shift.trim();
    let negative = shift.starts_with(['-', '–', '−']);
    let magnitude = shift.trim_start_matches(['-', '–', '−', '+', ' ']);
    let lower = magnitude.to_ascii_lowercase();
    let (number, unit) = match lower.strip_suffix("mhz") {
        Some(number) => (number, 1e6),
        None => (lower.strip_suffix("khz")?, 1e3),
    };
    let hz = hertz(number, unit)?;
    Some(if negative { -hz } else { hz })
}

/// Where each place the IARC names is, in degrees north and east: the town,
/// or for a mountain or forest its summit area, from maps to about a
/// kilometre; the page gives no sites. Names are matched on their letters
/// and digits alone, so `Be'er Sheva` and `Beer Sheva` are one place.
fn site(place: &str) -> Option<(f64, f64)> {
    let key: String = place
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let site = match key.as_str() {
        "ashkelon" => (31.67, 34.57),
        "beersheva" => (31.25, 34.79),
        "eilat" => (29.56, 34.95),
        "givatayim" => (32.07, 34.81),
        "haifa" => (32.79, 34.99),
        "hermon" => (33.30, 35.77),
        "jerusalem" => (31.78, 35.22),
        "kiryatono" => (32.06, 34.86),
        "maalegilboa" => (32.48, 35.41),
        "megiddo" => (32.58, 35.18),
        "mitzperamon" => (30.61, 34.80),
        "netanya" => (32.33, 34.86),
        "roshhaayin" => (32.10, 34.96),
        "telaviv" => (32.08, 34.78),
        "yatir" => (31.36, 35.05),
        _ => return None,
    };
    Some(site)
}

/// The repeaters a sync read, and when the cache was written.
#[derive(Clone, Debug)]
pub struct WorldList {
    /// Every FM repeater, of every source read, ordered by callsign within
    /// each source.
    pub repeaters: Vec<Repeater>,
    /// When the oldest cache read was last written.
    pub synced: SystemTime,
    /// Whether every source was read; a cache from before a source was
    /// added lacks it until the next sync.
    pub complete: bool,
}

/// Where each source's raw response is kept.
#[derive(Clone, Debug)]
pub struct WorldCache {
    /// Each source and its cache directory, if it has one.
    sources: Vec<(Source, Option<PathBuf>)>,
}

impl WorldCache {
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
    pub fn load(&self) -> Result<Option<WorldList>, String> {
        let mut repeaters = Vec::new();
        let mut synced: Option<SystemTime> = None;
        let mut complete = true;
        for (source, dir) in &self.sources {
            let Some(path) = dir
                .as_ref()
                .map(|dir| dir.join(source.file()))
                .filter(|path| path.exists())
            else {
                complete = false;
                continue;
            };
            let failed = |error: std::io::Error| format!("{}: {error}", path.display());
            let text = fs::read_to_string(&path).map_err(failed)?;
            let written = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map_err(failed)?;
            repeaters.extend(source.parse(&text)?);
            synced = Some(synced.map_or(written, |oldest| oldest.min(written)));
        }
        Ok(synced.map(|synced| WorldList {
            repeaters,
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
        fetch: impl Fn(&str) -> Result<String, String>,
    ) -> Result<WorldList, String> {
        let mut failures = Vec::new();
        for (source, dir) in &self.sources {
            let synced = (|| {
                let dir = dir
                    .as_ref()
                    .ok_or("no XDG cache directory or HOME is set")?;
                let text = fetch(source.url())?;
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
            .ok_or_else(|| "the repeater cache was not written".to_owned())
    }

    /// Fetches from each source, checks and caches what came, and returns
    /// it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn sync(&self) -> Result<WorldList, String> {
        self.sync_with(|url| fetch_within(url, SYNC_TIMEOUT))
    }

    /// Reads what the Pages build mirrored from each source beside the
    /// browser Studio (`ADR-121`); the page keeps nothing. A source the
    /// mirror lacks leaves the list incomplete rather than empty.
    #[cfg(target_arch = "wasm32")]
    pub async fn sync(&self) -> Result<WorldList, String> {
        let mut list: Option<WorldList> = None;
        let mut failures = Vec::new();
        for (source, _) in &self.sources {
            let read = crate::mirror::fetch(source.cache_dir(), [source.file()])
                .await
                .and_then(|([text], synced)| Ok((source.parse(&text)?, synced)));
            let (repeaters, synced) = match read {
                Ok(read) => read,
                Err(error) => {
                    log::warn!("repeaters from {}: {error}", source.cache_dir());
                    failures.push(error);
                    continue;
                }
            };
            match &mut list {
                Some(list) => {
                    list.repeaters.extend(repeaters);
                    list.synced = list.synced.min(synced);
                }
                None => {
                    list = Some(WorldList {
                        repeaters,
                        synced,
                        complete: true,
                    });
                }
            }
        }
        let mut list = list.ok_or_else(|| failures.join("; "))?;
        list.complete = failures.is_empty();
        Ok(list)
    }
}

/// A sync running on a worker thread, so the window keeps drawing.
pub struct SyncJob {
    outcome: mpsc::Receiver<Result<WorldList, String>>,
}

impl SyncJob {
    /// Starts fetching from hearham and the IARC into `cache`.
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
                Some(Err("the repeater sync stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use radio_domain::{Bandwidth, Tone};

    use super::{parse, parse_iarc, Source, WorldCache, IARC_URL, REPEATERS_URL};
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
         "frequency": 145612500, "offset": -600000, "operational": 1},
        {"callsign": "4X471R", "latitude": 32.10474, "longitude": 34.97169,
         "city": "Rosh Haain, Israel, Central", "mode": "FM", "encode": "91.5",
         "frequency": 438675000, "offset": -7600000, "operational": 1}
    ]"#;

    /// The IARC page's tables in the shape it publishes them, trimmed: the
    /// analogue VHF and UHF tables and a DMR one, with the page around them.
    const IARC: &str = "<html><body><h3>Analog VHF Repeaters</h3>
<table class=\"table table-striped\">
<thead>
<tr>
<th style=\"text-align: left;\">Repeater</th>
<th style=\"text-align: left;\">Frequency</th>
<th style=\"text-align: left;\">Shift</th>
<th style=\"text-align: left;\">PL(tx)</th>
<th style=\"text-align: left;\">PL(rx)</th>
<th style=\"text-align: left;\">Location</th>
<th style=\"text-align: left;\">Comments</th>
</tr>
</thead>
<tbody>
<tr>
<td>R-1</td><td>145.625</td><td>&#8211; 600KHz</td><td>91.5</td><td>91.5</td>
<td>Jerusalem</td><td>Connected to multi-site repeater</td>
</tr>
<tr>
<td>R-11.5</td><td>145.2875</td><td>&#8211; 600KHz</td><td>91.5</td><td>91.5</td>
<td>Be&#8217;er Sheva</td><td></td>
</tr>
<tr>
<td>R-15</td><td>144.775</td><td>+ 600KHz</td><td>91.5</td><td>114.8</td>
<td>Giv'atayim</td><td></td>
</tr>
<tr>
<td>R-17</td><td>145.425</td><td>&#8211; 600KHz</td><td>91.5</td><td></td>
<td>Netanya</td><td></td>
</tr>
<tr>
<td>R-99</td><td>145.500</td><td>&#8211; 600KHz</td><td>91.5</td><td>91.5</td>
<td>Atlantis</td><td></td>
</tr>
</tbody>
</table>
<h3>Analog UHF Repeaters</h3>
<table class=\"table table-striped\">
<thead>
<tr>
<th>Repeater</th><th>Frequency</th><th>Shift</th><th>PL(tx)</th><th>PL(rx)</th>
<th>Location</th><th>Comments</th>
</tr>
</thead>
<tbody>
<tr>
<td>R-70</td><td>438.650</td><td>-7.6MHz</td><td>91.5</td><td>91.5</td>
<td>Tel Aviv</td><td>Digital repeater by default</td>
</tr>
<tr>
<td>R-74</td><td>438.750</td><td>-7.6MHz</td><td>91.5</td><td>91.5</td>
<td>Netanya</td><td>Analog only</td>
</tr>
</tbody>
</table>
<h3>DMR Repeaters</h3>
<table class=\"table table-striped\">
<thead>
<tr>
<th>Repeater</th><th>Rx</th><th>Tx</th><th>Color Code</th><th>QTH</th>
</tr>
</thead>
<tbody>
<tr>
<td>R-68 (425101)</td><td>438.600</td><td>431.000</td><td>1</td><td>Jerusalem</td>
</tr>
</tbody>
</table>
</body></html>";

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
        // callsign, simplex (an APRS iGate, a gateway), UK callsigns, left to
        // the ETCC list, and Israeli ones, left to the IARC's, are left out.
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
    fn the_iarc_page_gives_its_analogue_repeaters_at_their_towns() {
        let repeaters = parse_iarc(IARC).expect("parses");
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
        // Digital by default (R-70), a place with no known site (R-99) and
        // the DMR table are left out; PL(rx) is the access tone, else PL(tx).
        assert_eq!(
            summary,
            [
                ("R1 JERUSALEM", 145_625_000, 145_025_000, Some(915)),
                ("R11.5 BEER S", 145_287_500, 144_687_500, Some(915)),
                ("R15 GIV'ATAY", 144_775_000, 145_375_000, Some(1148)),
                ("R17 NETANYA", 145_425_000, 144_825_000, Some(915)),
                ("R74 NETANYA", 438_750_000, 431_150_000, Some(915)),
            ]
        );
        let jerusalem = &repeaters[0];
        assert_eq!(jerusalem.town, "Jerusalem");
        assert!((jerusalem.latitude - 31.78).abs() < 0.01);
        // KM72JB, Tel Aviv: Giv'atayim and Netanya are in 50 km, Jerusalem
        // about 55 km and Be'er Sheva about 95 km are not.
        let near = nearby(&repeaters, "KM72JB", 50).unwrap();
        let names: Vec<_> = near.iter().map(|(r, _)| r.callsign.as_str()).collect();
        assert_eq!(names, ["R15 GIV'ATAY", "R17 NETANYA", "R74 NETANYA"]);
        assert!(parse_iarc("<table><tr><th>Repeater</th></tr></table>").is_err());
    }

    #[test]
    fn every_source_is_synced_and_read_as_one_list() {
        let dir = std::env::temp_dir().join(format!("afik-world-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = WorldCache {
            sources: Source::ALL
                .into_iter()
                .map(|source| (source, Some(dir.join(source.cache_dir()))))
                .collect(),
        };
        let list = cache
            .sync_with(|url| match url {
                REPEATERS_URL => Ok(REPEATERS.to_owned()),
                IARC_URL => Ok(IARC.to_owned()),
                _ => Err(format!("unexpected {url}")),
            })
            .unwrap();
        assert_eq!((list.repeaters.len(), list.complete), (3 + 5, true));
        // A failed source fails the sync, but keeps what it had.
        let error = cache
            .sync_with(|url| match url {
                IARC_URL => Err("HTTP 503".to_owned()),
                _ => Ok(REPEATERS.to_owned()),
            })
            .unwrap_err();
        assert_eq!(error, "HTTP 503");
        assert_eq!(cache.load().unwrap().unwrap().repeaters.len(), 3 + 5);
        // A cache from before the IARC was a source is read, but incomplete.
        std::fs::remove_dir_all(dir.join(Source::Iarc.cache_dir())).unwrap();
        let list = cache.load().unwrap().unwrap();
        assert_eq!((list.repeaters.len(), list.complete), (3, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sync_caches_only_a_list_that_parses() {
        let dir = std::env::temp_dir().join(format!("afik-hearham-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = WorldCache::in_dir(Source::Hearham, dir.clone());
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
        let list = WorldCache::in_dir(Source::Hearham, dir.clone())
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
