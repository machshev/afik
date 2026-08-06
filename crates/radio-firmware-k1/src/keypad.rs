//! Hardware-independent K1 main-key matrix decoding and debounce.

/// Stable interval required before accepting a press or release.
pub const DEBOUNCE_MILLISECONDS: u32 = 20;

/// One key in the evidenced 4-by-4 K1 main keypad matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    /// Menu key.
    Menu,
    /// Up key.
    Up,
    /// Down key.
    Down,
    /// Exit key.
    Exit,
    /// Digit zero.
    Digit0,
    /// Digit one.
    Digit1,
    /// Digit two.
    Digit2,
    /// Digit three.
    Digit3,
    /// Digit four.
    Digit4,
    /// Digit five.
    Digit5,
    /// Digit six.
    Digit6,
    /// Digit seven.
    Digit7,
    /// Digit eight.
    Digit8,
    /// Digit nine.
    Digit9,
    /// Star key.
    Star,
    /// Function/hash key.
    Function,
}

impl Key {
    /// Fixed ASCII label used by the display-only witness.
    #[must_use]
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::Menu => b"MENU",
            Self::Up => b"UP",
            Self::Down => b"DOWN",
            Self::Exit => b"EXIT",
            Self::Digit0 => b"0",
            Self::Digit1 => b"1",
            Self::Digit2 => b"2",
            Self::Digit3 => b"3",
            Self::Digit4 => b"4",
            Self::Digit5 => b"5",
            Self::Digit6 => b"6",
            Self::Digit7 => b"7",
            Self::Digit8 => b"8",
            Self::Digit9 => b"9",
            Self::Star => b"STAR",
            Self::Function => b"F",
        }
    }
}

const MATRIX: [[Key; 4]; 4] = [
    [Key::Menu, Key::Digit1, Key::Digit4, Key::Digit7],
    [Key::Up, Key::Digit2, Key::Digit5, Key::Digit8],
    [Key::Down, Key::Digit3, Key::Digit6, Key::Digit9],
    [Key::Exit, Key::Star, Key::Digit0, Key::Function],
];

/// Why a complete matrix observation cannot yield a single key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// A row mask contained bits outside the four evidenced rows.
    InvalidRowBits,
    /// More than one matrix cell was active.
    Ambiguous,
}

/// Decodes active-low row observations for selected columns PB6, PB5, PB4,
/// and PB3, in that order.
///
/// Each array element is a four-bit active-low mask ordered PB15 through PB12:
/// bit zero represents PB15 and bit three represents PB12. A set bit means the
/// corresponding row was low while that column alone was selected low.
pub fn decode(row_low_by_column: [u8; 4]) -> Result<Option<Key>, DecodeError> {
    let mut found = None;
    for (column, rows) in row_low_by_column.into_iter().enumerate() {
        if rows & !0x0F != 0 {
            return Err(DecodeError::InvalidRowBits);
        }
        for (row, key) in MATRIX[column].into_iter().enumerate() {
            if rows & (1 << row) != 0 {
                if found.is_some() {
                    return Err(DecodeError::Ambiguous);
                }
                found = Some(key);
            }
        }
    }
    Ok(found)
}

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
    use super::{decode, Debouncer, DecodeError, Edge, Key, Sample, DEBOUNCE_MILLISECONDS};

    const EXPECTED: [[Key; 4]; 4] = [
        [Key::Menu, Key::Digit1, Key::Digit4, Key::Digit7],
        [Key::Up, Key::Digit2, Key::Digit5, Key::Digit8],
        [Key::Down, Key::Digit3, Key::Digit6, Key::Digit9],
        [Key::Exit, Key::Star, Key::Digit0, Key::Function],
    ];

    #[test]
    fn all_sixteen_cells_decode_exactly() {
        assert_eq!(decode([0; 4]), Ok(None));
        for (column, keys) in EXPECTED.iter().enumerate() {
            for (row, expected) in keys.iter().enumerate() {
                let mut sample = [0_u8; 4];
                sample[column] = 1 << row;
                assert_eq!(decode(sample), Ok(Some(*expected)));
            }
        }
    }

    #[test]
    fn every_two_cell_combination_is_ambiguous() {
        for first in 0..16 {
            for second in first + 1..16 {
                let mut sample = [0_u8; 4];
                sample[first / 4] |= 1 << (first % 4);
                sample[second / 4] |= 1 << (second % 4);
                assert_eq!(decode(sample), Err(DecodeError::Ambiguous));
            }
        }
    }

    #[test]
    fn bits_outside_evidenced_rows_are_invalid() {
        for column in 0..4 {
            for bit in 4..8 {
                let mut sample = [0_u8; 4];
                sample[column] = 1 << bit;
                assert_eq!(decode(sample), Err(DecodeError::InvalidRowBits));
            }
        }
    }

    #[test]
    fn labels_are_exact_and_bounded() {
        let expected = [
            (Key::Menu, &b"MENU"[..]),
            (Key::Up, &b"UP"[..]),
            (Key::Down, &b"DOWN"[..]),
            (Key::Exit, &b"EXIT"[..]),
            (Key::Digit0, &b"0"[..]),
            (Key::Digit1, &b"1"[..]),
            (Key::Digit2, &b"2"[..]),
            (Key::Digit3, &b"3"[..]),
            (Key::Digit4, &b"4"[..]),
            (Key::Digit5, &b"5"[..]),
            (Key::Digit6, &b"6"[..]),
            (Key::Digit7, &b"7"[..]),
            (Key::Digit8, &b"8"[..]),
            (Key::Digit9, &b"9"[..]),
            (Key::Star, &b"STAR"[..]),
            (Key::Function, &b"F"[..]),
        ];
        for (key, label) in expected {
            assert_eq!(key.label(), label);
            assert!(key.label().len() <= 4);
        }
    }

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
        assert_eq!(debounce.update(0, Sample::Key(Key::Digit1)), Edge::None);
        assert_eq!(debounce.update(10, Sample::Released), Edge::None);
        assert_eq!(debounce.update(15, Sample::Key(Key::Digit1)), Edge::None);
        assert_eq!(debounce.update(34, Sample::Key(Key::Digit1)), Edge::None);
        assert_eq!(
            debounce.update(35, Sample::Key(Key::Digit1)),
            Edge::Pressed(Key::Digit1)
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
