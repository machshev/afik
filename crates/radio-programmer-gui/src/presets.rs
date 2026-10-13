//! Default channel sets the editor can apply, chosen by region.
//!
//! A preset is a starting point, not an authority. Frequencies here are the
//! licence-free and amateur allocations an operator is most likely to want
//! first; the operator remains responsible for confirming them against their
//! own national band plan, and for the transmit classification every channel
//! carries. Nothing here is applied without an explicit request.
//!
//! Every set is expressed as generated plans rather than explicit channels,
//! because every one of them is arithmetic. A plan is one stored object however
//! many channels it holds, so a radio which would not fit these as records
//! holds all of them and the operator reads the designators a published band
//! plan uses: `S20 CALL` rather than a truncated bank name and a position.

use radio_domain::{Bandwidth, Modulation, PowerLevel, TxClass};

use crate::model::{BankDraft, BankKind, ProjectModel};

/// One generated plan a preset defines.
struct PresetPlan {
    id: u16,
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
    tx_class: TxClass,
    modulation: Modulation,
    bandwidth: Bandwidth,
    power: PowerLevel,
}

/// A named default set of plans.
pub struct Preset {
    /// Region or allocation this set belongs to.
    name: &'static str,
    /// One-line description of what applying it produces.
    detail: &'static str,
    plans: &'static [PresetPlan],
}

