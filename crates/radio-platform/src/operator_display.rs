//! Hardware-independent ST7565-compatible display commands and fixed witness.

use crate::catch::Phase as CatchPhase;
use crate::display::{BootStage, ReceiveDiagnostic};
use crate::operator_shell::TX_POWER_NOMINAL_MW;
use crate::receive_app::Key;
use crate::s_meter::{self, SReading};
use radio_bk4819::SquelchThresholds;
use radio_domain::{Bandwidth, Modulation, SquelchLevel, Tone, TxClass};

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

/// Renders one shared application boot stage using the K1 framebuffer layout.
pub fn render_boot_stage(frame: &mut [u8; FRAME_BYTES], stage: BootStage) {
    frame.fill(0);
    draw_text(frame, 51, 20, b"AFIK");
    let text: &[u8] = match stage {
        BootStage::Reset => b"RESET",
        BootStage::BoardReady => b"BOARD",
        BootStage::SerialReady => b"SERIAL",
    };
    draw_text(frame, (WIDTH - (text.len() * 6 - 1)) / 2, 36, text);
}

/// Renders shared serial receive evidence using the K1 framebuffer layout.
pub fn render_receive_diagnostic(frame: &mut [u8; FRAME_BYTES], diagnostic: ReceiveDiagnostic) {
    frame.fill(0);
    draw_text(frame, 51, 12, b"AFIK");
    let mut count = *b"RX 000000";
    write_decimal(&mut count[3..], diagnostic.bytes);
    draw_text(frame, 37, 28, &count);
    let mut status = *b"E 00000000";
    write_hexadecimal(&mut status[2..], diagnostic.status);
    draw_text(frame, 34, 44, &status);
}

fn write_decimal(bytes: &mut [u8], mut value: u32) {
    for byte in bytes.iter_mut().rev() {
        *byte = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
    }
}

fn write_hexadecimal(bytes: &mut [u8], mut value: u32) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    for byte in bytes.iter_mut().rev() {
        *byte = DIGITS[usize::try_from(value & 0xF).unwrap_or(0)];
        value >>= 4;
    }
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

/// One slot as the operating screen shows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotView<'a> {
    /// `b'A'` or `b'B'`.
    pub letter: u8,
    /// Whether keys act on this slot and PTT transmits on it.
    pub selected: bool,
    /// What it listens to; `None` is a memory with no channels.
    pub channel: Option<SlotChannel<'a>>,
}

/// The channel or VFO one slot listens to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotChannel<'a> {
    /// Channel name, or `VFO`.
    pub name: &'a [u8],
    /// Frequency heard, in hertz: the receive frequency, or a listen-only
    /// duplex channel's transmit frequency while that side is talking.
    pub frequency_hz: u32,
    /// The channel is duplex and may not transmit, so both its sides are
    /// listened to (`ADR-108`); `DX` takes the place of the power level.
    pub listens_both: bool,
    /// The channel's bank and its one-based number in the view, or the VFO's
    /// step in hertz.
    pub place: SlotPlace,
    /// Modulation.
    pub modulation: Modulation,
    /// Bandwidth, which tells FM from NFM.
    pub bandwidth: Bandwidth,
    /// The TX power level, zero-based.
    pub power: u8,
    /// The chip's raw RSSI count while the receiver is on this slot and its
    /// squelch is open; no meter is drawn otherwise.
    pub meter: Option<u16>,
    /// CTCSS or DCS sent when transmitting on this channel.
    pub tx_tone: Tone,
    /// CTCSS or DCS the receiver requires before it opens.
    pub rx_tone: Tone,
    /// A tone heard on the open squelch of a channel without a receive tone.
    pub heard_tone: Option<Tone>,
}

/// Where one slot's channel sits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotPlace {
    /// A programmed channel: its bank, if it has one, and its number.
    Memory {
        /// The channel's bank identifier.
        bank: Option<u16>,
        /// One-based number in the slot's view.
        number: u16,
    },
    /// The VFO, with its tuning step in hertz.
    Vfo {
        /// Tuning step in hertz.
        step_hz: u32,
    },
}

/// Everything the operating screen displays.
///
/// The caller passes exactly what it read from the controllers and the
/// receiver, so the screen cannot compute a value the serial observation
/// disagrees with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct OperatingView<'a> {
    /// Slot A, drawn on top.
    pub a: SlotView<'a>,
    /// Slot B, drawn below.
    pub b: SlotView<'a>,
    /// Whether the receiver path is known healthy.
    pub receiver_ok: bool,
    /// Remaining battery charge, absent until the radio has a reading.
    pub battery_percent: Option<u8>,
    /// Whether the squelch override is held open.
    pub monitoring: bool,
    /// Whether a scan is walking the channels of the selected slot.
    pub scanning: bool,
    /// Whether dual watch is on.
    pub dual_watch: bool,
    /// Whether F waits for the key it modifies.
    pub function: bool,
    /// Number being typed, if any: a channel position, or VFO kilohertz.
    pub entry: Option<u32>,
    /// Whether the selected slot is on the VFO, so an entry is kilohertz.
    pub entry_vfo: bool,
    /// Why the TX policy refused the press PTT is still held after, if it did.
    pub tx_refused: Option<TxRefusal>,
}

/// Why a PTT press did not transmit, as the status row says it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxRefusal {
    /// The radio has no transmitter or calibration for this frequency.
    NoBand,
    /// The programmed ranges, a class switch, or the radio's state refused.
    Disabled,
}

impl TxRefusal {
    /// The status row text.
    #[must_use]
    pub const fn label(self) -> &'static [u8] {
        match self {
            Self::NoBand => b"NO TX BAND",
            Self::Disabled => b"TX DISABLED",
        }
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

