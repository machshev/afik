//! Default channel sets the editor can apply, chosen by region.
//!
//! A preset is a starting point, not an authority. Frequencies here are the
//! licence-free and amateur simplex allocations an operator is most likely to
//! want first; the operator remains responsible for confirming them against
//! their own national band plan, and for the transmit classification every
//! channel carries. Nothing here is applied without an explicit request.

use radio_domain::TxClass;

use crate::model::{BankDraft, BankKind, ChannelDraft, ProjectModel};

/// One channel a preset defines.
struct PresetChannel {
    name: &'static str,
    receive_hz: u32,
    bank: u16,
    tx_class: TxClass,
}

/// One bank a preset defines.
struct PresetBank {
    id: u16,
    name: &'static str,
}

/// A named default set of banks and channels.
pub struct Preset {
    /// Region or allocation this set belongs to.
    name: &'static str,
    /// One-line description of what applying it produces.
    detail: &'static str,
    banks: &'static [PresetBank],
    channels: &'static [PresetChannel],
}

/// Every selectable preset in display order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "UK and EU simplex",
        detail: "PMR446 plus 2 m and 70 cm amateur FM simplex: twelve channels in three banks.",
        banks: &[
            PresetBank {
                id: 0,
                name: "PMR446",
            },
            PresetBank {
                id: 1,
                name: "2M SIMPLEX",
            },
            PresetBank {
                id: 2,
                name: "70CM SIMPLEX",
            },
        ],
        channels: &[
            // The licence-free analogue PMR446 allocation, 12.5 kHz spaced from
            // 446.00625 MHz. Four of the eight leave room for amateur use.
            PresetChannel {
                name: "PMR 1",
                receive_hz: 446_006_250,
                bank: 0,
                tx_class: TxClass::LicenceFreePlan,
            },
            PresetChannel {
                name: "PMR 2",
                receive_hz: 446_018_750,
                bank: 0,
                tx_class: TxClass::LicenceFreePlan,
            },
            PresetChannel {
                name: "PMR 3",
                receive_hz: 446_031_250,
                bank: 0,
                tx_class: TxClass::LicenceFreePlan,
            },
            PresetChannel {
                name: "PMR 4",
                receive_hz: 446_043_750,
                bank: 0,
                tx_class: TxClass::LicenceFreePlan,
            },
            // 2 m FM simplex: the calling channel plus 25 kHz steps inside the
            // simplex segment.
            PresetChannel {
                name: "2M CALL",
                receive_hz: 145_500_000,
                bank: 1,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "2M 145.525",
                receive_hz: 145_525_000,
                bank: 1,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "2M 145.550",
                receive_hz: 145_550_000,
                bank: 1,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "2M 145.575",
                receive_hz: 145_575_000,
                bank: 1,
                tx_class: TxClass::Amateur,
            },
            // 70 cm FM simplex: the calling channel plus 25 kHz steps inside the
            // 433.400 to 433.575 MHz simplex segment.
            PresetChannel {
                name: "70CM CALL",
                receive_hz: 433_500_000,
                bank: 2,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "70CM 433.40",
                receive_hz: 433_400_000,
                bank: 2,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "70CM 433.45",
                receive_hz: 433_450_000,
                bank: 2,
                tx_class: TxClass::Amateur,
            },
            PresetChannel {
                name: "70CM 433.55",
                receive_hz: 433_550_000,
                bank: 2,
                tx_class: TxClass::Amateur,
            },
        ],
    },
    Preset {
        name: "PMR446 compact plan",
        detail: "One generated plan covering all sixteen PMR446 channels, for a \
                 target which expands compact plans.",
        banks: &[],
        channels: &[],
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

    /// Returns how many channels and banks applying this preset produces.
    pub const fn size(&self) -> (usize, usize) {
        (self.channels.len(), self.banks.len())
    }

    /// Builds the project this preset describes, replacing nothing.
    ///
    /// The generated-plan preset carries no explicit channels: it is one compact
    /// bank a target expands for itself, which the K1 receive image cannot do.
    pub fn build(&self) -> ProjectModel {
        let mut project = ProjectModel::new();
        if self.channels.is_empty() && self.banks.is_empty() {
            project.banks.push(BankDraft {
                id: 0,
                name: "PMR446".to_owned(),
                kind: BankKind::Generated,
                base_mhz: "446.006250".to_owned(),
                spacing_hz: 12_500,
                channel_count: 16,
                tx_class: TxClass::LicenceFreePlan,
                scan_enabled: true,
            });
            return project;
        }
        for bank in self.banks {
            project.banks.push(BankDraft {
                id: bank.id,
                name: bank.name.to_owned(),
                kind: BankKind::Named,
                ..BankDraft::default()
            });
        }
        for (index, channel) in self.channels.iter().enumerate() {
            let mut draft = ChannelDraft {
                id: u16::try_from(index + 1).unwrap_or(u16::MAX),
                name: channel.name.to_owned(),
                receive_mhz: crate::model::format_mhz(channel.receive_hz),
                // Simplex: the transmit frequency mirrors receive, and the class
                // decides whether transmission is permitted at all.
                transmit_mhz: crate::model::format_mhz(channel.receive_hz),
                tx_class: channel.tx_class,
                ..ChannelDraft::default()
            };
            if let Some(member) = draft.banks.get_mut(usize::from(channel.bank)) {
                *member = true;
            }
            project.channels.push(draft);
        }
        project
    }
}

#[cfg(test)]
mod tests {
    use super::PRESETS;
    use crate::model::{BankKind, ProjectModel};
    use radio_domain::TxClass;

    #[test]
    fn every_preset_validates_and_compiles_to_an_image() {
        assert!(!PRESETS.is_empty());
        for preset in PRESETS {
            let project = preset.build();
            let image = project
                .to_image()
                .unwrap_or_else(|errors| panic!("{}: {errors:?}", preset.name()));
            let loaded = ProjectModel::from_image(&image).expect("a canonical image");
            assert_eq!(loaded.channels.len(), project.channels.len());
            assert_eq!(loaded.banks.len(), project.banks.len());
            assert!(!preset.detail().is_empty());
        }
    }

    #[test]
    fn the_simplex_preset_fills_three_banks_and_classifies_every_channel() {
        let preset = &PRESETS[0];
        assert_eq!(preset.size(), (12, 3));
        let project = preset.build();
        assert_eq!(project.channels[0].name, "PMR 1");
        assert_eq!(project.channels[0].receive_mhz, "446.006250");
        assert_eq!(project.channels[0].tx_class, TxClass::LicenceFreePlan);
        assert!(project.channels[0].banks[0]);
        assert_eq!(project.channels[4].name, "2M CALL");
        assert_eq!(project.channels[4].tx_class, TxClass::Amateur);
        assert!(project.channels[4].banks[1]);
        assert!(project.channels[8].banks[2]);
        // Every channel is simplex, so no preset can imply a repeater pair.
        for channel in &project.channels {
            assert_eq!(channel.receive_mhz, channel.transmit_mhz);
        }
        assert!(project
            .banks
            .iter()
            .all(|bank| matches!(bank.kind, BankKind::Named)));
    }

    #[test]
    fn the_compact_preset_is_one_generated_plan_with_no_channels() {
        let preset = &PRESETS[1];
        let project = preset.build();
        assert!(project.channels.is_empty());
        assert_eq!(project.banks.len(), 1);
        assert_eq!(project.banks[0].kind, BankKind::Generated);
        assert_eq!(project.banks[0].channel_count, 16);
        assert_eq!(
            project.banks[0].generated_span(),
            Some(("446.006250".to_owned(), "446.193750".to_owned()))
        );
    }
}
