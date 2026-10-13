//! SARADC battery-sense conversions, per `EVID-K5-034`.
//!
//! The configuration is the reference firmware's `BOARD_ADC_Init` copied field
//! for field: channels four (pack voltage) and nine (pack current), eight-sample
//! hardware averaging, single CPU-triggered conversions, per-channel result
//! registers, the internal sample clock at half the system clock with a
//! fifteen-cycle window, factory offset and gain calibration enabled, and no
//! interrupt or DMA. Only the reading of channel four is used.

use crate::mmio::Register;
use crate::{SARADC_BASE, SYSCON_BASE};

const CFG: Register = Register::new(SARADC_BASE, 0x00);
const START: Register = Register::new(SARADC_BASE, 0x04);
const IE: Register = Register::new(SARADC_BASE, 0x08);
const IF: Register = Register::new(SARADC_BASE, 0x0C);
const EXTTRIG_SEL: Register = Register::new(SARADC_BASE, 0xB0);
const CALIB_OFFSET: Register = Register::new(SARADC_BASE, 0xF0);
const CALIB_KD: Register = Register::new(SARADC_BASE, 0xF4);
const CLK_SEL: Register = Register::new(SYSCON_BASE, 0x00);

/// The channel the pack voltage divider is wired to.
pub const VOLTAGE_CHANNEL: u32 = 4;
/// The channel the pack current sense is wired to; conversion ends on it.
pub const CURRENT_CHANNEL: u32 = 9;

const CFG_ENABLE: u32 = 1 << 27;
/// Every field the reference configuration replaces: `CH_SEL[14:0]`,
/// `AVG[17:16]`, `CONT[18]`, `SMPL_SETUP[21:19]`, `MEM_MODE[22]`,
/// `SMPL_CLK[23]`, `SMPL_WIN[26:24]`, `ADC_TRIG[28]` and `DMA_EN[29]`.
const CFG_FIELDS: u32 = 0x7FFF
    | (0x3 << 16)
    | (1 << 18)
    | (0x7 << 19)
    | (1 << 22)
    | (1 << 23)
    | (0x7 << 24)
    | (1 << 28)
    | (1 << 29);
const START_START: u32 = 1 << 0;
const START_SOFT_RESET: u32 = 1 << 2;
const CALIB_VALID: u32 = 1 << 16;
const IE_FIELDS: u32 = 0xFFFF | (1 << 16) | (1 << 17);
const CHANNEL_EOC: u32 = 1 << 0;
const CHANNEL_DATA: u32 = 0xFFF;

/// Polls of the end-of-conversion flag before a reading is abandoned.
///
/// Eight averaged samples of two channels take a few hundred system clocks;
/// this bound is far beyond that and exists only so a converter which never
/// answers cannot hang the main loop.
const CONVERSION_POLLS: u32 = 100_000;

/// The `CFG` value `BOARD_ADC_Init` writes, before the enable bit.
#[must_use]
pub const fn configuration(current: u32) -> u32 {
    let channels = (1 << VOLTAGE_CHANNEL) | (1 << CURRENT_CHANNEL);
    let average_eight = 3 << 16;
    let single = 0 << 18;
    let setup_one_cycle = 0 << 19;
    let per_channel_memory = 1 << 22;
    let internal_clock = 1 << 23;
    let window_fifteen_cycles = 7 << 24;
    let cpu_trigger = 0 << 28;
    let no_dma = 0 << 29;
    (current & !CFG_FIELDS)
        | channels
        | average_eight
        | single
        | setup_one_cycle
        | per_channel_memory
        | internal_clock
        | window_fifteen_cycles
        | cpu_trigger
        | no_dma
}

/// The `SYSCON_CLK_SEL` value that selects a half-rate SARADC sample clock.
///
/// The register reads back some fields at different positions from those it
/// is written at: the PLL source reads at bit 11 and writes at bit 7, and the
/// reference firmware moves the sample-clock divider it reads at `[8:7]` to its
/// write position `[11:10]`. This repeats that transform exactly, then sets the
/// divider to two.
#[must_use]
pub const fn clock_selection(read: u32) -> u32 {
    let read_pll = (read >> 11) & 1;
    let read_divider = (read >> 7) & 3;
    let moved = (read & !((1 << 11) | (3 << 7))) | (read_pll << 7) | (read_divider << 10);
    let divide_by_two = 1;
    (moved & !(3 << 10)) | (divide_by_two << 10)
}

const fn channel_status(channel: u32) -> Register {
    Register::new(SARADC_BASE, 0x10 + channel * 8)
}

const fn channel_data(channel: u32) -> Register {
    Register::new(SARADC_BASE, 0x14 + channel * 8)
}

/// Configures, enables and resets the converter, as `BOARD_ADC_Init` does.
///
/// The SARADC clock gate and the PA9/PA14 pin functions are the caller's.
pub fn initialise() {
    CFG.modify(|value| value & !CFG_ENABLE);
    CLK_SEL.write(clock_selection(CLK_SEL.read()));
    CFG.modify(configuration);
    EXTTRIG_SEL.write(0);
    CALIB_OFFSET.modify(|value| value | CALIB_VALID);
    CALIB_KD.modify(|value| value | CALIB_VALID);
    IF.write(0xFFFF_FFFF);
    IE.modify(|value| value & !IE_FIELDS);
    CFG.modify(|value| value | CFG_ENABLE);
    START.modify(|value| value & !START_SOFT_RESET);
    START.modify(|value| value | START_SOFT_RESET);
}

/// Takes one averaged conversion of the pack voltage channel.
///
/// Returns `None` if the conversion never completes, which the battery
/// arithmetic treats like any other missing reading.
pub fn read_voltage() -> Option<u16> {
    START.modify(|value| value | START_START);
    let mut polls = 0;
    while channel_status(CURRENT_CHANNEL).read() & CHANNEL_EOC == 0 {
        polls += 1;
        if polls >= CONVERSION_POLLS {
            return None;
        }
    }
    IF.write(1 << VOLTAGE_CHANNEL);
    let voltage = channel_data(VOLTAGE_CHANNEL).read() & CHANNEL_DATA;
    IF.write(1 << CURRENT_CHANNEL);
    let _ = channel_data(CURRENT_CHANNEL).read();
    u16::try_from(voltage).ok()
}

#[cfg(test)]
mod tests {
    use super::{channel_data, channel_status, clock_selection, configuration};

    #[test]
    fn the_configuration_matches_board_adc_init() {
        // Channels 4 and 9, AVG=3, MEM_MODE=1, SMPL_CLK=1, SMPL_WIN=7.
        assert_eq!(configuration(0), 0x07C3_0210);
        // Bits outside the replaced fields, such as the enable, survive.
        assert_eq!(configuration(0xFFFF_FFFF), 0xCFC3_8210);
    }

    #[test]
    fn the_clock_selection_moves_read_fields_and_divides_by_two() {
        assert_eq!(clock_selection(0), 1 << 10);
        // PLL source read at bit 11 is written back at bit 7.
        assert_eq!(clock_selection(1 << 11), (1 << 7) | (1 << 10));
        // The system clock select at bit 0 is left as found.
        assert_eq!(clock_selection(1), 1 | (1 << 10));
    }

    #[test]
    fn channel_registers_are_eight_bytes_apart_from_0x10() {
        assert_eq!(channel_status(4).address(), 0x400B_A030);
        assert_eq!(channel_data(4).address(), 0x400B_A034);
        assert_eq!(channel_status(9).address(), 0x400B_A058);
    }
}
