//! Hardware-independent K1 main-key matrix decoding and debounce.

/// Stable interval required before accepting a press or release.
pub const DEBOUNCE_MILLISECONDS: u32 = 20;

const COLUMN_BITS: u16 = 0x0078;

/// Volatile raw GPIOB snapshot latch for physical-routing diagnosis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawGpioLatch {
    baseline: Option<[u16; 4]>,
    captured: Option<[u16; 4]>,
}

impl RawGpioLatch {
    /// Creates an empty latch; the first observation becomes its baseline.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            baseline: None,
            captured: None,
        }
    }

    /// Observes four raw GPIOB IDR values, ignoring the scanner's PB3..PB6 bits.
    pub fn observe(&mut self, values: [u16; 4]) {
        let Some(baseline) = self.baseline else {
            self.baseline = Some(values);
            return;
        };
        if values
            .into_iter()
            .zip(baseline)
            .any(|(value, initial)| (value ^ initial) & !COLUMN_BITS != 0)
        {
            self.captured = Some(values);
        }
    }

    /// Returns and clears a captured deviation, or the supplied current values.
    pub fn take_or(&mut self, current: [u16; 4]) -> ([u16; 4], bool) {
        match self.captured.take() {
            Some(captured) => (captured, true),
            None => (current, false),
        }
    }
}

impl Default for RawGpioLatch {
    fn default() -> Self {
        Self::new()
    }
}

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

/// Active-low unselected-pass bit for PB15, evidenced as side key one.
pub const SIDE1_ROW_BIT: u8 = 0x01;

/// Active-low unselected-pass bit for PB14, evidenced as side key two.
pub const SIDE2_ROW_BIT: u8 = 0x02;

/// Unselected-pass bits PB13 and PB12, which the pinned source leaves invalid.
pub const UNDEFINED_UNSELECTED_BITS: u8 = 0x0C;

/// One key on the evidenced K1 keypad.
///
/// This covers the 4-by-4 main matrix, the two side keys read during the
/// unselected pass, and the separately wired PTT input. No variant carries
/// transmit authority; the display-only witness renders labels and nothing
/// else consumes them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    /// Side key one, PB15 low while every column stays high.
    Side1,
    /// Side key two, PB14 low while every column stays high.
    Side2,
    /// Push-to-talk input on PB10, active low and independent of the matrix.
    ///
    /// AFIK implements no transmit path, so this is an input observation only.
    Ptt,
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
            Self::Side1 => b"SIDE1",
            Self::Side2 => b"SIDE2",
            Self::Ptt => b"PTT",
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

/// Why a complete keypad observation cannot yield a single key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// A row mask contained bits outside the four evidenced rows.
    InvalidRowBits,
    /// More than one key was active.
    Ambiguous,
    /// PB13 or PB12 was low during the unselected pass.
    ///
    /// The pinned source fills those cells with `KEY_INVALID`, so AFIK records
    /// the observation and fails closed rather than decoding a key.
    UndefinedUnselectedRow,
}

/// One complete keypad observation.
///
/// Every field is an untrusted raw sample. Bits in the row masks are ordered
/// PB15 through PB12, where bit zero is PB15 and a set bit means that row read
/// low.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeypadScan {
    /// Rows read while all four columns PB6..PB3 remained high.
    ///
    /// No main-matrix button can pull a row low in this state, so a low row is
    /// a side key wired directly to it.
    pub unselected: u8,
    /// Rows read per selected-low column, ordered PB6, PB5, PB4, PB3.
    pub columns: [u8; 4],
    /// Whether the separately wired active-low PB10 PTT input read low.
    pub ptt_pressed: bool,
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

    /// Reads the separately wired active-low PB10 PTT input.
    ///
    /// Returns `true` when the input reads low. This is an input observation
    /// only; AFIK exposes no transmit path for it to reach.
    fn read_ptt_pressed(&mut self) -> Result<bool, Self::Error>;
}

/// Failure from one complete main-key matrix scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError<E> {
    /// A select or read operation failed; idle cleanup was still attempted.
    Operation(E),
    /// Restoring all columns high failed.
    Cleanup(E),
}

/// Scans the unselected pass, all four main-key columns, and the PTT input,
/// then restores all columns high.
///
/// The unselected pass is read first, matching the pinned source's scan order.
/// The bus owns the settling operation before each read, so this function
/// records no target clock rate or delay assumption.
pub fn scan<B: MatrixBus>(bus: &mut B) -> Result<KeypadScan, ScanError<B::Error>> {
    bus.drive_all_columns_high().map_err(ScanError::Cleanup)?;
    let unselected = match bus.read_active_rows() {
        Ok(value) => value,
        Err(error) => {
            let _ = bus.drive_all_columns_high();
            return Err(ScanError::Operation(error));
        }
    };
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
    let ptt_pressed = match bus.read_ptt_pressed() {
        Ok(value) => value,
        Err(error) => {
            let _ = bus.drive_all_columns_high();
            return Err(ScanError::Operation(error));
        }
    };
    Ok(KeypadScan {
        unselected,
        columns: rows,
        ptt_pressed,
    })
}

