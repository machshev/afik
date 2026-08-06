use core::{cmp::Ordering, fmt};

use crate::{Ax25Callsign, RepeaterAdvertisement, RepeaterEvent, ReportKind, ReportName};

/// Exact conservative identity for one untrusted discovery origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryKey {
    /// Object or Item identity class.
    pub kind: ReportKind,
    /// Case-sensitive entity name.
    pub name: ReportName,
    /// Originating AX.25 source and SSID.
    pub source: Ax25Callsign,
}

impl RepeaterAdvertisement {
    /// Returns the local kind/name/source discovery identity.
    pub const fn key(self) -> DiscoveryKey {
        DiscoveryKey {
            kind: self.kind,
            name: self.name,
            source: self.source,
        }
    }
}

impl RepeaterEvent {
    /// Returns the exact discovery identity affected by this event.
    pub const fn key(self) -> DiscoveryKey {
        match self {
            Self::Live(advertisement) => advertisement.key(),
            Self::Killed { kind, name, source } => DiscoveryKey { kind, name, source },
        }
    }
}

/// One currently visible receive-only discovery entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryEntry {
    /// Last accepted live untrusted advertisement.
    pub advertisement: RepeaterAdvertisement,
    /// Explicit monotonic receive time supplied by the caller.
    pub received_at: u64,
}

impl DiscoveryEntry {
    /// Returns the conservative identity of this entry.
    pub const fn key(self) -> DiscoveryKey {
        self.advertisement.key()
    }
}

/// Deterministic outcome of one accepted table input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryUpdate {
    /// A previously unseen live identity occupied a free slot.
    Inserted,
    /// Newer data replaced live data, refreshed a kill, or revived a key.
    Updated,
    /// A newer same-origin kill hid the live entry and retained its freshness.
    Removed,
    /// Identical live data, a repeated equal-time kill, or an unknown kill did nothing.
    Unchanged,
    /// An older same-key input did nothing.
    Stale,
    /// Equal-time differing data or lifecycle did nothing.
    Conflict,
}

/// A discovery table input failed without mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    /// No free live or retained-kill slot remained; no entry was evicted.
    Full,
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("APRS discovery table is full"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiscoverySlot {
    Empty,
    Live(DiscoveryEntry),
    Killed { key: DiscoveryKey, received_at: u64 },
}

impl DiscoverySlot {
    const fn key(self) -> Option<DiscoveryKey> {
        match self {
            Self::Empty => None,
            Self::Live(entry) => Some(entry.key()),
            Self::Killed { key, .. } => Some(key),
        }
    }

    const fn received_at(self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Live(entry) => Some(entry.received_at),
            Self::Killed { received_at, .. } => Some(received_at),
        }
    }
}

/// A fixed-capacity, allocation-free table of untrusted repeater observations.
pub struct DiscoveryTable<const CAPACITY: usize> {
    slots: [DiscoverySlot; CAPACITY],
}

impl<const CAPACITY: usize> Default for DiscoveryTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> DiscoveryTable<CAPACITY> {
    /// Constructs an empty table without a clock or allocator.
    pub const fn new() -> Self {
        Self {
            slots: [DiscoverySlot::Empty; CAPACITY],
        }
    }

    /// Returns the compile-time number of live or retained-kill slots.
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    /// Returns the number of currently visible live entries.
    pub fn len(&self) -> usize {
        self.entries().count()
    }

    /// Returns whether no live entry is currently visible.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns live entries in stable underlying slot order.
    pub fn entries(&self) -> impl Iterator<Item = &DiscoveryEntry> {
        self.slots.iter().filter_map(|slot| match slot {
            DiscoverySlot::Live(entry) => Some(entry),
            DiscoverySlot::Empty | DiscoverySlot::Killed { .. } => None,
        })
    }

    /// Returns one live same-kind/name/source entry, excluding retained kills.
    pub fn get(&self, key: DiscoveryKey) -> Option<&DiscoveryEntry> {
        self.entries().find(|entry| entry.key() == key)
    }

