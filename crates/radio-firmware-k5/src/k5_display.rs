//! V1 board adapter for the ST7565-compatible boot display.

use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::spi::Spi;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::SPI0_BASE;
use radio_firmware_k5::boot_display::{BootDisplay, BootStage};

const SPI0: Spi = Spi::new(SPI0_BASE);
const A0_PIN: u8 = 9;
const RESET_PIN: u8 = 11;

/// Infallible fixed display adapter; its controller has no return path.
pub struct K5BootDisplay;

impl K5BootDisplay {
    /// Configures only the evidenced display pins and SPI0 controller.
    pub fn initialise() -> Self {
        syscon::enable(&[Peripheral::GpioB, Peripheral::Spi0]);
        for pin in [7, 8, A0_PIN, 10, RESET_PIN] {
            gpio::set_output(Port::B, pin);
        }
        portcon::select_k5_display_spi0();
        portcon::select_gpio(Port::B, A0_PIN);
        portcon::select_gpio(Port::B, RESET_PIN);
        gpio::write_pin(Port::B, A0_PIN, false);
        gpio::write_pin(Port::B, RESET_PIN, true);
        SPI0.configure_display();

        delay(50_000);
        gpio::write_pin(Port::B, RESET_PIN, false);
        delay(1_000_000);
        gpio::write_pin(Port::B, RESET_PIN, true);
        delay(6_000_000);

        SPI0.select(true);
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
        SPI0.flush();
        SPI0.select(false);
        Self
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
        SPI0.select(true);
        clear();
        draw_text(1, 20, b"AFIK");
        draw_text(3, 26, b"K5");
        draw_text(5, centered(stage_text), stage_text);
        SPI0.flush();
        SPI0.select(false);
        Ok(())
    }
}

fn clear() {
    for page in 0..8 {
        select(page, 0);
        set_data_mode();
        for _ in 0..128 {
            SPI0.write_byte(0);
        }
    }
}

fn draw_text(page: u8, column: u8, text: &[u8]) {
    select(page, column + 4);
    set_data_mode();
    for character in text {
        for byte in glyph(*character) {
            SPI0.write_byte(byte);
        }
        SPI0.write_byte(0);
    }
}

fn select(page: u8, column: u8) {
    SPI0.flush();
    gpio::write_pin(Port::B, A0_PIN, false);
    SPI0.write_byte(0xB0 | (page & 7));
    SPI0.write_byte(0x10 | ((column >> 4) & 0x0F));
    SPI0.write_byte(column & 0x0F);
}

fn command_byte(byte: u8) {
    gpio::write_pin(Port::B, A0_PIN, false);
    SPI0.write_byte(byte);
}

fn set_data_mode() {
    SPI0.flush();
    gpio::write_pin(Port::B, A0_PIN, true);
}

fn centered(text: &[u8]) -> u8 {
    u8::try_from((128_usize.saturating_sub(text.len() * 6)) / 2).unwrap_or(0)
}

fn glyph(character: u8) -> [u8; 5] {
    match character {
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        b'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        b'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        b'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        b'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        b'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        b'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        b'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        b'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        b'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        b'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        b'5' => [0x4F, 0x49, 0x49, 0x49, 0x31],
        _ => [0; 5],
    }
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}
