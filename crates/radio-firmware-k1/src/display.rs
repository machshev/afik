//! Hardware-independent ST7565-compatible display commands and fixed witness.

use crate::keypad::Key;
use radio_bk4819::SquelchThresholds;
use radio_domain::SquelchLevel;

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

/// Longest channel name one list row shows.
pub const LIST_NAME_BYTES: usize = 14;

/// Channel list rows one screen shows.
pub const LIST_ROWS: usize = 5;

/// Everything the operating screen displays.
///
/// The caller passes exactly what it read from the controller and the receiver,
/// so the screen cannot compute a value the serial observation disagrees with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatingView<'a> {
    /// One-based position of the active channel in the active view.
    pub position: u16,
    /// Channels selectable in the active view.
    pub total: u16,
    /// Active channel name.
    pub name: &'a [u8],
    /// Active receive frequency in hertz.
    pub frequency_hz: u32,
    /// The chip's own raw RSSI count.
    pub rssi_raw: u16,
    /// Whether the carrier squelch link is open.
    pub squelch_open: bool,
    /// Remaining battery charge, absent until the radio has a reading.
    pub battery_percent: Option<u8>,
    /// Whether the squelch override is held open.
    pub monitoring: bool,
    /// Whether a scan is walking the channels of the active view.
    pub scanning: bool,
    /// Active bank filter, if any.
    pub bank: Option<BankIndicator<'a>>,
    /// Number being typed, if any: a channel position, or VFO kilohertz.
    pub entry: Option<u32>,
    /// VFO tuning step in hertz, set only while the VFO is the active source.
    pub vfo_step_hz: Option<u32>,
}

/// Columns the operating screen gives the bank indicator.
pub const BANK_INDICATOR_BYTES: usize = 5;

/// The active bank filter as the operating screen shows it.
///
/// The host names its banks, so the name is what the operator recognises. An
/// unnamed bank, which is what a built-in set or an older configuration has,
/// falls back to the identifier rather than to blank space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BankIndicator<'a> {
    /// Bank identifier.
    pub id: u16,
    /// Programmed bank name, which may be empty.
    pub name: &'a [u8],
}

impl BankIndicator<'_> {
    /// Returns the fixed-width indicator text.
    fn label(&self) -> [u8; BANK_INDICATOR_BYTES] {
        let mut label = *b"BK 00";
        if self.name.is_empty() {
            write_two_digits(&mut label[3..], self.id);
            return label;
        }
        label = [b' '; BANK_INDICATOR_BYTES];
        let length = self.name.len().min(BANK_INDICATOR_BYTES);
        label[..length].copy_from_slice(&self.name[..length]);
        label
    }
}

/// One selector-list row, carrying the exact text the row shows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectorRow {
    /// Row label bytes.
    label: [u8; LIST_NAME_BYTES],
    /// Used label bytes.
    label_len: u8,
    /// Whether this row is the filter in force.
    pub active: bool,
}

impl SelectorRow {
    /// Builds one row from exact label bytes.
    #[must_use]
    pub fn text(label: &[u8], active: bool) -> Self {
        Self::with_label(label, active)
    }

    /// Builds one bank row, falling back to the identifier when unnamed.
    #[must_use]
    pub fn bank(id: u16, name: &[u8], active: bool) -> Self {
        if name.is_empty() {
            let mut label = *b"BANK 00";
            write_two_digits(&mut label[5..], id);
            return Self::with_label(&label, active);
        }
        Self::with_label(name, active)
    }

    /// Builds one tuning-step row.
    #[must_use]
    pub fn step(step_hz: u32, active: bool) -> Self {
        Self::with_label(&step_label(step_hz), active)
    }

    /// Builds the settings-menu row for the radio-wide squelch level.
    ///
    /// The row carries its own value, so the operator can read the setting
    /// without opening it.
    #[must_use]
    pub fn squelch_setting(level: u8) -> Self {
        let mut label = [b' '; LIST_NAME_BYTES];
        label[..7].copy_from_slice(b"SQUELCH");
        if level == 0 {
            label[9..13].copy_from_slice(b"OPEN");
        } else {
            label[9] = b'0' + level.min(9);
        }
        Self::with_label(&label, false)
    }

    /// Builds one squelch-level row.
    ///
    /// Level zero is named rather than numbered, because "no squelch at all" is
    /// a different kind of choice from "one step quieter than four".
    ///
    /// Every other row carries the carrier strength it opens at. A level is
    /// otherwise a number with no scale, and the operator setting this is
    /// choosing a threshold against a signal — an interfering one they want
    /// shut out, or a weak one they want through — which they can only do if
    /// the row says what it is in the units the meter uses.
    #[must_use]
    pub fn squelch_level(level: u8, active: bool) -> Self {
        let Ok(squelch) = SquelchLevel::new(level) else {
            return Self::with_label(b"?", active);
        };
        let Some(dbm) = SquelchThresholds::open_dbm(squelch) else {
            return Self::with_label(b"0  OPEN", active);
        };
        let mut label = [b' '; LIST_NAME_BYTES];
        label[0] = b'0' + level.min(9);
        label[3..11].copy_from_slice(&dbm_label(dbm));
        Self::with_label(&label, active)
    }

