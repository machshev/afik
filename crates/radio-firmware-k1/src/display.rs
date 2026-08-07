//! Hardware-independent ST7565-compatible display commands and fixed witness.

use crate::keypad::Key;

/// Visible display width in columns.
pub const WIDTH: usize = 128;
/// Visible display height in pixels.
pub const HEIGHT: usize = 64;
/// Number of eight-pixel controller pages.
pub const PAGES: usize = HEIGHT / 8;
/// Complete visible framebuffer size.
pub const FRAME_BYTES: usize = WIDTH * PAGES;
/// Maximum display data bytes written before yielding to cooperative tasks.
pub const ASYNC_WRITE_CHUNK_BYTES: usize = 16;

/// Controller column offset for the visible panel.
pub const COLUMN_OFFSET: u8 = 4;
/// Fixed source-backed controller setup commands after software reset.
pub const SETUP_COMMANDS: [u8; 8] = [
    0xA2, // 1/9 bias
    0xC0, // normal COM direction
    0xA1, // reverse SEG direction
    0xA6, // normal display
    0xA4, // normal RAM display
    0x24, // regulator ratio 5.0
    0x81, // electronic volume follows
    0x1F, // pinned fixed startup contrast
];

/// Whether bytes are controller commands or display RAM data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferKind {
    /// A0 low.
    Command,
    /// A0 high.
    Data,
}

/// Bounded display transport implemented by a board adapter.
pub trait DisplayBus {
    /// Adapter error.
    type Error;

