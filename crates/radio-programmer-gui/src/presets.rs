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

use radio_channel_plan::SharedSettings;
use radio_domain::{Bandwidth, Modulation, PowerLevel, Tone, TxClass};

use crate::model::{
    format_mhz, BankDraft, BankKind, ChannelDraft, ProjectModel, ToneDraft, TxRangeDraft,
};

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

/// One channel of a split list: its own receive and transmit frequencies.
struct SplitChannel {
    name: &'static str,
    receive_hz: u32,
    transmit_hz: u32,
    /// CTCSS tone sent on transmit, in tenths of a hertz, if any.
    tx_ctcss_tenths_hz: Option<u16>,
}

/// One bank of channels whose receive and transmit frequencies are unrelated,
/// such as a satellite's downlink and uplink.
struct PresetSplitList {
    name: &'static str,
    channels: &'static [SplitChannel],
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
    splits: &'static [PresetSplitList],
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
    power: PowerLevel::Low4,
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
    power: PowerLevel::High,
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
    power: PowerLevel::High,
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
    power: PowerLevel::High,
};

/// UK 70 cm repeaters: outputs RU240 at 433.000 through RU270 at 433.375 MHz,
/// inputs 1.6 MHz above.
const SEVENTY_CENTIMETRE_REPEATERS: PresetPlan = PresetPlan {
    name: "70CM REPEATERS",
    designator: "RU",
    first_number: 240,
    base_hz: 433_000_000,
    spacing_hz: 12_500,
    channels: 31,
    calling_index: None,
    offset_hz: 1_600_000,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::High,
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
    power: PowerLevel::Low1,
};

/// A receive-only AM airband plan on the 25 kHz grid.
const fn air(name: &'static str, first_number: u16, base_hz: u32, channels: u16) -> PresetPlan {
    PresetPlan {
        modulation: Modulation::Am,
        ..listen(name, "MIL", first_number, base_hz, 25_000, channels)
    }
}

/// The military UHF airband, 225.000 to 399.975 MHz at 25 kHz: 7000 AM
/// channels as four plans numbered `MIL1` through `MIL7000`.
const MILITARY_AIRBAND: &[PresetPlan] = &[
    air("MILITARY UHF 1", 1, 225_000_000, 2048),
    air("MILITARY UHF 2", 2049, 276_200_000, 2048),
    air("MILITARY UHF 3", 4097, 327_400_000, 2048),
    air("MILITARY UHF 4", 6145, 378_600_000, 856),
];

/// The civil and military distress frequencies, both AM.
const AIR_GUARD: PresetList = PresetList {
    name: "AIR GUARD",
    channels: &[("CIVIL GUARD", 121_500_000), ("MIL GUARD", 243_000_000)],
    receive_offset_hz: 0,
    modulation: Modulation::Am,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low1,
    step_hz: 25_000,
};

/// Ship transmit frequency of an ITU-R M.1084 marine VHF channel number.
///
/// Channels 1 to 28 run from 156.050 MHz and 60 to 88 from 156.025 MHz, both
/// at 50 kHz, interleaving at 25 kHz.
pub(crate) const fn marine_hz(channel: u32) -> u32 {
    if channel >= 60 {
        156_025_000 + (channel - 60) * 50_000
    } else {
        156_000_000 + channel * 50_000
    }
}

/// Coast stations transmit 4.6 MHz above the ship on a duplex channel.
pub(crate) const MARINE_DUPLEX_OFFSET_HZ: u32 = 4_600_000;

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
    power: PowerLevel::High,
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
    power: PowerLevel::High,
    step_hz: 25_000,
};

/// A receive-only plan of narrow FM channels, for allocations which the
/// operator listens to but which no preset TX range covers.
const fn listen(
    name: &'static str,
    designator: &'static str,
    first_number: u16,
    base_hz: u32,
    spacing_hz: u32,
    channels: u16,
) -> PresetPlan {
    PresetPlan {
        name,
        designator,
        first_number,
        base_hz,
        spacing_hz,
        channels,
        calling_index: None,
        offset_hz: 0,
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: PowerLevel::Low1,
    }
}

