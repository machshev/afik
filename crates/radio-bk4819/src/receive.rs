//! Evidence-bounded BK4819 receive configuration and metering.
//!
//! Every register address and value in this module is taken from the pinned
//! K1 reference firmware recorded in `docs/hardware-evidence.md`. Nothing here
//! can request transmit mode: the only mode-control word written is the
//! documented receive block.

use core::fmt;

use radio_domain::{Bandwidth, Frequency, Modulation, SquelchLevel, Tone};

use crate::{
    Bk4819, DriverError, DriverState, FrequencyWord, RegisterAddress, RegisterBus, MODE_STANDBY,
    REG_FREQUENCY_HIGH, REG_FREQUENCY_LOW, REG_MODE_CONTROL,
};

const REG_SOFT_RESET: RegisterAddress = RegisterAddress::known(0x00);
const REG_INTERRUPT_FLAGS: RegisterAddress = RegisterAddress::known(0x02);
const REG_DTMF_COEFFICIENT: RegisterAddress = RegisterAddress::known(0x09);
const REG_CRYSTAL: RegisterAddress = RegisterAddress::known(0x36);
const REG_MIC_GAIN: RegisterAddress = RegisterAddress::known(0x7D);
pub(crate) const REG_AGC_CONTROL: RegisterAddress = RegisterAddress::known(0x7E);
const REG_SUB_AUDIO_FREQUENCY: RegisterAddress = RegisterAddress::known(0x07);
const REG_CDCSS_CODE: RegisterAddress = RegisterAddress::known(0x08);
const REG_DEMODULATOR_2A: RegisterAddress = RegisterAddress::known(0x2A);
const REG_DEMODULATOR_2B: RegisterAddress = RegisterAddress::known(0x2B);
const REG_DEMODULATOR_2F: RegisterAddress = RegisterAddress::known(0x2F);
pub(crate) const REG_AM_DEMODULATOR: RegisterAddress = RegisterAddress::known(0x31);
pub(crate) const REG_GPIO_OUT: RegisterAddress = RegisterAddress::known(0x33);
pub(crate) const REG_POWER_BLOCKS: RegisterAddress = RegisterAddress::known(0x37);
const REG_AF_TAIL: RegisterAddress = RegisterAddress::known(0x3D);
const REG_INTERRUPT_MASK: RegisterAddress = RegisterAddress::known(0x3F);
const REG_DEMODULATOR_42: RegisterAddress = RegisterAddress::known(0x42);
pub(crate) const REG_FILTER_BANDWIDTH: RegisterAddress = RegisterAddress::known(0x43);
const REG_AF_OUTPUT: RegisterAddress = RegisterAddress::known(0x47);
const REG_AF_DAC_GAIN: RegisterAddress = RegisterAddress::known(0x48);
/// Low nibble of `REG_48`, the AF DAC gain applied after the receive gains.
const AF_DAC_GAIN_MASK: u16 = 0x000F;
/// Maximum AF DAC gain, the value the pinned source selects for every mode.
const AF_DAC_GAIN_MAXIMUM: u16 = 0x000F;
const REG_SQUELCH_CLOSE_GLITCH: RegisterAddress = RegisterAddress::known(0x4D);
const REG_SQUELCH_OPEN_GLITCH: RegisterAddress = RegisterAddress::known(0x4E);
const REG_SQUELCH_NOISE: RegisterAddress = RegisterAddress::known(0x4F);
pub(crate) const REG_SUB_AUDIO_CONTROL: RegisterAddress = RegisterAddress::known(0x51);
const REG_AUDIO_54: RegisterAddress = RegisterAddress::known(0x54);
const REG_AUDIO_55: RegisterAddress = RegisterAddress::known(0x55);
const REG_GLITCH: RegisterAddress = RegisterAddress::known(0x63);
const REG_NOISE: RegisterAddress = RegisterAddress::known(0x65);
const REG_AFC: RegisterAddress = RegisterAddress::known(0x73);
const REG_SQUELCH_RSSI: RegisterAddress = RegisterAddress::known(0x78);

const SOFT_RESET_ASSERT: u16 = 0x8000;
const SOFT_RESET_RELEASE: u16 = 0x0000;
const CRYSTAL_INIT: u16 = 0x0022;
const AGC_FIX_INDEX: u16 = 3 << 12;
const AGC_AUTO_MODE_BIT: u16 = 1 << 15;
const AGC_INDEX_MASK: u16 = 0b111 << 12;
/// `REG_37` written by `BK4819_Sleep`, the same in the K5 and K1 trees.
const SLEEP_POWER_BLOCKS: u16 = 0x1D00;
/// Default `REG_33` output word the pinned source establishes at power on.
pub const GPIO_OUT_DEFAULT: u16 = 0x9000;
/// Receive-side DTMF detection coefficients written to `REG_09` at power on.
const DTMF_COEFFICIENTS: [u8; 16] = [
    111, 107, 103, 98, 80, 71, 58, 44, 65, 55, 37, 23, 228, 203, 181, 159,
];

const AF_MUTE: u16 = 0;
const AF_FM: u16 = 1;
const AF_BASEBAND2: u16 = 5;

const SUB_AUDIO_MODE_CTC1: u16 = 0;
/// `REG_07` bits 15:13 = 1: CTC2, the receive tail detector.
const SUB_AUDIO_MODE_CTC2: u16 = 1 << 13;
/// `BK4819_SetTailDetection(550)`: QS's 55 Hz tail tone, `(253910 + 275) /
/// 550` for the 26 MHz crystal. Both pinned trees use it on untoned channels.
const TAIL_DETECTION_55_HZ: u16 = 462;
const CDCSS_BAUD_CONTROL_WORD: u16 = 2775;
const CDCSS_HIGH_HALF: u16 = 1 << 15;
const CTCSS_CONTROL_NUMERATOR: u32 = 206_488;
const CTCSS_CONTROL_ROUNDING: u32 = 50_000;
const CTCSS_CONTROL_DENOMINATOR: u32 = 100_000;
const CTCSS_CONTROL_MASK: u32 = 0x1FFF;
const CDCSS_HALF_MASK: u32 = 0x0FFF;
const CDCSS_INVERT_MASK: u32 = 0x007F_FFFF;
const GOLAY_GENERATOR: u32 = 0x08EA;

const GLITCH_MASK: u16 = 0x00FF;
const NOISE_MASK: u16 = 0x007F;

const INTERRUPT_MASK_SQUELCH_AND_TONE: u16 = (1 << 3) | (1 << 2) | (1 << 10);
const INTERRUPT_MASK_CTCSS: u16 = (1 << 7) | (1 << 6);
const INTERRUPT_MASK_CDCSS: u16 = (1 << 9) | (1 << 8);
const INTERRUPT_CTCSS_FOUND: u16 = 1 << 7;
const INTERRUPT_CTCSS_LOST: u16 = 1 << 6;
const INTERRUPT_CDCSS_FOUND: u16 = 1 << 9;
const INTERRUPT_CDCSS_LOST: u16 = 1 << 8;
/// `REG_0C` bit 0: the chip has an interrupt request pending.
const INTERRUPT_REQUEST: u16 = 1;
/// Most pending interrupts serviced in one sample; the source loops without
/// a limit (`RISK` on the receive interrupt-clear loop).
const MAX_INTERRUPTS_PER_SAMPLE: usize = 8;

/// `REG_51` from `BK4819_SetScanFrequency` in both pinned trees: sub-audio TX
/// off, CDCSS mode, both auto-bandwidth modes disabled. With it the chip
/// reports any tone it hears in `REG_68`/`REG_69`. The reference writes it
/// only for its tone scan, never for ordinary receive: on `7.0.28`–`7.0.30`
/// untoned channels used it and a toned signal (GB3EL's output) went silent.
const SUB_AUDIO_SCAN: u16 = (1 << 9) | (1 << 8);
const REG_CTCSS_SCAN: RegisterAddress = RegisterAddress::known(0x68);
/// `REG_32`, frequency scan control: `BK4819_EnableFrequencyScan` and
/// `BK4819_DisableFrequencyScan`, 0.2 s scan time, enable in bit 0.
const REG_FREQUENCY_SCAN: RegisterAddress = RegisterAddress::known(0x32);
const FREQUENCY_SCAN_OFF: u16 = 290 << 1;
const FREQUENCY_SCAN_ON: u16 = FREQUENCY_SCAN_OFF | 1;
/// `REG_0D`/`REG_0E`, the frequency scan result in 10 Hz units; `REG_0D`
/// bit 15 is set while the scan is still running.
const REG_FREQUENCY_SCAN_HIGH: RegisterAddress = RegisterAddress::known(0x0D);
const REG_FREQUENCY_SCAN_LOW: RegisterAddress = RegisterAddress::known(0x0E);
const REG_CDCSS_SCAN_HIGH: RegisterAddress = RegisterAddress::known(0x69);
const REG_CDCSS_SCAN_LOW: RegisterAddress = RegisterAddress::known(0x6A);
/// `REG_68`/`REG_69` bit 15 is clear when the scan found a tone or code.
const SCAN_NOT_FOUND: u16 = 1 << 15;
/// `BK4819_GetCxCSSScanResult`: tenths of a hertz = word * 4843 / 10000.
const CTCSS_SCAN_NUMERATOR: u32 = 4_843;
const CTCSS_SCAN_DENOMINATOR: u32 = 10_000;
/// `DCS_GetCtcssCode` accepts the nearest tone only within 5 Hz.
const CTCSS_SCAN_TOLERANCE_TENTHS: u16 = 50;

/// `CTCSS_Options` in both pinned trees, in tenths of a hertz. Only its
/// packed form, [`CTCSS_STEPS`], is kept in the image.
const CTCSS_TONES: [u16; 50] = [
    670, 693, 719, 744, 770, 797, 825, 854, 885, 915, 948, 974, 1000, 1035, 1072, 1109, 1148, 1188,
    1230, 1273, 1318, 1365, 1413, 1462, 1514, 1567, 1598, 1622, 1655, 1679, 1713, 1738, 1773, 1799,
    1835, 1862, 1899, 1928, 1966, 1995, 2035, 2065, 2107, 2181, 2257, 2291, 2336, 2418, 2503, 2541,
];