    fn with_label(label: &[u8], active: bool) -> Self {
        let mut row = Self {
            label: [0; LIST_NAME_BYTES],
            label_len: 0,
            active,
        };
        let length = label.len().min(LIST_NAME_BYTES);
        row.label[..length].copy_from_slice(&label[..length]);
        row.label_len = u8::try_from(length).unwrap_or(0);
        row
    }

    /// Returns the row label.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        &self.label[..usize::from(self.label_len)]
    }
}

/// One channel list row.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ListRow {
    /// One-based position in the active view.
    pub position: u16,
    /// Channel name bytes.
    pub name: [u8; LIST_NAME_BYTES],
    /// Used name bytes.
    pub name_len: u8,
    /// Whether this row is the channel the receiver is tuned to.
    pub active: bool,
}

impl ListRow {
    /// Builds a row, truncating a longer name to the visible width.
    #[must_use]
    pub fn new(position: u16, name: &[u8], active: bool) -> Self {
        let mut row = Self {
            position,
            name: [0; LIST_NAME_BYTES],
            name_len: 0,
            active,
        };
        let length = name.len().min(LIST_NAME_BYTES);
        row.name[..length].copy_from_slice(&name[..length]);
        row.name_len = u8::try_from(length).unwrap_or(0);
        row
    }

    fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_len)]
    }
}

/// Produces the operating screen for one programmed or built-in channel.
pub fn render_operating_screen(frame: &mut [u8; FRAME_BYTES], view: &OperatingView<'_>) {
    frame.fill(0);

    // The top line is a status row: what the radio is listening to on the left,
    // and how much battery is left to keep doing it on the right. The battery
    // holds its corner whatever else changes, so an operator learns one place to
    // glance at rather than hunting for it among the things that move.
    if view.vfo_step_hz.is_some() {
        // The VFO has no position in a list and no bank, so the header names the
        // source instead.
        draw_text(frame, 0, 0, b"VFO");
    } else {
        let mut position = *b"00/00";
        write_two_digits(&mut position[..2], view.position);
        write_two_digits(&mut position[3..], view.total);
        draw_text(frame, 0, 0, &position);
    }
    draw_text(
        frame,
        WIDTH - BATTERY_LABEL_BYTES * 6,
        0,
        &battery_label(view.battery_percent),
    );

    let width = view.name.len() * 6;
    draw_text(frame, WIDTH.saturating_sub(width) / 2, 12, view.name);
    draw_text(frame, 4, 26, &megahertz(view.frequency_hz));

    draw_text(frame, 0, 42, &rssi_label(view.rssi_raw));
    draw_text(
        frame,
        WIDTH - 7 * 6,
        42,
        if view.squelch_open {
            b"SQ OPEN"
        } else {
            b"SQ SHUT"
        },
    );
    // The bottom line carries what the operator is doing rather than what the
    // radio is: the number being typed, the tuning step it will move by, or the
    // bank the channel list is filtered to. Typed digits take precedence so the
    // operator can see what the radio will act on before it acts.
    if let Some(entry) = view.entry {
        let typed = entry_label(entry, view.vfo_step_hz.is_some());
        draw_text(frame, 0, 55, &typed);
    } else if let Some(step_hz) = view.vfo_step_hz {
        draw_text(frame, 0, 55, &step_label(step_hz));
    } else {
        match view.bank {
            Some(bank) => draw_text(frame, 0, 55, &bank.label()),
            None => draw_text(frame, 0, 55, b"ALL  "),
        }
    }
    // A scan moves the channel under the operator without them touching a key,
    // so the screen has to say that is what is happening. It sits beside the
    // monitor marker rather than replacing it: both can be true at once, and a
    // radio which hid one behind the other would be lying about the other.
    if view.scanning {
        draw_text(frame, WIDTH - 8 * 6, 55, b"SCAN");
    }
    if view.monitoring {
        draw_text(frame, WIDTH - 3 * 6, 55, b"MON");
    }
}

/// Columns one carrier-strength label occupies.
pub const DBM_LABEL_BYTES: usize = 8;

/// Renders one carrier strength as signed whole dBm.
///
/// `EVID-BK4819-053` records the chip's meter as `dBm = count / 2 - 160`, so
/// every threshold in this radio is a negative number between about -130 and
/// -66. The sign is always drawn, because a threshold shown without one reads
/// as a positive power level.
#[must_use]
pub fn dbm_label(dbm: i16) -> [u8; DBM_LABEL_BYTES] {
    let mut label = *b"-000 DBM";
    let magnitude = dbm.unsigned_abs().min(999);
    if dbm >= 0 {
        label[0] = b'+';
    }
    label[1] = b'0' + u8::try_from(magnitude / 100).unwrap_or(0);
    label[2] = b'0' + u8::try_from(magnitude / 10 % 10).unwrap_or(0);
    label[3] = b'0' + u8::try_from(magnitude % 10).unwrap_or(0);
    label
}

/// Columns the battery indicator occupies.
pub const BATTERY_LABEL_BYTES: usize = 4;