/// UK business radio, 136.000 to 174.000 MHz and 400.000 to 470.000 MHz on
/// the 12.5 kHz raster.
///
/// One plan holds at most 2048 channels, so each band is split into plans
/// whose numbering continues: `BV1` is 136.000 and `BV3041` is 174.000, `BU1`
/// is 400.000 and `BU5601` is 470.000.
const BUSINESS_RECEIVE: &[PresetPlan] = &[
    listen("BUSINESS VHF 1", "BV", 1, 136_000_000, 12_500, 2048),
    listen("BUSINESS VHF 2", "BV", 2049, 161_600_000, 12_500, 993),
    listen("BUSINESS UHF 1", "BU", 1, 400_000_000, 12_500, 2048),
    listen("BUSINESS UHF 2", "BU", 2049, 425_600_000, 12_500, 2048),
    listen("BUSINESS UHF 3", "BU", 4097, 451_200_000, 12_500, 1505),
];

/// Simple UK light licence, VHF low band: 77.6875 to 86.3750 MHz at 12.5 kHz.
const SIMPLE_UK_LOW: PresetPlan = listen("SIMPLE UK LOW", "SL", 1, 77_687_500, 12_500, 696);

/// Simple UK light licence, VHF mid/high band: 164.0500 to 173.0875 MHz.
const SIMPLE_UK_HIGH: PresetPlan = listen("SIMPLE UK HIGH", "SH", 1, 164_050_000, 12_500, 724);

/// Licence-exempt VHF radio microphones (173.700 to 175.100 MHz): the five
/// frequencies UK licence-exempt VHF microphones commonly ship on.
const RADIO_MIC_VHF: PresetList = PresetList {
    name: "RADIO MIC VHF",
    channels: &[
        ("MIC 173.800", 173_800_000),
        ("MIC 174.100", 174_100_000),
        ("MIC 174.500", 174_500_000),
        ("MIC 174.800", 174_800_000),
        ("MIC 175.000", 175_000_000),
    ],
    receive_offset_hz: 0,
    modulation: Modulation::Fm,
    // Microphone FM deviates far wider than voice radio; wide is the nearest.
    bandwidth: Bandwidth::Wide,
    power: PowerLevel::Low1,
    step_hz: 25_000,
};

/// Licence-exempt UHF radio microphones in channel 70, 863.000 to 865.000 MHz,
/// on a 25 kHz grid for finding them by scan.
const RADIO_MIC_UHF: PresetPlan = PresetPlan {
    bandwidth: Bandwidth::Wide,
    ..listen("RADIO MIC UHF", "MU", 1, 863_000_000, 25_000, 81)
};

/// LPD433: 69 channels at 25 kHz from 433.075 to 434.775 MHz.
const LPD433: PresetPlan = listen("LPD433", "LPD", 1, 433_075_000, 25_000, 69);

/// UK licence-exempt telemetry and telecommand, 458.500 to 458.950 MHz.
const SRD_458: PresetPlan = listen("SRD 458", "T", 1, 458_500_000, 12_500, 37);

/// The 868.000 to 868.600 MHz short range device band at 25 kHz.
const SRD_868: PresetPlan = listen("SRD 868", "SRD", 1, 868_000_000, 25_000, 25);

/// APRS: the IARU Region 1 frequency and the ISS digipeater.
const APRS: PresetList = PresetList {
    name: "APRS",
    channels: &[("APRS", 144_800_000), ("ISS APRS", 145_825_000)],
    receive_offset_hz: 0,
    modulation: Modulation::Fm,
    // 1200 baud AFSK packet runs on 25 kHz channels.
    bandwidth: Bandwidth::Wide,
    power: PowerLevel::High,
    step_hz: 25_000,
};

/// UK 27/81 CB: forty FM channels at 10 kHz from 27.60125 MHz.
const CB_UK: PresetPlan = listen("CB UK 27/81", "UK ", 1, 27_601_250, 10_000, 40);

