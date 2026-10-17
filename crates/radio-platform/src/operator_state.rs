//! Where the operator left the radio, small enough to write down often.
//!
//! A radio which forgets its channel every time the battery is changed is a
//! radio the operator has to set up again before every use. This is the record
//! that stops that: for each of the two slots, the source in force, the bank
//! filtering it, the channel selected, and the frequency the VFO was on; which
//! slot was selected; and whether dual watch was on.
//!
//! It is deliberately not a configuration object. Configuration is what a host
//! programmed and is retained as one canonical image; this is what the operator
//! did afterwards, it changes on every arrow key, and it is written far more
//! often than a channel list ever is. Keeping the two apart means turning a
//! knob cannot rewrite the channels, and cannot cost the erase cycle that
//! rewriting them would.
//!
//! Nothing here mints transmit authority: it names a selection, not a channel.

use crate::operator_shell::VfoSettings;
use radio_channel_plan::{StartPlace, VfoStart, VFO_SLOTS};
use radio_domain::{tones, BankId, SquelchLevel, Tone};
use radio_tx_policy::ClassSet;

/// Bytes one retained record occupies.
///
/// A whole number of these divides both a program page and an erase sector, so
/// a record is written in one page program and never straddles two of them.
pub const OPERATOR_STATE_BYTES: usize = 32;

/// Bytes of a version 1 or 2 record, which an older image wrote and this one
/// still reads from the front of a slot.
const LEGACY_BYTES: usize = 16;

/// Record version this image writes.
///
/// An erased byte is `0xFF`, so a version which is never `0xFF` is also how an
/// unwritten slot is told from a written one. Version 5 adds each slot's VFO
/// tones and power, holding a bank filter in one byte (banks stop at 254) to
/// make room; version 4 names each slot's
/// channel by bank and position (`ADR-115`) where version 3 held a radio-wide
/// index; version 3 is two slots wide and adds the selected slot and dual
/// watch; version 2 added the handset squelch. Versions 1 to 3 are still read,
/// without their channel, and versions 1 and 2 as slot A with dual watch off.
const RECORD_VERSION: u8 = 5;
/// The version before the VFO tones and power were kept.
const RECORD_VERSION_BY_PLACE: u8 = 4;
/// The two-slot version that named a channel by its radio-wide index.
const RECORD_VERSION_BY_INDEX: u8 = 3;
/// The sixteen-byte version with the squelch.
const RECORD_VERSION_WITH_SQUELCH: u8 = 2;
/// The version before the squelch was kept here.
const RECORD_VERSION_WITHOUT_SQUELCH: u8 = 1;
/// The squelch byte of a record which holds none.
const NO_SQUELCH: u8 = 0xFF;
/// The bank field of a slot which names no channel.
const NO_CHANNEL: u16 = 0xFFFF;

/// The operator was listening to programmed channels rather than the VFO.
const MEMORY_MODE: u8 = 0b0000_0001;
/// A bank filter was in force, and the identifier that follows is meaningful.
const BANK_FILTERED: u8 = 0b0000_0010;
/// The flag bits holding the TX classes switched off on the handset: class
/// `n` (1 to 6) is bit `n + 1`. A record written before they existed has them
/// clear, which is every class on.
const TX_DISABLED_BITS: u8 = 0b1111_1100;

/// Second flags byte: the slot not selected listened to channels.
const OTHER_MEMORY_MODE: u8 = 0b0000_0001;
/// Second flags byte: a bank filtered the slot not selected.
const OTHER_BANK_FILTERED: u8 = 0b0000_0010;
/// Second flags byte: slot B was the selected one.
const B_SELECTED: u8 = 0b0000_0100;
/// Second flags byte: dual watch was on.
const DUAL_WATCH: u8 = 0b0000_1000;
/// Second flags byte: TX class 7 (CB), which the first byte has no room for,
/// was switched off. A record without the byte has it clear, which is on.
const TX_DISABLED_CLASS_7: u8 = 0b0001_0000;
/// The [`ClassSet`] bit of class 7.
const CLASS_7_BIT: u8 = 1 << 7;

