//! V1 board driver for the ST7565-compatible display.

use radio_app::drivers::Display;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::pwm_plus::PwmPlus;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::PWM_PLUS0_BASE;
use radio_platform::operator_display::{COLUMN_OFFSET, FRAME_BYTES, PAGES, WIDTH};
use radio_platform::transmit::Delay;

const BACKLIGHT: PwmPlus = PwmPlus::new(PWM_PLUS0_BASE);
const SELECT_PIN: u8 = 7;
const CLOCK_PIN: u8 = 8;
const A0_PIN: u8 = 9;
const DATA_PIN: u8 = 10;
const RESET_PIN: u8 = 11;

/// The V1 controller exposes four RAM columns before the 128 visible columns.
/// The existing direct-text path already used this offset; shared framebuffer
/// writes must use it as well or the frame is cropped on the left and wraps
/// stale RAM into the right edge.
pub const DISPLAY_COLUMN_OFFSET: u8 = COLUMN_OFFSET;
/// Number of visible columns in one controller page.
#[allow(clippy::cast_possible_truncation)]
pub const DISPLAY_VISIBLE_COLUMNS: u8 = WIDTH as u8;
/// Number of visible pages in the 128x64 panel.
#[allow(clippy::cast_possible_truncation)]
pub const DISPLAY_PAGES: u8 = PAGES as u8;

/// One bounded page transfer from the shared framebuffer to controller RAM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisiblePageWrite {
    /// Controller page selected before the data bytes.
    pub page: u8,
    /// Controller RAM column corresponding to visible framebuffer column zero.
    pub controller_column: u8,
    /// Number of visible framebuffer bytes written on this page.
    pub bytes: u8,
}

impl VisiblePageWrite {
    const fn new(page: u8) -> Self {
        Self {
            page,
            controller_column: DISPLAY_COLUMN_OFFSET,
            bytes: DISPLAY_VISIBLE_COLUMNS,
        }
    }
}

/// Returns the exact bounded transfer plan for the visible framebuffer.
pub const fn visible_page_write_plan() -> [VisiblePageWrite; DISPLAY_PAGES as usize] {
    [
        VisiblePageWrite::new(0),
        VisiblePageWrite::new(1),
        VisiblePageWrite::new(2),
        VisiblePageWrite::new(3),
        VisiblePageWrite::new(4),
        VisiblePageWrite::new(5),
        VisiblePageWrite::new(6),
        VisiblePageWrite::new(7),
    ]
}

/// Maps a visible framebuffer column to its controller RAM column.
pub const fn controller_column_for_visible(visible_column: u8) -> Option<u8> {
    if visible_column < DISPLAY_VISIBLE_COLUMNS {
        Some(DISPLAY_COLUMN_OFFSET + visible_column)
    } else {
        None
    }
}

/// Encodes the three controller address commands for one page transfer.
pub const fn page_address_commands(page: u8, controller_column: u8) -> [u8; 3] {
    [
        0xB0 | (page & 7),
        0x10 | ((controller_column >> 4) & 0x0F),
        controller_column & 0x0F,
    ]
}

/// The display on its bit-banged SPI pins. Its controller has no return
/// path, so every operation reports success.
pub struct K5Display<D> {
    delay: D,
}

impl<D: Delay> K5Display<D> {
    /// Takes the display; nothing is driven until [`Display::initialise`].
    pub const fn new(delay: D) -> Self {
        Self { delay }
    }
}

impl<D: Delay> Display for K5Display<D> {
    /// Configures only the evidenced display pins and switches it on.
    ///
    /// The waits are the reference firmware's (`driver/st7565.c`), in real
    /// milliseconds: spin-loop counts stood in for them until the size-built
    /// image made the same counts take about five seconds.
    async fn initialise(&mut self) -> bool {
        let delay = &mut self.delay;
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

        delay.delay_ms(1);
        gpio::write_pin(Port::B, RESET_PIN, false);
        delay.delay_ms(20);
        gpio::write_pin(Port::B, RESET_PIN, true);
        delay.delay_ms(120);

        select_display(true);
        for command in [0xE2, 0xA2, 0xC0, 0xA1, 0xA6, 0xA4, 0x24, 0x81, 31] {
            command_byte(command);
        }
        command_byte(0x2B);
        delay.delay_ms(1);
        command_byte(0x2E);
        delay.delay_ms(1);
        for _ in 0..4 {
            command_byte(0x2F);
        }
        delay.delay_ms(40);
        command_byte(0x40);
        command_byte(0xAF);
        select_display(false);
        true
    }

    async fn frame(&mut self, frame: &[u8; FRAME_BYTES]) -> bool {
        write_frame(frame);
        true
    }
}

fn write_frame(frame: &[u8; FRAME_BYTES]) {
    select_display(true);
    for transfer in visible_page_write_plan() {
        select(transfer.page, transfer.controller_column);
        set_data_mode();
        let start = usize::from(transfer.page) * usize::from(DISPLAY_VISIBLE_COLUMNS);
        let end = start + usize::from(transfer.bytes);
        for byte in &frame[start..end] {
            write_byte(*byte);
        }
    }
    select_display(false);
}

fn select(page: u8, column: u8) {
    gpio::write_pin(Port::B, A0_PIN, false);
    for command in page_address_commands(page, column) {
        write_byte(command);
    }
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

#[cfg(test)]
mod tests {
    use super::{
        controller_column_for_visible, page_address_commands, visible_page_write_plan,
        DISPLAY_COLUMN_OFFSET, DISPLAY_VISIBLE_COLUMNS,
    };

    #[test]
    fn the_page_plan_covers_the_whole_panel_without_edge_wrap() {
        let plan = visible_page_write_plan();
        for (page, transfer) in plan.iter().enumerate() {
            assert_eq!(usize::from(transfer.page), page);
            assert_eq!(transfer.controller_column, DISPLAY_COLUMN_OFFSET);
            assert_eq!(transfer.bytes, DISPLAY_VISIBLE_COLUMNS);
        }
        assert_eq!(
            controller_column_for_visible(0),
            Some(DISPLAY_COLUMN_OFFSET)
        );
        assert_eq!(
            controller_column_for_visible(DISPLAY_VISIBLE_COLUMNS - 1),
            Some(DISPLAY_COLUMN_OFFSET + DISPLAY_VISIBLE_COLUMNS - 1)
        );
        assert_eq!(controller_column_for_visible(DISPLAY_VISIBLE_COLUMNS), None);
        assert_eq!(page_address_commands(3, 0x84), [0xB3, 0x18, 0x04]);
    }
}
