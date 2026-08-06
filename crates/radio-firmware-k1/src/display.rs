//! Hardware-independent ST7565-compatible display commands and fixed witness.

/// Visible display width in columns.
pub const WIDTH: usize = 128;
/// Visible display height in pixels.
pub const HEIGHT: usize = 64;
/// Number of eight-pixel controller pages.
pub const PAGES: usize = HEIGHT / 8;
/// Complete visible framebuffer size.
pub const FRAME_BYTES: usize = WIDTH * PAGES;

const COLUMN_OFFSET: u8 = 4;
const SETUP_COMMANDS: [u8; 8] = [
    0xA2, // 1/9 bias
    0xC0, // normal COM direction
    0xA1, // reverse SEG direction
    0xA6, // normal display
    0xA4, // normal RAM display
    0x24, // regulator ratio 5.0
    0x81, // electronic volume follows
    0x15, // bounded initial contrast
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
        b'0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        b'1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        b'2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'F' => [0x7F, 0x09, 0x09, 0x09, 0x01],
        b'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        b'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        initialise, render_witness, write_frame, DisplayBus, TransferKind, FRAME_BYTES, WIDTH,
    };
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
            [0xA2, 0xC0, 0xA1, 0xA6, 0xA4, 0x24, 0x81, 0x15]
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
}