/// Renders the battery indicator.
///
/// The label carries no prefix: it sits alone in the top right corner, where
/// nothing else appears, so a percentage there needs no naming. The number is
/// right aligned so the digits do not shuffle sideways as the charge falls.
///
/// A radio which does not know its charge says so rather than showing a
/// plausible number, because the whole point of the indicator is to be trusted
/// when it says the pack is nearly flat.
#[must_use]
pub fn battery_label(percent: Option<u8>) -> [u8; BATTERY_LABEL_BYTES] {
    let Some(percent) = percent else {
        return *b" --%";
    };
    let mut label = *b"   %";
    let percent = percent.min(100);
    if percent >= 100 {
        label[0] = b'1';
        label[1] = b'0';
        label[2] = b'0';
        return label;
    }
    if percent >= 10 {
        label[1] = b'0' + percent / 10;
    }
    label[2] = b'0' + percent % 10;
    label
}

/// Produces the scrollable channel list.
///
/// `cursor_row` selects which of the supplied rows is marked. A view with no
/// channels renders an explicit empty message rather than a blank screen.
pub fn render_channel_list(
    frame: &mut [u8; FRAME_BYTES],
    rows: &[ListRow],
    cursor_row: usize,
    total: u16,
) {
    frame.fill(0);
    if rows.is_empty() {
        draw_text(frame, 4, 0, b"CHANNELS");
        draw_text(frame, 4, 24, b"NONE PROGRAMMED");
        draw_text(frame, 4, 40, b"USE AFIK STUDIO");
        return;
    }
    let mut header = *b"CHANNELS 00";
    write_two_digits(&mut header[9..], total);
    draw_text(frame, 0, 0, &header);
    for (index, row) in rows.iter().take(LIST_ROWS).enumerate() {
        let y = 12 + index * 11;
        if index == cursor_row {
            draw_text(frame, 0, y, b">");
        }
        let mut number = *b"00 ";
        write_two_digits(&mut number[..2], row.position);
        draw_text(frame, 6, y, &number);
        draw_text(frame, 6 + 3 * 6, y, row.name());
        if row.active {
            draw_text(frame, WIDTH - 6, y, b"*");
        }
    }
}

/// Produces one titled selector list: receive sources or tuning steps.
///
/// Every choice the operator has is shown as a row with the one in force marked,
/// so selecting the VFO, clearing a bank filter, or changing the tuning step is
/// a visible choice rather than a side effect of cycling. `cursor_row` selects
/// which of the supplied rows is marked.
pub fn render_selector_list(
    frame: &mut [u8; FRAME_BYTES],
    title: &[u8],
    rows: &[SelectorRow],
    cursor_row: usize,
) {
    frame.fill(0);
    draw_text(frame, 0, 0, title);
    if rows.is_empty() {
        draw_text(frame, 4, 24, b"NONE PROGRAMMED");
        draw_text(frame, 4, 40, b"USE AFIK STUDIO");
        return;
    }
    for (index, row) in rows.iter().take(LIST_ROWS).enumerate() {
        let y = 12 + index * 11;
        if index == cursor_row {
            draw_text(frame, 0, y, b">");
        }
        draw_text(frame, 6, y, row.label());
        if row.active {
            draw_text(frame, WIDTH - 6, y, b"*");
        }
    }
}

/// Produces the image and storage information screen.
///
/// This is the display-side witness for a flashed image: the identity string,
/// the active configuration generation the host programmed, and whether that
/// configuration was restored from the radio's own retained storage.
#[allow(clippy::too_many_arguments)]
pub fn render_info_screen(
    frame: &mut [u8; FRAME_BYTES],
    identity: &[u8],
    generation: u32,
    channels: u16,
    retained: bool,
    memory: MemoryState,
    serial: SerialCounters,
    peripheral_clock_khz: u32,
    reset: ResetCause,
) {
    frame.fill(0);
    let width = identity.len() * 6;
    draw_text(frame, WIDTH.saturating_sub(width) / 2, 0, identity);

    // The peripheral clock sits beside the generation because this image does
    // not configure the clock, it inherits whatever the bootloader left. Every
    // baud rate is derived from this number, so a serial link which hears bytes
    // and rejects every packet is asking to be told what the radio thinks its
    // clock is.
    let mut generation_label = *b"GEN000000 CLK00000";
    let mut value = generation;
    for index in (3..9).rev() {
        generation_label[index] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
    }
    let mut clock = peripheral_clock_khz;
    for index in (13..18).rev() {
        generation_label[index] = b'0' + u8::try_from(clock % 10).unwrap_or(0);
        clock /= 10;
    }
    draw_text(frame, 0, 18, &generation_label);

    let mut channel_label = *b"CHANNELS 00 RST ....";
    write_two_digits(&mut channel_label[9..11], channels);
    channel_label[16..20].copy_from_slice(&reset.name());
    draw_text(frame, 0, 32, &channel_label);

    draw_text(
        frame,
        0,
        46,
        if retained {
            b"STORED IN RADIO"
        } else {
            b"NOTHING STORED "
        },
    );

    // The external memory is where a configuration lives, so its state belongs
    // on the screen the operator can reach without a host.
    draw_text(frame, 0, 56, &memory.label());

    // Serial counters, so an operator with no host can see whether the radio is
    // hearing anything at all. `RISK-036` is what these are for: a silent
    // host exchange is otherwise indistinguishable from a dead interface.
    // `D` is packets which arrived complete and failed to decode. Without it,
    // a frame the radio heard and rejected is indistinguishable from one it
    // never received: both read as bytes in and nothing answered.
    // `E` is receiver errors: bytes the UART could not frame at the configured
    // baud. Without it, a link running at the wrong rate is indistinguishable
    // from a silent one, because both leave `RX` at zero — the receive path
    // only counts bytes it framed. Three digits each is what fits four fields
    // across this display; they wrap at a thousand, which is enough to tell
    // moving from not moving.
    let mut link = *b"RX000 TX000 D000 E000";
    write_three_digits(&mut link[2..5], serial.received);
    write_three_digits(&mut link[8..11], serial.answered);
    write_three_digits(&mut link[13..16], serial.discarded);
    write_three_digits(&mut link[18..21], serial.errors);
    draw_text(frame, 0, 8, &link);
}

