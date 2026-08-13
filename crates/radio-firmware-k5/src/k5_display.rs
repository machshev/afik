//! V1 board adapter for the ST7565-compatible boot display.

use radio_bk4819::ReceiveMetrics;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::pwm_plus::PwmPlus;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::PWM_PLUS0_BASE;
use radio_platform::display::{BootDisplay, BootStage, ReceiveDiagnostic};
use radio_platform::receive_app::View;

use crate::keypad::Key;

const BACKLIGHT: PwmPlus = PwmPlus::new(PWM_PLUS0_BASE);
const SELECT_PIN: u8 = 7;
const CLOCK_PIN: u8 = 8;
const A0_PIN: u8 = 9;
const DATA_PIN: u8 = 10;
const RESET_PIN: u8 = 11;

/// Infallible fixed display adapter; its controller has no return path.
pub struct K5BootDisplay;

impl K5BootDisplay {
    /// Configures only the evidenced display pins and SPI0 controller.
    pub fn initialise() -> Self {
        syscon::enable(&[Peripheral::GpioB, Peripheral::PwmPlus0]);
        for pin in [SELECT_PIN, CLOCK_PIN, A0_PIN, DATA_PIN, RESET_PIN] {
            gpio::set_output(Port::B, pin);
            portcon::select_gpio(Port::B, pin);
        }
        portcon::select_k5_backlight_pwm();
        gpio::write_pin(Port::B, SELECT_PIN, true);
        gpio::write_pin(Port::B, CLOCK_PIN, true);
        gpio::write_pin(Port::B, A0_PIN, false);
        gpio::write_pin(Port::B, RESET_PIN, true);
        BACKLIGHT.enable_diagnostic_backlight();

        delay(50_000);
        gpio::write_pin(Port::B, RESET_PIN, false);
        delay(1_000_000);
        gpio::write_pin(Port::B, RESET_PIN, true);
        delay(6_000_000);

        select_display(true);
        for command in [0xE2, 0xA2, 0xC0, 0xA1, 0xA6, 0xA4, 0x24, 0x81, 31] {
            command_byte(command);
        }
        command_byte(0x2B);
        delay(50_000);
        command_byte(0x2E);
        delay(50_000);
        for _ in 0..4 {
            command_byte(0x2F);
        }
        delay(2_000_000);
        command_byte(0x40);
        command_byte(0xAF);
        select_display(false);
        Self
    }

    /// Shows the read-only K5 operator-hardware validation state.
    pub fn show_validation(&mut self, eeprom_sum: Option<u16>, bk_register: u16, key: Option<Key>) {
        let mut eeprom = *b"----";
        if let Some(value) = eeprom_sum {
            hexadecimal_text(value.into(), &mut eeprom);
        }
        let mut bk = *b"0000";
        hexadecimal_text(bk_register.into(), &mut bk);
        let key = key_label(key);
        select_display(true);
        clear();
        draw_text(0, 8, b"AFIK K5 1.5E");
        draw_text(2, 8, b"EEP");
        draw_text(2, 38, &eeprom);
        draw_text(4, 8, b"BK");
        draw_text(4, 38, &bk);
        draw_text(6, 8, b"KEY");
        draw_text(6, 38, &key);
        select_display(false);
    }

    /// Shows the muted PMR446 receive-validation state and raw chip metrics.
    pub fn show_pmr_receive(
        &mut self,
        view: View,
        configured: Option<u16>,
        metrics: Option<ReceiveMetrics>,
    ) {
        let mut channel_text = *b"00";
        decimal_text(u32::from(view.channel), &mut channel_text);
        let mut configured_text = *b"----";
        if let Some(value) = configured {
            hexadecimal_text(value.into(), &mut configured_text);
        }
        let mut rssi = *b"-----";
        let mut glitch = *b"---";
        let mut noise = *b"---";
        let squelch = if view.receiver_ok {
            if let Some(sample) = metrics {
                signed_decimal_text(sample.rssi_dbm_x2, &mut rssi);
                decimal_text(u32::from(sample.glitch), &mut glitch);
                decimal_text(u32::from(sample.noise), &mut noise);
            }
            if view.squelch_open {
                b'1'
            } else {
                b'0'
            }
        } else {
            b'-'
        };

        select_display(true);
        clear();
        draw_text(0, 8, b"AFIK K5 1.8U");
        draw_text(2, 8, b"PMR");
        draw_text(2, 32, &channel_text);
        draw_text(2, 50, b"CFG");
        draw_text(2, 74, &configured_text);
        draw_text(2, 104, if view.audio { b"A1" } else { b"A0" });
        draw_text(4, 8, b"R2");
        draw_text(4, 26, &rssi);
        draw_text(4, 62, b"S");
        draw_text(4, 74, &[squelch]);
        draw_text(6, 8, b"G");
        draw_text(6, 20, &glitch);
        draw_text(6, 50, b"N");
        draw_text(6, 62, &noise);
        select_display(false);
    }
}