/// `DCS_Options` in both pinned trees: each code's nine bits, three per
/// octal digit. Only its packed form, [`DCS_STANDARD`], is kept in the image.
const DCS_CODES: [u16; 104] = [
    0x013, 0x015, 0x016, 0x019, 0x01A, 0x01E, 0x023, 0x027, 0x029, 0x02B, 0x02C, 0x035, 0x039,
    0x03A, 0x03B, 0x03C, 0x04C, 0x04D, 0x04E, 0x052, 0x055, 0x059, 0x05A, 0x05C, 0x063, 0x065,
    0x06A, 0x06D, 0x06E, 0x072, 0x075, 0x07A, 0x07C, 0x085, 0x08A, 0x093, 0x095, 0x096, 0x0A3,
    0x0A4, 0x0A5, 0x0A6, 0x0A9, 0x0AA, 0x0AD, 0x0B1, 0x0B3, 0x0B5, 0x0B6, 0x0B9, 0x0BC, 0x0C6,
    0x0C9, 0x0CD, 0x0D5, 0x0D9, 0x0DA, 0x0E3, 0x0E6, 0x0E9, 0x0EE, 0x0F4, 0x0F5, 0x0F9, 0x109,
    0x10A, 0x10B, 0x113, 0x119, 0x11A, 0x125, 0x126, 0x12A, 0x12C, 0x12D, 0x132, 0x134, 0x135,
    0x136, 0x143, 0x146, 0x14E, 0x153, 0x156, 0x15A, 0x166, 0x175, 0x186, 0x18A, 0x194, 0x197,
    0x199, 0x19A, 0x1AC, 0x1B2, 0x1B4, 0x1C3, 0x1CA, 0x1D3, 0x1D9, 0x1DA, 0x1DC, 0x1E3, 0x1EC,
];

/// The lowest CTCSS tone and each next tone's rise over the one before; no
/// rise in the table exceeds 8.5 Hz, so each fits a byte.
const FIRST_CTCSS: u16 = CTCSS_TONES[0];
const CTCSS_STEPS: [u8; CTCSS_TONES.len() - 1] = {
    let mut steps = [0; CTCSS_TONES.len() - 1];
    let mut index = 0;
    while index < steps.len() {
        let rise = CTCSS_TONES[index + 1] - CTCSS_TONES[index];
        assert!(rise > 0 && rise <= u8::MAX as u16);
        // Checked just above; `try_from` is not available in a constant.
        #[allow(clippy::cast_possible_truncation)]
        {
            steps[index] = rise as u8;
        }
        index += 1;
    }
    steps
};

/// One bit for each of the 512 nine-bit codes, set for the standard ones.
const DCS_STANDARD: [u8; 64] = {
    let mut bits = [0; 64];
    let mut index = 0;
    while index < DCS_CODES.len() {
        let code = DCS_CODES[index] as usize;
        bits[code / 8] |= 1 << (code % 8);
        index += 1;
    }
    bits
};

/// Whether `bits` is one of the standard DCS codes.
fn is_standard_dcs(bits: u16) -> bool {
    DCS_STANDARD
        .get(usize::from(bits / 8))
        .is_some_and(|byte| byte & (1 << (bits % 8)) != 0)
}

/// `REG_33` bit for logical GPIO0, `BK4819_GPIO0_PIN28_RX_ENABLE`.
pub(crate) const GPIO_RX_ENABLE: u16 = 0x40;
const GPIO_UHF_LNA: u16 = 0x40 >> 3;
const GPIO_VHF_LNA: u16 = 0x40 >> 4;
/// Boundary between the VHF and UHF receive filter paths in hertz.
///
/// The pinned source compares `28000000` against a frequency held in 10 Hz
/// units, so the split is 280 MHz, not 28 MHz.
pub const RECEIVE_FILTER_PATH_BOUNDARY_HZ: u32 = 280_000_000;

/// One chip variant's differing receive-path register values.
///
/// The pinned K1 tree ships two drivers with the same three-wire bus and the
/// same register map, but materially different values. `EVID-BK4829-055`
/// records which values differ; nothing here is inferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChipProfile {
    /// `REG_37` written during power-on initialisation.
    pub init_power_blocks: u16,
    /// `REG_37` written as part of the receive turn-on.
    pub receive_power_blocks: u16,
    /// `REG_30` receive mode block.
    pub receive_mode: u16,
    /// Fixed `REG_47` bits the audio type is combined with.
    pub af_fixed_bits: u16,
    /// `REG_48` audio level written during initialisation.
    pub af_level: u16,
    /// `REG_7D` microphone gain written during initialisation.
    pub mic_gain: u16,
    /// `REG_43` wide filter value.
    pub bandwidth_wide: u16,
    /// `REG_43` narrow filter value.
    pub bandwidth_narrow: u16,
    /// `REG_51` value enabling CTCSS decoding.
    pub sub_audio_ctcss: u16,
    /// `REG_51` value enabling CDCSS decoding.
    pub sub_audio_cdcss: u16,
    /// Gain table written for frequency-modulated reception.
    pub agc_fm: &'static [(u8, u16)],
    /// Gain table written for amplitude-modulated reception.
    pub agc_am: &'static [(u8, u16)],
    /// Whether initialisation switches the gain control to automatic mode.
    pub agc_auto_after_init: bool,
    /// Remaining initialisation writes in their source order.
    pub extra_init: &'static [(u8, u16)],
    /// `REG_50` written by `BK4819_ExitTxMute` before chip TX.
    pub tx_unmute: u16,
    /// `REG_37` written by `BK4819_TxOn_Beep`.
    pub tx_power_blocks: u16,
    /// Whether `BK4819_TxOn_Beep` zeroes `REG_36` before entering chip TX.
    pub tx_clears_bias_first: bool,
    /// `REG_33` bit driving the red LED (`0x40 >> pin`).
    pub gpio_red_led: u16,
    /// Whether receive setup writes `REG_2A/2B/2F/42/54/55`. The machshev
    /// K5 and the Armel K1 sources do; the Armel K5 V1 build never writes
    /// them, and transmits louder speech than AFIK did with them written.
    pub demodulator_tuning: bool,
    /// `REG_7D` microphone gain in force for TX. Armel writes
    /// `0xE940 | gMicGain_dB2[MIC_SENSITIVITY]` in every receive setup, which
    /// precedes TX; this is that word at the default sensitivity, 4.
    pub tx_mic_gain: u16,
}

const BK4819_AGC_FM: [(u8, u16); 8] = [
    (0x13, 0x03BE),
    (0x12, 0x037B),
    (0x11, 0x027B),
    (0x10, 0x007A),
    (0x14, 0x0019),
    (0x49, 0x2A38),
    (0x7B, 0x8420),
    (0x7E, 0x0000),
];

const BK4819_AGC_AM: [(u8, u16); 8] = [
    (0x13, 0x03BE),
    (0x12, 0x037B),
    (0x11, 0x027B),
    (0x10, 0x007A),
    (0x14, 0x0000),
    (0x49, 0x1920),
    (0x7B, 0x8420),
    (0x7E, 0x0000),
];

const BK4819_EXTRA_INIT: [(u8, u16); 3] = [(0x19, 0x1041), (0x1F, 0x5454), (0x3E, 0xA037)];

const BK4829_AGC: [(u8, u16); 7] = [
    (0x10, 0x0318),
    (0x11, 0x033A),
    (0x12, 0x03DB),
    (0x13, 0x03DF),
    (0x14, 0x0210),
    (0x49, 0x2AB2),
    (0x7B, 0x73DC),
];

const BK4829_EXTRA_INIT: [(u8, u16); 19] = [
    (0x40, 0x3516),
    (0x1C, 0x07C0),
    (0x1D, 0xE555),
    (0x1E, 0x4C58),
    (0x1F, 0xC65A),
    (0x3E, 0x94C6),
    (0x73, 0x4691),
    (0x77, 0x88EF),
    (0x19, 0x1041),
    (0x28, 0x0B40),
    (0x29, 0xAA00),
    (0x2A, 0x6600),
    (0x2C, 0x1822),
    (0x2F, 0x9890),
    (0x53, 0x2028),
    (0x7E, 0x303E),
    (0x46, 0x600A),
    (0x4A, 0x5430),
    (0x07, 0x61CE),
];

/// Values recorded for the BK4819 driver in the pinned tree.
pub const BK4819_PROFILE: ChipProfile = ChipProfile {
    init_power_blocks: 0x1D0F,
    receive_power_blocks: 0x1F0F,
    receive_mode: 0xBEF1,
    af_fixed_bits: (6 << 12) | (1 << 6),
    af_level: 0xB3A8,
    mic_gain: 0xE940,
    bandwidth_wide: 0x3628,
    bandwidth_narrow: 0x3648,
    sub_audio_ctcss: 0x904A,
    sub_audio_cdcss: 0x8033,
    agc_fm: &BK4819_AGC_FM,
    agc_am: &BK4819_AGC_AM,
    agc_auto_after_init: true,
    extra_init: &BK4819_EXTRA_INIT,
    tx_unmute: 0x3B20,
    tx_power_blocks: 0x1D0F,
    tx_clears_bias_first: false,
    // Armel K5 V1 `BK4819_GPIO5_PIN1_RED = 5`.
    gpio_red_led: 0x40 >> 5,
    // Armel K5 leaves these registers at their reset values.
    demodulator_tuning: false,
    // K5 `radio.c:737`, 5-bit field; `gMicGain_dB2[4] = 31`.
    tx_mic_gain: 0xE940 | 31,
};

