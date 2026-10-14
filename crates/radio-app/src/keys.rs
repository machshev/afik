//! Debouncing one decoded key, with explicit time.

use radio_platform::receive_app::Key;

/// Stable interval required before accepting a press or release.
pub const DEBOUNCE_MILLISECONDS: u32 = 20;

/// A complete sample supplied to the debounce state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sample {
    /// No main-matrix key is active.
    Released,
    /// Exactly one decoded main-matrix key is active.
    Key(Key),
    /// The scan was ambiguous, invalid, changing, or failed.
    Invalid,
}

/// A stable edge emitted by [`Debouncer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    /// No stable edge occurred.
    None,
    /// A key became stably pressed.
    Pressed(Key),
    /// A key became stably released.
    Released(Key),
}

/// Explicit-time, allocation-free debounce for one main-matrix key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Debouncer {
    candidate: Option<Key>,
    candidate_since_ms: u32,
    stable: Option<Key>,
    last_sample_ms: Option<u32>,
}

impl Debouncer {
    /// Creates a released debounce state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            candidate: None,
            candidate_since_ms: 0,
            stable: None,
            last_sample_ms: None,
        }
    }

    /// Returns the currently stable held key, if any.
    #[must_use]
    pub const fn held_key(&self) -> Option<Key> {
        self.stable
    }

    /// Applies one complete sample at a monotonic elapsed time in milliseconds.
    ///
    /// Invalid samples and time reversal reset to the fail-closed released
    /// state without emitting an application edge.
    pub fn update(&mut self, now_ms: u32, sample: Sample) -> Edge {
        if self.last_sample_ms.is_some_and(|last| now_ms < last) || sample == Sample::Invalid {
            self.reset(now_ms);
            return Edge::None;
        }
        self.last_sample_ms = Some(now_ms);

        let observed = match sample {
            Sample::Released => None,
            Sample::Key(key) => Some(key),
            Sample::Invalid => unreachable!(),
        };
        if observed != self.candidate {
            self.candidate = observed;
            self.candidate_since_ms = now_ms;
            return Edge::None;
        }
        if observed == self.stable
            || now_ms.saturating_sub(self.candidate_since_ms) < DEBOUNCE_MILLISECONDS
        {
            return Edge::None;
        }

        let previous = self.stable;
        self.stable = observed;
        match (previous, observed) {
            (_, Some(key)) => Edge::Pressed(key),
            (Some(key), None) => Edge::Released(key),
            (None, None) => Edge::None,
        }
    }

    fn reset(&mut self, now_ms: u32) {
        self.candidate = None;
        self.candidate_since_ms = now_ms;
        self.stable = None;
        self.last_sample_ms = Some(now_ms);
    }
}

impl Default for Debouncer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Debouncer, Edge, Key, Sample, DEBOUNCE_MILLISECONDS};

    #[test]
    fn press_and_release_require_stable_elapsed_time() {
        let mut debounce = Debouncer::new();
        assert_eq!(debounce.update(10, Sample::Key(Key::Menu)), Edge::None);
        assert_eq!(debounce.update(29, Sample::Key(Key::Menu)), Edge::None);
        assert_eq!(
            debounce.update(10 + DEBOUNCE_MILLISECONDS, Sample::Key(Key::Menu)),
            Edge::Pressed(Key::Menu)
        );
        assert_eq!(debounce.held_key(), Some(Key::Menu));
        assert_eq!(debounce.update(40, Sample::Released), Edge::None);
        assert_eq!(debounce.update(59, Sample::Released), Edge::None);
        assert_eq!(
            debounce.update(60, Sample::Released),
            Edge::Released(Key::Menu)
        );
        assert_eq!(debounce.held_key(), None);
    }

    #[test]
    fn bounce_restarts_candidate_interval() {
        let mut debounce = Debouncer::new();
        assert_eq!(debounce.update(0, Sample::Key(Key::Digit(1))), Edge::None);
        assert_eq!(debounce.update(10, Sample::Released), Edge::None);
        assert_eq!(debounce.update(15, Sample::Key(Key::Digit(1))), Edge::None);
        assert_eq!(debounce.update(34, Sample::Key(Key::Digit(1))), Edge::None);
        assert_eq!(
            debounce.update(35, Sample::Key(Key::Digit(1))),
            Edge::Pressed(Key::Digit(1))
        );
    }

    #[test]
    fn ambiguity_and_time_reversal_fail_closed_without_edges() {
        let mut debounce = Debouncer::new();
        assert_eq!(debounce.update(0, Sample::Key(Key::Up)), Edge::None);
        assert_eq!(
            debounce.update(20, Sample::Key(Key::Up)),
            Edge::Pressed(Key::Up)
        );
        assert_eq!(debounce.update(21, Sample::Invalid), Edge::None);
        assert_eq!(debounce.held_key(), None);

        assert_eq!(debounce.update(50, Sample::Key(Key::Down)), Edge::None);
        assert_eq!(debounce.update(49, Sample::Key(Key::Down)), Edge::None);
        assert_eq!(debounce.held_key(), None);
        assert_eq!(debounce.update(68, Sample::Key(Key::Down)), Edge::None);
        assert_eq!(
            debounce.update(88, Sample::Key(Key::Down)),
            Edge::Pressed(Key::Down)
        );
    }
}