fn key_label(key: Option<Key>) -> [u8; 4] {
    match key {
        None => *b"----",
        Some(Key::Menu) => *b"MENU",
        Some(Key::Up) => *b"UP  ",
        Some(Key::Down) => *b"DOWN",
        Some(Key::Exit) => *b"EXIT",
        Some(Key::Star) => *b"STAR",
        Some(Key::Function) => *b"F   ",
        Some(Key::Digit(value)) => [b'0' + value, b' ', b' ', b' '],
        Some(Key::Side1) => *b"S1  ",
        Some(Key::Side2) => *b"S2  ",
        Some(Key::Ptt) => *b"PTT ",
    }
}

impl BootDisplay for K5BootDisplay {
    type Error = core::convert::Infallible;

    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
        let stage_text: &[u8] = match stage {
            BootStage::Reset => b"RESET",
            BootStage::BoardReady => b"BOARD",
            BootStage::SerialReady => b"SERIAL",
        };
        select_display(true);
        clear();
        draw_text(1, 20, b"AFIK");
        draw_text(3, 26, b"K5");
        draw_text(5, centered(stage_text), stage_text);
        select_display(false);
        Ok(())
    }

    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
        let mut count = *b"000000";
        decimal_text(diagnostic.bytes, &mut count);
        let mut status = *b"00000000";
        hexadecimal_text(diagnostic.status, &mut status);
        select_display(true);
        clear();
        draw_text(1, 20, b"AFIK");
        draw_text(3, 10, b"RX");
        draw_text(3, 34, &count);
        draw_text(5, 10, b"ERR");
        draw_text(5, 34, &status);
        select_display(false);
        Ok(())
    }
}

fn clear() {
    for page in 0..8 {
        select(page, 0);
        set_data_mode();
        for _ in 0..128 {
            write_byte(0);
        }
    }
}

fn draw_text(page: u8, column: u8, text: &[u8]) {
    select(page, column + 4);
    set_data_mode();
    for character in text {
        for byte in glyph(*character) {
            write_byte(byte);
        }
        write_byte(0);
    }
}

fn select(page: u8, column: u8) {
    gpio::write_pin(Port::B, A0_PIN, false);
    write_byte(0xB0 | (page & 7));
    write_byte(0x10 | ((column >> 4) & 0x0F));
    write_byte(column & 0x0F);
}

fn command_byte(byte: u8) {
    gpio::write_pin(Port::B, A0_PIN, false);
    write_byte(byte);
}

fn set_data_mode() {
    gpio::write_pin(Port::B, A0_PIN, true);
}

fn select_display(selected: bool) {
    gpio::write_pin(Port::B, SELECT_PIN, !selected);
}

fn write_byte(byte: u8) {
    for bit in (0..8).rev() {
        gpio::write_pin(Port::B, DATA_PIN, byte & (1 << bit) != 0);
        gpio::write_pin(Port::B, CLOCK_PIN, false);
        core::hint::spin_loop();
        gpio::write_pin(Port::B, CLOCK_PIN, true);
        core::hint::spin_loop();
    }
}

fn centered(text: &[u8]) -> u8 {
    u8::try_from((128_usize.saturating_sub(text.len() * 6)) / 2).unwrap_or(0)
}

fn glyph(character: u8) -> [u8; 5] {
    match character {
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        b'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        b'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        b'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        b'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        b'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        b'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        b'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        b'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        b'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        b'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        b'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        b'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        b'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        b'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        b'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        b'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        b'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        b'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        b'Y' => [0x03, 0x04, 0x78, 0x04, 0x03],
        b'-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        b'5' => [0x4F, 0x49, 0x49, 0x49, 0x31],
        b'0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        b'1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        b'2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        b'3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        b'4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        b'6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        b'7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        b'8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        b'9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        _ => [0; 5],
    }
}

fn decimal_text(mut value: u32, text: &mut [u8]) {
    for character in text.iter_mut().rev() {
        *character = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
    }
}

fn signed_decimal_text(value: i16, text: &mut [u8]) {
    text.fill(b' ');
    let negative = value.is_negative();
    let mut magnitude = u32::from(value.unsigned_abs());
    let mut index = text.len();
    loop {
        index -= 1;
        text[index] = b'0' + u8::try_from(magnitude % 10).unwrap_or(0);
        magnitude /= 10;
        if magnitude == 0 || index == 0 {
            break;
        }
    }
    if negative && index > 0 {
        text[index - 1] = b'-';
    }
}

fn hexadecimal_text(mut value: u32, text: &mut [u8]) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    for character in text.iter_mut().rev() {
        *character = DIGITS[usize::try_from(value & 0xF).unwrap_or(0)];
        value >>= 4;
    }
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}