/// Why the radio last reset, as `RCC_CSR` recorded it.
///
/// The byte is bits 24 to 31 of that register, which is where every reset flag
/// on this part lives. Bit positions come from the pinned metapac definition
/// for this chip and nowhere else: 25 option-byte loader, 26 pin, 27 power,
/// 28 software, 29 independent watchdog, 30 window watchdog.
///
/// `RISK-036` is what this is for. A radio which restarts when a host speaks to
/// it is a different fault depending on whether that restart was a brown-out, a
/// watchdog, or the software asking for it, and the part already knows which.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResetCause(pub u8);

impl ResetCause {
    /// Option byte loader reset.
    const OBL: u8 = 1 << 1;
    /// Reset pin.
    const PIN: u8 = 1 << 2;
    /// Power or brown-out reset.
    const PWR: u8 = 1 << 3;
    /// Software-requested reset, which is what the panic handler now does.
    const SFT: u8 = 1 << 4;
    /// Independent watchdog reset.
    const IWDG: u8 = 1 << 5;
    /// Window watchdog reset.
    const WWDG: u8 = 1 << 6;

    /// Names the cause, or gives the raw flags when more than one is set.
    ///
    /// Naming one flag is what an operator can read back. More than one is
    /// unusual and interesting, so those are shown as `x` and the raw hex
    /// rather than collapsed into a name which would hide the others.
    #[must_use]
    pub fn name(&self) -> [u8; 4] {
        match self.0 {
            0 => *b"NONE",
            Self::OBL => *b"OBL ",
            Self::PIN => *b"PIN ",
            Self::PWR => *b"PWR ",
            Self::SFT => *b"SFT ",
            Self::IWDG => *b"IWDG",
            Self::WWDG => *b"WWDG",
            raw => {
                let mut name = *b"x   ";
                name[1] = hex_digit(raw >> 4);
                name[2] = hex_digit(raw & 0x0f);
                name
            }
        }
    }
}

/// Serial-link counters shown on the information screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SerialCounters {
    /// Bytes the radio has received, which wraps.
    pub received: u16,
    /// Frames the radio has answered, which wraps.
    pub answered: u16,
    /// Complete packets the radio rejected as malformed, which wraps.
    pub discarded: u16,
    /// Receiver errors: bytes which arrived and could not be framed, which
    /// wraps.
    ///
    /// This separates a link at the wrong baud rate from a link with nothing on
    /// it. Both leave `received` at zero, because a byte which does not frame is
    /// never delivered to be counted.
    pub errors: u16,
}

/// Writes one three-digit decimal field.
fn write_three_digits(field: &mut [u8], value: u16) {
    if field.len() < 3 {
        return;
    }
    let mut remaining = value % 1_000;
    for index in (0..3).rev() {
        field[index] = b'0' + u8::try_from(remaining % 10).unwrap_or(0);
        remaining /= 10;
    }
}

/// What the radio found when it looked for its external configuration memory.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MemoryState {
    /// The memory has not been looked for yet.
    #[default]
    Unknown,
    /// A device answered with these identification bytes.
    Present([u8; 3]),
    /// The bus answered but no device was there.
    Absent,
    /// The bus itself failed.
    Failed,
}

/// Bytes in one external-memory state label.
pub const MEMORY_LABEL_BYTES: usize = 15;

impl MemoryState {
    /// Returns the fixed-width label for this state.
    #[must_use]
    pub fn label(self) -> [u8; MEMORY_LABEL_BYTES] {
        let mut label = *b"MEM ...........";
        match self {
            Self::Unknown => label[4..].copy_from_slice(b"UNTRIED    "),
            Self::Absent => label[4..].copy_from_slice(b"NONE       "),
            Self::Failed => label[4..].copy_from_slice(b"BUS FAILED "),
            Self::Present(id) => {
                label[4..].copy_from_slice(b"ID 00 00 00");
                for (index, byte) in id.iter().enumerate() {
                    let at = 7 + index * 3;
                    label[4 + at - 4] = hex_digit(byte >> 4);
                    label[4 + at - 3] = hex_digit(byte & 0x0F);
                }
            }
        }
        label
    }
}

/// Returns the uppercase hexadecimal digit for one nibble.
const fn hex_digit(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + (nibble - 10)
    }
}