/// A slot's step byte from version 5: the step index in bits 0-2, the VFO
/// power level in bits 3-5 ([`NO_POWER`] for the programmed default), and
/// the polarity of a DCS receive and transmit code in bits 6 and 7. A step
/// index never reaches 7, so `0xFF` still marks no other slot.
const STEP_BITS: u8 = 0b0000_0111;
const POWER_SHIFT: u8 = 3;
const NO_POWER: u8 = 0b111;
const RX_INVERTED: u8 = 1 << 6;
const TX_INVERTED: u8 = 1 << 7;

/// Encodes one slot's step and VFO settings: the step byte, and the receive
/// and transmit tone positions ([`tones::standard_position`]). A tone the
/// reference does not list is kept as none.
fn encode_vfo(step_index: u8, vfo: VfoSettings) -> (u8, u8, u8) {
    let (rx, rx_inverted) = tones::standard_position(vfo.rx_tone).unwrap_or((0, false));
    let (tx, tx_inverted) = tones::standard_position(vfo.tx_tone).unwrap_or((0, false));
    let mut byte = step_index.min(6) & STEP_BITS;
    byte |= vfo
        .power
        .filter(|level| *level < NO_POWER)
        .unwrap_or(NO_POWER)
        << POWER_SHIFT;
    if rx_inverted {
        byte |= RX_INVERTED;
    }
    if tx_inverted {
        byte |= TX_INVERTED;
    }
    (byte, rx, tx)
}

/// Decodes what [`encode_vfo`] wrote: the step index and the VFO settings.
fn decode_vfo(byte: u8, rx: u8, tx: u8) -> (u8, VfoSettings) {
    let power = (byte >> POWER_SHIFT) & NO_POWER;
    let tone = |position, inverted| tones::standard_tone(position, inverted).unwrap_or(Tone::None);
    (
        byte & STEP_BITS,
        VfoSettings {
            rx_tone: tone(rx, byte & RX_INVERTED != 0),
            tx_tone: tone(tx, byte & TX_INVERTED != 0),
            power: (power != NO_POWER).then_some(power),
        }
        .checked(),
    )
}

/// A bank filter in one byte, as version 5 holds it; a bank past a byte,
/// which no image holds, is dropped.
fn bank_byte(bank: BankId) -> Option<u8> {
    u8::try_from(bank.get()).ok()
}

/// A memory channel, named by its bank and its position there.
///
/// A host may reprogram the radio between one power cycle and the next; other
/// banks changing then moves nothing, and a bank which lost the position
/// starts its slot from the top of the list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelPlace {
    /// The bank holding the channel.
    pub bank: BankId,
    /// Zero-based position in that bank.
    pub position: u16,
}

impl ChannelPlace {
    /// Writes the place, or its absence, into four bytes.
    fn encode(place: Option<Self>, bytes: &mut [u8]) {
        let (position, bank) =
            place.map_or((0, NO_CHANNEL), |place| (place.position, place.bank.get()));
        bytes[..2].copy_from_slice(&position.to_le_bytes());
        bytes[2..4].copy_from_slice(&bank.to_le_bytes());
    }

    /// Reads what [`Self::encode`] wrote.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let bank = u16::from_le_bytes([bytes[2], bytes[3]]);
        (bank != NO_CHANNEL).then(|| Self {
            bank: BankId::new(bank),
            position: u16::from_le_bytes([bytes[0], bytes[1]]),
        })
    }
}

/// Where the slot not selected was left.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtherSlot {
    /// Whether it listened to channels rather than its VFO.
    pub memory_mode: bool,
    /// The bank filtering its channels, if one was.
    pub bank: Option<BankId>,
    /// Its memory channel, if it had one.
    pub channel: Option<ChannelPlace>,
    /// Its VFO frequency in hertz.
    pub vfo_hz: u32,
    /// Index into the selectable tuning steps.
    pub step_index: u8,
    /// Its VFO tones and power.
    pub vfo: VfoSettings,
}