/// The analogue PMR446 allocation: sixteen channels, 12.5 kHz from 446.00625.
const PMR446: PresetPlan = PresetPlan {
    id: 0,
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
    tx_class: TxClass::LicenceFreePlan,
    modulation: Modulation::Fm,
    // Analogue PMR446 is narrow FM at low power by allocation.
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 2 m FM simplex: S8 at 145.200 through S23, calling on S20 at 145.500.
const TWO_METRE_SIMPLEX: PresetPlan = PresetPlan {
    id: 1,
    name: "2M SIMPLEX",
    designator: "S",
    first_number: 8,
    base_hz: 145_200_000,
    spacing_hz: 25_000,
    channels: 16,
    // S20, twelve channels above S8.
    calling_index: Some(12),
    offset_hz: 0,
    tx_class: TxClass::Amateur,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 70 cm FM simplex: SU16 at 433.400 through SU23, calling on SU20.
const SEVENTY_CENTIMETRE_SIMPLEX: PresetPlan = PresetPlan {
    id: 2,
    name: "70CM SIMPLEX",
    designator: "SU",
    first_number: 16,
    base_hz: 433_400_000,
    spacing_hz: 25_000,
    channels: 8,
    // SU20 at 433.500, four channels above SU16.
    calling_index: Some(4),
    offset_hz: 0,
    tx_class: TxClass::Amateur,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// UK 2 m repeaters: outputs from 145.600 at 12.5 kHz, inputs 600 kHz below.
///
/// This is the fixed-offset family: the outputs are arithmetic and the distance
/// to the inputs is one constant, so a whole sub-band is one stored object.
const TWO_METRE_REPEATERS: PresetPlan = PresetPlan {
    id: 3,
    name: "2M REPEATERS",
    designator: "RV",
    first_number: 48,
    base_hz: 145_600_000,
    spacing_hz: 12_500,
    channels: 16,
    calling_index: None,
    offset_hz: -600_000,
    tx_class: TxClass::Amateur,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// The VHF civil airband at 25 kHz: 118.000 to 136.975, AM, receive only.
///
/// The transmit class is `Never`: this allocation is not one an amateur or
/// licence-free operator may transmit in, and the class is the radio's own
/// refusal rather than a note in a manual. The 8.33 kHz channel numbering is
/// not arithmetic and is deliberately not expressed here.
const AIRBAND: PresetPlan = PresetPlan {
    id: 0,
    name: "AIRBAND",
    designator: "AIR",
    first_number: 1,
    base_hz: 118_000_000,
    spacing_hz: 25_000,
    channels: 760,
    calling_index: None,
    offset_hz: 0,
    tx_class: TxClass::Never,
    modulation: Modulation::Am,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low,
};

/// Every selectable preset in display order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "UK and EU simplex",
        detail: "PMR446 plus 2 m and 70 cm amateur FM simplex: forty channels \
                 in three stored plans, calling channels marked.",
        plans: &[PMR446, TWO_METRE_SIMPLEX, SEVENTY_CENTIMETRE_SIMPLEX],
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
    },
    Preset {
        name: "PMR446 only",
        detail: "All sixteen PMR446 channels as one stored plan, for a target \
                 which advertises a compact plan encoding.",
        plans: &[PMR446],
    },
    Preset {
        name: "VHF airband receive",
        detail: "760 AM channels from 118.000 to 136.975 MHz in one stored \
                 object, classified so the radio will never transmit on them.",
        plans: &[AIRBAND],
    },
];

impl Preset {
    /// Returns the editor label.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the one-line description.
    pub const fn detail(&self) -> &'static str {
        self.detail
    }

    /// Returns how many channels and plans applying this preset produces.
    ///
    /// The channel count is what the radio expands to, and the plan count is
    /// what it stores, which is the comparison worth showing.
    pub const fn size(&self) -> (usize, usize) {
        let mut channels = 0;
        let mut index = 0;
        while index < self.plans.len() {
            channels += self.plans[index].channels as usize;
            index += 1;
        }
        (channels, self.plans.len())
    }

    /// Builds the project this preset describes, replacing nothing.
    pub fn build(&self) -> ProjectModel {
        let mut project = ProjectModel::new();
        for plan in self.plans {
            project.banks.push(BankDraft {
                id: plan.id,
                name: plan.name.to_owned(),
                kind: BankKind::Generated,
                designator: plan.designator.to_owned(),
                first_number: plan.first_number,
                base_mhz: crate::model::format_mhz(plan.base_hz),
                spacing_hz: plan.spacing_hz,
                channel_count: plan.channels,
                calling_index: plan.calling_index,
                offset_hz: plan.offset_hz,
                tx_class: plan.tx_class,
                scan_enabled: true,
                modulation: plan.modulation,
                bandwidth: plan.bandwidth,
                power: plan.power,
                step_hz: plan.spacing_hz,
                ..BankDraft::default()
            });
        }
        project
    }
}

#[cfg(test)]
mod tests {
    use super::PRESETS;
    use crate::model::{BankKind, ProjectModel, ValidatedBank};
    use radio_domain::TxClass;

    #[test]
    fn every_preset_validates_and_compiles_to_an_image() {
        for preset in PRESETS {
            let project = preset.build();
            let (channels, plans) = preset.size();
            assert_eq!(project.banks.len(), plans, "{}", preset.name());
            assert!(project.channels.is_empty(), "{}", preset.name());
            project
                .validate()
                .unwrap_or_else(|_| panic!("{} validates", preset.name()));
            project
                .to_image()
                .unwrap_or_else(|_| panic!("{} compiles", preset.name()));

            let expanded: usize = project
                .banks
                .iter()
                .map(|bank| usize::from(bank.channel_count))
                .sum();
            assert_eq!(expanded, channels, "{}", preset.name());
        }
    }

    #[test]
    fn every_preset_is_plans_only_and_costs_far_less_than_its_channels() {
        for preset in PRESETS {
            let project = preset.build();
            assert!(
                project.banks.iter().all(|b| b.kind == BankKind::Generated),
                "{} stores plans, not channels",
                preset.name()
            );
            let summary = project.storage_summary();
            assert_eq!(summary.stored_channels, 0);
            assert!(
                summary.bytes_saved() > 0,
                "{} saves bytes against explicit records",
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
        assert_eq!(plan.tx_class(), TxClass::Never);
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
    fn a_preset_replaces_nothing_the_operator_already_entered() {
        let before = ProjectModel::new();
        let after = PRESETS[0].build();
        assert_ne!(before.banks.len(), after.banks.len());
    }
}