/// Decodes one complete keypad observation into at most one key.
///
/// The unselected pass yields [`Key::Side1`] for PB15 and [`Key::Side2`] for
/// PB14. PB13 and PB12 are undefined in that state and fail closed. Any two
/// simultaneously active keys are ambiguous, and PTT is reported only when no
/// matrix or side key is active, matching the pinned source's precedence.
///
/// A side key is wired directly from its row to ground, so it holds that row
/// low during every column pass as well as the unselected one. Those rows are
/// therefore removed from the column masks before matrix decoding; without
/// that, one held side key would look like five simultaneous keys and fail
/// closed as ambiguous.
pub fn decode(scan: KeypadScan) -> Result<Option<Key>, DecodeError> {
    if (scan.unselected | scan.columns.iter().fold(0, |bits, rows| bits | rows)) & !0x0F != 0 {
        return Err(DecodeError::InvalidRowBits);
    }
    if scan.unselected & UNDEFINED_UNSELECTED_BITS != 0 {
        return Err(DecodeError::UndefinedUnselectedRow);
    }

    let mut found = None;
    let mut record = |key: Key| -> Result<(), DecodeError> {
        if found.is_some() {
            return Err(DecodeError::Ambiguous);
        }
        found = Some(key);
        Ok(())
    };

    if scan.unselected & SIDE1_ROW_BIT != 0 {
        record(Key::Side1)?;
    }
    if scan.unselected & SIDE2_ROW_BIT != 0 {
        record(Key::Side2)?;
    }
    for (column, rows) in scan.columns.into_iter().enumerate() {
        let rows = rows & !scan.unselected;
        for (row, key) in MATRIX[column].into_iter().enumerate() {
            if rows & (1 << row) != 0 {
                record(key)?;
            }
        }
    }

    if found.is_none() && scan.ptt_pressed {
        found = Some(Key::Ptt);
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
        KeypadScan, MatrixBus, RawGpioLatch, Sample, ScanError, DEBOUNCE_MILLISECONDS,
        SIDE1_ROW_BIT, SIDE2_ROW_BIT,
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
        ReadPtt,
    }

    /// Reads are ordered unselected pass first, then the four selected columns.
    struct TraceBus {
        trace: Vec<Operation>,
        rows: [u8; 5],
        read_index: usize,
        ptt_pressed: bool,
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

        fn read_ptt_pressed(&mut self) -> Result<bool, Self::Error> {
            self.record(Operation::ReadPtt)?;
            Ok(self.ptt_pressed)
        }
    }

    impl TraceBus {
        fn with(rows: [u8; 5], ptt_pressed: bool, fail_at: Option<usize>) -> Self {
            Self {
                trace: Vec::new(),
                rows,
                read_index: 0,
                ptt_pressed,
                fail_at,
            }
        }
    }

    /// Builds a scan whose only active bits are selected-column rows.
    fn columns_scan(columns: [u8; 4]) -> KeypadScan {
        KeypadScan {
            unselected: 0,
            columns,
            ptt_pressed: false,
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
    fn raw_gpio_latch_ignores_columns_and_returns_other_changes_once() {
        let baseline = [0xF078, 0xF078, 0xF078, 0xF078];
        let mut latch = RawGpioLatch::new();
        latch.observe(baseline);
        latch.observe([0xF038, 0xF058, 0xF068, 0xF070]);
        assert_eq!(latch.take_or(baseline), (baseline, false));

        let changed = [0x7078, 0xF078, 0xF078, 0xF078];
        latch.observe(changed);
        assert_eq!(latch.take_or(baseline), (changed, true));
        assert_eq!(latch.take_or(baseline), (baseline, false));
    }

    #[test]
    fn scan_reads_unselected_pass_first_then_each_column_then_ptt() {
        let mut bus = TraceBus::with([3, 1, 2, 4, 8], true, None);
        assert_eq!(
            scan(&mut bus),
            Ok(KeypadScan {
                unselected: 3,
                columns: [1, 2, 4, 8],
                ptt_pressed: true,
            })
        );
        assert_eq!(
            bus.trace,
            [
                Operation::AllHigh,
                Operation::Read,
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
                Operation::ReadPtt,
            ]
        );
    }

    #[test]
    fn every_select_or_read_failure_attempts_idle_cleanup() {
        for fail_at in [1, 2, 3, 5, 6, 8, 9, 11, 12, 14] {
            let mut bus = TraceBus::with([0; 5], false, Some(fail_at));
            assert_eq!(scan(&mut bus), Err(ScanError::Operation(fail_at)));
            assert_eq!(bus.trace.last(), Some(&Operation::AllHigh));
        }
    }

    #[test]
    fn idle_cleanup_failure_is_reported() {
        for fail_at in [0, 4, 7, 10, 13] {
            let mut bus = TraceBus::with([0; 5], false, Some(fail_at));
            assert_eq!(scan(&mut bus), Err(ScanError::Cleanup(fail_at)));
            assert_eq!(bus.trace.last(), Some(&Operation::AllHigh));
        }
    }

    #[test]
    fn all_sixteen_cells_decode_exactly() {
        assert_eq!(decode(columns_scan([0; 4])), Ok(None));
        for (column, keys) in EXPECTED.iter().enumerate() {
            for (row, expected) in keys.iter().enumerate() {
                let mut sample = [0_u8; 4];
                sample[column] = 1 << row;
                assert_eq!(decode(columns_scan(sample)), Ok(Some(*expected)));
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
                assert_eq!(decode(columns_scan(sample)), Err(DecodeError::Ambiguous));
            }
        }
    }

    #[test]
    fn bits_outside_evidenced_rows_are_invalid() {
        for column in 0..4 {
            for bit in 4..8 {
                let mut sample = [0_u8; 4];
                sample[column] = 1 << bit;
                assert_eq!(
                    decode(columns_scan(sample)),
                    Err(DecodeError::InvalidRowBits)
                );
            }
        }
        for bit in 4..8 {
            let sample = KeypadScan {
                unselected: 1 << bit,
                ..columns_scan([0; 4])
            };
            assert_eq!(decode(sample), Err(DecodeError::InvalidRowBits));
        }
    }

    #[test]
    fn side_keys_decode_from_the_unselected_pass() {
        for (bit, expected) in [(SIDE1_ROW_BIT, Key::Side1), (SIDE2_ROW_BIT, Key::Side2)] {
            let sample = KeypadScan {
                unselected: bit,
                ..columns_scan([0; 4])
            };
            assert_eq!(decode(sample), Ok(Some(expected)));
        }
    }

    /// A side key grounds its row, so the physical scan reports that row low in
    /// every column pass too. This is the exact shape the unit produces.
    #[test]
    fn a_held_side_key_reads_low_in_every_pass_and_still_decodes() {
        for (bit, expected) in [(SIDE1_ROW_BIT, Key::Side1), (SIDE2_ROW_BIT, Key::Side2)] {
            let sample = KeypadScan {
                unselected: bit,
                columns: [bit; 4],
                ptt_pressed: false,
            };
            assert_eq!(decode(sample), Ok(Some(expected)));
        }
    }

    /// A side key held with a main key keeps both bits distinguishable: the
    /// grounded row is removed from every column, leaving the real matrix cell.
    #[test]
    fn a_held_side_key_does_not_mask_a_simultaneous_main_key() {
        // SIDE1 grounds PB15 while MENU is pressed at column 0, row 0. MENU
        // shares that row, so only the remaining columns stay clean.
        let sample = KeypadScan {
            unselected: SIDE1_ROW_BIT,
            columns: [
                SIDE1_ROW_BIT | 0b0010,
                SIDE1_ROW_BIT,
                SIDE1_ROW_BIT,
                SIDE1_ROW_BIT,
            ],
            ptt_pressed: false,
        };
        assert_eq!(decode(sample), Err(DecodeError::Ambiguous));
    }

    #[test]
    fn undefined_unselected_rows_fail_closed() {
        for bit in [0x04, 0x08] {
            let sample = KeypadScan {
                unselected: bit,
                ..columns_scan([0; 4])
            };
            assert_eq!(decode(sample), Err(DecodeError::UndefinedUnselectedRow));
        }
    }

    #[test]
    fn both_side_keys_together_are_ambiguous() {
        let sample = KeypadScan {
            unselected: SIDE1_ROW_BIT | SIDE2_ROW_BIT,
            ..columns_scan([0; 4])
        };
        assert_eq!(decode(sample), Err(DecodeError::Ambiguous));
    }

    #[test]
    fn ptt_decodes_only_when_no_other_key_is_active() {
        let released = KeypadScan {
            ptt_pressed: true,
            ..columns_scan([0; 4])
        };
        assert_eq!(decode(released), Ok(Some(Key::Ptt)));

        // A held key wins over PTT, matching the pinned source's precedence.
        let with_menu = KeypadScan {
            ptt_pressed: true,
            ..columns_scan([1, 0, 0, 0])
        };
        assert_eq!(decode(with_menu), Ok(Some(Key::Menu)));

        let with_side = KeypadScan {
            unselected: SIDE1_ROW_BIT,
            ptt_pressed: true,
            columns: [0; 4],
        };
        assert_eq!(decode(with_side), Ok(Some(Key::Side1)));
    }

    #[test]
    fn every_key_label_is_distinct_and_renderable() {
        let keys = [
            Key::Side1,
            Key::Side2,
            Key::Ptt,
            Key::Menu,
            Key::Up,
            Key::Down,
            Key::Exit,
            Key::Digit0,
            Key::Digit1,
            Key::Digit2,
            Key::Digit3,
            Key::Digit4,
            Key::Digit5,
            Key::Digit6,
            Key::Digit7,
            Key::Digit8,
            Key::Digit9,
            Key::Star,
            Key::Function,
        ];
        let mut seen: Vec<&[u8]> = Vec::new();
        for key in keys {
            let label = key.label();
            assert!(!label.is_empty() && label.len() <= 5);
            assert!(!seen.contains(&label), "duplicate label");
            seen.push(label);
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