/// Where the operator left the radio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorState {
    /// Whether the programmed channels rather than the VFO were the source.
    pub memory_mode: bool,
    /// The bank filtering the channel view, if one was in force.
    pub bank: Option<BankId>,
    /// The group the source list was narrowed to, if one was (`ADR-116`).
    pub group: Option<u8>,
    /// The memory channel selected, if one was; kept while the VFO is in use
    /// so leaving it returns there.
    pub channel: Option<ChannelPlace>,
    /// The VFO frequency in hertz.
    pub vfo_hz: u32,
    /// Index into the selectable tuning steps.
    pub step_index: u8,
    /// The VFO tones and power.
    pub vfo: VfoSettings,
    /// The radio-wide squelch the operator chose on the handset.
    ///
    /// The host's image carries a default; this is the operator's own choice,
    /// kept here so the handset never rewrites what the host programmed.
    pub squelch: Option<SquelchLevel>,
    /// The TX classes the operator switched off on the handset (`ADR-101`).
    ///
    /// Kept here, beside the place, so lending the radio out survives a power
    /// cycle; it can only narrow the programmed ranges.
    pub tx_disabled: ClassSet,
    /// The slot not selected; absent in a record an older image wrote.
    pub other: Option<OtherSlot>,
    /// Whether slot B rather than A was the selected one; the fields above
    /// describe whichever was.
    pub b_selected: bool,
    /// Whether dual watch was on.
    pub dual_watch: bool,
}

impl OperatorState {
    /// The record a studio writes so the radio starts at `start`
    /// (`ADR-116`): on its channels, in that group, bank and channel, and
    /// everything else where a radio nobody has used starts.
    #[must_use]
    pub fn starting_at(start: StartPlace) -> Self {
        Self {
            memory_mode: true,
            bank: start.bank,
            group: start.group,
            channel: start
                .channel
                .map(|(bank, position)| ChannelPlace { bank, position }),
            vfo_hz: crate::operator_shell::VFO_DEFAULT_HZ,
            step_index: 1,
            vfo: VfoSettings::DEFAULT,
            squelch: None,
            tx_disabled: ClassSet::NONE,
            other: None,
            b_selected: false,
            dual_watch: false,
        }
    }

    /// The record a studio writes (`ADR-116`): the radio starts at `start`,
    /// or on every channel without one, with each programmed slot's VFO and
    /// the programmed dual watch. A programmed slot B listens to its VFO, as a
    /// radio nobody has used does; a step the radio does not offer is its
    /// default.
    #[must_use]
    pub fn programmed(
        start: Option<StartPlace>,
        vfos: [Option<VfoStart>; VFO_SLOTS],
        dual_watch: bool,
    ) -> Self {
        let step_index = |vfo: VfoStart| {
            crate::operator_shell::VFO_STEPS_HZ
                .iter()
                .position(|step| *step == vfo.step_hz)
                .and_then(|index| u8::try_from(index).ok())
                .unwrap_or(1)
        };
        let settings = |vfo: VfoStart| {
            VfoSettings {
                rx_tone: vfo.rx_tone,
                tx_tone: vfo.tx_tone,
                power: vfo.power.map(|power| power as u8),
            }
            .checked()
        };
        let mut state = Self::starting_at(start.unwrap_or_default());
        state.dual_watch = dual_watch;
        if let Some(a) = vfos[0] {
            state.vfo_hz = a.hz;
            state.step_index = step_index(a);
            state.vfo = settings(a);
        }
        state.other = vfos[1].map(|b| OtherSlot {
            memory_mode: false,
            bank: None,
            channel: None,
            vfo_hz: b.hz,
            step_index: step_index(b),
            vfo: settings(b),
        });
        state
    }