/// Values recorded for the BK4829 driver the pinned K1 build actually compiles.
pub const BK4829_PROFILE: ChipProfile = ChipProfile {
    init_power_blocks: 0x9D1F,
    receive_power_blocks: 0x9F1F,
    receive_mode: 0xBFF1,
    af_fixed_bits: 0x6042,
    af_level: 0x33A8,
    mic_gain: 0xE920,
    bandwidth_wide: 0x3028,
    bandwidth_narrow: 0x4048,
    sub_audio_ctcss: 0x9040,
    sub_audio_cdcss: 0xA033,
    agc_fm: &BK4829_AGC,
    agc_am: &BK4829_AGC,
    agc_auto_after_init: false,
    extra_init: &BK4829_EXTRA_INIT,
    tx_unmute: 0x3B18,
    tx_power_blocks: 0x9D1F,
    tx_clears_bias_first: true,
    // Armel K1 `BK4819_GPIO5_PIN1_RED = 6`.
    gpio_red_led: 0x40 >> 6,
    demodulator_tuning: true,
    // K1 `radio.c:798`, 6-bit field; `gMicGain_dB2[4] = 32`.
    tx_mic_gain: 0xE940 | 32,
};

/// Requested audio-frequency output routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AfOutput {
    /// The demodulator runs but no audio reaches the output.
    Mute,
    /// Demodulated audio reaches the output.
    Demodulated,
}

/// RSSI count at which squelch level one opens, about -130 dBm.
///
/// This is the offset the per-level count is built from, so the constant itself
/// is one step below level one.
const SQUELCH_BASE_RSSI: u8 = 44;

/// RSSI counts each further squelch level adds, which is 8 dB.
///
/// The nine levels used to span 3 dB each, so the whole range was -130 dBm to
/// -106 dBm. That is entirely at the sensitive end, and it made the radio
/// unusable near a transmitter: a handheld a metre away delivers far more than
/// -106 dBm into adjacent channels, so every level opened on it and a scan
/// stopped on whichever channel it reached first rather than on the one being
/// transmitted on. `EVID-K1-070` is that observation.
///
/// Eight decibels a step spans -130 dBm to -66 dBm instead, which reaches above
/// a strong local signal while keeping level one where it was for weak-signal
/// work. The steps are coarser and deliberately so: an operator who cannot shut
/// the squelch at all has no useful resolution anywhere.
const SQUELCH_RSSI_STEP: u8 = 16;

/// RSSI counts between the open and close thresholds, which is 1 dB.
const SQUELCH_RSSI_HYSTERESIS: u8 = 2;

/// Board-supplied squelch thresholds in the chip's own integer units.
///
/// These values are calibration data. The driver validates their internal
/// consistency but never invents them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SquelchThresholds {
    open_rssi: u8,
    close_rssi: u8,
    open_noise: u8,
    close_noise: u8,
    open_glitch: u8,
    close_glitch: u8,
}

/// A squelch threshold set was internally inconsistent or out of range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SquelchError {
    /// The RSSI open threshold is not above the close threshold.
    RssiHysteresis,
    /// A noise threshold exceeded the documented seven-bit field.
    NoiseRange,
    /// The noise or glitch open threshold is not below its close threshold.
    NoiseHysteresis,
}

impl fmt::Display for SquelchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RssiHysteresis => {
                formatter.write_str("squelch RSSI open threshold must exceed close")
            }
            Self::NoiseRange => formatter.write_str("squelch noise threshold exceeds seven bits"),
            Self::NoiseHysteresis => {
                formatter.write_str("squelch noise and glitch open thresholds must be below close")
            }
        }
    }
}

impl SquelchThresholds {
    /// Returns the pinned source's exact squelch-off threshold set.
    ///
    /// `RADIO_ConfigureSquelchAndOutputPower` writes RSSI zero, noise 127, and
    /// glitch 255 for both edges when the operator squelch level is zero. That
    /// set deliberately has no hysteresis, so it cannot be built through
    /// [`SquelchThresholds::new`]. Carrier squelch then always reads open and
    /// audio gating must come from elsewhere.
    pub const fn squelch_off() -> Self {
        Self {
            open_rssi: 0,
            close_rssi: 0,
            open_noise: 0x7F,
            close_noise: 0x7F,
            open_glitch: 0xFF,
            close_glitch: 0xFF,
        }
    }

    /// Returns AFIK's own thresholds for one operator squelch level.
    ///
    /// The per-unit calibration this radio was factory-programmed with is not
    /// readable evidence yet, so these are AFIK's values, not the unit's. They
    /// are deliberately conservative about what they claim: only the RSSI pair
    /// varies with the level, and the noise and glitch pairs are left one step
    /// below their permissive maximum so they satisfy the hysteresis rule
    /// without ever being the reason the squelch shuts. An operator therefore
    /// gets one predictable control, carrier strength, rather than three
    /// interacting ones whose units AFIK cannot justify.
    ///
    /// RSSI counts are 0.5 dB per step with `dBm = count / 2 - 160`, so level
    /// one opens at about -130 dBm and level nine at about -66 dBm, in 8 dB
    /// increments with 1 dB of hysteresis. Level zero is
    /// [`SquelchThresholds::squelch_off`] and never shuts.
    ///
    /// Carrier strength alone cannot tell a signal on this channel from a
    /// strong one on the next, which is what noise-gated squelch is for. The
    /// noise thresholds are per-unit calibration this radio cannot yet read —
    /// `RISK-008` — so the range above is what an operator has to work with,
    /// and it is wide enough to place the threshold above the interferer.
    #[must_use]
    pub const fn for_level(level: SquelchLevel) -> Self {
        if level.is_open() {
            return Self::squelch_off();
        }
        let open_rssi = SQUELCH_BASE_RSSI + SQUELCH_RSSI_STEP * level.get();
        Self {
            open_rssi,
            close_rssi: open_rssi - SQUELCH_RSSI_HYSTERESIS,
            // One step below the permissive maximum, which is the largest pair
            // that still satisfies the open-below-close hysteresis rule.
            open_noise: 0x7E,
            close_noise: 0x7F,
            open_glitch: 0xFE,
            close_glitch: 0xFF,
        }
    }

    /// Returns the carrier strength one level opens at, in whole dBm.
    ///
    /// `EVID-BK4819-053` records the chip's own mapping as `dBm = count / 2 -
    /// 160` over 0.5 dB steps. A level is otherwise a number with no scale, and
    /// an operator setting a threshold against a signal they can measure needs
    /// the scale. Level zero never shuts and has no threshold.
    #[must_use]
    pub const fn open_dbm(level: SquelchLevel) -> Option<i16> {
        if level.is_open() {
            return None;
        }
        let count = SQUELCH_BASE_RSSI as i16 + SQUELCH_RSSI_STEP as i16 * level.get() as i16;
        Some(count / 2 - 160)
    }

    /// Validates one complete calibration-supplied threshold set.
    ///
    /// RSSI thresholds rise with signal strength, so opening requires the
    /// larger value. Noise and glitch indicators fall as the signal improves,
    /// so opening requires the smaller value.
    pub const fn new(
        open_rssi: u8,
        close_rssi: u8,
        open_noise: u8,
        close_noise: u8,
        open_glitch: u8,
        close_glitch: u8,
    ) -> Result<Self, SquelchError> {
        if open_rssi <= close_rssi {
            return Err(SquelchError::RssiHysteresis);
        }
        if open_noise > 0x7F || close_noise > 0x7F {
            return Err(SquelchError::NoiseRange);
        }
        if open_noise >= close_noise || open_glitch >= close_glitch {
            return Err(SquelchError::NoiseHysteresis);
        }
        Ok(Self {
            open_rssi,
            close_rssi,
            open_noise,
            close_noise,
            open_glitch,
            close_glitch,
        })
    }
}

/// One complete receive configuration request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveSetup {
    /// Exact receive frequency.
    pub frequency: Frequency,
    /// Requested demodulator family.
    pub modulation: Modulation,
    /// Requested channel bandwidth.
    pub bandwidth: Bandwidth,
    /// Receive-side tone squelch requirement.
    pub tone: Tone,
    /// Calibration-supplied squelch thresholds.
    pub squelch: SquelchThresholds,
    /// Initial audio routing.
    pub af: AfOutput,
}

/// One sampled receive-status observation in the chip's own units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveMetrics {
    /// Approximate RSSI multiplied by two, preserving the 0.5 dB step.
    pub rssi_dbm_x2: i16,
    /// Raw glitch indicator; lower values indicate a cleaner signal.
    pub glitch: u8,
    /// Raw excess-noise indicator; lower values indicate a cleaner signal.
    pub noise: u8,
    /// Read-only carrier squelch link result.
    pub squelch_open: bool,
    /// Tone-squelch result, absent when the channel requires no tone.
    pub tone: Option<ToneStatus>,
}

impl ReceiveMetrics {
    /// Reports whether audio should be heard for this sample.
    ///
    /// Audio requires both an open carrier squelch and, when the channel is
    /// tone coded, a currently matched tone. An indeterminate tone result is
    /// treated as not matched.
    pub const fn should_unmute(self) -> bool {
        if !self.squelch_open {
            return false;
        }
        match self.tone {
            None => true,
            Some(status) => matches!(status, ToneStatus::Matched),
        }
    }
}

/// One register the driver may read back to verify a receive configuration.
///
/// Read-back exists so a bring-up can prove the bus carries a non-trivial
/// value it wrote. It exposes no arbitrary address and performs no write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadbackRegister {
    /// Filter bandwidth, `REG_43`.
    FilterBandwidth,
    /// Squelch noise thresholds, `REG_4F`.
    SquelchNoise,
    /// Squelch RSSI thresholds, `REG_78`.
    SquelchRssi,
    /// Audio output routing, `REG_47`.
    AudioOutput,
}

impl ReadbackRegister {
    /// Returns the seven-bit register address.
    pub const fn address(self) -> u8 {
        match self {
            Self::FilterBandwidth => 0x43,
            Self::SquelchNoise => 0x4F,
            Self::SquelchRssi => 0x78,
            Self::AudioOutput => 0x47,
        }
    }

    const fn register(self) -> RegisterAddress {
        match self {
            Self::FilterBandwidth => REG_FILTER_BANDWIDTH,
            Self::SquelchNoise => REG_SQUELCH_NOISE,
            Self::SquelchRssi => REG_SQUELCH_RSSI,
            Self::AudioOutput => REG_AF_OUTPUT,
        }
    }
}

