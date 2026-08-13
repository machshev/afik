//! V1 board adapter for the ST7565-compatible boot display.

use radio_bk4819::ReceiveMetrics;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::pwm_plus::PwmPlus;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::PWM_PLUS0_BASE;
use radio_platform::display::{BootDisplay, BootStage, ReceiveDiagnostic};
use radio_platform::operator_display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    ListRow, MemoryState, OperatingView, ResetCause, SelectorRow, SerialCounters, COLUMN_OFFSET,
    FRAME_BYTES, LIST_ROWS, PAGES, WIDTH,
};
use radio_platform::operator_shell::{Screen, Shell, Source, SETTINGS, SQUELCH_LEVELS};
use radio_platform::receive_app::View;

use crate::keypad::Key;

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

/// Infallible fixed display adapter; its controller has no return path.
pub struct K5BootDisplay {
    previous_frame: [u8; FRAME_BYTES],
    has_previous_frame: bool,
}

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
        Self {
            previous_frame: [0; FRAME_BYTES],
            has_previous_frame: false,
        }
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
        draw_text(0, 8, Self::IMAGE_IDENTITY);
        draw_text(2, 8, b"EEP");
        draw_text(2, 38, &eeprom);
        draw_text(4, 8, b"BK");
        draw_text(4, 38, &bk);
        draw_text(6, 8, b"KEY");
        draw_text(6, 38, &key);
        select_display(false);
        self.has_previous_frame = false;
    }

    /// Printable identity shown by the shared Info screen and normal hello.
    pub const IMAGE_IDENTITY: &'static [u8] = crate::protocol::APPLICATION_VERSION;

    /// Renders a shared K1-layout operator frame through the K5 transport.
    pub fn show_operator(
        &mut self,
        shell: &Shell,
        view: View,
        metrics: Option<ReceiveMetrics>,
    ) -> Result<(), core::convert::Infallible> {
        let frame = render_operator_frame(shell, view, metrics);
        if !self.has_previous_frame || self.previous_frame != frame {
            write_frame(&frame);
            self.previous_frame = frame;
            self.has_previous_frame = true;
        }
        Ok(())
    }
}

