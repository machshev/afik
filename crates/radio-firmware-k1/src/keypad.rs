//! Hardware-independent K1 main-key matrix decoding and debounce.

/// Stable interval required before accepting a press or release.
pub const DEBOUNCE_MILLISECONDS: u32 = 20;

/// Exact GPIOB masks for the bounded main-key matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GpioPlan {
    /// GPIOB peripheral clock enable bit.
    pub clock_enable: u32,
    /// Row-mode fields to clear for PB12..PB15 inputs.
    pub row_mode_clear: u32,
    /// Row pull fields to clear.
    pub row_pull_clear: u32,
    /// Pull-up values for all four rows.
    pub row_pull_set: u32,
    /// Column-mode fields to clear for PB3..PB6.
    pub column_mode_clear: u32,
    /// Push-pull output-mode values for all four columns.
    pub column_mode_set: u32,
    /// Column output-type bits to clear for push-pull.
    pub column_type_clear: u32,
    /// Column speed fields to clear.
    pub column_speed_clear: u32,
    /// High-speed values from the pinned board configuration.
    pub column_speed_set: u32,
    /// Column pull fields to clear.
    pub column_pull_clear: u32,
    /// Pull-up values from the pinned board configuration.
    pub column_pull_set: u32,
    /// PB3..PB6 bits which must all be high at idle.
    pub columns_high: u32,
    /// Selected-low column masks, ordered PB6, PB5, PB4, PB3.
    pub selected_low: [u32; 4],
    /// PB12..PB15 input mask.
    pub rows: u32,
}

/// Returns the exact GPIOB-only main-key configuration plan.
#[must_use]
pub const fn gpio_plan() -> GpioPlan {
    GpioPlan {
        clock_enable: 1 << 1,
        row_mode_clear: 0xFF00_0000,
        row_pull_clear: 0xFF00_0000,
        row_pull_set: 0x5500_0000,
        column_mode_clear: 0x0000_3FC0,
        column_mode_set: 0x0000_1540,
        column_type_clear: 0x0000_0078,
        column_speed_clear: 0x0000_3FC0,
        column_speed_set: 0x0000_3FC0,
        column_pull_clear: 0x0000_3FC0,
        column_pull_set: 0x0000_1540,
        columns_high: 0x0000_0078,
        selected_low: [1 << 6, 1 << 5, 1 << 4, 1 << 3],
        rows: 0x0000_F000,
    }
}

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

/// Minimal board operation boundary used by the deterministic matrix scanner.
pub trait MatrixBus {
    /// Adapter error.
    type Error;

    /// Drives all four main-key columns high.
    fn drive_all_columns_high(&mut self) -> Result<(), Self::Error>;

    /// Drives one evidenced column low while the others remain high.
    fn drive_column_low(&mut self, column: usize) -> Result<(), Self::Error>;

    /// Reads PB12..PB15 after the adapter's independently bounded settling step.
    ///
    /// The returned low four bits are ordered PB15 through PB12 and are set for
    /// active-low rows. Bits above bit three must be zero.
    fn read_active_rows(&mut self) -> Result<u8, Self::Error>;
}

/// Failure from one complete main-key matrix scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError<E> {
    /// A select or read operation failed; idle cleanup was still attempted.
    Operation(E),
    /// Restoring all columns high failed.
    Cleanup(E),
}