/// CEPT CB: forty channels from 26.965 MHz. The numbering skips the
/// frequencies older radio control used and puts 23 above 24 and 25, so it is
/// listed rather than generated.
const CB_EU: PresetList = PresetList {
    name: "CB EU",
    channels: &[
        ("EU 1", 26_965_000),
        ("EU 2", 26_975_000),
        ("EU 3", 26_985_000),
        ("EU 4", 27_005_000),
        ("EU 5", 27_015_000),
        ("EU 6", 27_025_000),
        ("EU 7", 27_035_000),
        ("EU 8", 27_055_000),
        ("EU 9", 27_065_000),
        ("EU 10", 27_075_000),
        ("EU 11", 27_085_000),
        ("EU 12", 27_105_000),
        ("EU 13", 27_115_000),
        ("EU 14", 27_125_000),
        ("EU 15", 27_135_000),
        ("EU 16", 27_155_000),
        ("EU 17", 27_165_000),
        ("EU 18", 27_175_000),
        ("EU 19", 27_185_000),
        ("EU 20", 27_205_000),
        ("EU 21", 27_215_000),
        ("EU 22", 27_225_000),
        ("EU 23", 27_255_000),
        ("EU 24", 27_235_000),
        ("EU 25", 27_245_000),
        ("EU 26", 27_265_000),
        ("EU 27", 27_275_000),
        ("EU 28", 27_285_000),
        ("EU 29", 27_295_000),
        ("EU 30", 27_305_000),
        ("EU 31", 27_315_000),
        ("EU 32", 27_325_000),
        ("EU 33", 27_335_000),
        ("EU 34", 27_345_000),
        ("EU 35", 27_355_000),
        ("EU 36", 27_365_000),
        ("EU 37", 27_375_000),
        ("EU 38", 27_385_000),
        ("EU 39", 27_395_000),
        ("EU 40", 27_405_000),
    ],
    receive_offset_hz: 0,
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::Low1,
    step_hz: 10_000,
};

/// CTCSS 67.0 Hz, which the ISS repeater and SO-50 want on their uplinks.
const CTCSS_67: Option<u16> = Some(670);

/// The ISS and the amateur FM satellites, each receiving on the downlink and
/// transmitting on the uplink. Uplinks are IARU Region 1's. Doppler moves the
/// 70 cm downlinks by up to 10 kHz, which narrow FM partly tolerates.
const SATELLITES: PresetSplitList = PresetSplitList {
    name: "SATELLITES",
    channels: &[
        SplitChannel {
            name: "ISS VOICE",
            receive_hz: 145_800_000,
            transmit_hz: 145_200_000,
            tx_ctcss_tenths_hz: None,
        },
        SplitChannel {
            name: "ISS APRS",
            receive_hz: 145_825_000,
            transmit_hz: 145_825_000,
            tx_ctcss_tenths_hz: None,
        },
        SplitChannel {
            name: "ISS RPT",
            receive_hz: 437_800_000,
            transmit_hz: 145_990_000,
            tx_ctcss_tenths_hz: CTCSS_67,
        },
        SplitChannel {
            name: "SO-50",
            receive_hz: 436_795_000,
            transmit_hz: 145_850_000,
            tx_ctcss_tenths_hz: CTCSS_67,
        },
    ],
    modulation: Modulation::Fm,
    bandwidth: Bandwidth::Narrow,
    power: PowerLevel::High,
    step_hz: 5_000,
};

/// A broadcast plan on the 100 kHz raster, received by the BK1080 and never
/// transmitted (`ADR-105`).
const fn broadcast(name: &'static str, base_hz: u32, channels: u16) -> PresetPlan {
    PresetPlan {
        modulation: Modulation::Broadcast,
        bandwidth: Bandwidth::Wide,
        ..listen(name, "FM", 1, base_hz, 100_000, channels)
    }
}

/// FM broadcast as in Europe and the Americas: 87.5 to 108.0 MHz.
const FM_BROADCAST: PresetPlan = broadcast("FM BROADCAST", 87_500_000, 206);

/// FM broadcast including the Japanese band: 76.0 to 108.0 MHz.
const FM_BROADCAST_WIDE: PresetPlan = broadcast("FM 76-108", 76_000_000, 321);

