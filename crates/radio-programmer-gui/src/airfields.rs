//! Gliding, microlight and small-airfield frequencies near a Maidenhead
//! locator: the UK sporting and recreational assignments, then the small
//! fields `OurAirports` lists nearby.
//!
//! No new source is synced: the local frequencies come from the airports'
//! cache, and without it the bank holds the sporting core alone. The radius
//! and how many local channels to take are kept in
//! `$XDG_STATE_HOME/afik/airfield-area`, read and written as the airports'
//! are.

use std::{collections::HashSet, path::PathBuf};

use crate::{
    airports::{am_bank, am_channel, carrier_hz, nearby_matching, AirportArea, AirportFrequency},
    model::BankDraft,
    paths,
};

const AREA_FILE: &str = "airfield-area";

/// The UK sporting and recreational assignments every bank starts with, by
/// 8.33 kHz channel name as CAP 1606 (version 4, May 2019) gives them after
/// the 2019 conversion: Safetycom, the gliding common field, cross-country,
/// cloud flying and UK-mobile frequencies, microlight, balloon, and
/// parachute drop zone. The CAA withdrew CAP 1606 in favour of its frequency
/// assignments, so a site's own frequency, listed after, is the one to use.
const CORE: [(&str, &str); 9] = [
    ("SAFETYCOM", "135.480"),
    ("GLIDE FIELD", "129.980"),
    ("GLIDE XC", "130.105"),
    ("GLIDE XC 2", "130.130"),
    ("GLIDE CLOUD", "130.405"),
    ("GLIDE MOBILE", "129.905"),
    ("MICROLIGHT", "129.830"),
    ("BALLOON", "122.480"),
    ("PARA DZ", "130.530"),
];

/// How many channels a bank holds before any local one.
pub const CORE_CHANNELS: usize = CORE.len();

/// The core's names and carriers in hertz.
fn core() -> impl Iterator<Item = (&'static str, u32)> {
    CORE.iter()
        .filter_map(|&(name, channel)| Some((name, carrier_hz(channel)?)))
}

/// Every frequency of the small fields within `radius_km` of `locator`,
/// chosen and ordered as the airports' are, leaving out the core's carriers.
pub fn nearby<'a>(
    frequencies: &'a [AirportFrequency],
    locator: &str,
    radius_km: u16,
) -> Result<Vec<(&'a AirportFrequency, f64)>, String> {
    let core: HashSet<u32> = core().map(|(_, hz)| hz).collect();
    nearby_matching(
        frequencies,
        locator,
        radius_km,
        |frequency| frequency.small,
        core,
    )
}

/// Builds the receive-only AM bank: the core, then the local frequencies in
/// the order `nearby` chose.
pub fn airfield_bank(locator: &str, near: &[(&AirportFrequency, f64)]) -> BankDraft {
    let core = core().map(|(name, hz)| am_channel(name, hz));
    let local = near
        .iter()
        .map(|(frequency, _)| am_channel(&frequency.name, frequency.frequency_hz));
    am_bank("FLD", locator, core.chain(local).collect())
}

/// The state file the airfield radius and channel count are kept in.
pub fn area_file() -> Option<PathBuf> {
    paths::state_dir().map(|dir| dir.join(AREA_FILE))
}

/// The radius and how many local channels to take, as for the airports.
pub type AirfieldArea = AirportArea;

#[cfg(test)]
mod tests {
    use radio_domain::Modulation;

    use super::{airfield_bank, nearby, CORE_CHANNELS};
    use crate::{airports::AirportFrequency, model::ProjectModel};

    fn field(name: &str, hz: u32, latitude: f64, small: bool) -> AirportFrequency {
        AirportFrequency {
            name: name.to_owned(),
            ident: name.split(' ').next().unwrap_or_default().to_owned(),
            airport: String::new(),
            frequency_hz: hz,
            latitude,
            longitude: 0.0,
            small,
        }
    }

    #[test]
    fn the_core_is_the_sporting_carriers() {
        let bank = airfield_bank("io91wm", &[]);
        assert_eq!(bank.name, "FLD IO91WM");
        let core: Vec<_> = bank
            .channels
            .iter()
            .map(|c| (c.name.as_str(), c.receive_mhz.as_str()))
            .collect();
        // An 8.33 kHz name at the start of a 25 kHz block is that carrier.
        assert_eq!(
            core,
            [
                ("SAFETYCOM", "135.475000"),
                ("GLIDE FIELD", "129.975000"),
                ("GLIDE XC", "130.100000"),
                ("GLIDE XC 2", "130.125000"),
                ("GLIDE CLOUD", "130.400000"),
                ("GLIDE MOBILE", "129.900000"),
                ("MICROLIGHT", "129.825000"),
                ("BALLOON", "122.475000"),
                ("PARA DZ", "130.525000"),
            ]
        );
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Am && c.transmit_mhz.is_empty()));
    }

    #[test]
    fn local_channels_are_small_fields_not_in_the_core() {
        let frequencies = [
            field("EGXX TWR", 118_500_000, 51.5, false),
            field("GB-0001 A/G", 118_500_000, 51.6, true),
            field("GB-0002 A/G", 129_825_000, 51.55, true),
            field("GB-03 RADIO", 120_310_000, 51.7, true),
            field("GB-0004 A/G", 124_000_000, 55.0, true),
        ];
        let near = nearby(&frequencies, "IO91WM", 50).unwrap();
        let names: Vec<_> = near.iter().map(|(f, _)| f.name.as_str()).collect();
        // The large airport does not crowd out the strip sharing its
        // frequency; the microlight frequency is already in the core.
        assert_eq!(names, ["GB-0001 A/G", "GB-03 RADIO"]);
        let bank = airfield_bank("IO91WM", &near);
        assert_eq!(bank.channels.len(), CORE_CHANNELS + 2);
        assert_eq!(bank.channels[CORE_CHANNELS].name, "GB-0001 A/G");
        let project = ProjectModel {
            banks: vec![bank],
            ..ProjectModel::new()
        };
        assert!(project.to_radio_image().is_ok());
    }
}