/// Scans all four main-key columns and restores all columns high.
///
/// The bus owns the settling operation before each read. This function records
/// no target clock rate or delay assumption.
pub fn scan<B: MatrixBus>(bus: &mut B) -> Result<[u8; 4], ScanError<B::Error>> {
    bus.drive_all_columns_high().map_err(ScanError::Cleanup)?;
    let mut rows = [0_u8; 4];
    for (column, observed) in rows.iter_mut().enumerate() {
        if let Err(error) = bus.drive_column_low(column) {
            let _ = bus.drive_all_columns_high();
            return Err(ScanError::Operation(error));
        }
        match bus.read_active_rows() {
            Ok(value) => *observed = value,
            Err(error) => {
                let _ = bus.drive_all_columns_high();
                return Err(ScanError::Operation(error));
            }
        }
        bus.drive_all_columns_high().map_err(ScanError::Cleanup)?;
    }
    Ok(rows)
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

/// Converts a GPIOB input register value into PB15-through-PB12 active-low bits.
#[must_use]
pub const fn active_rows_from_gpio_idr(idr: u32) -> u8 {
    let low = ((!idr >> 12) & 0x0F) as u8;
    ((low & 0x01) << 3) | ((low & 0x02) << 1) | ((low & 0x04) >> 1) | ((low & 0x08) >> 3)
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
    use super::{
        active_rows_from_gpio_idr, decode, gpio_plan, scan, Debouncer, DecodeError, Edge, Key,
        MatrixBus, Sample, ScanError, DEBOUNCE_MILLISECONDS,
    };
    use std::vec::Vec;

    const EXPECTED: [[Key; 4]; 4] = [
        [Key::Menu, Key::Digit1, Key::Digit4, Key::Digit7],
        [Key::Up, Key::Digit2, Key::Digit5, Key::Digit8],
        [Key::Down, Key::Digit3, Key::Digit6, Key::Digit9],
        [Key::Exit, Key::Star, Key::Digit0, Key::Function],
    ];

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Operation {
        AllHigh,
        ColumnLow(usize),
        Read,
    }

    struct TraceBus {
        trace: Vec<Operation>,
        rows: [u8; 4],
        read_index: usize,
        fail_at: Option<usize>,
    }

    impl MatrixBus for TraceBus {
        type Error = usize;

        fn drive_all_columns_high(&mut self) -> Result<(), Self::Error> {
            self.record(Operation::AllHigh)
        }

        fn drive_column_low(&mut self, column: usize) -> Result<(), Self::Error> {
            self.record(Operation::ColumnLow(column))
        }

        fn read_active_rows(&mut self) -> Result<u8, Self::Error> {
            self.record(Operation::Read)?;
            let value = self.rows[self.read_index];
            self.read_index += 1;
            Ok(value)
        }
    }

    impl TraceBus {
        fn record(&mut self, operation: Operation) -> Result<(), usize> {
            let index = self.trace.len();
            self.trace.push(operation);
            if self.fail_at == Some(index) {
                Err(index)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn gpio_plan_touches_only_gpio_b_matrix_fields() {
        let plan = gpio_plan();
        assert_eq!(plan.clock_enable, 0x0000_0002);
        assert_eq!(plan.row_mode_clear, 0xFF00_0000);
        assert_eq!(plan.row_pull_clear, 0xFF00_0000);
        assert_eq!(plan.row_pull_set, 0x5500_0000);
        assert_eq!(plan.column_mode_clear, 0x0000_3FC0);
        assert_eq!(plan.column_mode_set, 0x0000_1540);
        assert_eq!(plan.column_type_clear, 0x0000_0078);
        assert_eq!(plan.column_speed_clear, 0x0000_3FC0);
        assert_eq!(plan.column_speed_set, 0x0000_3FC0);
        assert_eq!(plan.column_pull_clear, 0x0000_3FC0);
        assert_eq!(plan.column_pull_set, 0x0000_1540);
        assert_eq!(plan.columns_high, 0x0000_0078);
        assert_eq!(plan.selected_low, [0x40, 0x20, 0x10, 0x08]);
        assert_eq!(plan.rows, 0x0000_F000);
    }

    #[test]
    fn gpio_input_rows_are_active_low_and_reordered() {
        assert_eq!(active_rows_from_gpio_idr(0x0000_F000), 0);
        assert_eq!(active_rows_from_gpio_idr(0x0000_7000), 0b0001);
        assert_eq!(active_rows_from_gpio_idr(0x0000_B000), 0b0010);
        assert_eq!(active_rows_from_gpio_idr(0x0000_D000), 0b0100);
        assert_eq!(active_rows_from_gpio_idr(0x0000_E000), 0b1000);
        assert_eq!(active_rows_from_gpio_idr(0), 0b1111);
    }

    #[test]
    fn scan_trace_selects_each_column_and_restores_idle() {
        let mut bus = TraceBus {
            trace: Vec::new(),
            rows: [1, 2, 4, 8],
            read_index: 0,
            fail_at: None,
        };
        assert_eq!(scan(&mut bus), Ok([1, 2, 4, 8]));
        assert_eq!(
            bus.trace,
            [
                Operation::AllHigh,
                Operation::ColumnLow(0),
                Operation::Read,
                Operation::AllHigh,
                Operation::ColumnLow(1),
                Operation::Read,
                Operation::AllHigh,
                Operation::ColumnLow(2),
                Operation::Read,
                Operation::AllHigh,
                Operation::ColumnLow(3),
                Operation::Read,
                Operation::AllHigh,
            ]
        );
    }

    #[test]
    fn every_select_or_read_failure_attempts_idle_cleanup() {
        for fail_at in [1, 2, 4, 5, 7, 8, 10, 11] {
            let mut bus = TraceBus {
                trace: Vec::new(),
                rows: [0; 4],
                read_index: 0,
                fail_at: Some(fail_at),
            };
            assert_eq!(scan(&mut bus), Err(ScanError::Operation(fail_at)));
            assert_eq!(bus.trace.last(), Some(&Operation::AllHigh));
        }
    }

    #[test]
    fn idle_cleanup_failure_is_reported() {
        let mut bus = TraceBus {
            trace: Vec::new(),
            rows: [0; 4],
            read_index: 0,
            fail_at: Some(3),
        };
        assert_eq!(scan(&mut bus), Err(ScanError::Cleanup(3)));
        assert_eq!(bus.trace.last(), Some(&Operation::AllHigh));
    }

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