/// Longest tuning-step label, which is the widest of the fixed set.
pub const STEP_LABEL_BYTES: usize = 8;

/// Returns the fixed-width label for one tuning step in hertz.
///
/// Steps are whole or fractional kilohertz up to one megahertz, so the label is
/// derived from the value rather than tabulated beside it: the screen cannot
/// disagree with the step the shell will actually apply.
#[must_use]
pub fn step_label(step_hz: u32) -> [u8; STEP_LABEL_BYTES] {
    let mut label = [b' '; STEP_LABEL_BYTES];
    if step_hz >= 1_000_000 {
        let megahertz = (step_hz / 1_000_000).min(9);
        label[..5].copy_from_slice(b"0 MHZ");
        label[0] = b'0' + u8::try_from(megahertz).unwrap_or(0);
        return label;
    }
    let kilohertz = step_hz / 1_000;
    let fraction = step_hz % 1_000 / 10;
    let mut written = 0;
    for divisor in [100, 10, 1] {
        let digit = kilohertz / divisor % 10;
        if written == 0 && digit == 0 && divisor != 1 {
            continue;
        }
        label[written] = b'0' + u8::try_from(digit).unwrap_or(0);
        written += 1;
    }
    if fraction != 0 {
        label[written] = b'.';
        written += 1;
        label[written] = b'0' + u8::try_from(fraction / 10 % 10).unwrap_or(0);
        written += 1;
        if fraction % 10 != 0 {
            label[written] = b'0' + u8::try_from(fraction % 10).unwrap_or(0);
            written += 1;
        }
    }
    let tail = b" KHZ";
    for (offset, byte) in tail.iter().enumerate() {
        if written + offset < STEP_LABEL_BYTES {
            label[written + offset] = *byte;
        }
    }
    label
}

