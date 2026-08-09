//! Where the operator left the radio, small enough to write down often.
//!
//! A radio which forgets its channel every time the battery is changed is a
//! radio the operator has to set up again before every use. This is the record
//! that stops that: the source in force, the bank filtering it, the channel
//! selected, and the frequency the VFO was on.
//!
//! It is deliberately not a configuration object. Configuration is what a host
//! programmed and is retained as one canonical image; this is what the operator
//! did afterwards, it changes on every arrow key, and it is written far more
//! often than a channel list ever is. Keeping the two apart means turning a
//! knob cannot rewrite the channels, and cannot cost the erase cycle that
//! rewriting them would.
//!
//! Nothing here mints transmit authority: it names a selection, not a channel.

use radio_domain::BankId;

/// Bytes one retained record occupies.
///
/// A whole number of these divides both a program page and an erase sector, so
/// a record is written in one page program and never straddles two of them.
pub const OPERATOR_STATE_BYTES: usize = 16;

/// Record version this image writes and accepts.
///
/// An erased byte is `0xFF`, so a version which is never `0xFF` is also how an
/// unwritten slot is told from a written one.
const RECORD_VERSION: u8 = 1;

/// The operator was listening to programmed channels rather than the VFO.
const MEMORY_MODE: u8 = 0b0000_0001;
/// A bank filter was in force, and the identifier that follows is meaningful.
const BANK_FILTERED: u8 = 0b0000_0010;

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
        bytes[1] = flags;
        bytes[4..6].copy_from_slice(&self.index.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.channel_id.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.vfo_hz.to_le_bytes());
        bytes[12] = self.step_index;
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
        if bytes[0] != RECORD_VERSION {
            return None;
        }
        let expected = u16::from_le_bytes([
            bytes[OPERATOR_STATE_BYTES - 2],
            bytes[OPERATOR_STATE_BYTES - 1],
        ]);
        if crc16(&bytes[..OPERATOR_STATE_BYTES - 2]) != expected {
            return None;
        }
        let flags = bytes[1];
        Some(Self {
            memory_mode: flags & MEMORY_MODE != 0,
            bank: (flags & BANK_FILTERED != 0)
                .then(|| BankId::new(u16::from_le_bytes([bytes[2], bytes[3]]))),
            index: u16::from_le_bytes([bytes[4], bytes[5]]),
            channel_id: u16::from_le_bytes([bytes[6], bytes[7]]),
            vfo_hz: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            step_index: bytes[12],
        })
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
/// Sixteen bits over fourteen bytes, which is what the record has room for and
/// far more than it needs: this is guarding against a write cut short by a flat
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
    use super::{is_erased, OperatorState, OPERATOR_STATE_BYTES};
    use radio_domain::BankId;

    fn state() -> OperatorState {
        OperatorState {
            memory_mode: true,
            bank: Some(BankId::new(3)),
            index: 41,
            channel_id: 0x8103,
            vfo_hz: 433_500_000,
            step_index: 2,
        }
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

    /// A record a future image writes must not be read as one of these.
    #[test]
    fn a_foreign_version_is_refused() {
        let mut encoded = state().encode();
        encoded[0] = 2;
        assert_eq!(OperatorState::decode(&encoded), None);
    }
}