/// Tone-squelch state kept from the found and lost interrupts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToneStatus {
    /// The configured tone or code was last found.
    Matched,
    /// The configured tone or code was lost, or not yet found.
    Lost,
}

impl<B: RegisterBus> Bk4819<B> {
    /// Performs the pinned source's exact power-on initialisation.
    ///
    /// This is the register table `BK4819_Init` writes before any receive
    /// configuration: soft reset, power blocks, crystal, AGC tables and
    /// automatic mode, microphone and audio levels, receive-side DTMF
    /// coefficients, and the default output word. It writes no mode-control
    /// word, so it cannot start a transmission, and it leaves the chip in
    /// standby exactly like [`Bk4819::recover_to_standby`].
    pub fn initialise(&mut self) -> Result<(), DriverError<B::Error>> {
        self.write(REG_SOFT_RESET, SOFT_RESET_ASSERT)?;
        self.write(REG_SOFT_RESET, SOFT_RESET_RELEASE)?;
        self.write(REG_POWER_BLOCKS, self.profile.init_power_blocks)?;
        self.write(REG_CRYSTAL, CRYSTAL_INIT)?;

        self.configure_agc(Modulation::Fm)?;
        if self.profile.agc_auto_after_init {
            let agc = self.read(REG_AGC_CONTROL)?;
            self.write(
                REG_AGC_CONTROL,
                (agc & !AGC_AUTO_MODE_BIT & !AGC_INDEX_MASK) | AGC_FIX_INDEX,
            )?;
        }

        self.write(REG_MIC_GAIN, self.profile.mic_gain)?;
        self.write(REG_AF_DAC_GAIN, self.profile.af_level)?;

        for (index, coefficient) in DTMF_COEFFICIENTS.into_iter().enumerate() {
            let selector = u16::try_from(index).unwrap_or(0) << 12;
            self.write(REG_DTMF_COEFFICIENT, selector | u16::from(coefficient))?;
        }

        for (address, value) in self.profile.extra_init {
            let register = RegisterAddress::new(*address)
                .map_err(|_| DriverError::InvalidState(self.state))?;
            self.write(register, *value)?;
        }

        self.gpio_out = GPIO_OUT_DEFAULT;
        self.write(REG_GPIO_OUT, GPIO_OUT_DEFAULT)?;
        self.write(REG_INTERRUPT_MASK, 0)?;

        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.state = DriverState::Standby;
        Ok(())
    }

    /// Applies one complete receive configuration from standby or receive.
    ///
    /// The sequence follows the pinned reference firmware: power blocks and
    /// the receive mode block first, then the demodulator, bandwidth, AGC,
    /// frequency, squelch thresholds, sub-audio decoding, interrupt mask, RF
    /// filter path, and finally audio routing.
    pub fn configure_receive(&mut self, setup: &ReceiveSetup) -> Result<(), DriverError<B::Error>> {
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        let word = FrequencyWord::from_frequency(setup.frequency)?;

        // The pinned source's order matters: the receive mode word carries the
        // VCO calibration request, so it must be written after the frequency.
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(
            REG_FILTER_BANDWIDTH,
            match setup.bandwidth {
                Bandwidth::Wide => self.profile.bandwidth_wide,
                Bandwidth::Narrow => self.profile.bandwidth_narrow,
            },
        )?;
        self.write(REG_FREQUENCY_LOW, word.low())?;
        self.write(REG_FREQUENCY_HIGH, word.high())?;
        self.configure_demodulator(setup.modulation)?;
        self.configure_agc(setup.modulation)?;
        self.configure_squelch(setup.squelch)?;

        // Audio is muted across the turn-on, exactly as the source does.
        self.write_af_output(setup.modulation, AfOutput::Mute)?;
        self.write(REG_POWER_BLOCKS, self.profile.receive_power_blocks)?;
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        let receive_mode = self.profile.receive_mode;
        self.write(REG_MODE_CONTROL, receive_mode)?;

        self.select_filter_path(setup.frequency)?;
        // `RADIO_SetupRegisters` turns RX enable on after the filter path on
        // every receive setup; TX clears it. Without it `7.0.31` and earlier
        // heard nearby radios but not GB3EL at 12 km, which Armel hears.
        self.gpio_out |= GPIO_RX_ENABLE;
        self.write(REG_GPIO_OUT, self.gpio_out)?;
        self.configure_sub_audio(setup.tone)?;
        self.write(REG_INTERRUPT_MASK, interrupt_mask(setup.tone))?;
        self.write(REG_INTERRUPT_FLAGS, 0)?;
        self.tone_found = false;
        self.write_af_output(setup.modulation, setup.af)?;

        self.state = DriverState::Receiving {
            frequency: setup.frequency,
        };
        Ok(())
    }

    /// Puts the chip to sleep for battery save, from standby or receive.
    ///
    /// `BK4819_Sleep` in both pinned trees: every mode block off, then the
    /// power blocks down to `0x1D00`. [`Self::configure_receive`] wakes it.
    pub fn sleep(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(REG_POWER_BLOCKS, SLEEP_POWER_BLOCKS)?;
        self.state = DriverState::Standby;
        Ok(())
    }