/// Every selectable preset in display order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "UK and EU simplex",
        detail: "PMR446 plus 2 m and 70 cm amateur FM simplex: forty channels \
                 in three stored plans, calling channels marked.",
        plans: &[PMR446, TWO_METRE_SIMPLEX, SEVENTY_CENTIMETRE_SIMPLEX],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "UK simplex and repeaters",
        detail: "The simplex set plus the 2 m and 70 cm repeater sub-bands, each \
                 one plan: 2 m inputs 600 kHz below, 70 cm inputs 1.6 MHz above.",
        plans: &[
            PMR446,
            TWO_METRE_SIMPLEX,
            SEVENTY_CENTIMETRE_SIMPLEX,
            TWO_METRE_REPEATERS,
            SEVENTY_CENTIMETRE_REPEATERS,
        ],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "PMR446 only",
        detail: "All sixteen PMR446 channels as one stored plan, for a target \
                 which advertises a compact plan encoding.",
        plans: &[PMR446],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "VHF airband receive",
        detail: "760 AM channels from 118.000 to 136.975 MHz in one stored \
                 object. No TX range covers them, so the radio never transmits.",
        plans: &[AIRBAND],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "Military airband receive",
        detail: "225.000-399.975 MHz AM at 25 kHz, 7000 channels in four \
                 stored plans, plus civil (121.5) and military (243.0) guard.",
        plans: MILITARY_AIRBAND,
        lists: &[AIR_GUARD],
        splits: &[],
    },
    Preset {
        name: "Marine VHF",
        detail: "The international marine channels: a duplex bank receiving \
                 4.6 MHz above the ship frequency and a simplex bank with 16, \
                 M and M2. Transmit needs a Marine TX range.",
        plans: &[],
        lists: &[MARINE_DUPLEX, MARINE_SIMPLEX],
        splits: &[],
    },
    Preset {
        name: "UK business radio receive",
        detail: "136-174 MHz and 400-470 MHz at 12.5 kHz: 8642 channels in \
                 five stored plans, numbered BV and BU. Receive only.",
        plans: BUSINESS_RECEIVE,
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "UK Simple light licence",
        detail: "Simple UK business bands at 12.5 kHz: VHF low 77.6875-86.3750 \
                 MHz (SL) and VHF mid/high 164.0500-173.0875 MHz (SH).",
        plans: &[SIMPLE_UK_LOW, SIMPLE_UK_HIGH],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "Licence-exempt radio mics",
        detail: "The five common VHF microphone frequencies (173.7-175.1 MHz) \
                 and channel 70 at 863-865 MHz on a 25 kHz grid. Receive only.",
        plans: &[RADIO_MIC_UHF],
        lists: &[RADIO_MIC_VHF],
        splits: &[],
    },
    Preset {
        name: "Licence-exempt short range devices",
        detail: "LPD433 (69 channels), UK 458.5-458.95 MHz telemetry and \
                 868.0-868.6 MHz SRD. Mostly data; for spotting activity.",
        plans: &[LPD433, SRD_458, SRD_868],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "ISS and FM satellites",
        detail: "ISS voice, APRS and cross-band repeater, and SO-50: each \
                 receives the downlink and transmits the Region 1 uplink.",
        plans: &[],
        lists: &[],
        splits: &[SATELLITES],
    },
    Preset {
        name: "APRS",
        detail: "144.800 MHz, the Region 1 APRS frequency, and the ISS \
                 digipeater on 145.825 MHz.",
        plans: &[],
        lists: &[APRS],
        splits: &[],
    },
    Preset {
        name: "FM broadcast",
        detail: "87.5-108.0 MHz at 100 kHz, 206 stations in one stored plan, \
                 played through the broadcast receiver. Never transmits.",
        plans: &[FM_BROADCAST],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "FM broadcast 76-108 MHz",
        detail: "76.0-108.0 MHz at 100 kHz, 321 stations in one stored plan, \
                 covering the Japanese band as well. Never transmits.",
        plans: &[FM_BROADCAST_WIDE],
        lists: &[],
        splits: &[],
    },
    Preset {
        name: "CB 27 MHz receive",
        detail: "UK 27/81 (27.60125-27.99125 MHz) and CEPT (26.965-27.405 MHz) \
                 CB, forty FM channels each. The radio cannot transmit at 27 MHz.",
        plans: &[CB_UK],
        lists: &[CB_EU],
        splits: &[],
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
    TxRangePreset {
        name: "CB 27 MHz",
        detail: "UK 27/81 (27.60125-27.99125 MHz) and CEPT (26.965-27.405 MHz) \
                 CB, narrow FM at 4 W on the 10 kHz grid.",
        ranges: &[cb(27_601_250, 27_991_250), cb(26_965_000, 27_405_000)],
    },
];

