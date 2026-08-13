//! Bounded UV-K5 V1 main-keypad matrix adapter.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_platform::receive_app::Key as SharedKey;

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
    /// Upper side key.
    Side1,
    /// Lower side key.
    Side2,
    /// Push-to-talk switch, exposed only as an input label in this image.
    Ptt,
}

impl Key {
    /// Converts a decoded physical key to the shared application vocabulary.
    #[must_use]
    pub const fn shared(self) -> SharedKey {
        match self {
            Self::Menu => SharedKey::Menu,
            Self::Up => SharedKey::Up,
            Self::Down => SharedKey::Down,
            Self::Exit => SharedKey::Exit,
            Self::Star => SharedKey::Star,
            Self::Function => SharedKey::Function,
            Self::Digit(value) => SharedKey::Digit(value),
            Self::Side1 => SharedKey::Side1,
            Self::Side2 => SharedKey::Side2,
            Self::Ptt => SharedKey::Ptt,
        }
    }
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
    /// Reports the separately wired active-low PTT input.
    fn ptt_pressed(&mut self) -> bool;
}

/// Scans once, accepting only one stable key and rejecting noise/multiple keys.
pub fn scan<I: MatrixIo>(io: &mut I) -> Option<Key> {
    let mut result = None;
    // With every row high, PA3 and PA4 are the two directly wired side keys.
    io.select_row(ROWS.len());
    let Some(unselected) = stable_columns(io) else {
        io.restore_shared_pins();
        return None;
    };
    match !unselected & 0x0f {
        0 => {}
        1 => {
            io.restore_shared_pins();
            return Some(Key::Side1);
        }
        2 => {
            io.restore_shared_pins();
            return Some(Key::Side2);
        }
        _ => {
            io.restore_shared_pins();
            return None;
        }
    }
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
    if io.ptt_pressed() {
        result = if result.is_none() {
            Some(Key::Ptt)
        } else {
            None
        };
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
        gpio::set_input(Port::C, 5);
        portcon::select_gpio(Port::C, 5);
        portcon::enable_input(Port::C, 5);
        portcon::enable_pull_up(Port::C, 5);
        Self
    }
}

impl MatrixIo for K5Matrix {
    fn select_row(&mut self, row: usize) {
        for pin in ROWS {
            gpio::write_pin(Port::A, pin, true);
        }
        if row < ROWS.len() {
            gpio::write_pin(Port::A, ROWS[row], false);
        }
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
    fn ptt_pressed(&mut self) -> bool {
        !gpio::read_pin(Port::C, 5)
    }
}

#[cfg(test)]
mod tests {
    use super::{scan, Key, MatrixIo};
    use radio_platform::receive_app::{Event, Key as SharedKey, ReceiveApp};
    struct Fake {
        held: Option<(usize, usize)>,
        side: Option<Key>,
        ptt: bool,
        row: usize,
        restored: bool,
    }
    impl MatrixIo for Fake {
        fn select_row(&mut self, row: usize) {
            self.row = row;
        }
        fn columns(&mut self) -> u8 {
            if self.row == 4 {
                return match self.side {
                    Some(Key::Side1) => 0x0e,
                    Some(Key::Side2) => 0x0d,
                    _ => 0x0f,
                };
            }
            self.held
                .filter(|(r, _)| *r == self.row)
                .map_or(0x0f, |(_, c)| 0x0f & !(1 << c))
        }
        fn settle(&mut self) {}
        fn restore_shared_pins(&mut self) {
            self.restored = true;
        }
        fn ptt_pressed(&mut self) -> bool {
            self.ptt
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
                side: None,
                ptt: false,
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
            side: None,
            ptt: false,
            row: 0,
            restored: false,
        };
        assert_eq!(scan(&mut io), None);
        assert!(io.restored);
    }

    #[test]
    fn side_keys_and_ptt_decode_but_combinations_fail_closed() {
        for key in [Key::Side1, Key::Side2] {
            let mut io = Fake {
                held: None,
                side: Some(key),
                ptt: false,
                row: 0,
                restored: false,
            };
            assert_eq!(scan(&mut io), Some(key));
            assert!(io.restored);
        }
        let mut io = Fake {
            held: None,
            side: None,
            ptt: true,
            row: 0,
            restored: false,
        };
        assert_eq!(scan(&mut io), Some(Key::Ptt));
        let mut io = Fake {
            held: Some((0, 0)),
            side: None,
            ptt: true,
            row: 0,
            restored: false,
        };
        assert_eq!(scan(&mut io), None);
    }

    #[test]
    fn shared_key_mapping_matches_receive_application_meaning() {
        let mut app = ReceiveApp::new();
        for (key, expected) in [
            (Key::Up, SharedKey::Up),
            (Key::Menu, SharedKey::Menu),
            (Key::Down, SharedKey::Down),
            (Key::Digit(4), SharedKey::Digit(4)),
            (Key::Side1, SharedKey::Side1),
        ] {
            assert_eq!(key.shared(), expected);
            let _ = app.apply(Event::KeyPress(key.shared()));
        }
    }
}
