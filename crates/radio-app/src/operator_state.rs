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
/// unwritten slot is told from a written one. Version 3 is two slots wide and
/// adds the selected slot and dual watch; version 2 added the handset squelch.
/// Versions 1 and 2 are still read, as slot A with dual watch off.
const RECORD_VERSION: u8 = 3;
/// The sixteen-byte version with the squelch.
const RECORD_VERSION_WITH_SQUELCH: u8 = 2;
/// The version before the squelch was kept here.
const RECORD_VERSION_WITHOUT_SQUELCH: u8 = 1;
/// The squelch byte of a record which holds none.
const NO_SQUELCH: u8 = 0xFF;

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

/// Where the slot not selected was left.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtherSlot {
    /// Whether it listened to channels rather than its VFO.
    pub memory_mode: bool,
    /// The bank filtering its channels, if one was.
    pub bank: Option<BankId>,
    /// Zero-based selection index of its channel.
    pub index: u16,
    /// Identifier of the channel that index named, checked as
    /// [`OperatorState::channel_id`] is.
    pub channel_id: u16,
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
    /// Zero-based selection index of the active channel.
    pub index: u16,
    /// Identifier of the channel that index named when this was written.
    ///
    /// A host may reprogram the radio between one power cycle and the next, and
    /// then the index alone would restore a channel the operator never chose.
    /// Restoring checks the identifier still matches and starts from the top of
    /// the list when it does not.
    pub channel_id: u16,
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
        bytes[4..6].copy_from_slice(&self.index.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.channel_id.to_le_bytes());
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
            bytes[17..19].copy_from_slice(&other.index.to_le_bytes());
            bytes[19..21].copy_from_slice(&other.channel_id.to_le_bytes());
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
        bytes[14] = flags;
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
            RECORD_VERSION => OPERATOR_STATE_BYTES,
            RECORD_VERSION_WITH_SQUELCH | RECORD_VERSION_WITHOUT_SQUELCH => LEGACY_BYTES,
            _ => return None,
        };
        let expected = u16::from_le_bytes([bytes[covered - 2], bytes[covered - 1]]);
        if crc16(&bytes[..covered - 2]) != expected {
            return None;
        }
        let flags = bytes[1];
        let mut state = Self {
            memory_mode: flags & MEMORY_MODE != 0,
            bank: (flags & BANK_FILTERED != 0)
                .then(|| BankId::new(u16::from_le_bytes([bytes[2], bytes[3]]))),
            index: u16::from_le_bytes([bytes[4], bytes[5]]),
            channel_id: u16::from_le_bytes([bytes[6], bytes[7]]),
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
        if bytes[0] == RECORD_VERSION {
            let flags = bytes[14];
            state.b_selected = flags & B_SELECTED != 0;
            state.dual_watch = flags & DUAL_WATCH != 0;
            state.other = (bytes[25] != 0xFF).then(|| OtherSlot {
                memory_mode: flags & OTHER_MEMORY_MODE != 0,
                bank: (flags & OTHER_BANK_FILTERED != 0)
                    .then(|| BankId::new(u16::from_le_bytes([bytes[15], bytes[16]]))),
                index: u16::from_le_bytes([bytes[17], bytes[18]]),
                channel_id: u16::from_le_bytes([bytes[19], bytes[20]]),
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
    use super::{crc16, is_erased, OperatorState, OtherSlot, LEGACY_BYTES, OPERATOR_STATE_BYTES};
    use radio_domain::{BankId, SquelchLevel, TxClass};
    use radio_tx_policy::ClassSet;

    fn state() -> OperatorState {
        OperatorState {
            memory_mode: true,
            bank: Some(BankId::new(3)),
            index: 41,
            channel_id: 0x8103,
            vfo_hz: 433_500_000,
            step_index: 2,
            squelch: Some(SquelchLevel::new(4).expect("level")),
            tx_disabled: ClassSet::NONE.with(TxClass::Amateur, true),
            other: Some(OtherSlot {
                memory_mode: true,
                bank: Some(BankId::new(7)),
                index: 5,
                channel_id: 0x8207,
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
                tx_disabled: [
                    TxClass::LicenceFreePlan,
                    TxClass::Amateur,
                    TxClass::Marine,
                    TxClass::Aeronautical,
                    TxClass::Business,
                    TxClass::Experimental,
                ]
                .into_iter()
                .fold(ClassSet::NONE, |set, class| set.with(class, true)),
                bank: None,
                memory_mode: false,
                ..state()
            },
            OperatorState {
                index: 0,
                channel_id: 0,
                vfo_hz: 1_000_000,
                step_index: 0,
                ..state()
            },
            OperatorState {
                index: u16::MAX,
                channel_id: u16::MAX,
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
                    index: 0,
                    channel_id: 0,
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
        encoded[0] = 4;
        assert_eq!(OperatorState::decode(&encoded), None);
    }

    /// A radio upgraded from an image without the squelch keeps its place.
    #[test]
    fn a_version_one_record_is_read_without_a_squelch() {
        assert_eq!(
            OperatorState::decode(&legacy(state(), 1)),
            Some(OperatorState {
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
                other: None,
                b_selected: false,
                dual_watch: false,
                ..state()
            })
        );
    }
}