    /// Routes or mutes demodulated audio while receiving.
    pub fn set_af_output(
        &mut self,
        modulation: Modulation,
        af: AfOutput,
    ) -> Result<(), DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write_af_output(modulation, af)
    }

    /// Reads back one configured receive register while receiving.
    pub fn read_back(&mut self, register: ReadbackRegister) -> Result<u16, DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.read(register.register())
    }

    /// Samples RSSI, glitch, noise, carrier squelch, and tone status.
    pub fn receive_metrics(&mut self, tone: Tone) -> Result<ReceiveMetrics, DriverError<B::Error>> {
        let status = self.receive_status()?;
        let glitch = self.read(REG_GLITCH)? & GLITCH_MASK;
        let noise = self.read(REG_NOISE)? & NOISE_MASK;
        let tone = match tone {
            Tone::None => None,
            Tone::Ctcss(_) => Some(self.tone_status(INTERRUPT_CTCSS_FOUND, INTERRUPT_CTCSS_LOST)?),
            Tone::Dcs { .. } => {
                Some(self.tone_status(INTERRUPT_CDCSS_FOUND, INTERRUPT_CDCSS_LOST)?)
            }
        };
        Ok(ReceiveMetrics {
            rssi_dbm_x2: status.rssi_dbm_x2,
            glitch: u8::try_from(glitch).unwrap_or(u8::MAX),
            noise: u8::try_from(noise).unwrap_or(u8::MAX),
            squelch_open: status.squelch_open,
            tone,
        })
    }

    /// Services pending interrupts as `CheckRadioInterrupts` does, clearing
    /// `REG_02` before reading it, and keeps the last found or lost event.
    fn tone_status(&mut self, found: u16, lost: u16) -> Result<ToneStatus, DriverError<B::Error>> {
        for _ in 0..MAX_INTERRUPTS_PER_SAMPLE {
            if self.read(crate::REG_SQUELCH_STATUS)? & INTERRUPT_REQUEST == 0 {
                break;
            }
            self.write(REG_INTERRUPT_FLAGS, 0)?;
            let flags = self.read(REG_INTERRUPT_FLAGS)?;
            if flags & lost != 0 {
                self.tone_found = false;
            }
            if flags & found != 0 {
                self.tone_found = true;
            }
        }
        Ok(if self.tone_found {
            ToneStatus::Matched
        } else {
            ToneStatus::Lost
        })
    }

    /// Chooses how the next receive setup treats a channel without a receive
    /// tone: the reference's tail detection (`false`, the default) or the tone
    /// scan, so [`Self::detected_tone`] reads what it hears during ordinary
    /// receive, as `7.0.28`–`7.0.30` did. An operator experiment (`RISK-046`).
    pub fn set_untoned_tone_scan(&mut self, scan: bool) {
        self.scan_untoned = scan;
    }

    /// Starts, or restarts after a reading, the reference's tone scan on the
    /// tuned frequency, as `BK4819_SetScanFrequency` does: its `REG_51`, then
    /// receive turned on again (`BK4819_RX_TurnOn`). The next receive
    /// configuration leaves it.
    pub fn start_tone_scan(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write(REG_SUB_AUDIO_CONTROL, SUB_AUDIO_SCAN)?;
        self.write(REG_POWER_BLOCKS, self.profile.receive_power_blocks)?;
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(REG_MODE_CONTROL, self.profile.receive_mode)
    }

    /// Starts the reference's frequency scan (`SCANNER_Start(false)`): both
    /// LNA paths cleared, as `BK4819_PickRXFilterPathBasedOnFrequency` does for
    /// its `0xFFFFFFFF` sentinel, then `BK4819_EnableFrequencyScan`.
    pub fn start_frequency_scan(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.gpio_out &= !(GPIO_VHF_LNA | GPIO_UHF_LNA);
        self.write(REG_GPIO_OUT, self.gpio_out)?;
        self.write(REG_FREQUENCY_SCAN, FREQUENCY_SCAN_ON)
    }

    /// Starts the next frequency scan after a reading, as the reference's
    /// scanner does: disabled, then enabled again.
    pub fn restart_frequency_scan(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write(REG_FREQUENCY_SCAN, FREQUENCY_SCAN_OFF)?;
        self.write(REG_FREQUENCY_SCAN, FREQUENCY_SCAN_ON)
    }

    /// The strongest carrier the frequency scan found, as
    /// `BK4819_GetFrequencyScanResult` reads it, or `None` while it runs.
    pub fn frequency_scan_result(&mut self) -> Result<Option<Frequency>, DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        let high = self.read(REG_FREQUENCY_SCAN_HIGH)?;
        if high & SCAN_NOT_FOUND != 0 {
            return Ok(None);
        }
        let low = self.read(REG_FREQUENCY_SCAN_LOW)?;
        let word = (u32::from(high & 0x07FF) << 16) | u32::from(low);
        Ok(word
            .checked_mul(10)
            .and_then(|hertz| Frequency::from_hz(hertz).ok()))
    }

    /// Ends a frequency or tone scan as `BK4819_StopScan` does: frequency
    /// scan off and the chip disabled (`REG_30 = 0`), in standby until the
    /// next receive configuration.
    pub fn stop_scan(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write(REG_FREQUENCY_SCAN, FREQUENCY_SCAN_OFF)?;
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.state = DriverState::Standby;
        Ok(())
    }

    /// Reads the tone or code the chip hears during a tone scan, as
    /// `BK4819_GetCxCSSScanResult` does: DCS first, then CTCSS.
    ///
    /// A CTCSS reading is the nearest standard tone within 5 Hz and a DCS
    /// word a standard code in normal polarity (`DCS_GetCtcssCode`,
    /// `DCS_GetCdcssCode`); anything else is `None`.
    pub fn detected_tone(&mut self) -> Result<Option<Tone>, DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        let high = self.read(REG_CDCSS_SCAN_HIGH)?;
        if high & SCAN_NOT_FOUND == 0 {
            let low = self.read(REG_CDCSS_SCAN_LOW)?;
            let word = (u32::from(high & 0x0FFF) << 12) | u32::from(low & 0x0FFF);
            return Ok(dcs_from_word(word));
        }
        let ctcss = self.read(REG_CTCSS_SCAN)?;
        if ctcss & SCAN_NOT_FOUND != 0 {
            return Ok(None);
        }
        let tenths = u32::from(ctcss & 0x1FFF) * CTCSS_SCAN_NUMERATOR / CTCSS_SCAN_DENOMINATOR;
        Ok(nearest_ctcss(u16::try_from(tenths).unwrap_or(u16::MAX)).map(Tone::Ctcss))
    }

    fn configure_demodulator(
        &mut self,
        modulation: Modulation,
    ) -> Result<(), DriverError<B::Error>> {
        let am_control = self.read(REG_AM_DEMODULATOR)?;
        let legacy = self.profile.demodulator_tuning;
        match modulation {
            Modulation::Am => {
                self.write(REG_AM_DEMODULATOR, am_control | 1)?;
                if legacy {
                    self.write(REG_DEMODULATOR_42, 0x6F5C)?;
                    self.write(REG_DEMODULATOR_2A, 0x7434)?;
                }
            }
            Modulation::Fm | Modulation::Usb | Modulation::Broadcast => {
                self.write(REG_AM_DEMODULATOR, am_control & !1)?;
                if legacy {
                    self.write(REG_DEMODULATOR_42, 0x6B5A)?;
                    self.write(REG_DEMODULATOR_2A, 0x7400)?;
                    self.write(REG_DEMODULATOR_2B, 0x0000)?;
                    self.write(REG_DEMODULATOR_2F, 0x9890)?;
                }
            }
        }
        if legacy {
            self.write(REG_AUDIO_54, 0x9009)?;
            self.write(REG_AUDIO_55, 0x31A9)?;
        }
        // The evidence records this step as the AF DAC gain, which is the low
        // nibble. Writing the bare value would also clear the AF receive gain
        // fields the profile's audio level established, leaving the output
        // barely audible.
        self.write(
            REG_AF_DAC_GAIN,
            (self.profile.af_level & !AF_DAC_GAIN_MASK) | AF_DAC_GAIN_MAXIMUM,
        )?;
        self.write(
            REG_AF_TAIL,
            if matches!(modulation, Modulation::Usb) {
                0x0000
            } else {
                0x2AAB
            },
        )?;
        let afc = self.read(REG_AFC)?;
        let afc_disable = 1 << 4;
        self.write(
            REG_AFC,
            if matches!(modulation, Modulation::Fm) {
                afc & !afc_disable
            } else {
                afc | afc_disable
            },
        )
    }

    fn configure_agc(&mut self, modulation: Modulation) -> Result<(), DriverError<B::Error>> {
        let table = if matches!(modulation, Modulation::Am) {
            self.profile.agc_am
        } else {
            self.profile.agc_fm
        };
        for (address, value) in table {
            if *address == 0x7E && *value == 0 {
                // The BK4819 profile leaves automatic gain mode to the
                // separate read-modify-write during initialisation.
                continue;
            }
            let register = RegisterAddress::new(*address)
                .map_err(|_| DriverError::InvalidState(self.state))?;
            self.write(register, *value)?;
        }
        Ok(())
    }

    fn configure_squelch(
        &mut self,
        squelch: SquelchThresholds,
    ) -> Result<(), DriverError<B::Error>> {
        self.write(
            REG_SQUELCH_CLOSE_GLITCH,
            0xA000 | u16::from(squelch.close_glitch),
        )?;
        self.write(
            REG_SQUELCH_OPEN_GLITCH,
            (1 << 14) | (5 << 11) | (6 << 9) | u16::from(squelch.open_glitch),
        )?;
        self.write(
            REG_SQUELCH_NOISE,
            (u16::from(squelch.close_noise) << 8) | u16::from(squelch.open_noise),
        )?;
        self.write(
            REG_SQUELCH_RSSI,
            (u16::from(squelch.open_rssi) << 8) | u16::from(squelch.close_rssi),
        )
    }

    pub(crate) fn configure_sub_audio(&mut self, tone: Tone) -> Result<(), DriverError<B::Error>> {
        match tone {
            // `RADIO_SetupRegisters`' `CODE_TYPE_OFF`: `BK4819_SetCTCSSFrequency`
            // then `BK4819_SetTailDetection`, whose `REG_07` replaces the first.
            Tone::None if self.scan_untoned => self.write(REG_SUB_AUDIO_CONTROL, SUB_AUDIO_SCAN),
            Tone::None => {
                self.write(REG_SUB_AUDIO_CONTROL, self.profile.sub_audio_ctcss)?;
                self.write(
                    REG_SUB_AUDIO_FREQUENCY,
                    SUB_AUDIO_MODE_CTC2 | TAIL_DETECTION_55_HZ,
                )
            }
            Tone::Ctcss(tenths_hz) => {
                let ctcss = self.profile.sub_audio_ctcss;
                self.write(REG_SUB_AUDIO_CONTROL, ctcss)?;
                self.write(
                    REG_SUB_AUDIO_FREQUENCY,
                    SUB_AUDIO_MODE_CTC1 | ctcss_control_word(tenths_hz),
                )
            }
            Tone::Dcs { code, inverted } => {
                let word = cdcss_code_word(code, inverted);
                let cdcss = self.profile.sub_audio_cdcss;
                self.write(REG_SUB_AUDIO_CONTROL, cdcss)?;
                self.write(
                    REG_SUB_AUDIO_FREQUENCY,
                    SUB_AUDIO_MODE_CTC1 | CDCSS_BAUD_CONTROL_WORD,
                )?;
                self.write(
                    REG_CDCSS_CODE,
                    u16::try_from(word & CDCSS_HALF_MASK).unwrap_or(0),
                )?;
                self.write(
                    REG_CDCSS_CODE,
                    CDCSS_HIGH_HALF | u16::try_from((word >> 12) & CDCSS_HALF_MASK).unwrap_or(0),
                )
            }
        }
    }

    pub(crate) fn select_filter_path(
        &mut self,
        frequency: Frequency,
    ) -> Result<(), DriverError<B::Error>> {
        // The pinned source keeps one output word and toggles single bits, so
        // the power-on defaults must survive a filter-path change.
        let selected = if frequency.as_hz() < RECEIVE_FILTER_PATH_BOUNDARY_HZ {
            GPIO_VHF_LNA
        } else {
            GPIO_UHF_LNA
        };
        let gpio = (self.gpio_out & !(GPIO_VHF_LNA | GPIO_UHF_LNA)) | selected;
        self.gpio_out = gpio;
        self.write(REG_GPIO_OUT, gpio)
    }

    pub(crate) fn write_af_output(
        &mut self,
        modulation: Modulation,
        af: AfOutput,
    ) -> Result<(), DriverError<B::Error>> {
        let af_type = match af {
            AfOutput::Mute => AF_MUTE,
            AfOutput::Demodulated => match modulation {
                Modulation::Fm | Modulation::Am => AF_FM,
                Modulation::Usb => AF_BASEBAND2,
                // Broadcast audio comes from the BK1080, never this chip.
                Modulation::Broadcast => AF_MUTE,
            },
        };
        let fixed = self.profile.af_fixed_bits;
        self.write(REG_AF_OUTPUT, fixed | (af_type << 8))
    }
}

fn interrupt_mask(tone: Tone) -> u16 {
    INTERRUPT_MASK_SQUELCH_AND_TONE
        | match tone {
            Tone::None => 0,
            Tone::Ctcss(_) => INTERRUPT_MASK_CTCSS,
            Tone::Dcs { .. } => INTERRUPT_MASK_CDCSS,
        }
}

/// Converts a CTCSS frequency in tenths of a hertz to the `REG_07` control word.
pub fn ctcss_control_word(tenths_hz: u16) -> u16 {
    let scaled = (u32::from(tenths_hz) * CTCSS_CONTROL_NUMERATOR + CTCSS_CONTROL_ROUNDING)
        / CTCSS_CONTROL_DENOMINATOR;
    u16::try_from(scaled & CTCSS_CONTROL_MASK).unwrap_or(0)
}

/// Converts a DCS code held as octal digits to its 23-bit Golay code word.
pub fn cdcss_code_word(code: u16, inverted: bool) -> u32 {
    let mut binary = 0_u32;
    let mut remaining = code;
    let mut shift = 0;
    while remaining != 0 {
        binary |= u32::from(remaining % 10) << shift;
        remaining /= 10;
        shift += 3;
    }
    let word = golay_23_12(binary | 0x800);
    if inverted {
        word ^ CDCSS_INVERT_MASK
    } else {
        word
    }
}

/// The standard CTCSS tone nearest `tenths_hz`, if one is within 5 Hz.
fn nearest_ctcss(tenths_hz: u16) -> Option<u16> {
    let mut tone = FIRST_CTCSS;
    let mut nearest = tone;
    for step in CTCSS_STEPS {
        tone += u16::from(step);
        if tone.abs_diff(tenths_hz) < nearest.abs_diff(tenths_hz) {
            nearest = tone;
        }
    }
    Some(nearest).filter(|tone| tone.abs_diff(tenths_hz) < CTCSS_SCAN_TOLERANCE_TENTHS)
}