    /// Builds the settings-menu row for the selected slot's VFO step, such as
    /// `VFO STEP 6.25K`.
    #[must_use]
    pub fn vfo_step_setting(step_hz: u32) -> Self {
        let mut label = [b' '; LIST_NAME_BYTES];
        label[..8].copy_from_slice(b"VFO STEP");
        label[9..].copy_from_slice(&short_step_label(step_hz));
        Self::with_label(&label, false)
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

    /// Builds the settings-menu row for TX power: the override's level, or
    /// `CHAN` while each channel transmits at its own.
    #[must_use]
    pub fn tx_power_setting(level: Option<u8>) -> Self {
        let mut label = [b' '; LIST_NAME_BYTES];
        label[..8].copy_from_slice(b"TX POWER");
        label[9..].copy_from_slice(&level.map_or(*b" CHAN", milliwatts_label));
        Self::with_label(&label, false)
    }

    /// Builds the TX power row which drops the override, numbered zero.
    #[must_use]
    pub fn tx_power_channel(active: bool) -> Self {
        Self::with_label(b"0  CHANNEL", active)
    }

    /// Builds one TX power-level row, numbered from one.
    #[must_use]
    pub fn tx_power_level(level: u8, active: bool) -> Self {
        let mut label = [b' '; LIST_NAME_BYTES];
        label[0] = b'1' + level.min(8);
        label[3..8].copy_from_slice(&milliwatts_label(level));
        Self::with_label(&label[..8], active)
    }

    /// Builds the settings-menu row for the TX timeout, with its value.
    #[must_use]
    pub fn tx_timeout_setting(choice: u8) -> Self {
        let mut label = [b' '; LIST_NAME_BYTES];
        label[..10].copy_from_slice(b"TX TIMEOUT");
        label[11] = b'1' + choice.min(8);
        label[12] = b'M';
        Self::with_label(&label[..13], false)
    }

    /// Builds one TX timeout row in whole minutes, such as `1 MIN`.
    #[must_use]
    pub fn tx_timeout_choice(choice: u8, active: bool) -> Self {
        let mut label = *b"1 MIN";
        label[0] = b'1' + choice.min(8);
        Self::with_label(&label, active)
    }

    /// Builds the settings-menu row for the TX classes: how many of the
    /// programmed ones are switched on, such as `TX CLASSES 1/2`.
    #[must_use]
    pub fn tx_classes_setting(on: usize, programmed: usize) -> Self {
        let mut label = *b"TX CLASSES 0/0";
        label[11] = b'0' + u8::try_from(on.min(9)).unwrap_or(9);
        label[13] = b'0' + u8::try_from(programmed.min(9)).unwrap_or(9);
        Self::with_label(&label, false)
    }

    /// Builds the settings-menu row for dual watch, with its switch.
    #[must_use]
    pub fn dual_watch_setting(on: bool) -> Self {
        Self::with_label(
            if on {
                b"DUAL WATCH ON"
            } else {
                b"DUAL WATCH OFF"
            },
            false,
        )
    }

    /// Builds the settings-menu row for the heard-tone experiment.
    #[must_use]
    pub fn tone_heard_setting(on: bool) -> Self {
        Self::with_label(
            if on {
                b"TONE HEARD ON"
            } else {
                b"TONE HEARD OFF"
            },
            false,
        )
    }

    /// Builds one TX class row with its switch, such as `AMATEUR    OFF`;
    /// a class switched on is marked active.
    #[must_use]
    pub fn tx_class(class: TxClass, on: bool) -> Self {
        let name: &[u8] = match class {
            TxClass::LicenceFreePlan => b"LIC FREE",
            TxClass::Amateur => b"AMATEUR",
            TxClass::Marine => b"MARINE",
            TxClass::Aeronautical => b"AERO",
            TxClass::Business => b"BUSINESS",
            TxClass::Experimental => b"EXPERIMENT",
        };
        let mut label = [b' '; LIST_NAME_BYTES];
        label[..name.len()].copy_from_slice(name);
        let state: &[u8] = if on { b"ON" } else { b"OFF" };
        label[11..11 + state.len()].copy_from_slice(state);
        Self::with_label(&label[..11 + state.len()], on)
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

/// A five-byte nominal power label such as ` 20MW`, `500MW` or `   5W`.
fn milliwatts_label(level: u8) -> [u8; 5] {
    let milliwatts = TX_POWER_NOMINAL_MW
        .get(usize::from(level))
        .copied()
        .unwrap_or(0);
    if milliwatts >= 1_000 {
        let mut label = *b"    W";
        label[3] = b'0' + u8::try_from((milliwatts / 1_000).min(9)).unwrap_or(9);
        return label;
    }
    let milliwatts = milliwatts.min(999);
    let mut label = *b"   MW";
    let digits = [milliwatts / 100, milliwatts / 10 % 10, milliwatts % 10];
    let mut started = false;
    for (offset, digit) in digits.into_iter().enumerate() {
        started |= digit != 0 || offset == 2;
        if started {
            label[offset] = b'0' + u8::try_from(digit).unwrap_or(0);
        }
    }
    label
}

/// Produces the operating screen: a status row, then slot A and slot B.
pub fn render_operating_screen(frame: &mut [u8; FRAME_BYTES], view: &OperatingView<'_>) {
    frame.fill(0);

    // The status row carries what the operator is doing on the left, and the
    // battery in the right corner, where nothing else appears. A failed
    // receiver must not resemble a quiet but healthy channel, so it takes the
    // row over; a refused PTT press must not resemble a silent transmission,
    // so it comes next; typed digits follow, so the operator sees what the
    // radio will act on before it acts.
    if !view.receiver_ok {
        draw_text(frame, 0, 0, b"RX FAULT");
    } else if let Some(refused) = view.tx_refused {
        draw_text(frame, 0, 0, refused.label());
    } else if let Some(entry) = view.entry {
        draw_text(frame, 0, 0, &entry_label(entry, view.entry_vfo));
    } else {
        if view.function {
            draw_inverted(frame, 0, 0, b'F');
        }
        if view.dual_watch {
            draw_text(frame, 12, 0, b"DW");
        }
        // A scan moves the channel under the operator without them touching
        // a key, so the screen says so; both markers can be true at once.
        if view.scanning {
            draw_text(frame, 30, 0, b"SCAN");
        }
        if view.monitoring {
            draw_text(frame, 60, 0, b"MON");
        }
    }
    draw_text(
        frame,
        WIDTH - BATTERY_LABEL_BYTES * 6,
        0,
        &battery_label(view.battery_percent),
    );

    draw_slot(frame, SLOT_A_Y, &view.a);
    draw_horizontal(frame, SLOT_B_Y - 3);
    draw_slot(frame, SLOT_B_Y, &view.b);
}

/// Top pixel row of slot A.
const SLOT_A_Y: usize = 10;
/// Top pixel row of slot B.
const SLOT_B_Y: usize = 38;
/// Left column of a slot's text and meter, after its letter.
const SLOT_TEXT_X: usize = 10;

/// Draws one slot in three rows: letter, name and place; frequency,
/// modulation and power; and the meter while its squelch is open, else the
/// channel's TX and RX tones.
fn draw_slot(frame: &mut [u8; FRAME_BYTES], y: usize, slot: &SlotView<'_>) {
    if slot.selected {
        draw_inverted(frame, 0, y, slot.letter);
    } else {
        draw_text(frame, 1, y, &[slot.letter]);
    }
    let Some(channel) = slot.channel else {
        draw_text(frame, SLOT_TEXT_X, y, b"NO CHANNELS");
        return;
    };
    let name = &channel.name[..channel.name.len().min(12)];
    draw_text(frame, SLOT_TEXT_X, y, name);
    match channel.place {
        SlotPlace::Memory { bank, number } => {
            draw_text(frame, WIDTH - 7 * 6, y, &place_label(bank, number));
        }
        SlotPlace::Vfo { step_hz } => {
            let label = short_step_label(step_hz);
            let used = label
                .iter()
                .rposition(|byte| *byte != b' ')
                .map_or(0, |at| at + 1);
            draw_text(frame, WIDTH - used * 6, y, &label[..used]);
        }
    }
    let frequency = megahertz(channel.frequency_hz);
    // Below 1 GHz the leading zero says nothing.
    let frequency = if frequency[0] == b'0' {
        &frequency[1..]
    } else {
        &frequency[..]
    };
    draw_text(frame, SLOT_TEXT_X, y + 9, frequency);
    let mode = modulation_label(channel.modulation, channel.bandwidth);
    draw_text(frame, WIDTH - 7 * 6, y + 9, &mode);
    let power = if channel.listens_both {
        *b"DX"
    } else {
        power_label(channel.power)
    };
    draw_text(frame, WIDTH - 2 * 6, y + 9, &power);
    if let Some(rssi_raw) = channel.meter {
        // The receive tone, or one heard on a channel without one, is what
        // opened this squelch.
        let tone = match channel.rx_tone {
            Tone::None => channel.heard_tone,
            tone => Some(tone),
        };
        draw_meter(frame, y + 18, rssi_raw, channel.frequency_hz, tone);
    } else {
        for (x, marker, tone) in [
            (SLOT_TEXT_X, b'T', channel.tx_tone),
            (SLOT_TEXT_X + TONE_FIELD_BYTES * 6, b'R', channel.rx_tone),
        ] {
            if tone.is_some() {
                draw_text(frame, x, y + 18, &[marker]);
                draw_text(frame, x + 6, y + 18, &tone_label(tone));
            }
        }
    }
}

/// Bytes of one tone field on the tone row: its marker, the tone and a gap.
const TONE_FIELD_BYTES: usize = 7;

/// A tone in five bytes: CTCSS hertz such as ` 88.5` or `123.0`, or a DCS
/// code and polarity such as `D023N` or `D754I`.
#[must_use]
pub fn tone_label(tone: Tone) -> [u8; 5] {
    let digit = |value: u16| b'0' + u8::try_from(value % 10).unwrap_or(0);
    match tone {
        Tone::None => *b"     ",
        Tone::Ctcss(tenths) => {
            let hundreds = if tenths >= 1_000 {
                digit(tenths / 1_000)
            } else {
                b' '
            };
            [
                hundreds,
                digit(tenths / 100),
                digit(tenths / 10),
                b'.',
                digit(tenths),
            ]
        }
        Tone::Dcs { code, inverted } => [
            b'D',
            digit(code / 100),
            digit(code / 10),
            digit(code),
            if inverted { b'I' } else { b'N' },
        ],
    }
}

/// Pixel columns in one meter segment, the last of them a gap.
const METER_SEGMENT: usize = 4;
/// Narrower segments, so a tone fits where the dBm reading was.
const METER_SEGMENT_WITH_TONE: usize = 3;

/// Draws the calibrated S meter (`s_meter`): a bar of one segment per S unit
/// and a taller one per 10 dB over S9, the reading such as `S9+20`, and the
/// carrier in dBm at the right, or `tone` there in a narrower bar. An open
/// squelch always draws at least one segment, so the meter is never present
/// but empty.
fn draw_meter(
    frame: &mut [u8; FRAME_BYTES],
    y: usize,
    rssi_raw: u16,
    frequency_hz: u32,
    tone: Option<Tone>,
) {
    let reading = SReading::from_rssi(rssi_raw, frequency_hz);
    let width = if tone.is_some() {
        METER_SEGMENT_WITH_TONE
    } else {
        METER_SEGMENT
    };
    for segment in 0..usize::from(reading.segments().max(1)) {
        let x = SLOT_TEXT_X + segment * width;
        let bits = if segment < 9 {
            0b0111_1100
        } else {
            0b0111_1111
        };
        for column in x..x + width - 1 {
            draw_column(frame, column, y, bits);
        }
    }
    let label_x = SLOT_TEXT_X + usize::from(s_meter::SEGMENTS) * width + 2;
    draw_text(frame, label_x, y, &reading.label());
    if let Some(tone) = tone {
        draw_text(frame, WIDTH - 5 * 6, y, &tone_label(tone));
        return;
    }
    let dbm = dbm_digits(reading.dbm);
    let used = dbm.iter().position(|byte| *byte != b' ').unwrap_or(0);
    draw_text(frame, WIDTH - (dbm.len() - used) * 6, y, &dbm[used..]);
}

/// Renders whole dBm right-aligned in four bytes, such as `-105` or ` -93`.
fn dbm_digits(dbm: i16) -> [u8; 4] {
    let mut digits = *b"    ";
    let magnitude = dbm.unsigned_abs().min(999);
    let mut at = digits.len();
    let mut rest = magnitude;
    loop {
        at -= 1;
        digits[at] = b'0' + u8::try_from(rest % 10).unwrap_or(0);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    if dbm < 0 {
        digits[at - 1] = b'-';
    }
    digits
}

/// Draws one character in a filled box, marking a selection.
fn draw_inverted(frame: &mut [u8; FRAME_BYTES], x: usize, y: usize, byte: u8) {
    let bits = glyph(byte);
    for column in 0..7 {
        let glyph_bits = if (1..6).contains(&column) {
            bits[column - 1]
        } else {
            0
        };
        // One pixel of box above and below the seven-pixel glyph.
        for row in 0..9 {
            let lit = match row {
                1..=7 => glyph_bits & (1 << (row - 1)) == 0,
                _ => true,
            };
            let pixel_y = (y + row).wrapping_sub(1);
            if lit && x + column < WIDTH && pixel_y < HEIGHT {
                frame[(pixel_y / 8) * WIDTH + x + column] |= 1 << (pixel_y % 8);
            }
        }
    }
}

/// Draws a dotted rule across the screen between the slots.
fn draw_horizontal(frame: &mut [u8; FRAME_BYTES], y: usize) {
    for x in (0..WIDTH).step_by(2) {
        frame[(y / 8) * WIDTH + x] |= 1 << (y % 8);
    }
}

/// Renders a memory channel's bank and number, such as `B03 012`; `B--`
/// for a channel in no bank.
#[must_use]
pub fn place_label(bank: Option<u16>, number: u16) -> [u8; 7] {
    let mut label = *b"B-- 000";
    if let Some(bank) = bank {
        write_two_digits(&mut label[1..3], bank);
    }
    let number = number.min(999);
    label[4] = b'0' + u8::try_from(number / 100).unwrap_or(0);
    write_two_digits(&mut label[5..], number % 100);
    label
}

/// Renders a modulation as the stock firmware names it: `NFM`, ` FM`, ` AM`
/// or `USB`, with broadcast as `WFM`.
#[must_use]
pub fn modulation_label(modulation: Modulation, bandwidth: Bandwidth) -> [u8; 3] {
    match (modulation, bandwidth) {
        (Modulation::Fm, Bandwidth::Narrow) => *b"NFM",
        (Modulation::Fm, Bandwidth::Wide) => *b" FM",
        (Modulation::Am, _) => *b" AM",
        (Modulation::Usb, _) => *b"USB",
        (Modulation::Broadcast, _) => *b"WFM",
    }
}

/// Renders a TX power level: `L1` to `L4`, ` M` or ` H`.
#[must_use]
pub fn power_label(level: u8) -> [u8; 2] {
    match level {
        0..=3 => [b'L', b'1' + level],
        4 => *b" M",
        _ => *b" H",
    }
}

/// Renders a VFO step in at most five columns, such as `12.5K` or `1M`.
#[must_use]
pub fn short_step_label(step_hz: u32) -> [u8; 5] {
    let mut label = [b' '; 5];
    let (value, unit) = if step_hz >= 1_000_000 && step_hz % 1_000_000 == 0 {
        (step_hz / 1_000_000, b'M')
    } else if step_hz % 1_000 == 0 {
        (step_hz / 1_000, b'K')
    } else {
        // Hundredths of a kilohertz: 6.25K, 12.5K.
        let hundredths = step_hz / 10;
        let whole = hundredths / 100;
        let fraction = hundredths % 100;
        let mut text = [b' '; 5];
        let mut at = 0;
        if whole >= 10 {
            text[at] = b'0' + u8::try_from(whole / 10 % 10).unwrap_or(0);
            at += 1;
        }
        text[at] = b'0' + u8::try_from(whole % 10).unwrap_or(0);
        text[at + 1] = b'.';
        text[at + 2] = b'0' + u8::try_from(fraction / 10).unwrap_or(0);
        if fraction % 10 == 0 {
            text[at + 3] = b'K';
        } else {
            text[at + 3] = b'0' + u8::try_from(fraction % 10).unwrap_or(0);
            text[at + 4] = b'K';
        }
        return text;
    };
    let mut digits = [0_u8; 4];
    let mut count = 0;
    let mut rest = value.min(9_999);
    loop {
        digits[count] = b'0' + u8::try_from(rest % 10).unwrap_or(0);
        count += 1;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    for (at, digit) in digits[..count].iter().rev().enumerate() {
        label[at] = *digit;
    }
    label[count] = unit;
    label
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

/// What the frequency and tone catch screen shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatchView {
    /// What the catch is doing.
    pub phase: CatchPhase,
    /// The caught frequency, once three readings agree.
    pub frequency_hz: Option<u32>,
    /// The tone or code read so far.
    pub tone: Option<Tone>,
    /// Whether Menu can put the frequency in the VFO.
    pub to_vfo: bool,
}

/// Produces the frequency and tone catch screen (F then 4, `FC`).
pub fn render_catch_screen(frame: &mut [u8; FRAME_BYTES], view: &CatchView) {
    frame.fill(0);
    let title = b"FREQ CATCH";
    draw_text(frame, (WIDTH - title.len() * 6) / 2, 0, title);
    let state: &[u8] = match view.phase {
        CatchPhase::Frequency => b"FINDING FREQUENCY",
        CatchPhase::Tone => b"FINDING TONE",
        CatchPhase::Done => b"CAUGHT",
    };
    draw_text(frame, 0, 14, state);
    match view.frequency_hz {
        Some(hertz) => {
            let text = megahertz(hertz);
            // Below 1 GHz the leading zero says nothing.
            let text = if text[0] == b'0' {
                &text[1..]
            } else {
                &text[..]
            };
            draw_text(frame, 0, 28, text);
            draw_text(frame, text.len() * 6 + 6, 28, b"MHZ");
        }
        None => draw_text(frame, 0, 28, b"---.----- MHZ"),
    }
    draw_text(frame, 0, 42, b"TONE");
    match view.tone {
        Some(tone) => draw_text(frame, 5 * 6, 42, &tone_label(tone)),
        None if view.phase == CatchPhase::Done => draw_text(frame, 5 * 6, 42, b"NONE"),
        None => draw_text(frame, 5 * 6, 42, b"-----"),
    }
    if view.frequency_hz.is_some() && view.to_vfo {
        draw_text(frame, 0, 56, b"MENU TO VFO");
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
    boot: BootTimes,
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
    write_digits(&mut generation_label[3..9], generation);
    write_digits(&mut generation_label[13..18], peripheral_clock_khz);
    draw_text(frame, 0, 18, &generation_label);

    let mut channel_label = *b"CHANNELS 00 RST ....";
    write_two_digits(&mut channel_label[9..11], channels);
    channel_label[16..20].copy_from_slice(&reset.name());
    draw_text(frame, 0, 32, &channel_label);

    // When boot reached each stage, so a slow start can be placed without a
    // host: `S` the serial task starting, `L` the stored configuration loaded,
    // `F` the interface's first frame, each in milliseconds since reset.
    let mut timing = *b"S0000 L0000 F0000";
    write_digits(&mut timing[1..5], boot.serial_start_ms.min(9_999));
    write_digits(&mut timing[7..11], boot.loaded_ms.min(9_999));
    write_digits(&mut timing[13..17], boot.first_frame_ms.min(9_999));
    draw_text(frame, 0, 39, &timing);

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
    write_digits(&mut link[2..5], u32::from(serial.received));
    write_digits(&mut link[8..11], u32::from(serial.answered));
    write_digits(&mut link[13..16], u32::from(serial.discarded));
    write_digits(&mut link[18..21], u32::from(serial.errors));
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

/// Milliseconds since reset at which boot reached each stage, or zero for a
/// stage not reached yet.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BootTimes {
    /// The serial task started and is about to open the memory.
    pub serial_start_ms: u32,
    /// The stored configuration was loaded and published.
    pub loaded_ms: u32,
    /// The interface drew its first frame.
    pub first_frame_ms: u32,
}

/// Writes `value` in decimal across the whole field, keeping its low digits.
fn write_digits(field: &mut [u8], value: u32) {
    let mut remaining = value;
    for byte in field.iter_mut().rev() {
        *byte = b'0' + u8::try_from(remaining % 10).unwrap_or(0);
        remaining /= 10;
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
mod tx_class_rows {
    use super::SelectorRow;
    use radio_domain::TxClass;

    #[test]
    fn tx_class_rows_name_the_class_and_its_switch() {
        assert_eq!(
            SelectorRow::tx_classes_setting(1, 2).label(),
            b"TX CLASSES 1/2"
        );
        let on = SelectorRow::tx_class(TxClass::Amateur, true);
        assert_eq!(on.label(), b"AMATEUR    ON");
        assert!(on.active);
        let off = SelectorRow::tx_class(TxClass::Experimental, false);
        assert_eq!(off.label(), b"EXPERIMENT OFF");
        assert!(!off.active);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        initialise, render_key_witness, render_witness, write_frame, DisplayBus, TransferKind,
        FRAME_BYTES, WIDTH,
    };
    use crate::receive_app::Key;
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
            Key::Digit(0),
            Key::Digit(1),
            Key::Digit(2),
            Key::Digit(3),
            Key::Digit(4),
            Key::Digit(5),
            Key::Digit(6),
            Key::Digit(7),
            Key::Digit(8),
            Key::Digit(9),
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
mod catch_screen_tests {
    use super::{draw_text, render_catch_screen, CatchView, FRAME_BYTES};
    use crate::catch::Phase;
    use radio_domain::Tone;

    fn shows(frame: &[u8; FRAME_BYTES], x: usize, y: usize, text: &[u8]) -> bool {
        let mut drawn = *frame;
        draw_text(&mut drawn, x, y, text);
        drawn == *frame
    }

    fn render(view: CatchView) -> [u8; FRAME_BYTES] {
        let mut frame = [0_u8; FRAME_BYTES];
        render_catch_screen(&mut frame, &view);
        frame
    }

    #[test]
    fn the_catch_shows_its_phase_frequency_and_tone() {
        let finding = render(CatchView {
            phase: Phase::Frequency,
            frequency_hz: None,
            tone: None,
            to_vfo: true,
        });
        assert!(shows(&finding, 0, 14, b"FINDING FREQUENCY"));
        assert!(shows(&finding, 0, 28, b"---.----- MHZ"));
        assert!(shows(&finding, 0, 42, b"TONE -----"));
        assert!(!shows(&finding, 0, 56, b"MENU"));

        let caught = render(CatchView {
            phase: Phase::Done,
            frequency_hz: Some(430_837_500),
            tone: Some(Tone::Ctcss(1567)),
            to_vfo: true,
        });
        assert!(shows(&caught, 0, 14, b"CAUGHT"));
        assert!(shows(&caught, 0, 28, b"430.83750 MHZ"));
        assert!(shows(&caught, 0, 42, b"TONE 156.7"));
        assert!(shows(&caught, 0, 56, b"MENU TO VFO"));

        let untoned = render(CatchView {
            phase: Phase::Done,
            frequency_hz: Some(145_500_000),
            tone: None,
            to_vfo: false,
        });
        assert!(shows(&untoned, 0, 42, b"TONE NONE"));
        assert!(!shows(&untoned, 0, 56, b"MENU"));
    }
}

#[cfg(test)]
mod operating_screen_tests {
    use super::{modulation_label, place_label, power_label, short_step_label};
    use super::{
        render_info_screen, render_operating_screen, render_selector_list, step_label, BootTimes,
        MemoryState, OperatingView, ResetCause, SelectorRow, SerialCounters, SlotChannel,
        SlotPlace, SlotView, FRAME_BYTES, LIST_NAME_BYTES, MEMORY_LABEL_BYTES,
    };
    use radio_domain::{Bandwidth, Modulation, Tone};

    const CHANNEL: SlotChannel<'static> = SlotChannel {
        name: b"2M CALL",
        frequency_hz: 145_500_000,
        listens_both: false,
        place: SlotPlace::Memory {
            bank: Some(3),
            number: 3,
        },
        modulation: Modulation::Fm,
        bandwidth: Bandwidth::Narrow,
        power: 0,
        meter: None,
        tx_tone: Tone::None,
        rx_tone: Tone::None,
        heard_tone: None,
    };

    fn view() -> OperatingView<'static> {
        OperatingView {
            a: SlotView {
                letter: b'A',
                selected: true,
                channel: Some(CHANNEL),
            },
            b: SlotView {
                letter: b'B',
                selected: false,
                channel: Some(SlotChannel {
                    name: b"VFO",
                    frequency_hz: 433_500_000,
                    place: SlotPlace::Vfo { step_hz: 12_500 },
                    ..CHANNEL
                }),
            },
            receiver_ok: true,
            battery_percent: Some(87),
            monitoring: false,
            scanning: false,
            dual_watch: false,
            function: false,
            entry: None,
            entry_vfo: false,
            tx_refused: None,
        }
    }

    fn with_a(channel: SlotChannel<'static>) -> OperatingView<'static> {
        let mut view = view();
        view.a.channel = Some(channel);
        view
    }

    fn render(view: &OperatingView<'_>) -> [u8; FRAME_BYTES] {
        let mut frame = [0xFF_u8; FRAME_BYTES];
        render_operating_screen(&mut frame, view);
        frame
    }

    #[test]
    fn a_channel_heard_on_both_sides_shows_dx_for_its_power() {
        use super::{draw_text, SLOT_A_Y, WIDTH};

        let shows = |frame: &[u8; FRAME_BYTES], text: &[u8]| {
            let mut drawn = *frame;
            draw_text(&mut drawn, WIDTH - 2 * 6, SLOT_A_Y + 9, text);
            drawn == *frame
        };
        let mut frame = [0_u8; FRAME_BYTES];
        render_operating_screen(&mut frame, &with_a(CHANNEL));
        assert!(shows(&frame, b"L1") && !shows(&frame, b"DX"));
        render_operating_screen(
            &mut frame,
            &with_a(SlotChannel {
                listens_both: true,
                power: 5,
                ..CHANNEL
            }),
        );
        assert!(shows(&frame, b"DX"));
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
        use super::{
            battery_label, glyph, megahertz, modulation_label, place_label, power_label,
            short_step_label, step_label,
        };
        use crate::operator_shell::VFO_STEPS_HZ;

        let mut fixed = std::vec::Vec::new();
        fixed.extend_from_slice(b"VFO");
        fixed.extend_from_slice(b"AB");
        fixed.extend_from_slice(b"F");
        fixed.extend_from_slice(b"DW");
        fixed.extend_from_slice(b"SCAN");
        fixed.extend_from_slice(b"MON");
        fixed.extend_from_slice(b"RX FAULT");
        fixed.extend_from_slice(super::TxRefusal::Disabled.label());
        fixed.extend_from_slice(super::TxRefusal::NoBand.label());
        fixed.extend_from_slice(b"NO CHANNELS");
        fixed.extend_from_slice(b"DUAL WATCH ON");
        fixed.extend_from_slice(b"DUAL WATCH OFF");
        fixed.extend_from_slice(&place_label(None, 12));
        fixed.extend_from_slice(&place_label(Some(3), 999));
        for modulation in [
            Modulation::Fm,
            Modulation::Am,
            Modulation::Usb,
            Modulation::Broadcast,
        ] {
            for bandwidth in [Bandwidth::Narrow, Bandwidth::Wide] {
                fixed.extend_from_slice(&modulation_label(modulation, bandwidth));
            }
        }
        for level in 0..crate::operator_shell::TX_POWER_LEVELS {
            fixed.extend_from_slice(&power_label(level));
        }
        fixed.extend_from_slice(b"BANK 00");
        fixed.extend_from_slice(b"SETTINGS");
        fixed.extend_from_slice(b"SQUELCH");
        fixed.extend_from_slice(b"SOURCE");
        fixed.extend_from_slice(b"STEP");
        fixed.extend_from_slice(b"0  OPEN");
        fixed.extend_from_slice(&battery_label(None));
        fixed.extend_from_slice(&battery_label(Some(16)));
        fixed.extend_from_slice(&battery_label(Some(100)));
        fixed.extend_from_slice(&megahertz(145_500_000));
        for step in VFO_STEPS_HZ {
            fixed.extend_from_slice(&step_label(step));
            fixed.extend_from_slice(&short_step_label(step));
        }
        fixed.extend_from_slice(SelectorRow::squelch_setting(0).label());
        fixed.extend_from_slice(SelectorRow::squelch_setting(5).label());
        fixed.extend_from_slice(SelectorRow::tx_power_setting(None).label());
        fixed.extend_from_slice(SelectorRow::tx_power_channel(false).label());
        for level in 0..crate::operator_shell::TX_POWER_LEVELS {
            fixed.extend_from_slice(SelectorRow::tx_power_setting(Some(level)).label());
            fixed.extend_from_slice(SelectorRow::tx_power_level(level, false).label());
        }
        for choice in 0..crate::operator_shell::TX_TIMEOUT_CHOICES {
            fixed.extend_from_slice(SelectorRow::tx_timeout_setting(choice).label());
            fixed.extend_from_slice(SelectorRow::tx_timeout_choice(choice, false).label());
        }
        fixed.extend_from_slice(SelectorRow::tx_classes_setting(1, 2).label());
        for class in crate::operator_shell::TX_CLASSES {
            fixed.extend_from_slice(SelectorRow::tx_class(class, true).label());
            fixed.extend_from_slice(SelectorRow::tx_class(class, false).label());
        }

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

        let swapped = {
            let mut view = view();
            view.a.selected = false;
            view.b.selected = true;
            view
        };
        let empty_b = {
            let mut view = view();
            view.b.channel = None;
            view
        };
        for changed in [
            swapped,
            empty_b,
            with_a(SlotChannel {
                place: SlotPlace::Memory {
                    bank: Some(3),
                    number: 4,
                },
                ..CHANNEL
            }),
            with_a(SlotChannel {
                place: SlotPlace::Memory {
                    bank: Some(2),
                    number: 3,
                },
                ..CHANNEL
            }),
            with_a(SlotChannel {
                place: SlotPlace::Memory {
                    bank: None,
                    number: 3,
                },
                ..CHANNEL
            }),
            with_a(SlotChannel {
                frequency_hz: 145_512_500,
                ..CHANNEL
            }),
            with_a(SlotChannel {
                bandwidth: Bandwidth::Wide,
                ..CHANNEL
            }),
            with_a(SlotChannel {
                modulation: Modulation::Am,
                ..CHANNEL
            }),
            with_a(SlotChannel {
                power: 5,
                ..CHANNEL
            }),
            with_a(SlotChannel {
                meter: Some(100),
                ..CHANNEL
            }),
            OperatingView {
                receiver_ok: false,
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
                dual_watch: true,
                ..view()
            },
            OperatingView {
                function: true,
                ..view()
            },
            OperatingView {
                entry: Some(7),
                ..view()
            },
        ] {
            assert_ne!(base, render(&changed), "state must be visible on screen");
        }
    }

    /// A PTT press the TX policy refused says so in place of whatever the
    /// status row held, typed digits included.
    #[test]
    fn a_refused_transmission_says_why() {
        use super::TxRefusal;
        let base = render(&view());
        let disabled = render(&OperatingView {
            tx_refused: Some(TxRefusal::Disabled),
            ..view()
        });
        let no_band = render(&OperatingView {
            tx_refused: Some(TxRefusal::NoBand),
            ..view()
        });
        assert_ne!(base, disabled);
        assert_ne!(disabled, no_band);
        let typing_refused = render(&OperatingView {
            tx_refused: Some(TxRefusal::Disabled),
            entry: Some(7),
            ..view()
        });
        assert_eq!(disabled, typing_refused);
    }

    /// The meter is the squelch indicator: present while open, absent while
    /// shut, and longer for a stronger carrier.
    #[test]
    fn the_meter_is_drawn_only_while_the_squelch_is_open_and_grows_with_the_carrier() {
        let shut = render(&view());
        let lit =
            |frame: &[u8; FRAME_BYTES]| -> u32 { frame.iter().map(|byte| byte.count_ones()).sum() };
        let weak = render(&with_a(SlotChannel {
            meter: Some(0),
            ..CHANNEL
        }));
        let middle = render(&with_a(SlotChannel {
            meter: Some(2 * 70), // -90 dBm
            ..CHANNEL
        }));
        let strong = render(&with_a(SlotChannel {
            meter: Some(2 * 120), // -40 dBm
            ..CHANNEL
        }));
        assert!(
            lit(&shut) < lit(&weak),
            "an open squelch always draws a segment"
        );
        assert!(lit(&weak) < lit(&middle));
        assert!(lit(&middle) < lit(&strong));
    }

    /// The meter reads in S units and dBm with the band's correction applied:
    /// count 140 is -90 dBm at the chip, -110 dBm at 2 m, which is S6.
    #[test]
    fn the_meter_shows_the_calibrated_s_reading_and_dbm() {
        use super::{draw_text, SLOT_A_Y, SLOT_TEXT_X, WIDTH};
        const METER_LABEL_X: usize = SLOT_TEXT_X + 15 * 4 + 2;

        let shows = |frame: &[u8; FRAME_BYTES], x: usize, text: &[u8]| {
            let mut drawn = *frame;
            draw_text(&mut drawn, x, SLOT_A_Y + 18, text);
            drawn == *frame
        };
        let frame = |meter| {
            let mut frame = [0_u8; FRAME_BYTES];
            render_operating_screen(
                &mut frame,
                &with_a(SlotChannel {
                    meter: Some(meter),
                    ..CHANNEL
                }),
            );
            frame
        };
        let s6 = frame(140);
        assert!(shows(&s6, METER_LABEL_X, b"S6"));
        assert!(shows(&s6, WIDTH - 4 * 6, b"-110"));
        assert!(!shows(&s6, METER_LABEL_X, b"S9+20"));
        // -50 dBm at the chip is -70 dBm at 2 m, 23 dB over S9.
        let over = frame(220);
        assert!(shows(&over, METER_LABEL_X, b"S9+20"));
        assert!(shows(&over, WIDTH - 3 * 6, b"-70"));
    }

    /// The tone row shows what the channel sends and requires; on an open
    /// squelch the tone that opened it takes the dBm's place.
    #[test]
    fn tones_show_on_the_tone_row_and_beside_the_meter() {
        use super::{draw_text, tone_label, SLOT_A_Y, SLOT_TEXT_X, WIDTH};

        assert_eq!(&tone_label(Tone::Ctcss(885)), b" 88.5");
        assert_eq!(&tone_label(Tone::Ctcss(1_230)), b"123.0");
        assert_eq!(
            &tone_label(Tone::Dcs {
                code: 23,
                inverted: false
            }),
            b"D023N"
        );
        assert_eq!(
            &tone_label(Tone::Dcs {
                code: 754,
                inverted: true
            }),
            b"D754I"
        );

        let shows = |frame: &[u8; FRAME_BYTES], x: usize, text: &[u8]| {
            let mut drawn = *frame;
            draw_text(&mut drawn, x, SLOT_A_Y + 18, text);
            drawn == *frame
        };
        let render = |channel| {
            let mut frame = [0_u8; FRAME_BYTES];
            render_operating_screen(&mut frame, &with_a(channel));
            frame
        };
        let repeater = SlotChannel {
            tx_tone: Tone::Ctcss(770),
            ..CHANNEL
        };
        let idle = render(repeater);
        assert!(shows(&idle, SLOT_TEXT_X, b"T 77.0"));
        assert!(!shows(&idle, SLOT_TEXT_X + 42, b"R"));
        let toned = render(SlotChannel {
            rx_tone: Tone::Ctcss(770),
            ..repeater
        });
        assert!(shows(&toned, SLOT_TEXT_X + 42, b"R 77.0"));
        // No tone anywhere: an empty row, as before.
        assert_eq!(
            render(CHANNEL),
            render(SlotChannel {
                heard_tone: None,
                ..CHANNEL
            })
        );

        // Open on a channel without a receive tone: the heard tone, not dBm.
        let heard = render(SlotChannel {
            meter: Some(140),
            heard_tone: Some(Tone::Ctcss(1_230)),
            ..repeater
        });
        assert!(shows(&heard, WIDTH - 5 * 6, b"123.0"));
        assert!(shows(&heard, SLOT_TEXT_X + 15 * 3 + 2, b"S6"));
        assert!(!shows(&heard, SLOT_TEXT_X, b"T"));
        // The receive tone wins over anything heard.
        let filtered = render(SlotChannel {
            meter: Some(140),
            rx_tone: Tone::Dcs {
                code: 23,
                inverted: false,
            },
            heard_tone: Some(Tone::Ctcss(1_230)),
            ..repeater
        });
        assert!(shows(&filtered, WIDTH - 5 * 6, b"D023N"));
        // Nothing heard: the dBm reading stays.
        let plain = render(SlotChannel {
            meter: Some(140),
            ..CHANNEL
        });
        assert!(shows(&plain, WIDTH - 4 * 6, b"-110"));
    }

    #[test]
    fn slot_labels_name_bank_channel_mode_power_and_step() {
        assert_eq!(&place_label(Some(3), 12), b"B03 012");
        assert_eq!(&place_label(None, 7), b"B-- 007");
        assert_eq!(&modulation_label(Modulation::Fm, Bandwidth::Narrow), b"NFM");
        assert_eq!(&modulation_label(Modulation::Fm, Bandwidth::Wide), b" FM");
        assert_eq!(&modulation_label(Modulation::Am, Bandwidth::Narrow), b" AM");
        assert_eq!(&power_label(0), b"L1");
        assert_eq!(&power_label(3), b"L4");
        assert_eq!(&power_label(4), b" M");
        assert_eq!(&power_label(5), b" H");
        assert_eq!(&short_step_label(6_250), b"6.25K");
        assert_eq!(
            SelectorRow::vfo_step_setting(6_250).label(),
            b"VFO STEP 6.25K"
        );
        assert_eq!(&short_step_label(12_500), b"12.5K");
        assert_eq!(&short_step_label(25_000), b"25K  ");
        assert_eq!(&short_step_label(100_000), b"100K ");
        assert_eq!(&short_step_label(1_000_000), b"1M   ");
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
    fn the_vfo_slot_shows_its_step_instead_of_a_bank_and_number() {
        let memory = render(&view());
        let vfo = render(&with_a(SlotChannel {
            name: b"VFO",
            place: SlotPlace::Vfo { step_hz: 12_500 },
            ..CHANNEL
        }));
        assert_ne!(memory, vfo);
        let coarser = render(&with_a(SlotChannel {
            name: b"VFO",
            place: SlotPlace::Vfo { step_hz: 1_000_000 },
            ..CHANNEL
        }));
        assert_ne!(vfo, coarser, "the step in force is visible");

        // Typed kilohertz digits are shown while they are being entered.
        let typing = render(&OperatingView {
            entry: Some(4335),
            entry_vfo: true,
            ..with_a(SlotChannel {
                name: b"VFO",
                place: SlotPlace::Vfo { step_hz: 12_500 },
                ..CHANNEL
            })
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
            BootTimes::default(),
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
            BootTimes::default(),
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
            BootTimes::default(),
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
            BootTimes::default(),
        );
        assert_ne!(silent, erroring);
    }

    #[test]
    fn boot_timings_reach_the_information_screen() {
        let render = |boot| {
            let mut frame = [0_u8; FRAME_BYTES];
            render_info_screen(
                &mut frame,
                b"AFIK-K5-7.0",
                0,
                0,
                false,
                MemoryState::Absent,
                SerialCounters::default(),
                48_000,
                ResetCause::default(),
                boot,
            );
            frame
        };
        let early = render(BootTimes {
            serial_start_ms: 40,
            loaded_ms: 300,
            first_frame_ms: 60,
        });
        let late = render(BootTimes {
            serial_start_ms: 40,
            loaded_ms: 1_500,
            first_frame_ms: 60,
        });
        assert_ne!(early, late);
        assert_ne!(render(BootTimes::default()), early);
        // A field keeps the low digits of a larger value.
        let mut field = [0_u8; 4];
        super::write_digits(&mut field, 12_345);
        assert_eq!(&field, b"2345");
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
            BootTimes::default(),
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
            BootTimes::default(),
        );
        assert_ne!(power, software);
    }
}