    /// Writes one complete chip-selected transfer.
    fn write(&mut self, kind: TransferKind, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Waits for the controller power sequence without owning a target clock.
    fn delay_ms(&mut self, milliseconds: u8);
}

/// Sends the fixed, source-backed controller setup sequence.
pub fn initialise<B: DisplayBus>(bus: &mut B) -> Result<(), B::Error> {
    bus.write(TransferKind::Command, &[0xE2])?;
    bus.delay_ms(120);
    bus.write(TransferKind::Command, &SETUP_COMMANDS)?;
    bus.write(TransferKind::Command, &[0x2B])?;
    bus.delay_ms(1);
    bus.write(TransferKind::Command, &[0x2E])?;
    bus.delay_ms(1);
    bus.write(TransferKind::Command, &[0x2F])?;
    bus.delay_ms(40);
    bus.write(TransferKind::Command, &[0x40, 0xAF])
}

/// Writes all eight visible pages in deterministic order.
pub fn write_frame<B: DisplayBus>(bus: &mut B, frame: &[u8; FRAME_BYTES]) -> Result<(), B::Error> {
    bus.write(TransferKind::Command, &[0x40])?;
    for page in 0_u8..8 {
        let address = [
            0xB0 | page,
            0x10 | (COLUMN_OFFSET >> 4),
            COLUMN_OFFSET & 0x0F,
        ];
        bus.write(TransferKind::Command, &address)?;
        let start = usize::from(page) * WIDTH;
        bus.write(TransferKind::Data, &frame[start..start + WIDTH])?;
    }
    Ok(())
}

/// Produces the fixed AFIK K1 display witness without allocation.
pub fn render_witness(frame: &mut [u8; FRAME_BYTES]) {
    frame.fill(0);
    draw_text(frame, 51, 20, b"AFIK");
    draw_text(frame, 43, 36, b"K1 0.2");
}

/// Produces the fixed witness plus one centered debounced main-key label.
pub fn render_key_witness(frame: &mut [u8; FRAME_BYTES], key: Key) {
    frame.fill(0);
    draw_text(frame, 51, 20, b"AFIK");
    let label = key.label();
    let width = label.len() * 6 - 1;
    draw_text(frame, (WIDTH - width) / 2, 36, label);
}

/// Produces the operating screen: channel name, frequency, and receive state.
///
/// `frequency_hz` is rendered as megahertz with five decimals, and `rssi_raw`
/// is the chip's own 0.5 dB step value, so the screen and the serial
/// observation cannot disagree.
pub fn render_channel_screen(
    frame: &mut [u8; FRAME_BYTES],
    name: &[u8],
    frequency_hz: u32,
    rssi_raw: u16,
    squelch_open: bool,
    audio_routed: bool,
) {
    frame.fill(0);
    let width = name.len() * 6 - 1;
    draw_text(frame, WIDTH.saturating_sub(width) / 2, 2, name);

    let mut megahertz = *b"0000.00000";
    let whole = frequency_hz / 1_000_000;
    let fraction = frequency_hz % 1_000_000 / 10;
    for (index, divisor) in [1000, 100, 10, 1].into_iter().enumerate() {
        megahertz[index] = b'0' + u8::try_from(whole / divisor % 10).unwrap_or(0);
    }
    for (index, divisor) in [10_000, 1_000, 100, 10, 1].into_iter().enumerate() {
        megahertz[5 + index] = b'0' + u8::try_from(fraction / divisor % 10).unwrap_or(0);
    }
    megahertz[4] = b'.';
    draw_text(frame, 4, 20, &megahertz);

    let mut meter = *b"RSSI ---";
    let value = rssi_raw.min(999);
    meter[5] = b'0' + u8::try_from(value / 100).unwrap_or(0);
    meter[6] = b'0' + u8::try_from(value / 10 % 10).unwrap_or(0);
    meter[7] = b'0' + u8::try_from(value % 10).unwrap_or(0);
    draw_text(frame, 4, 38, &meter);
    draw_text(
        frame,
        66,
        38,
        if squelch_open { b"SQ OPEN" } else { b"SQ SHUT" },
    );
    draw_text(
        frame,
        4,
        52,
        if audio_routed {
            b"AUDIO ON "
        } else {
            b"AUDIO OFF"
        },
    );
}

/// Produces the receive witness: audio state, RSSI, and the squelch link.
///
/// `rssi_raw` is the chip's own 0.5 dB step value, rendered without conversion
/// so the screen and the serial observation cannot disagree.
pub fn render_receive_witness(
    frame: &mut [u8; FRAME_BYTES],
    audio_routed: bool,
    rssi_raw: u16,
    squelch_open: bool,
) {
    frame.fill(0);
    draw_text(frame, 51, 4, b"AFIK");
    draw_text(
        frame,
        30,
        20,
        if audio_routed {
            b"AUDIO ON "
        } else {
            b"AUDIO OFF"
        },
    );

    let mut label = *b"RSSI ---";
    let value = rssi_raw.min(999);
    label[5] = b'0' + u8::try_from(value / 100).unwrap_or(0);
    label[6] = b'0' + u8::try_from(value / 10 % 10).unwrap_or(0);
    label[7] = b'0' + u8::try_from(value % 10).unwrap_or(0);
    draw_text(frame, 36, 36, &label);

    draw_text(
        frame,
        36,
        52,
        if squelch_open {
            b"SQ OPEN "
        } else {
            b"SQ SHUT "
        },
    );
}

fn draw_text(frame: &mut [u8; FRAME_BYTES], mut x: usize, y: usize, text: &[u8]) {
    for byte in text {
        let glyph = glyph(*byte);
        for (column, bits) in glyph.iter().enumerate() {
            draw_column(frame, x + column, y, *bits);
        }
        x += 6;
    }
}

fn draw_column(frame: &mut [u8; FRAME_BYTES], x: usize, y: usize, bits: u8) {
    if x >= WIDTH {
        return;
    }
    for bit in 0..7 {
        if bits & (1 << bit) != 0 {
            let pixel_y = y + bit;
            if pixel_y < HEIGHT {
                frame[(pixel_y / 8) * WIDTH + x] |= 1 << (pixel_y % 8);
            }
        }
    }
}

fn glyph(byte: u8) -> [u8; 5] {
    match byte {
        b'.' => [0x00, 0x60, 0x60, 0x00, 0x00],
        b'B' => [0x7F, 0x49, 0x49, 0x49, 0x36],
        b'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        b'G' => [0x3E, 0x41, 0x49, 0x49, 0x7A],
        b'H' => [0x7F, 0x08, 0x08, 0x08, 0x7F],
        b'L' => [0x7F, 0x40, 0x40, 0x40, 0x40],
        b'V' => [0x1F, 0x20, 0x40, 0x20, 0x1F],
        b'0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        b'1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        b'2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        b'3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        b'4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        b'5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        b'6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        b'7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        b'8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        b'9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        b'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        b'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        b'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        b'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        b'M' => [0x7F, 0x02, 0x0C, 0x02, 0x7F],
        b'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        b'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        b'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        b'Q' => [0x3E, 0x41, 0x51, 0x21, 0x5E],
        b'R' => [0x7F, 0x09, 0x19, 0x29, 0x46],
        b'S' => [0x46, 0x49, 0x49, 0x49, 0x31],
        b'T' => [0x01, 0x01, 0x7F, 0x01, 0x01],
        b'U' => [0x3F, 0x40, 0x40, 0x40, 0x3F],
        b'W' => [0x3F, 0x40, 0x38, 0x40, 0x3F],
        b'X' => [0x63, 0x14, 0x08, 0x14, 0x63],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

#[cfg(test)]
mod receive_witness_tests {
    use super::{render_channel_screen, render_receive_witness, FRAME_BYTES};

    #[test]
    fn the_channel_screen_shows_name_frequency_and_receive_state() {
        let mut first = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut first, b"PMR 1", 446_006_250, 52, false, false);
        let mut second = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut second, b"PMR 1", 446_018_750, 52, false, false);
        assert_ne!(first, second, "the frequency must be visible");

        let mut renamed = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut renamed, b"CALL", 446_006_250, 52, false, false);
        assert_ne!(first, renamed, "the channel name must be visible");

        let mut metered = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut metered, b"PMR 1", 446_006_250, 148, false, false);
        assert_ne!(first, metered, "RSSI must be visible");