const fn cb(first_hz: u32, last_hz: u32) -> PresetRange {
    PresetRange {
        class: TxClass::CitizensBand,
        first_hz,
        last_hz,
        max_mw: 4_000,
        wide_fm: false,
        raster_hz: 10_000,
    }
}

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
        index = 0;
        while index < self.splits.len() {
            channels += self.splits[index].channels.len();
            index += 1;
        }
        (
            channels,
            self.plans.len() + self.lists.len() + self.splits.len(),
        )
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
            // Everything but the frequency is the same across a list, so the
            // bank shares it all and each channel carries a name and a frequency.
            project.banks.push(BankDraft {
                name: list.name.to_owned(),
                kind: BankKind::Named,
                scan_enabled: true,
                shared: SharedSettings::from_bits(!SharedSettings::RESERVED).unwrap_or_default(),
                modulation: list.modulation,
                bandwidth: list.bandwidth,
                power: list.power,
                step_hz: list.step_hz,
                offset_hz: -i32::try_from(list.receive_offset_hz).unwrap_or(0),
                channels,
                ..BankDraft::default()
            });
        }
        for list in self.splits {
            project.banks.push(split_bank(
                list,
                list.channels.iter().map(|channel| {
                    (
                        channel.name,
                        channel.receive_hz,
                        channel.transmit_hz,
                        channel.tx_ctcss_tenths_hz,
                    )
                }),
            ));
        }
        project
    }
}

/// Name of the preset whose satellite bank a SatNOGS sync replaces.
pub const SATELLITE_PRESET: &str = "ISS and FM satellites";

/// Builds the satellite bank from synced channels, with the built-in bank's
/// settings (`crate::satellites`).
pub fn satellite_bank(channels: &[crate::satellites::SatelliteChannel]) -> BankDraft {
    split_bank(
        &SATELLITES,
        channels.iter().map(|channel| {
            (
                channel.name.as_str(),
                channel.receive_hz,
                channel.transmit_hz,
                channel.tx_ctcss_tenths_hz,
            )
        }),
    )
}

/// Builds one split bank: each channel keeps its own split and transmit tone,
/// and the bank shares the rest.
fn split_bank<'a>(
    list: &PresetSplitList,
    channels: impl Iterator<Item = (&'a str, u32, u32, Option<u16>)>,
) -> BankDraft {
    let channels = channels
        .map(|(name, receive_hz, transmit_hz, tx_ctcss)| ChannelDraft {
            name: name.to_owned(),
            receive_mhz: format_mhz(receive_hz),
            transmit_mhz: format_mhz(transmit_hz),
            tx_tone: tx_ctcss.map_or_else(ToneDraft::default, |tenths| {
                ToneDraft::from_tone(Tone::Ctcss(tenths))
            }),
            modulation: list.modulation,
            bandwidth: list.bandwidth,
            power: list.power,
            step_hz: list.step_hz,
            ..ChannelDraft::default()
        })
        .collect();
    BankDraft {
        name: list.name.to_owned(),
        kind: BankKind::Named,
        scan_enabled: true,
        shared: SharedSettings::from_bits(
            SharedSettings::MODULATION
                | SharedSettings::BANDWIDTH
                | SharedSettings::STEP
                | SharedSettings::POWER,
        )
        .unwrap_or_default(),
        modulation: list.modulation,
        bandwidth: list.bandwidth,
        power: list.power,
        step_hz: list.step_hz,
        channels,
        ..BankDraft::default()
    }
}