    /// Encodes one record, checksum included.
    #[must_use]
    pub fn encode(&self) -> [u8; OPERATOR_STATE_BYTES] {
        let mut bytes = [0_u8; OPERATOR_STATE_BYTES];
        bytes[0] = RECORD_VERSION;
        let mut flags = 0;
        if self.memory_mode {
            flags |= MEMORY_MODE;
        }
        if let Some(bank) = self.bank.and_then(bank_byte) {
            flags |= BANK_FILTERED;
            bytes[2] = bank;
        }
        flags |= (self.tx_disabled.bits() << 1) & TX_DISABLED_BITS;
        bytes[1] = flags;
        ChannelPlace::encode(self.channel, &mut bytes[4..8]);
        bytes[8..12].copy_from_slice(&self.vfo_hz.to_le_bytes());
        (bytes[12], bytes[3], bytes[27]) = encode_vfo(self.step_index, self.vfo);
        bytes[13] = self.squelch.map_or(NO_SQUELCH, SquelchLevel::get);
        let mut flags = 0;
        if let Some(other) = self.other {
            if other.memory_mode {
                flags |= OTHER_MEMORY_MODE;
            }
            if let Some(bank) = other.bank.and_then(bank_byte) {
                flags |= OTHER_BANK_FILTERED;
                bytes[15] = bank;
            }
            ChannelPlace::encode(other.channel, &mut bytes[17..21]);
            bytes[21..25].copy_from_slice(&other.vfo_hz.to_le_bytes());
            (bytes[25], bytes[16], bytes[28]) = encode_vfo(other.step_index, other.vfo);
        } else {
            // No other slot is written as an out-of-range step, which decodes
            // back to none.
            bytes[25] = 0xFF;
        }
        if self.b_selected {
            flags |= B_SELECTED;
        }
        if self.dual_watch {
            flags |= DUAL_WATCH;
        }
        if self.tx_disabled.bits() & CLASS_7_BIT != 0 {
            flags |= TX_DISABLED_CLASS_7;
        }
        bytes[14] = flags;
        // Zero is no group, which is what a record from before groups holds.
        bytes[26] = self.group.map_or(0, |group| group.saturating_add(1));
        let checksum = crc16(&bytes[..OPERATOR_STATE_BYTES - 2]);
        bytes[OPERATOR_STATE_BYTES - 2..].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    /// Decodes one record, refusing an erased, foreign, or torn one.
    ///
    /// A record is programmed in one operation, but a battery removed part way
    /// through one still leaves a partly written slot behind. The checksum is
    /// what makes that slot unreadable rather than plausible, so the reader
    /// falls back to the last complete record instead of restoring rubbish.
    #[must_use]
    pub fn decode(bytes: &[u8; OPERATOR_STATE_BYTES]) -> Option<Self> {
        let covered = match bytes[0] {
            RECORD_VERSION | RECORD_VERSION_BY_PLACE | RECORD_VERSION_BY_INDEX => {
                OPERATOR_STATE_BYTES
            }
            RECORD_VERSION_WITH_SQUELCH | RECORD_VERSION_WITHOUT_SQUELCH => LEGACY_BYTES,
            _ => return None,
        };
        let expected = u16::from_le_bytes([bytes[covered - 2], bytes[covered - 1]]);
        if crc16(&bytes[..covered - 2]) != expected {
            return None;
        }
        let flags = bytes[1];
        // An older record's channel was a radio-wide index into an image this
        // one no longer reads, so the slot starts from the top of its list.
        let by_place = matches!(bytes[0], RECORD_VERSION | RECORD_VERSION_BY_PLACE);
        // Version 5 holds a bank in one byte and a VFO tone in the next.
        let with_vfo = bytes[0] == RECORD_VERSION;
        let bank = |low: u8, high: u8| {
            BankId::new(if with_vfo {
                u16::from(low)
            } else {
                u16::from_le_bytes([low, high])
            })
        };
        let (step_index, vfo) = if with_vfo {
            decode_vfo(bytes[12], bytes[3], bytes[27])
        } else {
            (bytes[12], VfoSettings::DEFAULT)
        };
        let mut state = Self {
            memory_mode: flags & MEMORY_MODE != 0,
            bank: (flags & BANK_FILTERED != 0).then(|| bank(bytes[2], bytes[3])),
            group: None,
            channel: ChannelPlace::decode(&bytes[4..8]).filter(|_| by_place),
            vfo_hz: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            step_index,
            vfo,
            squelch: if bytes[0] == RECORD_VERSION_WITHOUT_SQUELCH {
                None
            } else {
                SquelchLevel::new(bytes[13]).ok()
            },
            tx_disabled: ClassSet::from_bits((flags & TX_DISABLED_BITS) >> 1),
            other: None,
            b_selected: false,
            dual_watch: false,
        };
        if matches!(
            bytes[0],
            RECORD_VERSION | RECORD_VERSION_BY_PLACE | RECORD_VERSION_BY_INDEX
        ) {
            let flags = bytes[14];
            state.b_selected = flags & B_SELECTED != 0;
            state.dual_watch = flags & DUAL_WATCH != 0;
            state.group = bytes[26].checked_sub(1);
            if flags & TX_DISABLED_CLASS_7 != 0 {
                state.tx_disabled = ClassSet::from_bits(state.tx_disabled.bits() | CLASS_7_BIT);
            }
            state.other = (bytes[25] != 0xFF).then(|| {
                let (step_index, vfo) = if with_vfo {
                    decode_vfo(bytes[25], bytes[16], bytes[28])
                } else {
                    (bytes[25], VfoSettings::DEFAULT)
                };
                OtherSlot {
                    memory_mode: flags & OTHER_MEMORY_MODE != 0,
                    bank: (flags & OTHER_BANK_FILTERED != 0).then(|| bank(bytes[15], bytes[16])),
                    channel: ChannelPlace::decode(&bytes[17..21]).filter(|_| by_place),
                    vfo_hz: u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]),
                    step_index,
                    vfo,
                }
            });
        }
        Some(state)
    }
}

