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

use radio_channel_plan::StartPlace;
use radio_domain::{BankId, SquelchLevel};
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
/// unwritten slot is told from a written one. Version 4 names each slot's
/// channel by bank and position (`ADR-115`) where version 3 held a radio-wide
/// index; version 3 is two slots wide and adds the selected slot and dual
/// watch; version 2 added the handset squelch. Versions 1 to 3 are still read,
/// without their channel, and versions 1 and 2 as slot A with dual watch off.
const RECORD_VERSION: u8 = 4;
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
            squelch: None,
            tx_disabled: ClassSet::NONE,
            other: None,
            b_selected: false,
            dual_watch: false,
        }
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
        if let Some(bank) = self.bank {
            flags |= BANK_FILTERED;
            bytes[2..4].copy_from_slice(&bank.get().to_le_bytes());
        }
        flags |= (self.tx_disabled.bits() << 1) & TX_DISABLED_BITS;
        bytes[1] = flags;
        ChannelPlace::encode(self.channel, &mut bytes[4..8]);
        bytes[8..12].copy_from_slice(&self.vfo_hz.to_le_bytes());
        bytes[12] = self.step_index;
        bytes[13] = self.squelch.map_or(NO_SQUELCH, SquelchLevel::get);
        let mut flags = 0;
        if let Some(other) = self.other {
            if other.memory_mode {
                flags |= OTHER_MEMORY_MODE;
            }
            if let Some(bank) = other.bank {
                flags |= OTHER_BANK_FILTERED;
                bytes[15..17].copy_from_slice(&bank.get().to_le_bytes());
            }
            ChannelPlace::encode(other.channel, &mut bytes[17..21]);
            bytes[21..25].copy_from_slice(&other.vfo_hz.to_le_bytes());
            bytes[25] = other.step_index;
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
            RECORD_VERSION | RECORD_VERSION_BY_INDEX => OPERATOR_STATE_BYTES,
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
        let by_place = bytes[0] == RECORD_VERSION;
        let mut state = Self {
            memory_mode: flags & MEMORY_MODE != 0,
            bank: (flags & BANK_FILTERED != 0)
                .then(|| BankId::new(u16::from_le_bytes([bytes[2], bytes[3]]))),
            group: None,
            channel: ChannelPlace::decode(&bytes[4..8]).filter(|_| by_place),
            vfo_hz: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            step_index: bytes[12],
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
        if bytes[0] == RECORD_VERSION || bytes[0] == RECORD_VERSION_BY_INDEX {
            let flags = bytes[14];
            state.b_selected = flags & B_SELECTED != 0;
            state.dual_watch = flags & DUAL_WATCH != 0;
            state.group = bytes[26].checked_sub(1);
            if flags & TX_DISABLED_CLASS_7 != 0 {
                state.tx_disabled = ClassSet::from_bits(state.tx_disabled.bits() | CLASS_7_BIT);
            }
            state.other = (bytes[25] != 0xFF).then(|| OtherSlot {
                memory_mode: flags & OTHER_MEMORY_MODE != 0,
                bank: (flags & OTHER_BANK_FILTERED != 0)
                    .then(|| BankId::new(u16::from_le_bytes([bytes[15], bytes[16]]))),
                channel: ChannelPlace::decode(&bytes[17..21]).filter(|_| by_place),
                vfo_hz: u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]),
                step_index: bytes[25],
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
        crc16, is_erased, ChannelPlace, OperatorState, OtherSlot, LEGACY_BYTES,
        OPERATOR_STATE_BYTES,
    };
    use radio_domain::{BankId, SquelchLevel, TxClass};
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
            }),
            b_selected: true,
            dual_watch: true,
        }
    }

    /// Encodes `state` as an older image would have: sixteen bytes, version
    /// `version`, the rest of the slot erased.
    fn legacy(state: OperatorState, version: u8) -> [u8; OPERATOR_STATE_BYTES] {
        let mut encoded = state.encode();
        encoded[0] = version;
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
                step_index: u8::MAX,
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
        encoded[0] = 5;
        assert_eq!(OperatorState::decode(&encoded), None);
    }

    /// A radio upgraded from an image that named channels by a radio-wide
    /// index keeps everything but the channels.
    #[test]
    fn a_version_three_record_is_read_without_its_channels() {
        let mut encoded = state().encode();
        encoded[0] = 3;
        let checksum = crc16(&encoded[..OPERATOR_STATE_BYTES - 2]);
        encoded[OPERATOR_STATE_BYTES - 2..].copy_from_slice(&checksum.to_le_bytes());
        let mut expected = state();
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
                ..state()
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
                ..state()
            })
        );
    }
}