    /// Returns slots occupied by either live entries or retained kill freshness.
    pub fn occupied_len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| !matches!(slot, DiscoverySlot::Empty))
            .count()
    }

    /// Applies one parsed event at an explicit monotonic receive time.
    pub fn apply(
        &mut self,
        event: RepeaterEvent,
        received_at: u64,
    ) -> Result<DiscoveryUpdate, DiscoveryError> {
        let key = event.key();
        if let Some(index) = self.find_slot(key) {
            return Ok(self.apply_existing(index, event, received_at));
        }
        match event {
            RepeaterEvent::Killed { .. } => Ok(DiscoveryUpdate::Unchanged),
            RepeaterEvent::Live(advertisement) => {
                let Some(slot) = self
                    .slots
                    .iter_mut()
                    .find(|slot| matches!(slot, DiscoverySlot::Empty))
                else {
                    return Err(DiscoveryError::Full);
                };
                *slot = DiscoverySlot::Live(DiscoveryEntry {
                    advertisement,
                    received_at,
                });
                Ok(DiscoveryUpdate::Inserted)
            }
        }
    }

    /// Removes live entries and retained kills strictly older than `cutoff`.
    pub fn expire_before(&mut self, cutoff: u64) -> usize {
        let mut expired = 0_usize;
        for slot in &mut self.slots {
            if slot.received_at().is_some_and(|time| time < cutoff) {
                *slot = DiscoverySlot::Empty;
                expired += 1;
            }
        }
        expired
    }

    fn find_slot(&self, key: DiscoveryKey) -> Option<usize> {
        self.slots.iter().position(|slot| slot.key() == Some(key))
    }

    fn apply_existing(
        &mut self,
        index: usize,
        event: RepeaterEvent,
        received_at: u64,
    ) -> DiscoveryUpdate {
        let slot = self.slots[index];
        let ordering = received_at.cmp(&slot.received_at().unwrap_or(received_at));
        match ordering {
            Ordering::Less => DiscoveryUpdate::Stale,
            Ordering::Equal => match (slot, event) {
                (DiscoverySlot::Live(entry), RepeaterEvent::Live(advertisement))
                    if entry.advertisement == advertisement =>
                {
                    DiscoveryUpdate::Unchanged
                }
                (DiscoverySlot::Killed { .. }, RepeaterEvent::Killed { .. }) => {
                    DiscoveryUpdate::Unchanged
                }
                _ => DiscoveryUpdate::Conflict,
            },
            Ordering::Greater => self.replace_newer(index, slot, event, received_at),
        }
    }

    fn replace_newer(
        &mut self,
        index: usize,
        previous: DiscoverySlot,
        event: RepeaterEvent,
        received_at: u64,
    ) -> DiscoveryUpdate {
        match event {
            RepeaterEvent::Live(advertisement) => {
                self.slots[index] = DiscoverySlot::Live(DiscoveryEntry {
                    advertisement,
                    received_at,
                });
                DiscoveryUpdate::Updated
            }
            RepeaterEvent::Killed { .. } => {
                self.slots[index] = DiscoverySlot::Killed {
                    key: event.key(),
                    received_at,
                };
                if matches!(previous, DiscoverySlot::Live(_)) {
                    DiscoveryUpdate::Removed
                } else {
                    DiscoveryUpdate::Updated
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{DiscoveryError, DiscoveryTable, DiscoveryUpdate};
    use crate::{parse_repeater_event, RepeaterEvent};
    use std::vec::Vec;

    fn fcs_accumulator(bytes: &[u8]) -> u16 {
        let mut fcs = 0xffff_u16;
        for byte in bytes {
            fcs ^= u16::from(*byte);
            for _ in 0..8 {
                fcs = if fcs & 1 == 0 {
                    fcs >> 1
                } else {
                    (fcs >> 1) ^ 0x8408
                };
            }
        }
        fcs
    }

    fn address(callsign: &[u8], ssid: u8, final_address: bool) -> [u8; 7] {
        let mut encoded = [b' ' << 1; 7];
        for (destination, source) in encoded.iter_mut().zip(callsign.iter().copied()) {
            *destination = source << 1;
        }
        encoded[6] = 0x60 | (ssid << 1) | u8::from(final_address);
        encoded
    }

    fn event(source: &[u8], ssid: u8, information: &[u8]) -> RepeaterEvent {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&address(b"APRS", 0, false));
        bytes.extend_from_slice(&address(source, ssid, true));
        bytes.extend_from_slice(&[0x03, 0xf0]);
        bytes.extend_from_slice(information);
        let fcs = !fcs_accumulator(&bytes);
        bytes.extend_from_slice(&fcs.to_le_bytes());
        parse_repeater_event(&bytes).unwrap()
    }

    fn live(source: &[u8], ssid: u8, name: &[u8; 9], suffix: &[u8]) -> RepeaterEvent {
        let mut information = Vec::from(b";".as_slice());
        information.extend_from_slice(name);
        information.extend_from_slice(b"*111111z4903.50N/07201.75Wr");
        information.extend_from_slice(suffix);
        event(source, ssid, &information)
    }

    fn killed(source: &[u8], ssid: u8, name: &[u8; 9]) -> RepeaterEvent {
        let mut information = Vec::from(b";".as_slice());
        information.extend_from_slice(name);
        information.extend_from_slice(b"_111111z4903.50N/07201.75Wr");
        event(source, ssid, &information)
    }

    fn live_item(source: &[u8], ssid: u8, name: &[u8]) -> RepeaterEvent {
        let mut information = Vec::from(b")".as_slice());
        information.extend_from_slice(name);
        information.extend_from_slice(b"!4903.50N/07201.75Wr146.94 MHz");
        event(source, ssid, &information)
    }

    #[test]
    fn same_key_updates_are_monotonic_and_equal_time_conflicts_do_not_mutate() {
        let first = live(b"DIGI", 1, b"146.940-A", b"T107");
        let changed = live(b"DIGI", 1, b"146.940-A", b"T088");
        let mut table = DiscoveryTable::<2>::new();

        assert_eq!(table.apply(first, 10), Ok(DiscoveryUpdate::Inserted));
        let key = first.key();
        assert_eq!(table.get(key).unwrap().received_at, 10);
        assert_eq!(table.apply(changed, 9), Ok(DiscoveryUpdate::Stale));
        assert_eq!(table.apply(changed, 10), Ok(DiscoveryUpdate::Conflict));
        assert_eq!(table.apply(first, 10), Ok(DiscoveryUpdate::Unchanged));
        assert_eq!(
            table.get(key).unwrap().advertisement,
            match first {
                RepeaterEvent::Live(advertisement) => advertisement,
                RepeaterEvent::Killed { .. } => unreachable!(),
            }
        );

        assert_eq!(table.apply(changed, 11), Ok(DiscoveryUpdate::Updated));
        assert_eq!(table.get(key).unwrap().received_at, 11);
    }

    #[test]
    fn keys_preserve_source_ssid_case_and_report_kind() {
        let upper = live(b"DIGI", 1, b"146.940-A", b"");
        let lower = live(b"DIGI", 1, b"146.940-a", b"");
        let other_ssid = live(b"DIGI", 2, b"146.940-A", b"");
        let item = live_item(b"DIGI", 1, b"146.940-A");
        let mut table = DiscoveryTable::<4>::new();

        assert_eq!(table.apply(upper, 1), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(table.apply(lower, 1), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(table.apply(other_ssid, 1), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(table.apply(item, 1), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(table.len(), 4);
        assert_ne!(upper.key(), lower.key());
        assert_ne!(upper.key(), other_ssid.key());
        assert_ne!(upper.key(), item.key());
    }

    #[test]
    fn full_table_never_evicts_and_unknown_kills_consume_no_slot() {
        let first = live(b"DIGI", 0, b"146.940-A", b"");
        let second = live(b"DIGI", 0, b"147.000-A", b"");
        let unknown_kill = killed(b"OTHER", 0, b"145.000-A");
        let mut table = DiscoveryTable::<1>::new();

        assert_eq!(table.apply(unknown_kill, 1), Ok(DiscoveryUpdate::Unchanged));
        assert_eq!(table.occupied_len(), 0);
        assert_eq!(table.apply(first, 2), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(table.apply(second, 3), Err(DiscoveryError::Full));
        assert!(table.get(first.key()).is_some());
        assert!(table.get(second.key()).is_none());
    }

    #[test]
    fn same_origin_kill_retains_freshness_and_blocks_stale_resurrection() {
        let report = live(b"DIGI", 3, b"146.940-A", b"");
        let remove = killed(b"DIGI", 3, b"146.940-A");
        let other_origin = killed(b"OTHER", 3, b"146.940-A");
        let mut table = DiscoveryTable::<2>::new();

        assert_eq!(table.apply(report, 10), Ok(DiscoveryUpdate::Inserted));
        assert_eq!(
            table.apply(other_origin, 20),
            Ok(DiscoveryUpdate::Unchanged)
        );
        assert_eq!(table.apply(remove, 11), Ok(DiscoveryUpdate::Removed));
        assert!(table.is_empty());
        assert_eq!(table.occupied_len(), 1);
        assert_eq!(table.apply(report, 10), Ok(DiscoveryUpdate::Stale));
        assert_eq!(table.apply(report, 11), Ok(DiscoveryUpdate::Conflict));
        assert_eq!(table.apply(report, 12), Ok(DiscoveryUpdate::Updated));
        assert_eq!(table.get(report.key()).unwrap().received_at, 12);
    }

    #[test]
    fn explicit_expiry_removes_live_entries_and_kill_freshness_only_before_cutoff() {
        let first = live(b"DIGI", 0, b"146.940-A", b"");
        let second = live(b"DIGI", 0, b"147.000-A", b"");
        let remove = killed(b"DIGI", 0, b"147.000-A");
        let mut table = DiscoveryTable::<3>::new();
        table.apply(first, 5).unwrap();
        table.apply(second, 10).unwrap();
        table.apply(remove, 11).unwrap();

        assert_eq!(table.expire_before(5), 0);
        assert_eq!(table.expire_before(6), 1);
        assert_eq!(table.len(), 0);
        assert_eq!(table.occupied_len(), 1);
        assert_eq!(table.expire_before(12), 1);
        assert_eq!(table.occupied_len(), 0);
    }

    #[test]
    fn zero_capacity_is_bounded_and_non_panicking() {
        let report = live(b"DIGI", 0, b"146.940-A", b"");
        let mut table = DiscoveryTable::<0>::new();
        assert_eq!(table.capacity(), 0);
        assert_eq!(table.apply(report, 1), Err(DiscoveryError::Full));
        assert!(table.is_empty());
    }
}