        let mut open = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut open, b"PMR 1", 446_006_250, 52, true, false);
        assert_ne!(first, open, "the squelch link must be visible");

        let mut routed = [0_u8; FRAME_BYTES];
        render_channel_screen(&mut routed, b"PMR 1", 446_006_250, 52, false, true);
        assert_ne!(first, routed, "the audio state must be visible");
    }

    #[test]
    fn the_receive_witness_renders_distinct_states() {
        let mut muted = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut muted, false, 0, false);
        let mut routed = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut routed, true, 0, false);
        assert_ne!(muted, routed, "audio state must be visible");

        let mut quiet = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut quiet, true, 52, false);
        let mut loud = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut loud, true, 148, false);
        assert_ne!(quiet, loud, "RSSI must be visible");

        let mut open = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut open, true, 52, true);
        assert_ne!(quiet, open, "squelch state must be visible");

        // Values above the three-digit field are clamped, never wrapped.
        let mut clamped = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut clamped, true, 999, false);
        let mut over = [0_u8; FRAME_BYTES];
        render_receive_witness(&mut over, true, 1_500, false);
        assert_eq!(clamped, over);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        initialise, render_key_witness, render_witness, write_frame, DisplayBus, TransferKind,
        FRAME_BYTES, WIDTH,
    };
    use crate::keypad::Key;
    use std::vec::Vec;

    #[derive(Debug, Eq, PartialEq)]
    struct Transfer {
        kind: TransferKind,
        bytes: Vec<u8>,
    }

    #[derive(Default)]
    struct TraceBus {
        transfers: Vec<Transfer>,
        fail_at: Option<usize>,
        delays: Vec<u8>,
    }

    impl DisplayBus for TraceBus {
        type Error = usize;

        fn write(&mut self, kind: TransferKind, bytes: &[u8]) -> Result<(), Self::Error> {
            let index = self.transfers.len();
            if self.fail_at == Some(index) {
                return Err(index);
            }
            self.transfers.push(Transfer {
                kind,
                bytes: bytes.to_vec(),
            });
            Ok(())
        }

        fn delay_ms(&mut self, milliseconds: u8) {
            self.delays.push(milliseconds);
        }
    }

    #[test]
    fn init_trace_is_exact_and_bounded() {
        let mut bus = TraceBus::default();
        initialise(&mut bus).unwrap();
        assert_eq!(bus.transfers.len(), 6);
        assert_eq!(bus.transfers[0].kind, TransferKind::Command);
        assert_eq!(bus.transfers[0].bytes, [0xE2]);
        assert_eq!(
            bus.transfers[1].bytes,
            [0xA2, 0xC0, 0xA1, 0xA6, 0xA4, 0x24, 0x81, 0x1F]
        );
        assert_eq!(bus.transfers[2].bytes, [0x2B]);
        assert_eq!(bus.transfers[3].bytes, [0x2E]);
        assert_eq!(bus.transfers[4].bytes, [0x2F]);
        assert_eq!(bus.transfers[5].bytes, [0x40, 0xAF]);
        assert_eq!(bus.delays, [120, 1, 1, 40]);
    }

    #[test]
    fn frame_trace_addresses_every_visible_page() {
        let mut frame = [0_u8; FRAME_BYTES];
        for (index, byte) in frame.iter_mut().enumerate() {
            *byte = index.to_le_bytes()[0];
        }
        let mut bus = TraceBus::default();
        write_frame(&mut bus, &frame).unwrap();

        assert_eq!(bus.transfers.len(), 17);
        assert_eq!(bus.transfers[0].bytes, [0x40]);
        for page in 0_u8..8 {
            let page_index = usize::from(page);
            let command = &bus.transfers[1 + page_index * 2];
            let data = &bus.transfers[2 + page_index * 2];
            assert_eq!(command.kind, TransferKind::Command);
            assert_eq!(command.bytes, [0xB0 | page, 0x10, 0x04]);
            assert_eq!(data.kind, TransferKind::Data);
            assert_eq!(
                data.bytes,
                frame[page_index * WIDTH..(page_index + 1) * WIDTH]
            );
        }
    }

    #[test]
    fn first_bus_failure_stops_the_trace() {
        let frame = [0_u8; FRAME_BYTES];
        let mut bus = TraceBus {
            transfers: Vec::new(),
            fail_at: Some(6),
            delays: Vec::new(),
        };
        assert_eq!(write_frame(&mut bus, &frame), Err(6));
        assert_eq!(bus.transfers.len(), 6);
    }

    #[test]
    fn fixed_witness_is_deterministic_and_within_visible_pages() {
        let mut first = [0xFF_u8; FRAME_BYTES];
        let mut second = [0_u8; FRAME_BYTES];
        render_witness(&mut first);
        render_witness(&mut second);
        assert_eq!(first, second);
        assert!(first.iter().any(|byte| *byte != 0));
        assert!(first[..2 * WIDTH].iter().all(|byte| *byte == 0));
        assert!(first[6 * WIDTH..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn all_key_labels_render_distinct_visible_frames() {
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
        let mut frames: Vec<[u8; FRAME_BYTES]> = Vec::new();
        for key in keys {
            let mut frame = [0_u8; FRAME_BYTES];
            render_key_witness(&mut frame, key);
            assert!(frame[4 * WIDTH..6 * WIDTH].iter().any(|byte| *byte != 0));
            assert!(frame[6 * WIDTH..].iter().all(|byte| *byte == 0));
            frames.push(frame);
        }
        for first in 0..frames.len() {
            for second in first + 1..frames.len() {
                assert_ne!(frames[first], frames[second]);
            }
        }
    }
}
