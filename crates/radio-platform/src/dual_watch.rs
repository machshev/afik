//! Dual watch: which of the two slots the one receiver listens to.
//!
//! The radio has one receiver and two slots. With dual watch on it listens to
//! the selected slot and the other in turn, a dwell each, until one of them is
//! busy; it then stays there while the squelch is open and for a hang time
//! after, so a reply on the same channel is not missed. Any operator action
//! and every transmission return it to the selected slot, because that is
//! the one keys act on and PTT transmits on, as on the stock and Armel
//! firmware.
//!
//! A duplex channel the radio may not transmit on (`ADR-108`) is watched
//! the same way: the receiver also stops on its second frequency, so both
//! sides of a duplex conversation are heard. Every stop is listened to in
//! turn, the selected slot's two sides and then the other slot's.
//!
//! The state machine is pure: the caller passes milliseconds, the stops
//! there are and the squelch it read, and retunes when told to.

/// Milliseconds the receiver stays on a slot after its squelch shuts, and on
/// the selected slot after an operator action or a transmission.
///
/// Chosen, not taken from a reference: long enough for the other station to
/// key up in reply.
pub const HANG_MILLISECONDS: u32 = 3_000;

/// Which slot the receiver is tuned to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Listening {
    /// The slot keys act on and PTT transmits on.
    #[default]
    Selected,
    /// The slot not selected.
    Other,
}

/// Which of a channel's frequencies the receiver is tuned to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Side {
    /// The channel's receive frequency.
    #[default]
    Receive,
    /// A listen-only duplex channel's other frequency, where the other
    /// side of the conversation transmits.
    Transmit,
}

/// One place the receiver can listen: a slot, and a side of its channel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stop {
    /// The slot.
    pub slot: Listening,
    /// The side of its channel.
    pub side: Side,
}

impl Stop {
    /// The selected slot's receive frequency, where every action returns.
    pub const HOME: Self = Self {
        slot: Listening::Selected,
        side: Side::Receive,
    };
    /// Every stop, in the order they are visited.
    const ORDER: [Self; 4] = [
        Self::HOME,
        Self {
            slot: Listening::Selected,
            side: Side::Transmit,
        },
        Self {
            slot: Listening::Other,
            side: Side::Receive,
        },
        Self {
            slot: Listening::Other,
            side: Side::Transmit,
        },
    ];
}

/// Which stops there are on this pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stops {
    /// The selected slot's channel is duplex and may not transmit.
    pub selected_duplex: bool,
    /// Dual watch includes the other slot.
    pub other: bool,
    /// The other slot's channel is duplex and may not transmit.
    pub other_duplex: bool,
}

impl Stops {
    /// Reports whether the receiver can listen at `stop`.
    #[must_use]
    pub const fn contains(self, stop: Stop) -> bool {
        match (stop.slot, stop.side) {
            (Listening::Selected, Side::Receive) => true,
            (Listening::Selected, Side::Transmit) => self.selected_duplex,
            (Listening::Other, Side::Receive) => self.other,
            (Listening::Other, Side::Transmit) => self.other && self.other_duplex,
        }
    }

    /// Reports whether there is anywhere to move to from the selected slot's
    /// receive frequency.
    #[must_use]
    pub const fn any_beyond_home(self) -> bool {
        self.selected_duplex || self.other
    }

    /// The stop after `stop`, coming round to the first.
    fn after(self, stop: Stop) -> Stop {
        let at = Stop::ORDER
            .iter()
            .position(|candidate| *candidate == stop)
            .unwrap_or(0);
        (1..=Stop::ORDER.len())
            .map(|ahead| Stop::ORDER[(at + ahead) % Stop::ORDER.len()])
            .find(|candidate| self.contains(*candidate))
            .unwrap_or(Stop::HOME)
    }
}

/// Dual-watch timing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DualWatch {
    stop: Stop,
    /// When the receiver next moves on.
    deadline_ms: u32,
}

impl DualWatch {
    /// Starts on the selected slot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stop: Stop::HOME,
            deadline_ms: 0,
        }
    }

    /// Returns which slot the receiver is tuned to.
    #[must_use]
    pub const fn listening(&self) -> Listening {
        self.stop.slot
    }

    /// Returns which slot, and which side of its channel, the receiver is
    /// tuned to.
    #[must_use]
    pub const fn stop(&self) -> Stop {
        self.stop
    }

    /// Returns to the selected slot's receive frequency and stays for the
    /// hang time.
    ///
    /// Returns whether the receiver was anywhere else, and so has to be
    /// tuned back.
    pub fn reset(&mut self, now_ms: u32) -> bool {
        let was_away = self.stop != Stop::HOME;
        self.stop = Stop::HOME;
        self.deadline_ms = now_ms.wrapping_add(HANG_MILLISECONDS);
        was_away
    }

    /// Advances one pass.
    ///
    /// `busy` is whether the stop tuned now has its squelch open on a
    /// settled reading. Returns the stop to tune when the receiver should
    /// move: at once if its stop has gone, otherwise once it has been quiet
    /// for the dwell, or the hang time after a signal.
    pub fn poll(&mut self, now_ms: u32, dwell_ms: u32, stops: Stops, busy: bool) -> Option<Stop> {
        let gone = !stops.contains(self.stop);
        if busy && !gone {
            self.deadline_ms = now_ms.wrapping_add(HANG_MILLISECONDS);
            return None;
        }
        if !gone && !reached(now_ms, self.deadline_ms) {
            return None;
        }
        let next = stops.after(self.stop);
        self.deadline_ms = now_ms.wrapping_add(dwell_ms);
        if next == self.stop {
            return None;
        }
        self.stop = next;
        Some(next)
    }
}