#[cfg(test)]
mod tests {
    use super::PRESETS;
    use crate::model::{BankKind, ProjectModel, ValidatedBank, RADIO_IMAGE_BYTES};
    use radio_domain::{Bandwidth, Modulation};

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
        let cb = policy_of("CB 27 MHz");
        // UK channels 1 and 40, CEPT channels 1 and 40.
        for hz in [27_601_250, 27_991_250, 26_965_000, 27_405_000] {
            assert!(contains(&cb, hz), "CB {hz}");
        }
        assert!(cb
            .ranges()
            .all(|range| range.class == radio_domain::TxClass::CitizensBand));
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
                preset.lists.iter().map(|l| l.channels.len()).sum::<usize>()
                    + preset
                        .splits
                        .iter()
                        .map(|l| l.channels.len())
                        .sum::<usize>(),
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
        for preset in PRESETS
            .iter()
            .filter(|p| p.lists.is_empty() && p.splits.is_empty())
        {
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
    fn the_seventy_centimetre_repeaters_listen_low_and_transmit_high() {
        let project = project_of("UK simplex and repeaters");
        let ValidatedBank::Generated(plan) = project.banks[4].validate(4).expect("plan") else {
            panic!("the 70 cm repeater row is a generated plan");
        };
        let first = plan.channel_record(0).expect("channel");
        assert_eq!(first.name().as_str(), "RU240");
        assert_eq!(first.receive().as_hz(), 433_000_000);
        assert_eq!(first.transmit().as_hz(), 434_600_000);
        let last = plan.channel_record(30).expect("channel");
        assert_eq!(last.name().as_str(), "RU270");
        assert_eq!(last.receive().as_hz(), 433_375_000);
    }

    #[test]
    fn the_cb_preset_lands_on_both_channel_plans() {
        let project = project_of("CB 27 MHz receive");
        assert_eq!(plan_channel(&project, 0, 0), ("UK 1".into(), 27_601_250));
        assert_eq!(plan_channel(&project, 0, 39), ("UK 40".into(), 27_991_250));
        let eu = &project.banks[1].channels;
        assert_eq!(eu.len(), 40);
        let at = |name: &str| {
            eu.iter()
                .find(|c| c.name == name)
                .unwrap()
                .receive_mhz
                .clone()
        };
        assert_eq!(at("EU 1"), "26.965000");
        assert_eq!(at("EU 4"), "27.005000");
        assert_eq!(at("EU 19"), "27.185000");
        assert_eq!(at("EU 23"), "27.255000");
        assert_eq!(at("EU 24"), "27.235000");
        assert_eq!(at("EU 40"), "27.405000");
    }

    #[test]
    fn the_military_airband_covers_225_to_400_mhz_in_am() {
        let project = project_of("Military airband receive");
        assert_eq!(plan_channel(&project, 0, 0), ("MIL1".into(), 225_000_000));
        assert_eq!(plan_channel(&project, 0, 2047).1 + 25_000, 276_200_000);
        assert_eq!(plan_channel(&project, 1, 2047).1 + 25_000, 327_400_000);
        assert_eq!(plan_channel(&project, 2, 2047).1 + 25_000, 378_600_000);
        assert_eq!(
            plan_channel(&project, 3, 855),
            ("MIL7000".into(), 399_975_000)
        );
        let guard = &project.banks[4];
        assert_eq!(guard.channels[1].receive_mhz, "243.000000");
        assert!(project
            .banks
            .iter()
            .all(|bank| bank.modulation == radio_domain::Modulation::Am));
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
    fn the_broadcast_presets_span_their_bands_on_the_broadcast_receiver() {
        for (name, first, last) in [
            ("FM broadcast", 87_500_000, 108_000_000),
            ("FM broadcast 76-108 MHz", 76_000_000, 108_000_000),
        ] {
            let preset = PRESETS.iter().find(|p| p.name() == name).expect(name);
            let project = preset.build();
            assert_eq!(project.banks.len(), 1, "{name} is one stored plan");
            let ValidatedBank::Generated(plan) = project.banks[0].validate(0).expect("plan") else {
                panic!("{name} is a generated plan");
            };
            let count = plan.channel_count();
            let first_record = plan.channel_record(0).unwrap();
            let last_record = plan.channel_record(count - 1).unwrap();
            assert_eq!(first_record.receive().as_hz(), first);
            assert_eq!(last_record.receive().as_hz(), last);
            assert_eq!(first_record.modulation(), Modulation::Broadcast);
            assert_eq!(last_record.transmit(), last_record.receive());
        }
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

        // Each bank shares everything but the frequency, and the duplex bank's
        // shared offset reproduces every listed transmit frequency.
        assert!(project.banks.iter().all(|bank| bank.shared.contains(0x7F)));
        assert_eq!(duplex.offset_hz, -4_600_000);
        assert_eq!(simplex.offset_hz, 0);
        let validated = project.validate().unwrap();
        for (channel, draft) in validated.channels().iter().zip(&duplex.channels) {
            assert_eq!(
                crate::model::format_mhz(channel.transmit().as_hz()),
                draft.transmit_mhz
            );
            assert_eq!(channel.bandwidth(), Bandwidth::Wide);
        }
    }

    fn project_of(name: &str) -> ProjectModel {
        PRESETS
            .iter()
            .find(|p| p.name() == name)
            .unwrap_or_else(|| panic!("{name}"))
            .build()
    }

    /// Receive frequency of a generated plan's channel, and its name.
    fn plan_channel(project: &ProjectModel, row: usize, index: u16) -> (String, u32) {
        let ValidatedBank::Generated(plan) = project.banks[row].validate(row).expect("plan") else {
            panic!("row {row} is a generated plan");
        };
        let record = plan.channel_record(index).expect("channel");
        (record.name().as_str().to_owned(), record.receive().as_hz())
    }

    #[test]
    fn the_business_preset_covers_both_bands_continuously() {
        let project = project_of("UK business radio receive");
        assert_eq!(project.banks.len(), 5);
        assert_eq!(plan_channel(&project, 0, 0), ("BV1".into(), 136_000_000));
        assert_eq!(plan_channel(&project, 0, 2047).1 + 12_500, 161_600_000);
        assert_eq!(plan_channel(&project, 1, 0), ("BV2049".into(), 161_600_000));
        assert_eq!(
            plan_channel(&project, 1, 992),
            ("BV3041".into(), 174_000_000)
        );
        assert_eq!(plan_channel(&project, 2, 0), ("BU1".into(), 400_000_000));
        assert_eq!(plan_channel(&project, 2, 2047).1 + 12_500, 425_600_000);
        assert_eq!(plan_channel(&project, 3, 2047).1 + 12_500, 451_200_000);
        assert_eq!(
            plan_channel(&project, 4, 1504),
            ("BU5601".into(), 470_000_000)
        );
    }

    #[test]
    fn the_simple_uk_preset_spans_the_published_band_edges() {
        let project = project_of("UK Simple light licence");
        assert_eq!(plan_channel(&project, 0, 0).1, 77_687_500);
        assert_eq!(plan_channel(&project, 0, 695).1, 86_375_000);
        assert_eq!(plan_channel(&project, 1, 0).1, 164_050_000);
        assert_eq!(plan_channel(&project, 1, 723).1, 173_087_500);
    }

    #[test]
    fn the_radio_mic_and_srd_presets_land_inside_their_allocations() {
        let mics = project_of("Licence-exempt radio mics");
        assert_eq!(plan_channel(&mics, 0, 0).1, 863_000_000);
        assert_eq!(plan_channel(&mics, 0, 80).1, 865_000_000);
        // Every rendering has the same width, so text order is frequency order.
        assert!(mics.banks[1]
            .channels
            .iter()
            .all(|c| ("173.700000".."175.100001").contains(&c.receive_mhz.as_str())));

        let srd = project_of("Licence-exempt short range devices");
        assert_eq!(plan_channel(&srd, 0, 0), ("LPD1".into(), 433_075_000));
        assert_eq!(plan_channel(&srd, 0, 68), ("LPD69".into(), 434_775_000));
        assert_eq!(plan_channel(&srd, 1, 36).1, 458_950_000);
        assert_eq!(plan_channel(&srd, 2, 24).1, 868_600_000);
    }

    #[test]
    fn the_satellite_preset_receives_downlinks_and_transmits_uplinks() {
        use radio_domain::Tone;

        let project = project_of(super::SATELLITE_PRESET);
        let validated = project.validate().expect("validates");
        let channel = |name: &str| {
            validated
                .channels()
                .iter()
                .find(|c| c.name().as_str() == name)
                .copied()
                .unwrap_or_else(|| panic!("{name}"))
        };
        let iss = channel("ISS VOICE");
        assert_eq!(
            (iss.receive().as_hz(), iss.transmit().as_hz()),
            (145_800_000, 145_200_000)
        );
        let repeater = channel("ISS RPT");
        assert_eq!(
            (repeater.receive().as_hz(), repeater.transmit().as_hz()),
            (437_800_000, 145_990_000)
        );
        assert_eq!(repeater.tx_tone(), Tone::Ctcss(670));
        assert_eq!(channel("ISS APRS").tx_tone(), Tone::None);
        assert_eq!(channel("SO-50").receive().as_hz(), 436_795_000);
    }

    #[test]
    fn a_preset_replaces_nothing_the_operator_already_entered() {
        let before = ProjectModel::new();
        let after = PRESETS[0].build();
        assert_ne!(before.banks.len(), after.banks.len());
    }
}
