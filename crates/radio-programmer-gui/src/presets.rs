//! Default channel sets the editor can apply, chosen by region.
//!
//! A preset is a starting point, not an authority. Frequencies here are the
//! licence-free and amateur allocations an operator is most likely to want
//! first; the operator remains responsible for confirming them against their
//! own national band plan. Whether a channel may transmit is decided by the
//! programmed TX ranges, not here (`ADR-101`). Nothing here is applied without
//! an explicit request.
//!
//! A set is expressed as generated plans wherever its allocation is
//! arithmetic. A plan is one stored object however many channels it holds, so
//! a radio which would not fit these as records holds all of them and the
//! operator reads the designators a published band plan uses: `S20 CALL`
//! rather than a truncated bank name and a position. The marine channel
//! numbering is not arithmetic, so it is stored as named channels.

use radio_domain::{Bandwidth, Modulation, PowerLevel, TxClass};

use crate::model::{format_mhz, BankDraft, BankKind, ChannelDraft, ProjectModel, TxRangeDraft};

/// One generated plan a preset defines.
struct PresetPlan {
    /// Full name, which is what this editor labels the bank with.
    name: &'static str,
    /// Prefix the radio names each expanded channel with.
    designator: &'static str,
    /// Number the first expanded channel carries.
    first_number: u16,
    base_hz: u32,
    spacing_hz: u32,
    channels: u16,
    /// Zero-based index of the band's calling channel, if it has one.
    calling_index: Option<u16>,
    /// Fixed transmit offset. Zero is simplex.
    offset_hz: i32,
    modulation: Modulation,
    bandwidth: Bandwidth,
    power: PowerLevel,
}

/// One bank of explicitly listed channels a preset defines.
struct PresetList {
    name: &'static str,
    /// Channel name and the frequency the radio transmits on.
    channels: &'static [(&'static str, u32)],
    /// Receive offset from the transmit frequency. Zero is simplex.
    receive_offset_hz: u32,
    modulation: Modulation,
    bandwidth: Bandwidth,
    power: PowerLevel,
    step_hz: u32,
}

/// A named default set of plans and channel banks.
pub struct Preset {
    /// Region or allocation this set belongs to.
    name: &'static str,
    /// One-line description of what applying it produces.
    detail: &'static str,
    plans: &'static [PresetPlan],
    lists: &'static [PresetList],
}