/// Reports whether one slot has never been programmed.
///
/// A serial NOR memory reads erased bytes as `0xFF`, so an untouched slot is
/// recognisable without a marker of its own.
#[must_use]
pub fn is_erased(bytes: &[u8; OPERATOR_STATE_BYTES]) -> bool {
    bytes.iter().all(|byte| *byte == 0xFF)
}

/// CRC-16/CCITT-FALSE over one record's covered bytes.
///
/// Sixteen bits over thirty bytes, which is far more than the record needs: this is guarding against a write cut short by a flat
/// battery, not against a hostile one.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xFFFF_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::{
        crc16, is_erased, ChannelPlace, OperatorState, OtherSlot, VfoSettings, LEGACY_BYTES,
        OPERATOR_STATE_BYTES,
    };
    use radio_domain::{BankId, SquelchLevel, Tone, TxClass};
    use radio_tx_policy::ClassSet;

    fn state() -> OperatorState {
        OperatorState {
            memory_mode: true,
            bank: Some(BankId::new(3)),
            group: Some(2),
            channel: Some(ChannelPlace {
                bank: BankId::new(3),
                position: 41,
            }),
            vfo_hz: 433_500_000,
            step_index: 2,
            vfo: VfoSettings {
                rx_tone: Tone::Ctcss(885),
                tx_tone: Tone::Dcs {
                    code: 23,
                    inverted: true,
                },
                power: Some(3),
            },
            squelch: Some(SquelchLevel::new(4).expect("level")),
            tx_disabled: ClassSet::NONE.with(TxClass::Amateur, true),
            other: Some(OtherSlot {
                memory_mode: true,
                bank: Some(BankId::new(7)),
                channel: Some(ChannelPlace {
                    bank: BankId::new(200),
                    position: 5,
                }),
                vfo_hz: 145_500_000,
                step_index: 1,
                vfo: VfoSettings {
                    rx_tone: Tone::None,
                    tx_tone: Tone::Ctcss(1000),
                    power: None,
                },
            }),
            b_selected: true,
            dual_watch: true,
        }
    }

    /// `state` without its VFO settings, as a record before version 5 reads.
    fn without_vfo(state: OperatorState) -> OperatorState {
        OperatorState {
            vfo: VfoSettings::DEFAULT,
            other: state.other.map(|other| OtherSlot {
                vfo: VfoSettings::DEFAULT,
                ..other
            }),
            ..state
        }
    }

    /// Encodes `state` as a version `version` image of 32 bytes would have:
    /// banks in two bytes, steps in whole bytes, and no VFO settings.
    fn two_slot(state: OperatorState, version: u8) -> [u8; OPERATOR_STATE_BYTES] {
        let mut encoded = state.encode();
        encoded[0] = version;
        encoded[3] = 0;
        encoded[12] = state.step_index;
        encoded[16] = 0;
        encoded[25] = state.other.map_or(0xFF, |other| other.step_index);
        encoded[27] = 0;
        encoded[28] = 0;
        let checksum = crc16(&encoded[..OPERATOR_STATE_BYTES - 2]);
        encoded[OPERATOR_STATE_BYTES - 2..].copy_from_slice(&checksum.to_le_bytes());
        encoded
    }

    /// Encodes `state` as an older image would have: sixteen bytes, version
    /// `version`, the rest of the slot erased.
    fn legacy(state: OperatorState, version: u8) -> [u8; OPERATOR_STATE_BYTES] {
        let mut encoded = two_slot(state, version);
        if version == 1 {
            encoded[13] = 0;
        }
        encoded[14..].fill(0xFF);
        let checksum = crc16(&encoded[..LEGACY_BYTES - 2]);
        encoded[LEGACY_BYTES - 2..LEGACY_BYTES].copy_from_slice(&checksum.to_le_bytes());
        encoded
    }

    #[test]
    fn a_record_survives_a_round_trip_in_every_variant() {
        for variant in [
            state(),
            OperatorState {
                bank: None,
                ..state()
            },
            OperatorState {
                memory_mode: false,
                ..state()
            },
            OperatorState {
                squelch: None,
                ..state()
            },
            OperatorState {
                tx_disabled: ClassSet::NONE,
                ..state()
            },
            OperatorState {
                tx_disabled: ClassSet::NONE.with(TxClass::CitizensBand, true),
                ..state()
            },
            OperatorState {
                tx_disabled: [
                    TxClass::LicenceFreePlan,
                    TxClass::Amateur,
                    TxClass::Marine,
                    TxClass::Aeronautical,
                    TxClass::Business,
                    TxClass::Experimental,
                    TxClass::CitizensBand,
                ]
                .into_iter()
                .fold(ClassSet::NONE, |set, class| set.with(class, true)),
                bank: None,
                memory_mode: false,
                ..state()
            },
            OperatorState {
                channel: None,
                vfo_hz: 1_000_000,
                step_index: 0,
                ..state()
            },
            OperatorState {
                channel: Some(ChannelPlace {
                    bank: BankId::new(253),
                    position: u16::MAX,
                }),
                vfo_hz: u32::MAX,
                step_index: 5,
                vfo: VfoSettings {
                    rx_tone: Tone::Dcs {
                        code: 754,
                        inverted: true,
                    },
                    tx_tone: Tone::Ctcss(2541),
                    power: Some(5),
                },
                ..state()
            },
            OperatorState {
                vfo: VfoSettings::DEFAULT,
                ..state()
            },
            OperatorState {
                other: None,
                b_selected: false,
                dual_watch: false,
                ..state()
            },
            OperatorState {
                other: Some(OtherSlot {
                    memory_mode: false,
                    bank: None,
                    channel: None,
                    vfo_hz: 433_500_000,
                    step_index: 0,
                    vfo: VfoSettings::DEFAULT,
                }),
                ..state()
            },
        ] {
            let encoded = variant.encode();
            assert!(!is_erased(&encoded));
            assert_eq!(OperatorState::decode(&encoded), Some(variant));
        }
    }

    #[test]
    fn a_studio_record_holds_its_start_both_vfos_and_dual_watch() {
        use radio_channel_plan::{StartPlace, VfoStart};
        use radio_domain::PowerLevel;
        let start = StartPlace {
            group: Some(1),
            bank: Some(BankId::new(3)),
            channel: Some((BankId::new(3), 4)),
        };
        let a = VfoStart {
            hz: 433_500_000,
            step_hz: 25_000,
            rx_tone: Tone::Ctcss(885),
            tx_tone: Tone::Ctcss(1000),
            power: Some(PowerLevel::Mid),
        };
        let b = VfoStart {
            hz: 446_068_750,
            step_hz: 7_000,
            rx_tone: Tone::None,
            tx_tone: Tone::Dcs {
                code: 23,
                inverted: false,
            },
            power: None,
        };
        let state = OperatorState::programmed(Some(start), [Some(a), Some(b)], true);
        assert!(state.memory_mode && state.dual_watch);
        assert_eq!((state.group, state.bank), (start.group, start.bank));
        assert_eq!((state.vfo_hz, state.step_index), (433_500_000, 2));
        assert_eq!(state.vfo.power, Some(4));
        assert_eq!(state.vfo.rx_tone, Tone::Ctcss(885));
        let other = state.other.expect("slot B");
        assert!(!other.memory_mode, "B listens to its VFO");
        assert_eq!(
            (other.vfo_hz, other.step_index),
            (446_068_750, 1),
            "unoffered step"
        );
        assert_eq!(other.vfo.tx_tone, b.tx_tone);
        assert_eq!(OperatorState::decode(&state.encode()), Some(state));

        // Without a start or VFOs: every channel, defaults, and dual watch
        // as programmed.
        let plain = OperatorState::programmed(None, [None, None], false);
        assert_eq!(plain, OperatorState::starting_at(StartPlace::default()));
    }

    #[test]
    fn an_erased_slot_is_not_a_record() {
        let erased = [0xFF_u8; OPERATOR_STATE_BYTES];
        assert!(is_erased(&erased));
        assert_eq!(OperatorState::decode(&erased), None);
    }

    /// A half-written record must be unreadable, not plausible.
    #[test]
    fn any_single_changed_byte_is_refused() {
        let encoded = state().encode();
        for index in 0..OPERATOR_STATE_BYTES {
            let mut torn = encoded;
            torn[index] ^= 0xFF;
            assert_eq!(
                OperatorState::decode(&torn),
                None,
                "byte {index} was accepted"
            );
        }
    }

    /// A record written before the class switches existed has their bits
    /// clear, which is every class on.
    #[test]
    fn an_older_record_switches_no_class_off() {
        let older = OperatorState {
            tx_disabled: ClassSet::NONE,
            ..state()
        };
        let mut encoded = older.encode();
        assert_eq!(encoded[1] & 0b1111_1100, 0);
        let checksum = crc16(&encoded[..OPERATOR_STATE_BYTES - 2]);
        encoded[OPERATOR_STATE_BYTES - 2..].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(
            OperatorState::decode(&encoded).map(|state| state.tx_disabled),
            Some(ClassSet::NONE)
        );
    }

    /// A record a future image writes must not be read as one of these.
    #[test]
    fn a_foreign_version_is_refused() {
        let mut encoded = state().encode();
        encoded[0] = 6;
        assert_eq!(OperatorState::decode(&encoded), None);
    }

    /// A tone the reference does not list, which no handset list offers, is
    /// kept as none rather than as a neighbour.
    #[test]
    fn an_unlisted_tone_is_kept_as_none() {
        let mut odd = state();
        odd.vfo.rx_tone = Tone::Ctcss(1001);
        let decoded = OperatorState::decode(&odd.encode()).expect("record");
        assert_eq!(decoded.vfo.rx_tone, Tone::None);
        assert_eq!(decoded.vfo.tx_tone, odd.vfo.tx_tone);
    }

    /// A radio upgraded from an image before the VFO settings keeps its place
    /// and starts both VFOs without tones, at the programmed power.
    #[test]
    fn a_version_four_record_is_read_without_vfo_settings() {
        assert_eq!(
            OperatorState::decode(&two_slot(state(), 4)),
            Some(without_vfo(state()))
        );
    }

    /// A radio upgraded from an image that named channels by a radio-wide
    /// index keeps everything but the channels.
    #[test]
    fn a_version_three_record_is_read_without_its_channels() {
        let encoded = two_slot(state(), 3);
        let mut expected = without_vfo(state());
        expected.channel = None;
        if let Some(other) = expected.other.as_mut() {
            other.channel = None;
        }
        assert_eq!(OperatorState::decode(&encoded), Some(expected));
    }

    /// A radio upgraded from an image without the squelch keeps its place.
    #[test]
    fn a_version_one_record_is_read_without_a_squelch() {
        assert_eq!(
            OperatorState::decode(&legacy(state(), 1)),
            Some(OperatorState {
                channel: None,
                group: None,
                squelch: None,
                other: None,
                b_selected: false,
                dual_watch: false,
                ..without_vfo(state())
            })
        );
    }

    /// A radio upgraded from a one-slot image keeps its place as slot A, with
    /// dual watch off.
    #[test]
    fn a_version_two_record_is_read_as_slot_a() {
        assert_eq!(
            OperatorState::decode(&legacy(state(), 2)),
            Some(OperatorState {
                channel: None,
                group: None,
                other: None,
                b_selected: false,
                dual_watch: false,
                ..without_vfo(state())
            })
        );
    }
}