/// Builds the exact K5 operator frame before target transport writes it.
#[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
pub fn render_operator_frame(
    shell: &Shell,
    view: View,
    metrics: Option<ReceiveMetrics>,
) -> [u8; FRAME_BYTES] {
    let mut frame = [0_u8; FRAME_BYTES];
    let channel = radio_platform::configuration::Pmr446Channel::new(view.channel)
        .unwrap_or(radio_platform::configuration::Pmr446Channel::FIRST);
    let name = channel.display_name();
    let rssi_raw = metrics.map_or(0, |sample| {
        u16::try_from(sample.rssi_dbm_x2.saturating_add(320)).unwrap_or(0)
    });
    match shell.screen() {
        Screen::Operating => {
            render_operating_screen(
                &mut frame,
                &OperatingView {
                    position: u16::from(view.channel),
                    total: u16::from(radio_platform::configuration::PMR446_CHANNELS),
                    name: &name,
                    frequency_hz: channel.frequency_hz(),
                    rssi_raw,
                    squelch_open: view.squelch_open,
                    receiver_ok: view.receiver_ok,
                    battery_percent: None,
                    monitoring: view.monitoring,
                    scanning: false,
                    bank: None,
                    entry: shell.entry(),
                    vfo_step_hz: None,
                },
            );
        }
        Screen::ChannelList => {
            let mut rows = [ListRow::default(); LIST_ROWS];
            let first = shell.cursor() / u16::try_from(LIST_ROWS).unwrap_or(1)
                * u16::try_from(LIST_ROWS).unwrap_or(1);
            let mut count = 0;
            for (offset, row) in rows.iter_mut().enumerate() {
                let position = first + u16::try_from(offset).unwrap_or(0);
                if position >= u16::from(radio_platform::configuration::PMR446_CHANNELS) {
                    break;
                }
                let channel = radio_platform::configuration::Pmr446Channel::new(
                    u8::try_from(position + 1).unwrap_or(1),
                )
                .expect("bounded PMR channel");
                let row_name = channel.display_name();
                *row = ListRow::new(
                    position + 1,
                    &row_name,
                    position + 1 == u16::from(view.channel),
                );
                count += 1;
            }
            render_channel_list(
                &mut frame,
                &rows[..count],
                usize::from(shell.cursor() - first),
                u16::from(radio_platform::configuration::PMR446_CHANNELS),
            );
        }
        Screen::SourceList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let first = shell.source_cursor() / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for (offset, row_slot) in rows.iter_mut().enumerate() {
                let row = first + offset;
                let Some(source) = shell.source_at(row) else {
                    break;
                };
                *row_slot = match source {
                    Source::Vfo => SelectorRow::text(b"VFO", shell.is_active_source(row)),
                    Source::AllChannels => {
                        SelectorRow::text(b"ALL CHANNELS", shell.is_active_source(row))
                    }
                    Source::Bank(bank) => {
                        SelectorRow::bank(bank.get(), b"", shell.is_active_source(row))
                    }
                };
                count += 1;
            }
            render_selector_list(
                &mut frame,
                b"SOURCE",
                &rows[..count],
                shell.source_cursor() - first,
            );
        }
        Screen::Info => {
            render_info_screen(
                &mut frame,
                K5BootDisplay::IMAGE_IDENTITY,
                0,
                u16::from(radio_platform::configuration::PMR446_CHANNELS),
                false,
                MemoryState::Unknown,
                SerialCounters::default(),
                0,
                ResetCause::default(),
            );
        }
        Screen::Settings => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let mut count = 0;
            for (offset, setting) in SETTINGS.iter().take(LIST_ROWS).enumerate() {
                rows[offset] = match setting {
                    radio_platform::operator_shell::Setting::Squelch => {
                        SelectorRow::squelch_setting(shell.squelch().get())
                    }
                };
                count += 1;
            }
            render_selector_list(
                &mut frame,
                b"SETTINGS",
                &rows[..count],
                shell.settings_cursor(),
            );
        }
        Screen::SquelchList => {
            let mut rows = [SelectorRow::default(); LIST_ROWS];
            let cursor = usize::from(shell.squelch_cursor());
            let first = cursor / LIST_ROWS * LIST_ROWS;
            let mut count = 0;
            for (offset, row) in rows.iter_mut().enumerate() {
                let Ok(level) = u8::try_from(first + offset) else {
                    break;
                };
                if level >= SQUELCH_LEVELS {
                    break;
                }
                *row = SelectorRow::squelch_level(level, level == shell.squelch().get());
                count += 1;
            }
            render_selector_list(&mut frame, b"SQUELCH", &rows[..count], cursor - first);
        }
        Screen::StepList => render_selector_list(&mut frame, b"NO CAP", &[], 0),
    }
    frame
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
        self.has_previous_frame = false;
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
        self.has_previous_frame = false;
        Ok(())
    }
}

fn clear() {
    for transfer in visible_page_write_plan() {
        select(transfer.page, transfer.controller_column);
        set_data_mode();
        for _ in 0..DISPLAY_VISIBLE_COLUMNS {
            write_byte(0);
        }
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

fn draw_text(page: u8, column: u8, text: &[u8]) {
    select(page, column.saturating_add(DISPLAY_COLUMN_OFFSET));
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
        b'>' => [0x00, 0x41, 0x22, 0x14, 0x08],
        b'*' => [0x14, 0x08, 0x3E, 0x08, 0x14],
        _ => [0; 5],
    }
}

fn decimal_text(mut value: u32, text: &mut [u8]) {
    for character in text.iter_mut().rev() {
        *character = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
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