/// The standard DCS code a received 23-bit word carries, in any of its
/// rotations, as `DCS_GetCdcssCode` finds it.
fn dcs_from_word(mut word: u32) -> Option<Tone> {
    for _ in 0..23 {
        // The code's marker bit, 0x800, sits above its nine data bits.
        if (word >> 9) & 0b111 == 0b100 {
            let bits = u16::try_from(word & 0x1FF).unwrap_or(0);
            if is_standard_dcs(bits) {
                let code = (bits >> 6) * 100 + ((bits >> 3) & 7) * 10 + (bits & 7);
                if cdcss_code_word(code, false) == word {
                    return Some(Tone::Dcs {
                        code,
                        inverted: false,
                    });
                }
            }
        }
        word = (word >> 1) | ((word & 1) << 22);
    }
    None
}

fn golay_23_12(code: u32) -> u32 {
    let mut word = code;
    for _ in 0..12 {
        word <<= 1;
        if word & 0x1000 != 0 {
            word ^= GOLAY_GENERATOR;
        }
    }
    code | ((word & 0x0FFE) << 11)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        cdcss_code_word, ctcss_control_word, AfOutput, ReceiveMetrics, ReceiveSetup, SquelchError,
        SquelchThresholds, ToneStatus, BK4819_PROFILE, BK4829_PROFILE, REG_AF_DAC_GAIN,
        REG_AF_OUTPUT, REG_FILTER_BANDWIDTH, REG_GPIO_OUT, REG_INTERRUPT_FLAGS, REG_SQUELCH_RSSI,
        REG_SUB_AUDIO_CONTROL, REG_SUB_AUDIO_FREQUENCY,
    };
    use crate::tests_support::{FakeBus, Operation};
    use crate::{
        Bk4819, DriverError, DriverState, MODE_RECEIVE, REG_FREQUENCY_HIGH, REG_MODE_CONTROL,
    };
    use radio_domain::{Bandwidth, Frequency, Modulation, Tone};

    fn thresholds() -> SquelchThresholds {
        SquelchThresholds::new(72, 70, 46, 47, 8, 10).unwrap()
    }

    fn setup(modulation: Modulation, tone: Tone, hertz: u32) -> ReceiveSetup {
        ReceiveSetup {
            frequency: Frequency::from_hz(hertz).unwrap(),
            modulation,
            bandwidth: Bandwidth::Narrow,
            tone,
            squelch: thresholds(),
            af: AfOutput::Demodulated,
        }
    }

    #[test]
    fn receive_turns_rx_enable_on_and_restores_it_after_transmit() {
        use crate::tx_fixture::{channel, policy, request};
        use radio_domain::TxClass;

        let last_gpio = |radio: &Bk4819<FakeBus>| {
            radio
                .bus()
                .operations
                .iter()
                .rev()
                .find_map(|operation| match operation {
                    Operation::Write(address, value) if *address == REG_GPIO_OUT => Some(*value),
                    _ => None,
                })
                .expect("REG_33 is written")
        };
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio.initialise().unwrap();
        // GB3EL's output: UHF LNA (0x08) and RX enable (0x40) over the
        // default outputs, as `RADIO_SetupRegisters` leaves them.
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 430_837_500))
            .unwrap();
        assert_eq!(last_gpio(&radio), 0x9048);

        // TX clears RX enable; the next receive setup sets it again.
        let policy = policy(true);
        let request = request(channel(TxClass::LicenceFreePlan));
        let token = policy.authorise(&request).unwrap();
        radio.prepare_transmit(&request, &token).unwrap();
        assert_eq!(last_gpio(&radio) & 0x40, 0);
        radio.enable_transmit(&request, &token).unwrap();
        radio.disable_pa().unwrap();
        radio.disable_transmit().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 430_837_500))
            .unwrap();
        assert_eq!(last_gpio(&radio), 0x9048);
    }

    #[test]
    fn sleep_matches_the_pinned_source_and_receive_wakes_it() {
        use super::{REG_POWER_BLOCKS, SLEEP_POWER_BLOCKS};
        use crate::{DriverState, MODE_STANDBY, REG_MODE_CONTROL};

        let mut radio = Bk4819::new(FakeBus::new(None));
        assert!(
            radio.sleep().is_err(),
            "an unknown chip is not put to sleep"
        );
        radio.recover_to_standby().unwrap();
        let tuned = setup(Modulation::Fm, Tone::None, 145_500_000);
        radio.configure_receive(&tuned).unwrap();
        radio.bus_mut().operations.clear();
        radio.sleep().unwrap();
        assert_eq!(
            radio.bus().operations,
            [
                Operation::Write(REG_MODE_CONTROL, MODE_STANDBY),
                Operation::Write(REG_POWER_BLOCKS, SLEEP_POWER_BLOCKS),
            ]
        );
        assert_eq!(radio.state(), DriverState::Standby);
        radio.configure_receive(&tuned).unwrap();
        assert!(matches!(radio.state(), DriverState::Receiving { .. }));
    }

    #[test]
    fn the_squelch_off_set_matches_the_pinned_source() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        let mut setup = setup(Modulation::Fm, Tone::None, 145_500_000);
        setup.squelch = SquelchThresholds::squelch_off();
        radio.configure_receive(&setup).unwrap();
        let operations = &radio.bus().operations;
        assert!(operations.contains(&Operation::Write(REG_SQUELCH_RSSI, 0x0000)));
        assert!(operations.contains(&Operation::Write(super::REG_SQUELCH_NOISE, 0x7F7F)));
        assert!(operations.contains(&Operation::Write(super::REG_SQUELCH_CLOSE_GLITCH, 0xA0FF)));
    }

    /// The operator's level has to reach the chip as one predictable control.
    #[test]
    fn each_squelch_level_gates_on_carrier_strength_alone() {
        use radio_domain::{SquelchLevel, MAX_SQUELCH_LEVEL};

        assert_eq!(
            SquelchThresholds::for_level(SquelchLevel::OPEN),
            SquelchThresholds::squelch_off(),
            "level zero is the pinned squelch-off set and never shuts"
        );

        let mut previous = 0;
        for level in 1..=MAX_SQUELCH_LEVEL {
            let set = SquelchThresholds::for_level(SquelchLevel::new(level).unwrap());
            assert!(
                set.open_rssi > previous,
                "a higher level must demand a stronger carrier"
            );
            previous = set.open_rssi;
            assert!(
                set.open_rssi > set.close_rssi,
                "the set must carry hysteresis"
            );
            assert_eq!(
                (
                    set.open_noise,
                    set.close_noise,
                    set.open_glitch,
                    set.close_glitch
                ),
                (0x7E, 0x7F, 0xFE, 0xFF),
                "noise and glitch stay permissive: AFIK has no calibration for them"
            );
            // Every level must be a set the driver would accept from a board.
            SquelchThresholds::new(
                set.open_rssi,
                set.close_rssi,
                set.open_noise,
                set.close_noise,
                set.open_glitch,
                set.close_glitch,
            )
            .expect("a derived set must satisfy the same rules as a supplied one");
        }

        // The documented endpoints, in the chip's 0.5 dB steps from -160 dBm.
        assert_eq!(
            SquelchThresholds::for_level(SquelchLevel::new(1).unwrap()).open_rssi,
            60,
            "level one opens at about -130 dBm"
        );
        assert_eq!(
            SquelchThresholds::for_level(SquelchLevel::new(MAX_SQUELCH_LEVEL).unwrap()).open_rssi,
            188,
            "level nine opens at about -66 dBm"
        );

        // The range has to reach above a strong local signal, which is the
        // whole reason it is this wide: a handheld transmitting a metre away
        // delivers far more than -106 dBm into the channels either side of the
        // one it is on, and every level used to open on it.
        assert_eq!(
            SquelchThresholds::open_dbm(SquelchLevel::new(1).unwrap()),
            Some(-130)
        );
        assert_eq!(
            SquelchThresholds::open_dbm(SquelchLevel::new(MAX_SQUELCH_LEVEL).unwrap()),
            Some(-66)
        );
        assert_eq!(
            SquelchThresholds::open_dbm(SquelchLevel::OPEN),
            None,
            "a level which never shuts has no threshold to name"
        );
    }

    #[test]
    fn initialisation_writes_the_pinned_power_on_table_and_ends_in_standby() {
        use super::{
            BK4819_PROFILE, BK4829_PROFILE, CRYSTAL_INIT, DTMF_COEFFICIENTS, GPIO_OUT_DEFAULT,
            REG_AF_DAC_GAIN, REG_CRYSTAL, REG_DTMF_COEFFICIENT, REG_MIC_GAIN, REG_POWER_BLOCKS,
            REG_SOFT_RESET,
        };
        use crate::{RegisterAddress, MODE_STANDBY, REG_MODE_CONTROL};

        for profile in [BK4819_PROFILE, BK4829_PROFILE] {
            let mut radio = Bk4819::with_profile(FakeBus::new(None), profile);
            radio.initialise().unwrap();
            assert_eq!(radio.state(), DriverState::Standby);

            let operations = &radio.bus().operations;
            assert_eq!(operations[0], Operation::Write(REG_SOFT_RESET, 0x8000));
            assert_eq!(operations[1], Operation::Write(REG_SOFT_RESET, 0x0000));
            assert!(operations.contains(&Operation::Write(
                REG_POWER_BLOCKS,
                profile.init_power_blocks
            )));
            assert!(operations.contains(&Operation::Write(REG_CRYSTAL, CRYSTAL_INIT)));
            assert!(operations.contains(&Operation::Write(REG_MIC_GAIN, profile.mic_gain)));
            assert!(operations.contains(&Operation::Write(REG_AF_DAC_GAIN, profile.af_level)));
            assert!(operations.contains(&Operation::Write(REG_GPIO_OUT, GPIO_OUT_DEFAULT)));
            for (index, coefficient) in DTMF_COEFFICIENTS.into_iter().enumerate() {
                let expected = u16::try_from(index).unwrap() << 12 | u16::from(coefficient);
                assert!(operations.contains(&Operation::Write(REG_DTMF_COEFFICIENT, expected)));
            }
            for (address, value) in profile.extra_init {
                let register = RegisterAddress::new(*address).unwrap();
                assert!(
                    operations.contains(&Operation::Write(register, *value)),
                    "missing init write {address:02x}={value:04x}"
                );
            }
            assert_eq!(
                operations.last(),
                Some(&Operation::Write(REG_MODE_CONTROL, MODE_STANDBY))
            );
            assert!(!operations.iter().any(
                |operation| matches!(operation, Operation::Write(_, value) if *value == 0x80FE)
            ));
        }
    }

    #[test]
    fn the_two_chip_profiles_differ_where_the_pinned_drivers_differ() {
        use super::{BK4819_PROFILE, BK4829_PROFILE};

        assert_eq!(BK4819_PROFILE.receive_mode, 0xBEF1);
        assert_eq!(BK4829_PROFILE.receive_mode, 0xBFF1);
        assert_eq!(BK4819_PROFILE.receive_power_blocks, 0x1F0F);
        assert_eq!(BK4829_PROFILE.receive_power_blocks, 0x9F1F);
        assert_eq!(BK4819_PROFILE.bandwidth_narrow, 0x3648);
        assert_eq!(BK4829_PROFILE.bandwidth_narrow, 0x4048);
        assert_eq!(BK4819_PROFILE.af_fixed_bits, 0x6040);
        assert_eq!(BK4829_PROFILE.af_fixed_bits, 0x6042);
        assert_eq!(BK4829_PROFILE.sub_audio_cdcss, 0xA033);
    }

    #[test]
    fn read_back_returns_configured_values_and_is_state_gated() {
        use super::ReadbackRegister;

        let mut radio = Bk4819::new(FakeBus::new(None));
        assert_eq!(
            radio.read_back(ReadbackRegister::FilterBandwidth),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
            .unwrap();
        assert_eq!(
            radio.read_back(ReadbackRegister::FilterBandwidth).unwrap(),
            0x3648
        );
        assert_eq!(
            radio.read_back(ReadbackRegister::SquelchNoise).unwrap(),
            0x2F2E
        );
        assert_eq!(
            radio.read_back(ReadbackRegister::SquelchRssi).unwrap(),
            0x4846
        );
        assert_eq!(
            radio.read_back(ReadbackRegister::AudioOutput).unwrap(),
            0x6140
        );
        assert_eq!(ReadbackRegister::FilterBandwidth.address(), 0x43);
        assert_eq!(ReadbackRegister::SquelchNoise.address(), 0x4F);
        assert_eq!(ReadbackRegister::SquelchRssi.address(), 0x78);
        assert_eq!(ReadbackRegister::AudioOutput.address(), 0x47);
    }

    #[test]
    fn squelch_thresholds_require_consistent_hysteresis() {
        assert_eq!(
            SquelchThresholds::new(70, 70, 46, 47, 8, 10),
            Err(SquelchError::RssiHysteresis)
        );
        assert_eq!(
            SquelchThresholds::new(72, 70, 0x80, 47, 8, 10),
            Err(SquelchError::NoiseRange)
        );
        assert_eq!(
            SquelchThresholds::new(72, 70, 47, 46, 8, 10),
            Err(SquelchError::NoiseHysteresis)
        );
        assert_eq!(
            SquelchThresholds::new(72, 70, 46, 47, 10, 8),
            Err(SquelchError::NoiseHysteresis)
        );
    }

    #[test]
    fn sub_audio_control_words_match_the_pinned_source() {
        assert_eq!(ctcss_control_word(670), 1383);
        assert_eq!(ctcss_control_word(1000), 2065);
        assert_eq!(cdcss_code_word(23, false), 0x0076_3813);
        assert_eq!(cdcss_code_word(23, true), 0x0009_C7EC);
        assert_eq!(cdcss_code_word(754, false), 0x0020_F9EC);
        assert_eq!(cdcss_code_word(131, false), 0x003D_3859);
    }

    #[test]
    fn receive_configuration_writes_the_sourced_sequence() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::Ctcss(1_000), 145_500_000))
            .unwrap();
        assert_eq!(
            radio.state(),
            DriverState::Receiving {
                frequency: Frequency::from_hz(145_500_000).unwrap()
            }
        );

        let operations = &radio.bus().operations;
        let receive_mode = operations
            .iter()
            .position(|operation| *operation == Operation::Write(REG_MODE_CONTROL, MODE_RECEIVE))
            .expect("the receive mode word is written");
        let frequency_high = operations
            .iter()
            .position(|operation| matches!(operation, Operation::Write(address, _) if *address == REG_FREQUENCY_HIGH))
            .expect("the frequency is written");
        assert!(
            frequency_high < receive_mode,
            "the mode word carries VCO calibration and must follow the frequency"
        );
        assert!(operations.contains(&Operation::Write(REG_FILTER_BANDWIDTH, 0x3648)));
        assert!(operations.contains(&Operation::Write(REG_SQUELCH_RSSI, 0x4846)));
        assert!(operations.contains(&Operation::Write(REG_SUB_AUDIO_CONTROL, 0x904A)));
        assert!(operations.contains(&Operation::Write(REG_SUB_AUDIO_FREQUENCY, 2065)));
        assert!(operations.contains(&Operation::Write(REG_AF_OUTPUT, 0x6140)));
        assert!(operations.contains(&Operation::Write(REG_GPIO_OUT, 0x9004)));
        assert!(!operations
            .iter()
            .any(|operation| matches!(operation, Operation::Write(_, value) if *value == 0x80FE)));
    }

    #[test]
    fn modulation_selects_the_sourced_demodulator_and_audio_path() {
        for (modulation, expected_af) in [
            (Modulation::Fm, 0x6140),
            (Modulation::Am, 0x6140),
            (Modulation::Usb, 0x6540),
        ] {
            let mut radio = Bk4819::new(FakeBus::new(None));
            radio.recover_to_standby().unwrap();
            radio
                .configure_receive(&setup(modulation, Tone::None, 145_500_000))
                .unwrap();
            assert!(radio
                .bus()
                .operations
                .contains(&Operation::Write(REG_AF_OUTPUT, expected_af)));
            radio.set_af_output(modulation, AfOutput::Mute).unwrap();
            assert_eq!(
                radio.bus().operations.last(),
                Some(&Operation::Write(REG_AF_OUTPUT, 0x6040))
            );
        }
    }

    #[test]
    fn configuring_receive_keeps_the_profile_audio_gains_at_maximum_dac_gain() {
        // Writing the bare DAC gain would clear the AF receive gain fields the
        // profile established, which is audible as a nearly silent speaker.
        for profile in [BK4819_PROFILE, BK4829_PROFILE] {
            let mut radio = Bk4819::with_profile(FakeBus::new(None), profile);
            radio.recover_to_standby().unwrap();
            radio
                .configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
                .unwrap();

            let expected = (profile.af_level & !0x000F) | 0x000F;
            assert_eq!(expected & 0x000F, 0x000F);
            assert_eq!(expected & !0x000F, profile.af_level & !0x000F);
            assert!(radio
                .bus()
                .operations
                .contains(&Operation::Write(REG_AF_DAC_GAIN, expected)));
            assert!(!radio
                .bus()
                .operations
                .contains(&Operation::Write(REG_AF_DAC_GAIN, 0x000F)));
        }
    }

    #[test]
    fn vhf_and_uhf_paths_use_distinct_low_noise_amplifier_bits() {
        // The split is 280 MHz: a 2 m channel takes the VHF path and a 70 cm
        // channel takes the UHF path.
        let mut vhf = Bk4819::new(FakeBus::new(None));
        vhf.recover_to_standby().unwrap();
        vhf.configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
            .unwrap();
        assert!(vhf
            .bus()
            .operations
            .contains(&Operation::Write(REG_GPIO_OUT, 0x9004)));

        let mut uhf = Bk4819::new(FakeBus::new(None));
        uhf.recover_to_standby().unwrap();
        uhf.configure_receive(&setup(Modulation::Fm, Tone::None, 433_500_000))
            .unwrap();
        assert!(uhf
            .bus()
            .operations
            .contains(&Operation::Write(REG_GPIO_OUT, 0x9008)));
    }

    #[test]
    fn metrics_combine_carrier_squelch_with_tone_status() {
        let bus = FakeBus::new(None)
            .with_register(crate::REG_RSSI, 0x0064)
            .with_register(crate::REG_SQUELCH_STATUS, 0x0002)
            .with_register(super::REG_GLITCH, 0x00AB)
            .with_register(super::REG_NOISE, 0x00FF);
        let mut radio = Bk4819::new(bus);
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::Ctcss(1_000), 145_500_000))
            .unwrap();
        // Nothing found yet: the tone holds the audio.
        assert_eq!(
            radio.receive_metrics(Tone::Ctcss(1_000)).unwrap().tone,
            Some(ToneStatus::Lost)
        );

        // Squelch found, then CTCSS found, raised as two requests.
        radio.bus_mut().interrupts.extend([1 << 3, 1 << 7]);
        radio.bus_mut().operations.clear();
        let metrics = radio.receive_metrics(Tone::Ctcss(1_000)).unwrap();
        assert_eq!(
            metrics,
            ReceiveMetrics {
                rssi_dbm_x2: -220,
                glitch: 0xAB,
                noise: 0x7F,
                squelch_open: true,
                tone: Some(ToneStatus::Matched),
            }
        );
        assert!(metrics.should_unmute());
        // Each request is cleared before it is read, as the source does.
        let serviced = radio
            .bus()
            .operations
            .iter()
            .filter(|operation| **operation == Operation::Write(REG_INTERRUPT_FLAGS, 0))
            .count();
        assert_eq!(serviced, 2);

        // With no new request the tone stays found.
        assert_eq!(
            radio.receive_metrics(Tone::Ctcss(1_000)).unwrap().tone,
            Some(ToneStatus::Matched)
        );

        radio.bus_mut().interrupts.push(1 << 6);
        let lost = radio.receive_metrics(Tone::Ctcss(1_000)).unwrap();
        assert_eq!(lost.tone, Some(ToneStatus::Lost));
        assert!(!lost.should_unmute());

        let carrier_only = radio.receive_metrics(Tone::None).unwrap();
        assert_eq!(carrier_only.tone, None);
        assert!(carrier_only.should_unmute());
    }

    #[test]
    fn a_stuck_interrupt_request_is_serviced_a_bounded_number_of_times() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(
                Modulation::Fm,
                Tone::Dcs {
                    code: 23,
                    inverted: false,
                },
                145_500_000,
            ))
            .unwrap();
        radio.bus_mut().interrupts.extend([1 << 9; 20]);
        let metrics = radio
            .receive_metrics(Tone::Dcs {
                code: 23,
                inverted: false,
            })
            .unwrap();
        assert_eq!(metrics.tone, Some(ToneStatus::Matched));
        assert_eq!(radio.bus().interrupts.len(), 12);
    }

    #[test]
    fn the_packed_tone_tables_hold_exactly_the_reference_lists() {
        let mut tone = super::FIRST_CTCSS;
        let mut tones = std::vec![tone];
        for step in super::CTCSS_STEPS {
            tone += u16::from(step);
            tones.push(tone);
        }
        assert_eq!(tones, super::CTCSS_TONES);
        for tone in super::CTCSS_TONES {
            assert_eq!(super::nearest_ctcss(tone), Some(tone));
            assert_eq!(super::nearest_ctcss(tone + 1), Some(tone));
        }
        let standard: std::vec::Vec<u16> = (0..512)
            .filter(|bits| super::is_standard_dcs(*bits))
            .collect();
        assert_eq!(standard, super::DCS_CODES);
        assert!(!super::is_standard_dcs(512));
    }

    #[test]
    fn without_a_receive_tone_the_chip_detects_tails_as_the_reference_does() {
        // (profile, CTCSS REG_51) from Armel K5 `bk4819.c` and K1 `bk4829.c`.
        for (profile, ctcss) in [
            (crate::BK4819_PROFILE, 0x904A),
            (crate::BK4829_PROFILE, 0x9040),
        ] {
            let mut radio = Bk4819::with_profile(FakeBus::new(None), profile);
            radio.recover_to_standby().unwrap();
            radio
                .configure_receive(&setup(Modulation::Fm, Tone::None, 430_837_500))
                .unwrap();
            let sub_audio: std::vec::Vec<_> = radio
                .bus()
                .operations
                .iter()
                .filter(|operation| {
                    matches!(
                        operation,
                        Operation::Write(register, _)
                            if *register == REG_SUB_AUDIO_CONTROL
                                || *register == REG_SUB_AUDIO_FREQUENCY
                    )
                })
                .copied()
                .collect();
            // 55 Hz tail detection, CTC2: 0x2000 | 462.
            assert_eq!(
                sub_audio,
                [
                    Operation::Write(REG_SUB_AUDIO_CONTROL, ctcss),
                    Operation::Write(REG_SUB_AUDIO_FREQUENCY, 0x21CE),
                ]
            );
        }
    }

    #[test]
    fn the_untoned_tone_scan_experiment_restores_the_old_receive_word() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio.set_untoned_tone_scan(true);
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 430_837_500))
            .unwrap();
        assert!(radio
            .bus()
            .operations
            .contains(&Operation::Write(REG_SUB_AUDIO_CONTROL, 0x0300)));
        assert!(!radio
            .bus()
            .operations
            .contains(&Operation::Write(REG_SUB_AUDIO_FREQUENCY, 0x21CE)));
        // A receive tone is unaffected, and switching back restores the
        // reference's words.
        radio.bus_mut().operations.clear();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::Ctcss(1567), 430_837_500))
            .unwrap();
        assert!(radio
            .bus()
            .operations
            .contains(&Operation::Write(REG_SUB_AUDIO_CONTROL, 0x904A)));
        radio.set_untoned_tone_scan(false);
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 430_837_500))
            .unwrap();
        assert!(radio
            .bus()
            .operations
            .contains(&Operation::Write(REG_SUB_AUDIO_FREQUENCY, 0x21CE)));
    }

    #[test]
    fn a_frequency_scan_follows_the_reference_scanner() {
        use super::{REG_FREQUENCY_SCAN, REG_FREQUENCY_SCAN_HIGH, REG_FREQUENCY_SCAN_LOW};
        use crate::REG_MODE_CONTROL;

        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        assert!(
            radio.start_frequency_scan().is_err(),
            "only while receiving"
        );
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
            .unwrap();
        radio.bus_mut().operations.clear();
        radio.start_frequency_scan().unwrap();
        // Both LNA paths cleared over RX enable, then `REG_32 = 0x0245`.
        assert_eq!(
            radio.bus().operations,
            [
                Operation::Write(REG_GPIO_OUT, 0x9040),
                Operation::Write(REG_FREQUENCY_SCAN, 0x0245),
            ]
        );

        // Still scanning: bit 15 set.
        radio
            .bus_mut()
            .set_register(REG_FREQUENCY_SCAN_HIGH, 0x8000);
        assert_eq!(radio.frequency_scan_result().unwrap(), None);
        // 430.8375 MHz is 43_083_750 tens of hertz: 0x0291_67E6.
        radio
            .bus_mut()
            .set_register(REG_FREQUENCY_SCAN_HIGH, 0x0291);
        radio.bus_mut().set_register(REG_FREQUENCY_SCAN_LOW, 0x67E6);
        assert_eq!(
            radio.frequency_scan_result().unwrap(),
            Some(Frequency::from_hz(430_837_500).unwrap())
        );

        radio.bus_mut().operations.clear();
        radio.restart_frequency_scan().unwrap();
        assert_eq!(
            radio.bus().operations,
            [
                Operation::Write(REG_FREQUENCY_SCAN, 0x0244),
                Operation::Write(REG_FREQUENCY_SCAN, 0x0245),
            ]
        );

        radio.bus_mut().operations.clear();
        radio.stop_scan().unwrap();
        assert_eq!(
            radio.bus().operations,
            [
                Operation::Write(REG_FREQUENCY_SCAN, 0x0244),
                Operation::Write(REG_MODE_CONTROL, 0),
            ]
        );
        assert_eq!(radio.state(), DriverState::Standby);
        assert!(radio.frequency_scan_result().is_err());
        // Receive comes back from standby.
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
            .unwrap();
    }

    #[test]
    fn a_tone_scan_reads_the_tone_the_chip_hears() {
        let mut radio = Bk4819::with_profile(FakeBus::new(None), crate::BK4829_PROFILE);
        radio.recover_to_standby().unwrap();
        assert!(radio.start_tone_scan().is_err(), "only while receiving");
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000))
            .unwrap();
        radio.bus_mut().operations.clear();
        radio.start_tone_scan().unwrap();
        // `BK4819_SetScanFrequency`: the scan word, then `BK4819_RX_TurnOn`
        // with the BK4829 power blocks.
        assert_eq!(
            radio.bus().operations,
            [
                Operation::Write(REG_SUB_AUDIO_CONTROL, 0x0300),
                Operation::Write(super::REG_POWER_BLOCKS, 0x9F1F),
                Operation::Write(crate::REG_MODE_CONTROL, 0),
                Operation::Write(crate::REG_MODE_CONTROL, 0xBFF1),
            ]
        );

        let found = |radio: &mut Bk4819<FakeBus>, ctcss: u16, high: u16, low: u16| {
            radio.bus_mut().set_register(super::REG_CTCSS_SCAN, ctcss);
            radio
                .bus_mut()
                .set_register(super::REG_CDCSS_SCAN_HIGH, high);
            radio.bus_mut().set_register(super::REG_CDCSS_SCAN_LOW, low);
            radio.detected_tone().unwrap()
        };
        assert_eq!(found(&mut radio, 0x8000, 0x8000, 0), None);
        // 1828 * 4843 / 10000 = 885 tenths: 88.5 Hz.
        assert_eq!(found(&mut radio, 1828, 0x8000, 0), Some(Tone::Ctcss(885)));
        // 52.0 Hz is more than 5 Hz below 67.0 Hz.
        assert_eq!(found(&mut radio, 1074, 0x8000, 0), None);
        // DCS 023N, word 0x763813, read in any rotation.
        assert_eq!(
            found(&mut radio, 0x8000, 0x0763, 0x0813),
            Some(Tone::Dcs {
                code: 23,
                inverted: false
            })
        );
        let rotated = (0x0076_3813_u32 << 5 | 0x0076_3813 >> 18) & 0x007F_FFFF;
        assert_eq!(
            found(
                &mut radio,
                0x8000,
                u16::try_from(rotated >> 12).unwrap(),
                u16::try_from(rotated & 0xFFF).unwrap()
            ),
            Some(Tone::Dcs {
                code: 23,
                inverted: false
            })
        );
        // A word that is no standard code.
        assert_eq!(found(&mut radio, 0x8000, 0x0123, 0x0456), None);
    }

    #[test]
    fn receive_configuration_is_denied_from_unknown_and_faulted_states() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        assert_eq!(
            radio.configure_receive(&setup(Modulation::Fm, Tone::None, 145_500_000)),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        assert!(radio.bus().operations.is_empty());
        assert_eq!(
            radio.set_af_output(Modulation::Fm, AfOutput::Mute),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
    }

    #[test]
    fn any_configuration_bus_failure_faults_the_driver() {
        for failing_call in 1..12 {
            let mut radio = Bk4819::new(FakeBus::new(Some(failing_call)));
            radio.recover_to_standby().unwrap();
            let result = radio.configure_receive(&setup(Modulation::Am, Tone::None, 145_500_000));
            assert!(result.is_err(), "call {failing_call} did not fail");
            assert_eq!(radio.state(), DriverState::Faulted);
        }
    }
}