/// The analogue PMR446 allocation: sixteen channels, 12.5 kHz from 446.00625.
const PMR446: PresetPlan = PresetPlan {
    name: "PMR446",
    // A trailing space separates the number, so channels read `PMR 1`.
    designator: "PMR ",
    first_number: 1,
    base_hz: 446_006_250,
    spacing_hz: 12_500,
    channels: 16,
    // The allocation designates no calling channel.
    calling_index: None,
    offset_hz: 0,
    modulation: Modulation::Fm,
    // Analogue PMR446 is narrow FM at low power by allocation.
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 2 m FM simplex: S8 at 145.200 through S23, calling on S20 at 145.500.
const TWO_METRE_SIMPLEX: PresetPlan = PresetPlan {
    name: "2M SIMPLEX",
    designator: "S",
    first_number: 8,
    base_hz: 145_200_000,
    spacing_hz: 25_000,
    channels: 16,
    // S20, twelve channels above S8.
    calling_index: Some(12),
    offset_hz: 0,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 70 cm FM simplex: SU16 at 433.400 through SU23, calling on SU20.
const SEVENTY_CENTIMETRE_SIMPLEX: PresetPlan = PresetPlan {
    name: "70CM SIMPLEX",
    designator: "SU",
    first_number: 16,
    base_hz: 433_400_000,
    spacing_hz: 25_000,
    channels: 8,
    // SU20 at 433.500, four channels above SU16.
    calling_index: Some(4),
    offset_hz: 0,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 2 m repeaters: outputs from 145.600 at 12.5 kHz, inputs 600 kHz below.
///
/// This is the fixed-offset family: the outputs are arithmetic and the distance
/// to the inputs is one constant, so a whole sub-band is one stored object.
const TWO_METRE_REPEATERS: PresetPlan = PresetPlan {
    name: "2M REPEATERS",
    designator: "RV",
    first_number: 48,
    base_hz: 145_600_000,
    spacing_hz: 12_500,
    channels: 16,
    calling_index: None,
    offset_hz: -600_000,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// The VHF civil airband at 25 kHz: 118.000 to 136.975, AM, receive only.
///
/// No TX range covers it, so the radio refuses to transmit here. The 8.33 kHz
/// channel numbering is not arithmetic and is deliberately not expressed here.
const AIRBAND: PresetPlan = PresetPlan {
    name: "AIRBAND",
    designator: "AIR",
    first_number: 1,
    base_hz: 118_000_000,
    spacing_hz: 25_000,
    channels: 760,
    calling_index: None,
    offset_hz: 0,
    modulation: Modulation::Am,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// Ship transmit frequency of an ITU-R M.1084 marine VHF channel number.
///
/// Channels 1 to 28 run from 156.050 MHz and 60 to 88 from 156.025 MHz, both
/// at 50 kHz, interleaving at 25 kHz.
const fn marine_hz(channel: u32) -> u32 {
    if channel >= 60 {
        156_025_000 + (channel - 60) * 50_000
    } else {
        156_000_000 + channel * 50_000
    }
}

/// Coast stations transmit 4.6 MHz above the ship on a duplex channel.
const MARINE_DUPLEX_OFFSET_HZ: u32 = 4_600_000;

/// The international duplex channels of ITU Radio Regulations Appendix 18,
/// used for public correspondence and port operations through coast stations.
/// Channel 70 (DSC) and the AIS channels are not voice channels and are left
/// out.
const MARINE_DUPLEX: PresetList = PresetList {
    name: "MARINE DUPLEX",
    channels: &[
        ("1", marine_hz(1)),
        ("2", marine_hz(2)),
        ("3", marine_hz(3)),
        ("4", marine_hz(4)),
        ("5", marine_hz(5)),
        ("7", marine_hz(7)),
        ("18", marine_hz(18)),
        ("19", marine_hz(19)),
        ("20", marine_hz(20)),
        ("21", marine_hz(21)),
        ("22", marine_hz(22)),
        ("23", marine_hz(23)),
        ("24", marine_hz(24)),
        ("25", marine_hz(25)),
        ("26", marine_hz(26)),
        ("27", marine_hz(27)),
        ("28", marine_hz(28)),
        ("60", marine_hz(60)),
        ("61", marine_hz(61)),
        ("62", marine_hz(62)),
        ("63", marine_hz(63)),
        ("64", marine_hz(64)),
        ("65", marine_hz(65)),
        ("66", marine_hz(66)),
        ("78", marine_hz(78)),
        ("79", marine_hz(79)),
        ("80", marine_hz(80)),
        ("81", marine_hz(81)),
        ("82", marine_hz(82)),
        ("83", marine_hz(83)),
        ("84", marine_hz(84)),
        ("85", marine_hz(85)),
        ("86", marine_hz(86)),
    ],
    receive_offset_hz: MARINE_DUPLEX_OFFSET_HZ,
    modulation: Modulation::Fm,
    // Marine VHF is 25 kHz FM.
    bandwidth: Bandwidth::Wide,
    power: PowerLevel::Low,
    step_hz: 25_000,
};

/// The international simplex channels of Appendix 18, distress and calling on
/// 16, plus the UK marina channels M (157.850) and M2 (161.425).
const MARINE_SIMPLEX: PresetList = PresetList {
    name: "MARINE SIMPLEX",
    channels: &[
        ("6", marine_hz(6)),
        ("8", marine_hz(8)),
        ("9", marine_hz(9)),
        ("10", marine_hz(10)),
        ("11", marine_hz(11)),
        ("12", marine_hz(12)),
        ("13", marine_hz(13)),
        ("14", marine_hz(14)),
        ("15", marine_hz(15)),
        ("16 CALL", marine_hz(16)),
        ("17", marine_hz(17)),
        ("67", marine_hz(67)),
        ("68", marine_hz(68)),
        ("69", marine_hz(69)),
        ("71", marine_hz(71)),
        ("72", marine_hz(72)),
        ("73", marine_hz(73)),
        ("74", marine_hz(74)),
        ("77", marine_hz(77)),
        ("M", 157_850_000),
        ("M2", 161_425_000),
    ],
    receive_offset_hz: 0,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Wide,
    power: PowerLevel::Low,
    step_hz: 25_000,
};

/// Every selectable preset in display order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "UK and EU simplex",
        detail: "PMR446 plus 2 m and 70 cm amateur FM simplex: forty channels \
                 in three stored plans, calling channels marked.",
        plans: &[PMR446, TWO_METRE_SIMPLEX, SEVENTY_CENTIMETRE_SIMPLEX],
        lists: &[],
    },
    Preset {
        name: "UK simplex and repeaters",
        detail: "The simplex set plus the 2 m repeater sub-band, which one plan \
                 holds by offsetting every input 600 kHz below its output.",
        plans: &[
            PMR446,
            TWO_METRE_SIMPLEX,
            SEVENTY_CENTIMETRE_SIMPLEX,
            TWO_METRE_REPEATERS,
        ],
        lists: &[],
    },
    Preset {
        name: "PMR446 only",
        detail: "All sixteen PMR446 channels as one stored plan, for a target \
                 which advertises a compact plan encoding.",
        plans: &[PMR446],
        lists: &[],
    },
    Preset {
        name: "VHF airband receive",
        detail: "760 AM channels from 118.000 to 136.975 MHz in one stored \
                 object. No TX range covers them, so the radio never transmits.",
        plans: &[AIRBAND],
        lists: &[],
    },
    Preset {
        name: "Marine VHF",
        detail: "The international marine channels: a duplex bank receiving \
                 4.6 MHz above the ship frequency and a simplex bank with 16, \
                 M and M2. Transmit needs a Marine TX range.",
        plans: &[],
        lists: &[MARINE_DUPLEX, MARINE_SIMPLEX],
    },
];

/// One TX range a TX preset adds.
struct PresetRange {
    class: TxClass,
    first_hz: u32,
    last_hz: u32,
    max_mw: u16,
    wide_fm: bool,
    raster_hz: u16,
}

/// A named set of TX ranges for one region or service (`ADR-101`).
///
/// Like the channel sets, a starting point: the operator confirms each range
/// against their own licence and national rules before writing it.
pub struct TxRangePreset {
    name: &'static str,
    detail: &'static str,
    ranges: &'static [PresetRange],
}

/// PMR446: sixteen 12.5 kHz channels, narrow FM, 500 mW ERP.
const PMR446_RANGE: PresetRange = PresetRange {
    class: TxClass::LicenceFreePlan,
    first_hz: 446_006_250,
    last_hz: 446_193_750,
    max_mw: 500,
    wide_fm: false,
    raster_hz: 12_500,
};

const fn amateur(first_hz: u32, last_hz: u32) -> PresetRange {
    PresetRange {
        class: TxClass::Amateur,
        first_hz,
        last_hz,
        max_mw: 5_000,
        wide_fm: true,
        raster_hz: 0,
    }
}

/// Every selectable TX range preset in display order.
pub const TX_RANGE_PRESETS: &[TxRangePreset] = &[
    TxRangePreset {
        name: "UK",
        detail: "PMR446, and 2 m (144-146 MHz) and 70 cm (430-440 MHz) amateur.",
        ranges: &[
            PMR446_RANGE,
            amateur(144_000_000, 146_000_000),
            amateur(430_000_000, 440_000_000),
        ],
    },
    TxRangePreset {
        name: "EU (CEPT)",
        detail: "PMR446, and IARU Region 1 2 m (144-146 MHz) and 70 cm \
                 (430-440 MHz) amateur.",
        ranges: &[
            PMR446_RANGE,
            amateur(144_000_000, 146_000_000),
            amateur(430_000_000, 440_000_000),
        ],
    },
    TxRangePreset {
        name: "US",
        detail: "FRS (2 W, 0.5 W on 8-14), MURS (2 W), and 2 m (144-148 MHz) \
                 and 70 cm (420-450 MHz) amateur.",
        ranges: &[
            // FRS 1-7 and 15-22 interleave on a 12.5 kHz grid.
            PresetRange {
                class: TxClass::LicenceFreePlan,
                first_hz: 462_550_000,
                last_hz: 462_725_000,
                max_mw: 2_000,
                wide_fm: false,
                raster_hz: 12_500,
            },
            // FRS 8-14.
            PresetRange {
                class: TxClass::LicenceFreePlan,
                first_hz: 467_562_500,
                last_hz: 467_712_500,
                max_mw: 500,
                wide_fm: false,
                raster_hz: 25_000,
            },
            // MURS 1-3, narrow.
            PresetRange {
                class: TxClass::LicenceFreePlan,
                first_hz: 151_820_000,
                last_hz: 151_940_000,
                max_mw: 2_000,
                wide_fm: false,
                raster_hz: 60_000,
            },
            // MURS 4-5, which may be wide.
            PresetRange {
                class: TxClass::LicenceFreePlan,
                first_hz: 154_570_000,
                last_hz: 154_600_000,
                max_mw: 2_000,
                wide_fm: true,
                raster_hz: 30_000,
            },
            amateur(144_000_000, 148_000_000),
            amateur(420_000_000, 450_000_000),
        ],
    },
    TxRangePreset {
        name: "Marine VHF",
        detail: "Ship transmit channels 156.025-157.425 MHz on the 25 kHz \
                 grid; needs a ship radio licence.",
        ranges: &[PresetRange {
            class: TxClass::Marine,
            first_hz: 156_025_000,
            last_hz: 157_425_000,
            max_mw: 5_000,
            wide_fm: true,
            raster_hz: 25_000,
        }],
    },
];

impl TxRangePreset {
    /// Returns the editor label.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the one-line description.
    pub const fn detail(&self) -> &'static str {
        self.detail
    }

    /// Returns the ranges as editable drafts.
    pub fn drafts(&self) -> Vec<TxRangeDraft> {
        self.ranges
            .iter()
            .map(|range| TxRangeDraft {
                class: range.class,
                first_mhz: format_mhz(range.first_hz),
                last_mhz: format_mhz(range.last_hz),
                max_mw: range.max_mw,
                wide_fm: range.wide_fm,
                raster_hz: range.raster_hz,
            })
            .collect()
    }
}

impl Preset {
    /// Returns the editor label.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the one-line description.
    pub const fn detail(&self) -> &'static str {
        self.detail
    }

    /// Returns how many channels and banks applying this preset produces.
    ///
    /// The channel count is what the radio expands to, and the bank count is
    /// what it stores, which is the comparison worth showing.
    pub const fn size(&self) -> (usize, usize) {
        let mut channels = 0;
        let mut index = 0;
        while index < self.plans.len() {
            channels += self.plans[index].channels as usize;
            index += 1;
        }
        index = 0;
        while index < self.lists.len() {
            channels += self.lists[index].channels.len();
            index += 1;
        }
        (channels, self.plans.len() + self.lists.len())
    }

    /// Builds the project this preset describes, replacing nothing.
    pub fn build(&self) -> ProjectModel {
        let mut project = ProjectModel::new();
        for plan in self.plans {
            project.banks.push(BankDraft {
                name: plan.name.to_owned(),
                kind: BankKind::Generated,
                designator: plan.designator.to_owned(),
                first_number: plan.first_number,
                base_mhz: format_mhz(plan.base_hz),
                spacing_hz: plan.spacing_hz,
                channel_count: plan.channels,
                calling_index: plan.calling_index,
                offset_hz: plan.offset_hz,
                scan_enabled: true,
                modulation: plan.modulation,
                bandwidth: plan.bandwidth,
                power: plan.power,
                step_hz: plan.spacing_hz,
                ..BankDraft::default()
            });
        }
        for list in self.lists {
            let channels = list
                .channels
                .iter()
                .map(|&(name, transmit_hz)| ChannelDraft {
                    name: name.to_owned(),
                    receive_mhz: format_mhz(transmit_hz + list.receive_offset_hz),
                    transmit_mhz: format_mhz(transmit_hz),
                    modulation: list.modulation,
                    bandwidth: list.bandwidth,
                    power: list.power,
                    step_hz: list.step_hz,
                    ..ChannelDraft::default()
                })
                .collect();
            project.banks.push(BankDraft {
                name: list.name.to_owned(),
                kind: BankKind::Named,
                scan_enabled: true,
                modulation: list.modulation,
                bandwidth: list.bandwidth,
                power: list.power,
                step_hz: list.step_hz,
                channels,
                ..BankDraft::default()
            });
        }
        project
    }
}

#[cfg(test)]
mod tests {
    use super::PRESETS;
    use crate::model::{BankKind, ProjectModel, ValidatedBank, RADIO_IMAGE_BYTES};
    use radio_domain::Bandwidth;

    #[test]
    fn every_tx_range_preset_validates_and_covers_its_calling_frequencies() {
        use super::TX_RANGE_PRESETS;
        use radio_tx_policy::TxPolicy;

        for preset in TX_RANGE_PRESETS {
            let mut project = ProjectModel::new();
            project.tx_ranges = preset.drafts();
            let compiled = project
                .validate()
                .unwrap_or_else(|_| panic!("{} validates", preset.name()));
            assert!(TxPolicy::new(compiled.tx_ranges()).is_ok());
        }
        let policy_of = |name: &str| {
            let preset = TX_RANGE_PRESETS.iter().find(|p| p.name() == name).unwrap();
            let mut project = ProjectModel::new();
            project.tx_ranges = preset.drafts();
            TxPolicy::new(project.validate().unwrap().tx_ranges()).unwrap()
        };
        let contains = |policy: &TxPolicy, hz: u32| {
            policy
                .ranges()
                .any(|range| (range.first_hz..=range.last_hz).contains(&hz))
        };
        let uk = policy_of("UK");
        for hz in [446_006_250, 446_193_750, 145_500_000, 433_500_000] {
            assert!(contains(&uk, hz), "UK {hz}");
        }
        assert!(!contains(&uk, 147_000_000));
        let us = policy_of("US");
        for hz in [
            462_562_500,
            462_550_000,
            467_712_500,
            151_940_000,
            146_520_000,
        ] {
            assert!(contains(&us, hz), "US {hz}");
        }
        assert!(
            contains(&policy_of("Marine VHF"), 156_800_000),
            "channel 16"
        );
    }

    #[test]
    fn every_preset_validates_and_compiles_to_an_image() {
        for preset in PRESETS {
            let project = preset.build();
            let (channels, banks) = preset.size();
            assert_eq!(project.banks.len(), banks, "{}", preset.name());
            assert_eq!(
                project.channel_count(),
                preset.lists.iter().map(|l| l.channels.len()).sum::<usize>(),
                "{}",
                preset.name()
            );
            project
                .validate()
                .unwrap_or_else(|_| panic!("{} validates", preset.name()));
            project
                .to_image()
                .unwrap_or_else(|_| panic!("{} compiles", preset.name()));
            project
                .to_radio_image()
                .unwrap_or_else(|_| panic!("{} fits a radio", preset.name()));

            let expanded: usize = project
                .banks
                .iter()
                .map(|bank| match bank.kind {
                    BankKind::Generated => usize::from(bank.channel_count),
                    BankKind::Named => bank.channels.len(),
                })
                .sum();
            assert_eq!(expanded, channels, "{}", preset.name());
        }
    }

    #[test]
    fn every_arithmetic_preset_is_plans_only_and_fits_a_radio() {
        for preset in PRESETS.iter().filter(|p| p.lists.is_empty()) {
            let project = preset.build();
            assert!(
                project.banks.iter().all(|b| b.kind == BankKind::Generated),
                "{} stores plans, not channels",
                preset.name()
            );
            let summary = project.storage_summary();
            assert_eq!(summary.stored_channels, 0);
            assert!(
                summary.image_bytes.unwrap() < RADIO_IMAGE_BYTES,
                "{} fits a radio",
                preset.name()
            );
        }
    }

    #[test]
    fn the_uk_simplex_preset_expands_to_the_designators_a_band_plan_uses() {
        let preset = PRESETS
            .iter()
            .find(|p| p.name() == "UK and EU simplex")
            .expect("preset");
        let project = preset.build();
        let names = |row: usize| -> Vec<String> {
            project.banks[row]
                .expansion(usize::MAX)
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };

        assert_eq!(names(0).first().unwrap(), "PMR 1");
        assert_eq!(names(0).last().unwrap(), "PMR 16");

        let two_metre = names(1);
        assert_eq!(two_metre.first().unwrap(), "S8");
        assert_eq!(two_metre[12], "S20 CALL");
        assert_eq!(two_metre.last().unwrap(), "S23");

        let seventy = names(2);
        assert_eq!(seventy.first().unwrap(), "SU16");
        assert_eq!(seventy[4], "SU20 CALL");
        assert_eq!(seventy.last().unwrap(), "SU23");
    }

    #[test]
    fn the_calling_channels_land_on_the_published_frequencies() {
        let preset = PRESETS
            .iter()
            .find(|p| p.name() == "UK and EU simplex")
            .expect("preset");
        let project = preset.build();
        let frequency = |row: usize, index: usize| -> String {
            project.banks[row].expansion(usize::MAX)[index].1.clone()
        };
        assert_eq!(frequency(1, 12), "145.500000", "S20");
        assert_eq!(frequency(2, 4), "433.500000", "SU20");
    }

    #[test]
    fn the_repeater_plan_offsets_its_inputs_and_stays_one_object() {
        let preset = PRESETS
            .iter()
            .find(|p| p.name() == "UK simplex and repeaters")
            .expect("preset");
        let project = preset.build();
        let ValidatedBank::Generated(plan) = project.banks[3].validate(3).expect("plan") else {
            panic!("the repeater row is a generated plan");
        };
        let first = plan.channel_record(0).expect("channel");
        assert_eq!(first.name().as_str(), "RV48");
        assert_eq!(first.receive().as_hz(), 145_600_000);
        assert_eq!(first.transmit().as_hz(), 145_000_000);
        assert_eq!(plan.channel_count(), 16);
    }

    #[test]
    fn the_airband_preset_is_one_object_and_can_never_transmit() {
        let preset = PRESETS
            .iter()
            .find(|p| p.name() == "VHF airband receive")
            .expect("preset");
        let project = preset.build();
        assert_eq!(project.banks.len(), 1, "760 channels, one stored object");

        let ValidatedBank::Generated(plan) = project.banks[0].validate(0).expect("plan") else {
            panic!("the airband row is a generated plan");
        };
        assert_eq!(plan.channel_count(), 760);
        assert_eq!(
            plan.channel_record(0).unwrap().receive().as_hz(),
            118_000_000
        );
        assert_eq!(
            plan.channel_record(759).unwrap().receive().as_hz(),
            136_975_000
        );
    }

    #[test]
    fn the_marine_preset_has_a_duplex_and_a_simplex_bank_classed_marine() {
        let preset = PRESETS
            .iter()
            .find(|p| p.name() == "Marine VHF")
            .expect("preset");
        let project = preset.build();
        assert_eq!(project.banks.len(), 2);
        assert!(
            project.storage_summary().image_bytes.unwrap() < RADIO_IMAGE_BYTES,
            "both marine banks fit a radio"
        );
        let channel = |row: usize, name: &str| {
            project.banks[row]
                .channels
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name}"))
                .clone()
        };

        let duplex = &project.banks[0];
        assert_eq!(duplex.name, "MARINE DUPLEX");
        assert_eq!(duplex.kind, BankKind::Named);
        assert_eq!(duplex.channels.len(), 33);
        // The ship transmits low and hears the coast station 4.6 MHz above.
        let one = channel(0, "1");
        assert_eq!(
            (one.transmit_mhz.as_str(), one.receive_mhz.as_str()),
            ("156.050000", "160.650000")
        );
        let twenty_six = channel(0, "26");
        assert_eq!(twenty_six.transmit_mhz, "157.300000");
        assert_eq!(twenty_six.receive_mhz, "161.900000");
        let eighty_six = channel(0, "86");
        assert_eq!(eighty_six.transmit_mhz, "157.325000");
        assert_eq!(eighty_six.receive_mhz, "161.925000");

        let simplex = &project.banks[1];
        assert_eq!(simplex.name, "MARINE SIMPLEX");
        assert_eq!(simplex.channels.len(), 21);
        assert!(simplex
            .channels
            .iter()
            .all(|c| c.receive_mhz == c.transmit_mhz));
        assert_eq!(channel(1, "16 CALL").receive_mhz, "156.800000");
        assert_eq!(channel(1, "6").receive_mhz, "156.300000");
        assert_eq!(channel(1, "67").receive_mhz, "156.375000");
        assert_eq!(channel(1, "72").receive_mhz, "156.625000");
        assert_eq!(channel(1, "77").receive_mhz, "156.875000");
        assert_eq!(channel(1, "M").receive_mhz, "157.850000");
        assert_eq!(channel(1, "M2").receive_mhz, "161.425000");

        assert!(project
            .banks
            .iter()
            .all(|bank| bank.channels.iter().all(|c| c.bandwidth == Bandwidth::Wide)));
    }

    #[test]
    fn a_preset_replaces_nothing_the_operator_already_entered() {
        let before = ProjectModel::new();
        let after = PRESETS[0].build();
        assert_ne!(before.banks.len(), after.banks.len());
    }
}
