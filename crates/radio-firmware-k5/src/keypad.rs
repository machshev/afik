//! Bounded UV-K5 V1 main-keypad matrix adapter.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;

const COLUMNS: [u8; 4] = [3, 4, 5, 6];
const ROWS: [u8; 4] = [10, 11, 12, 13];

/// One physical key on the K5's four-by-four main matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    /// Menu/confirm key.
    Menu,
    /// Up key.
    Up,
    /// Down key.
    Down,
    /// Exit/back key.
    Exit,
    /// Star key.
    Star,
    /// Function/hash key.
    Function,
    /// Decimal digit.
    Digit(u8),
}

const MATRIX: [[Key; 4]; 4] = [
    [Key::Menu, Key::Digit(1), Key::Digit(4), Key::Digit(7)],
    [Key::Up, Key::Digit(2), Key::Digit(5), Key::Digit(8)],
    [Key::Down, Key::Digit(3), Key::Digit(6), Key::Digit(9)],
    [Key::Exit, Key::Star, Key::Digit(0), Key::Function],
];

/// Electrical operations needed by the hardware-independent scan algorithm.
pub trait MatrixIo {
    /// Drives every row high, then drives only `row` low.
    fn select_row(&mut self, row: usize);
    /// Returns column levels in bits zero through three; low is held.
    fn columns(&mut self) -> u8;
    /// Provides the settling interval used between samples.
    fn settle(&mut self);
    /// Restores EEPROM/voice shared pins to their evidenced idle state.
    fn restore_shared_pins(&mut self);
}

/// Scans once, accepting only one stable key and rejecting noise/multiple keys.
pub fn scan<I: MatrixIo>(io: &mut I) -> Option<Key> {
    let mut result = None;
    for (row, keys) in MATRIX.iter().enumerate() {
        io.select_row(row);
        let Some(columns) = stable_columns(io) else {
            result = None;
            break;
        };
        let held = !columns & 0x0f;
        if held == 0 {
            continue;
        }
        if held.count_ones() != 1 || result.is_some() {
            result = None;
            break;
        }
        result = Some(keys[held.trailing_zeros() as usize]);
    }
    io.restore_shared_pins();
    result
}

fn stable_columns<I: MatrixIo>(io: &mut I) -> Option<u8> {
    let mut previous = io.columns() & 0x0f;
    let mut matches = 0_u8;
    for _ in 0..8 {
        io.settle();
        let current = io.columns() & 0x0f;
        if current == previous {
            matches += 1;
            if matches == 3 {
                return Some(current);
            }
        } else {
            previous = current;
            matches = 0;
        }
    }
    None
}

/// Direct GPIO implementation for the evidenced V1 board binding.
pub struct K5Matrix;

impl K5Matrix {
    /// Configures keypad pins as GPIO with pulled-up input columns.
    pub fn initialise() -> Self {
        for pin in COLUMNS {
            gpio::set_input(Port::A, pin);
            portcon::select_gpio(Port::A, pin);
            portcon::enable_input(Port::A, pin);
            portcon::enable_pull_up(Port::A, pin);
        }
        for pin in ROWS {
            gpio::set_output(Port::A, pin);
            portcon::select_gpio(Port::A, pin);
            gpio::write_pin(Port::A, pin, true);
        }
        Self
    }
}

impl MatrixIo for K5Matrix {
    fn select_row(&mut self, row: usize) {
        for pin in ROWS {
            gpio::write_pin(Port::A, pin, true);
        }
        gpio::write_pin(Port::A, ROWS[row], false);
    }
    fn columns(&mut self) -> u8 {
        COLUMNS.iter().enumerate().fold(0, |value, (bit, pin)| {
            value | (u8::from(gpio::read_pin(Port::A, *pin)) << bit)
        })
    }
    fn settle(&mut self) {
        for _ in 0..48 {
            core::hint::spin_loop();
        }
    }
    fn restore_shared_pins(&mut self) {
        gpio::write_pin(Port::A, 10, true);
        gpio::write_pin(Port::A, 11, true);
        gpio::write_pin(Port::A, 12, false);
        gpio::write_pin(Port::A, 13, true);
    }
}

#[cfg(test)]
mod tests {
    use super::{scan, Key, MatrixIo};
    struct Fake {
        held: Option<(usize, usize)>,
        row: usize,
        restored: bool,
    }
    impl MatrixIo for Fake {
        fn select_row(&mut self, row: usize) {
            self.row = row;
        }
        fn columns(&mut self) -> u8 {
            self.held
                .filter(|(r, _)| *r == self.row)
                .map_or(0x0f, |(_, c)| 0x0f & !(1 << c))
        }
        fn settle(&mut self) {}
        fn restore_shared_pins(&mut self) {
            self.restored = true;
        }
    }
    #[test]
    fn maps_all_positions_and_restores_pins() {
        let expected = [
            Key::Menu,
            Key::Digit(1),
            Key::Digit(4),
            Key::Digit(7),
            Key::Up,
            Key::Digit(2),
            Key::Digit(5),
            Key::Digit(8),
            Key::Down,
            Key::Digit(3),
            Key::Digit(6),
            Key::Digit(9),
            Key::Exit,
            Key::Star,
            Key::Digit(0),
            Key::Function,
        ];
        for (index, key) in expected.into_iter().enumerate() {
            let mut io = Fake {
                held: Some((index / 4, index % 4)),
                row: 0,
                restored: false,
            };
            assert_eq!(scan(&mut io), Some(key));
            assert!(io.restored);
        }
    }
    #[test]
    fn no_key_restores_pins() {
        let mut io = Fake {
            held: None,
            row: 0,
            restored: false,
        };
        assert_eq!(scan(&mut io), None);
        assert!(io.restored);
    }
}
