//! What the radio listens with, which sets how far each generator reaches.
//!
//! The operator picks a handheld, a mobile, or a base station with its
//! aerial on the roof. Each sets the generators' radii, scales the airband
//! services' hearing ranges, and gives the aerial height the FM ranking's
//! radio horizon uses. The choice is kept in `$XDG_STATE_HOME/afik/receiver`.
//!
//! The figures are estimates from aerial height and gain, not measurements.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::paths;

const FILE: &str = "receiver";

/// The radius, in kilometres, each generator with one starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Radii {
    /// UK and worldwide repeaters.
    pub repeaters: u16,
    /// Gateways: low-power simplex nodes.
    pub gateways: u16,
    /// Beacons, heard on SSB or CW.
    pub beacons: u16,
    /// Airports: approach, aircraft at height.
    pub airports: u16,
    /// Airfields: gliders at height and small ground stations.
    pub airfields: u16,
    /// Coast stations, ships, marinas and locks.
    pub marine: u16,
}

/// What the radio listens with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Receiver {
    /// A handheld on its own aerial, at head height.
    #[default]
    Handheld,
    /// A mobile with a roof-mounted aerial on a car.
    Mobile,
    /// A base station with its aerial on the roof of a house.
    Base,
}

impl Receiver {
    /// Every receiver, in the order the dropdown shows them.
    pub const ALL: [Self; 3] = [Self::Handheld, Self::Mobile, Self::Base];

    /// What the dropdown calls it.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Handheld => "Handheld",
            Self::Mobile => "Mobile",
            Self::Base => "Base station",
        }
    }

    /// What it means, for the dropdown's hover text.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Handheld => "A handheld on its own aerial, at head height",
            Self::Mobile => "A car with a roof-mounted aerial",
            Self::Base => "A base station with its aerial on the roof, about 10 m up",
        }
    }

    /// The word the state file keeps it under.
    const fn word(self) -> &'static str {
        match self {
            Self::Handheld => "handheld",
            Self::Mobile => "mobile",
            Self::Base => "base",
        }
    }

    /// The aerial's height above the ground, in metres, for the radio
    /// horizon.
    pub const fn aerial_height_m(self) -> f64 {
        match self {
            Self::Handheld => 2.0,
            Self::Mobile => 1.5,
            Self::Base => 10.0,
        }
    }

    /// `km`, a handheld's hearing range, as far as this receiver hears: a
    /// mobile's better aerial reaches half as far again, a roof aerial's
    /// height twice as far.
    pub const fn reach_km(self, km: u16) -> u16 {
        let percent: u32 = match self {
            Self::Handheld => 100,
            Self::Mobile => 150,
            Self::Base => 200,
        };
        #[allow(clippy::cast_possible_truncation)]
        let reach = (km as u32 * percent / 100) as u16;
        reach
    }

    /// The radius each generator starts at, each one the studio offers.
    pub const fn radii(self) -> Radii {
        match self {
            Self::Handheld => Radii {
                repeaters: 50,
                gateways: 20,
                beacons: 150,
                airports: 150,
                airfields: 50,
                marine: 30,
            },
            Self::Mobile => Radii {
                repeaters: 75,
                gateways: 30,
                beacons: 200,
                airports: 200,
                airfields: 75,
                marine: 50,
            },
            Self::Base => Radii {
                repeaters: 100,
                gateways: 50,
                beacons: 300,
                airports: 300,
                airfields: 100,
                marine: 75,
            },
        }
    }

    /// The state file the receiver is kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(FILE))
    }

    /// Reads the receiver; anything missing or unreadable is a handheld.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|receiver| text.trim() == receiver.word())
            .unwrap_or_default()
    }

    /// Writes the receiver for the next start.
    pub fn save(self, file: &Path) -> Result<(), String> {
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, format!("{}\n", self.word())))
            .map_err(|error| format!("{}: {error}", file.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::Receiver;
    use crate::repeaters::RADII_KM;

    #[test]
    fn each_receiver_reaches_further_than_the_last() {
        for pair in Receiver::ALL.windows(2) {
            let (less, more) = (pair[0].radii(), pair[1].radii());
            assert!(less.repeaters < more.repeaters, "{pair:?}");
            assert!(less.gateways < more.gateways, "{pair:?}");
            assert!(less.beacons < more.beacons, "{pair:?}");
            assert!(less.airports < more.airports, "{pair:?}");
            assert!(less.airfields < more.airfields, "{pair:?}");
            assert!(less.marine < more.marine, "{pair:?}");
            assert!(pair[0].reach_km(30) < pair[1].reach_km(30), "{pair:?}");
        }
        assert_eq!(Receiver::Handheld.reach_km(150), 150);
        assert_eq!(Receiver::Base.reach_km(150), 300);
    }

    #[test]
    fn every_radius_is_one_the_dropdowns_offer() {
        for receiver in Receiver::ALL {
            let radii = receiver.radii();
            for km in [
                radii.repeaters,
                radii.gateways,
                radii.beacons,
                radii.airports,
                radii.airfields,
                radii.marine,
            ] {
                assert!(RADII_KM.contains(&km), "{receiver:?} {km}");
            }
        }
    }

    #[test]
    fn the_receiver_comes_back_as_saved() {
        let dir = std::env::temp_dir().join(format!("afik-receiver-{}", std::process::id()));
        let file = dir.join("receiver");
        assert_eq!(Receiver::load(&file), Receiver::Handheld, "no file");
        for receiver in Receiver::ALL {
            receiver.save(&file).unwrap();
            assert_eq!(Receiver::load(&file), receiver);
        }
        std::fs::write(&file, "satellite dish\n").unwrap();
        assert_eq!(Receiver::load(&file), Receiver::Handheld);
        let _ = std::fs::remove_dir_all(dir);
    }
}