/// Returns the label for digits being typed, in either entry mode.
fn entry_label(entry: u32, vfo: bool) -> [u8; 8] {
    let mut label = [b' '; 8];
    if !vfo {
        label[..5].copy_from_slice(b"CH --");
        write_two_digits(&mut label[3..5], u16::try_from(entry).unwrap_or(u16::MAX));
        return label;
    }
    // Kilohertz digits are shown left to right exactly as typed, so a partial
    // frequency is never mistaken for a complete one.
    let mut value = entry;
    let mut digits = [b' '; 6];
    let mut used = 0;
    while used < digits.len() {
        digits[digits.len() - 1 - used] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        used += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let start = digits.len() - used;
    label[..used].copy_from_slice(&digits[start..]);
    label
}

fn write_two_digits(destination: &mut [u8], value: u16) {
    if destination.len() < 2 {
        return;
    }
    let bounded = value.min(99);
    destination[0] = b'0' + u8::try_from(bounded / 10).unwrap_or(0);
    destination[1] = b'0' + u8::try_from(bounded % 10).unwrap_or(0);
}

fn megahertz(frequency_hz: u32) -> [u8; 10] {
    let mut text = *b"0000.00000";
    let whole = frequency_hz / 1_000_000;
    let fraction = frequency_hz % 1_000_000 / 10;
    for (index, divisor) in [1000, 100, 10, 1].into_iter().enumerate() {
        text[index] = b'0' + u8::try_from(whole / divisor % 10).unwrap_or(0);
    }
    for (index, divisor) in [10_000, 1_000, 100, 10, 1].into_iter().enumerate() {
        text[5 + index] = b'0' + u8::try_from(fraction / divisor % 10).unwrap_or(0);
    }
    text[4] = b'.';
    text
}

fn rssi_label(rssi_raw: u16) -> [u8; 8] {
    let mut label = *b"RSSI ---";
    let value = rssi_raw.min(999);
    label[5] = b'0' + u8::try_from(value / 100).unwrap_or(0);
    label[6] = b'0' + u8::try_from(value / 10 % 10).unwrap_or(0);
    label[7] = b'0' + u8::try_from(value % 10).unwrap_or(0);
    label
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
    // Programmed names are printable ASCII including lowercase, and this
    // panel's fixed five-column font has one case only.
    match byte.to_ascii_uppercase() {
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
        b'J' => [0x20, 0x40, 0x41, 0x3F, 0x01],
        b'Y' => [0x07, 0x08, 0x70, 0x08, 0x07],
        b'Z' => [0x61, 0x51, 0x49, 0x45, 0x43],
        b'-' => [0x08, 0x08, 0x08, 0x08, 0x08],
        b'/' => [0x20, 0x10, 0x08, 0x04, 0x02],
        b'>' => [0x41, 0x22, 0x14, 0x08, 0x00],
        b'<' => [0x08, 0x14, 0x22, 0x41, 0x00],
        b'*' => [0x14, 0x08, 0x3E, 0x08, 0x14],
        b'+' => [0x08, 0x08, 0x3E, 0x08, 0x08],
        b':' => [0x00, 0x36, 0x36, 0x00, 0x00],
        b'#' => [0x14, 0x7F, 0x14, 0x7F, 0x14],
        b'(' => [0x00, 0x1C, 0x22, 0x41, 0x00],
        b')' => [0x00, 0x41, 0x22, 0x1C, 0x00],
        b'_' => [0x40, 0x40, 0x40, 0x40, 0x40],
        b'%' => [0x23, 0x13, 0x08, 0x64, 0x62],
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

#[cfg(test)]
mod operating_screen_tests {
    use super::{
        render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
        step_label, BankIndicator, ListRow, MemoryState, OperatingView, ResetCause, SelectorRow,
        SerialCounters, FRAME_BYTES, LIST_NAME_BYTES, MEMORY_LABEL_BYTES,
    };

    fn view() -> OperatingView<'static> {
        OperatingView {
            position: 3,
            total: 16,
            name: b"2M CALL",
            frequency_hz: 145_500_000,
            rssi_raw: 148,
            squelch_open: false,
            battery_percent: Some(87),
            monitoring: false,
            scanning: false,
            bank: None,
            entry: None,
            vfo_step_hz: None,
        }
    }

    fn render(view: &OperatingView<'_>) -> [u8; FRAME_BYTES] {
        let mut frame = [0xFF_u8; FRAME_BYTES];
        render_operating_screen(&mut frame, view);
        frame
    }

    /// An indicator is only useful if it is believed when it says nearly flat.
    #[test]
    fn the_battery_indicator_says_when_it_does_not_know() {
        use super::{battery_label, BATTERY_LABEL_BYTES};

        assert_eq!(&battery_label(None), b" --%");
        assert_eq!(&battery_label(Some(0)), b"  0%");
        assert_eq!(&battery_label(Some(7)), b"  7%");
        assert_eq!(&battery_label(Some(16)), b" 16%");
        assert_eq!(&battery_label(Some(87)), b" 87%");
        assert_eq!(&battery_label(Some(100)), b"100%");
        assert_eq!(
            &battery_label(Some(200)),
            b"100%",
            "a reading past full is clamped, not rendered as nonsense"
        );

        // The label fits the corner it is drawn into.
        assert_eq!(
            BATTERY_LABEL_BYTES * 6,
            24,
            "four columns of the fixed font"
        );
    }

    /// A character the font does not have draws as blank space.
    ///
    /// The battery label read `16` with no sign on the exact unit because the
    /// font had no `%`. Nothing failed and nothing warned: the glyph table
    /// simply returned five empty columns. Every character the screen puts in a
    /// fixed label is therefore checked here, so the next one added to a label
    /// without a glyph is caught on the host rather than on the radio.
    #[test]
    fn every_character_the_screen_draws_has_a_glyph() {
        use super::{battery_label, glyph, megahertz, rssi_label, step_label};
        use crate::shell::VFO_STEPS_HZ;

        let mut fixed = std::vec::Vec::new();
        fixed.extend_from_slice(b"VFO");
        fixed.extend_from_slice(b"ALL  ");
        fixed.extend_from_slice(b"MON");
        fixed.extend_from_slice(b"SQ OPEN");
        fixed.extend_from_slice(b"SQ SHUT");
        fixed.extend_from_slice(b"BANK 00");
        fixed.extend_from_slice(b"SETTINGS");
        fixed.extend_from_slice(b"SQUELCH");
        fixed.extend_from_slice(b"SOURCE");
        fixed.extend_from_slice(b"STEP");
        fixed.extend_from_slice(b"0  OPEN");
        fixed.extend_from_slice(&rssi_label(148));
        fixed.extend_from_slice(&battery_label(None));
        fixed.extend_from_slice(&battery_label(Some(16)));
        fixed.extend_from_slice(&battery_label(Some(100)));
        fixed.extend_from_slice(&megahertz(145_500_000));
        for step in VFO_STEPS_HZ {
            fixed.extend_from_slice(&step_label(step));
        }
        fixed.extend_from_slice(SelectorRow::squelch_setting(0).label());
        fixed.extend_from_slice(SelectorRow::squelch_setting(5).label());

        for byte in fixed {
            if byte == b' ' {
                continue;
            }
            assert_ne!(
                glyph(byte),
                [0; 5],
                "the screen draws {:?} but the font has no glyph for it",
                char::from(byte)
            );
        }
    }

    #[test]
    fn the_operating_screen_is_deterministic_and_reflects_every_state() {
        let base = render(&view());
        assert_eq!(base, render(&view()));
        assert!(base.iter().any(|byte| *byte != 0));

        for changed in [
            OperatingView {
                position: 4,
                ..view()
            },
            OperatingView {
                squelch_open: true,
                ..view()
            },
            OperatingView {
                battery_percent: Some(12),
                ..view()
            },
            OperatingView {
                battery_percent: None,
                ..view()
            },
            OperatingView {
                monitoring: true,
                ..view()
            },
            OperatingView {
                scanning: true,
                ..view()
            },
            OperatingView {
                scanning: true,
                monitoring: true,
                ..view()
            },
            OperatingView {
                bank: Some(BankIndicator {
                    id: 2,
                    name: b"PMR446",
                }),
                ..view()
            },
            OperatingView {
                bank: Some(BankIndicator { id: 2, name: b"" }),
                ..view()
            },
            OperatingView {
                entry: Some(7),
                ..view()
            },
            OperatingView {
                rssi_raw: 200,
                ..view()
            },
            OperatingView {
                frequency_hz: 433_500_000,
                ..view()
            },
        ] {
            assert_ne!(base, render(&changed), "state must be visible on screen");
        }
    }

    #[test]
    fn a_typed_channel_number_replaces_the_bank_indicator() {
        let typed = render(&OperatingView {
            bank: Some(BankIndicator { id: 2, name: b"" }),
            entry: Some(7),
            ..view()
        });
        let bank = render(&OperatingView {
            bank: Some(BankIndicator { id: 2, name: b"" }),
            ..view()
        });
        assert_ne!(typed, bank);
    }

    /// A threshold the operator cannot read is a threshold they cannot set.
    #[test]
    fn a_squelch_row_names_the_carrier_strength_it_opens_at() {
        use super::dbm_label;

        assert_eq!(&dbm_label(-130), b"-130 DBM");
        assert_eq!(&dbm_label(-66), b"-066 DBM");
        assert_eq!(&dbm_label(0), b"+000 DBM");

        // Every level renders as its own row: an operator stepping the list has
        // to be able to see that something changed.
        let mut seen = [[0_u8; FRAME_BYTES]; 10];
        for level in 0..=9_u8 {
            let mut frame = [0_u8; FRAME_BYTES];
            render_selector_list(
                &mut frame,
                b"SQUELCH",
                &[SelectorRow::squelch_level(level, false)],
                0,
            );
            for (other, earlier) in seen.iter().enumerate().take(usize::from(level)) {
                assert!(
                    *earlier != frame,
                    "level {level} draws the same row as level {other}"
                );
            }
            seen[usize::from(level)] = frame;
        }
    }

    #[test]
    fn the_channel_list_marks_the_cursor_and_the_active_channel() {
        let rows = [
            ListRow::new(1, b"2M CALL", false),
            ListRow::new(2, b"2M FM", true),
            ListRow::new(3, b"70CM", false),
        ];
        let mut cursor_first = [0_u8; FRAME_BYTES];
        render_channel_list(&mut cursor_first, &rows, 0, 3);
        let mut cursor_second = [0_u8; FRAME_BYTES];
        render_channel_list(&mut cursor_second, &rows, 1, 3);
        assert_ne!(cursor_first, cursor_second);
        assert!(cursor_first.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn an_empty_channel_list_says_so_instead_of_going_blank() {
        let mut frame = [0_u8; FRAME_BYTES];
        render_channel_list(&mut frame, &[], 0, 0);
        assert!(frame.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn a_long_channel_name_is_truncated_to_the_visible_width() {
        let row = ListRow::new(1, b"A VERY LONG CHANNEL NAME", false);
        assert_eq!(usize::from(row.name_len), LIST_NAME_BYTES);
    }

    #[test]
    fn a_bank_indicator_prefers_the_programmed_name_over_the_identifier() {
        let named = render(&OperatingView {
            bank: Some(BankIndicator {
                id: 2,
                name: b"PMR446",
            }),
            ..view()
        });
        let numbered = render(&OperatingView {
            bank: Some(BankIndicator { id: 2, name: b"" }),
            ..view()
        });
        let unfiltered = render(&view());
        assert_ne!(named, numbered, "a named bank shows its name");
        assert_ne!(named, unfiltered);
        assert_ne!(numbered, unfiltered);
    }

    #[test]
    fn the_selector_list_marks_the_cursor_and_the_choice_in_force() {
        let rows = [
            SelectorRow::text(b"ALL CHANNELS", false),
            SelectorRow::bank(1, b"AMATEUR 2M", true),
            SelectorRow::bank(3, b"", false),
        ];
        assert_eq!(rows[0].label(), b"ALL CHANNELS");
        assert_eq!(
            rows[2].label(),
            b"BANK 03",
            "an unnamed bank shows its number"
        );

        let mut cursor_first = [0_u8; FRAME_BYTES];
        render_selector_list(&mut cursor_first, b"SOURCE", &rows, 0);
        let mut cursor_second = [0_u8; FRAME_BYTES];
        render_selector_list(&mut cursor_second, b"SOURCE", &rows, 1);
        assert_ne!(cursor_first, cursor_second);
        assert!(cursor_first.iter().any(|byte| *byte != 0));

        let mut unmarked = [0_u8; FRAME_BYTES];
        render_selector_list(
            &mut unmarked,
            b"SOURCE",
            &[
                SelectorRow::text(b"ALL CHANNELS", true),
                SelectorRow::bank(1, b"AMATEUR 2M", false),
                SelectorRow::bank(3, b"", false),
            ],
            0,
        );
        assert_ne!(cursor_first, unmarked, "the filter in force is marked");
    }

    #[test]
    fn the_vfo_screen_shows_its_step_instead_of_a_position_and_bank() {
        let memory = render(&view());
        let vfo = render(&OperatingView {
            vfo_step_hz: Some(12_500),
            ..view()
        });
        assert_ne!(memory, vfo);
        let coarser = render(&OperatingView {
            vfo_step_hz: Some(1_000_000),
            ..view()
        });
        assert_ne!(vfo, coarser, "the step in force is visible");

        // Typed kilohertz digits replace the step indicator while they are being
        // entered, in either mode.
        let typing = render(&OperatingView {
            vfo_step_hz: Some(12_500),
            entry: Some(4335),
            ..view()
        });
        assert_ne!(vfo, typing);
    }

    #[test]
    fn tuning_step_labels_are_derived_from_the_step_itself() {
        assert_eq!(&step_label(6_250)[..8], b"6.25 KHZ");
        assert_eq!(&step_label(12_500)[..8], b"12.5 KHZ");
        assert_eq!(&step_label(25_000)[..6], b"25 KHZ");
        assert_eq!(&step_label(100_000)[..7], b"100 KHZ");
        assert_eq!(&step_label(1_000_000)[..5], b"1 MHZ");
    }

    #[test]
    fn an_empty_selector_list_says_so_instead_of_going_blank() {
        let mut frame = [0_u8; FRAME_BYTES];
        render_selector_list(&mut frame, b"SOURCE", &[], 0);
        assert!(frame.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn a_long_bank_name_is_truncated_to_the_visible_width() {
        let row = SelectorRow::bank(1, b"A VERY LONG BANK NAME", false);
        assert_eq!(row.label().len(), LIST_NAME_BYTES);
    }

    #[test]
    fn the_info_screen_separates_retained_and_unstored_configurations() {
        let mut retained = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut retained,
            b"AFIK-K1-3.2",
            7,
            16,
            true,
            MemoryState::Present([0x68, 0x40, 0x15]),
            SerialCounters {
                received: 12,
                answered: 3,
                discarded: 2,
                errors: 0,
            },
            24_000,
            ResetCause::default(),
        );
        let mut unstored = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut unstored,
            b"AFIK-K1-3.2",
            0,
            5,
            false,
            MemoryState::Absent,
            SerialCounters::default(),
            24_000,
            ResetCause::default(),
        );
        assert_ne!(retained, unstored);
        assert!(retained.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn every_memory_state_has_a_distinct_fixed_width_label() {
        let states = [
            MemoryState::Unknown,
            MemoryState::Absent,
            MemoryState::Failed,
            MemoryState::Present([0x68, 0x40, 0x15]),
        ];
        for state in states {
            assert_eq!(state.label().len(), MEMORY_LABEL_BYTES);
        }
        // The identification is what distinguishes a working memory from a
        // plausible-looking failure, so it is shown exactly.
        // The exact unit answers with this identification: a 16 Mbit serial NOR
        // memory, manufacturer 0x68, recorded by `EVID-K1-061`.
        assert_eq!(
            &MemoryState::Present([0x68, 0x40, 0x15]).label(),
            b"MEM ID 68 40 15"
        );
        assert_eq!(&MemoryState::Absent.label(), b"MEM NONE       ");
        assert_ne!(MemoryState::Unknown.label(), MemoryState::Failed.label());
    }

    #[test]
    fn a_receiver_error_is_visible_even_when_nothing_was_received() {
        // The case RISK-036 could not distinguish: a link at the wrong baud
        // rate delivers no bytes to count, so `RX` stays at zero whether or not
        // anything is on the wire. `E` is what separates them.
        let mut silent = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut silent,
            b"AFIK-K1-5.9",
            0,
            0,
            false,
            MemoryState::Absent,
            SerialCounters::default(),
            48_000,
            ResetCause::default(),
        );
        let mut erroring = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut erroring,
            b"AFIK-K1-5.9",
            0,
            0,
            false,
            MemoryState::Absent,
            SerialCounters {
                received: 0,
                answered: 0,
                discarded: 0,
                errors: 42,
            },
            48_000,
            ResetCause::default(),
        );
        assert_ne!(silent, erroring);
    }

    #[test]
    fn every_single_reset_flag_has_its_own_name() {
        // Bit positions are the metapac's, offset by 24 because the byte kept
        // is bits 24 to 31 of RCC_CSR.
        let named = [
            (0b0000_0000, *b"NONE"),
            (0b0000_0010, *b"OBL "),
            (0b0000_0100, *b"PIN "),
            (0b0000_1000, *b"PWR "),
            (0b0001_0000, *b"SFT "),
            (0b0010_0000, *b"IWDG"),
            (0b0100_0000, *b"WWDG"),
        ];
        for (raw, expected) in named {
            assert_eq!(ResetCause(raw).name(), expected);
        }
    }

    #[test]
    fn more_than_one_flag_shows_the_raw_byte_rather_than_hiding_flags() {
        // A pin reset and a power reset together is not either one of them, and
        // naming one would discard the other.
        assert_eq!(ResetCause(0b0000_1100).name(), *b"x0C ");
        assert_eq!(ResetCause(0b1111_1111).name(), *b"xFF ");
    }

    #[test]
    fn the_reset_cause_reaches_the_information_screen() {
        let mut power = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut power,
            b"AFIK-K1-6.0",
            0,
            0,
            false,
            MemoryState::Absent,
            SerialCounters::default(),
            48_000,
            ResetCause(0b0000_1000),
        );
        let mut software = [0_u8; FRAME_BYTES];
        render_info_screen(
            &mut software,
            b"AFIK-K1-6.0",
            0,
            0,
            false,
            MemoryState::Absent,
            SerialCounters::default(),
            48_000,
            ResetCause(0b0001_0000),
        );
        assert_ne!(power, software);
    }
}