/// Reports whether `now` is at or past `deadline` on a wrapping clock.
const fn reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < 1 << 31
}

#[cfg(test)]
mod tests {
    use super::{DualWatch, Listening, Side, Stop, Stops, HANG_MILLISECONDS};

    const DWELL: u32 = 90;
    /// Dual watch between two simplex slots.
    const SLOTS: Stops = Stops {
        selected_duplex: false,
        other: true,
        other_duplex: false,
    };

    fn slot(stop: Option<Stop>) -> Option<Listening> {
        stop.map(|stop| {
            assert_eq!(stop.side, Side::Receive);
            stop.slot
        })
    }

    #[test]
    fn it_alternates_by_the_dwell_while_both_slots_are_quiet() {
        let mut watch = DualWatch::new();
        assert_eq!(
            slot(watch.poll(0, DWELL, SLOTS, false)),
            Some(Listening::Other)
        );
        assert_eq!(slot(watch.poll(DWELL - 1, DWELL, SLOTS, false)), None);
        assert_eq!(
            slot(watch.poll(DWELL, DWELL, SLOTS, false)),
            Some(Listening::Selected)
        );
        assert_eq!(
            slot(watch.poll(2 * DWELL, DWELL, SLOTS, false)),
            Some(Listening::Other)
        );
    }

    #[test]
    fn a_busy_slot_holds_the_receiver_until_the_hang_ends() {
        let mut watch = DualWatch::new();
        slot(watch.poll(0, DWELL, SLOTS, false));
        assert_eq!(watch.listening(), Listening::Other);
        // A signal on the other slot holds it for as long as it lasts.
        for now in (10..5_000).step_by(10) {
            assert_eq!(slot(watch.poll(now, DWELL, SLOTS, true)), None);
        }
        // Then for the hang time after it ends.
        assert_eq!(slot(watch.poll(5_000, DWELL, SLOTS, false)), None);
        assert_eq!(
            slot(watch.poll(4_990 + HANG_MILLISECONDS - 1, DWELL, SLOTS, false)),
            None
        );
        assert_eq!(
            slot(watch.poll(4_990 + HANG_MILLISECONDS, DWELL, SLOTS, false)),
            Some(Listening::Selected)
        );
    }

    #[test]
    fn reset_returns_to_the_selected_slot_and_stays_for_the_hang() {
        let mut watch = DualWatch::new();
        slot(watch.poll(0, DWELL, SLOTS, false));
        assert!(watch.reset(10), "it was on the other slot");
        assert_eq!(watch.listening(), Listening::Selected);
        assert_eq!(
            slot(watch.poll(10 + HANG_MILLISECONDS - 1, DWELL, SLOTS, false)),
            None
        );
        assert_eq!(
            slot(watch.poll(10 + HANG_MILLISECONDS, DWELL, SLOTS, false)),
            Some(Listening::Other)
        );
        assert!(watch.reset(20_000));
        assert!(!watch.reset(20_010), "already on the selected slot");
    }

    #[test]
    fn the_clock_may_wrap() {
        let mut watch = DualWatch::new();
        assert!(!watch.reset(u32::MAX - 100));
        assert_eq!(slot(watch.poll(u32::MAX, DWELL, SLOTS, false)), None);
        assert_eq!(
            slot(watch.poll(
                (u32::MAX - 100).wrapping_add(HANG_MILLISECONDS),
                DWELL,
                SLOTS,
                false
            )),
            Some(Listening::Other)
        );
    }

    #[test]
    fn a_duplex_channel_is_heard_on_both_sides() {
        let duplex = Stops {
            selected_duplex: true,
            ..Stops::default()
        };
        let ship = Stop {
            slot: Listening::Selected,
            side: Side::Transmit,
        };
        let mut watch = DualWatch::new();
        assert_eq!(watch.poll(0, DWELL, duplex, false), Some(ship));
        assert_eq!(watch.poll(DWELL, DWELL, duplex, false), Some(Stop::HOME));
        assert_eq!(watch.poll(2 * DWELL, DWELL, duplex, false), Some(ship));
        // The ship talking holds the receiver on its side.
        assert_eq!(watch.poll(3 * DWELL, DWELL, duplex, true), None);
        assert_eq!(watch.stop(), ship);
        assert_eq!(
            watch.poll(3 * DWELL + HANG_MILLISECONDS, DWELL, duplex, false),
            Some(Stop::HOME)
        );
        // Nowhere else to go: it stays on the channel's receive frequency.
        assert_eq!(watch.poll(20_000, DWELL, Stops::default(), false), None);
    }

    #[test]
    fn every_stop_is_visited_in_turn_and_a_vanished_one_is_left_at_once() {
        let all = Stops {
            selected_duplex: true,
            other: true,
            other_duplex: true,
        };
        let mut watch = DualWatch::new();
        for (n, expected) in
            (0_u32..).zip([Stop::ORDER[1], Stop::ORDER[2], Stop::ORDER[3], Stop::HOME])
        {
            assert_eq!(watch.poll(n * DWELL, DWELL, all, false), Some(expected));
        }
        // On the other slot's far side, which then stops being duplex: the
        // receiver leaves even while it is busy there.
        watch.poll(4 * DWELL, DWELL, all, false);
        watch.poll(5 * DWELL, DWELL, all, false);
        assert_eq!(
            watch.poll(6 * DWELL, DWELL, all, false),
            Some(Stop::ORDER[3])
        );
        assert_eq!(
            watch.poll(6 * DWELL + 1, DWELL, SLOTS, true),
            Some(Stop::HOME)
        );
        assert!(!SLOTS.contains(Stop::ORDER[1]));
        assert!(SLOTS.any_beyond_home() && !Stops::default().any_beyond_home());
    }
}
