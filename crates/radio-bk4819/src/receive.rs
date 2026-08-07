//! Evidence-bounded BK4819 receive configuration and metering.
//!
//! Every register address and value in this module is taken from the pinned
//! K1 reference firmware recorded in `docs/hardware-evidence.md`. Nothing here
//! can request transmit mode: the only mode-control word written is the
//! documented receive block.

use core::fmt;

use radio_domain::{Bandwidth, Frequency, Modulation, Tone};

use crate::{
    Bk4819, DriverError, DriverState, FrequencyWord, RegisterAddress, RegisterBus, MODE_STANDBY,
    REG_FREQUENCY_HIGH, REG_FREQUENCY_LOW, REG_MODE_CONTROL,
};

const REG_SOFT_RESET: RegisterAddress = RegisterAddress::known(0x00);
const REG_INTERRUPT_FLAGS: RegisterAddress = RegisterAddress::known(0x02);
const REG_DTMF_COEFFICIENT: RegisterAddress = RegisterAddress::known(0x09);
const REG_CRYSTAL: RegisterAddress = RegisterAddress::known(0x36);
const REG_MIC_GAIN: RegisterAddress = RegisterAddress::known(0x7D);
const REG_AGC_CONTROL: RegisterAddress = RegisterAddress::known(0x7E);
const REG_SUB_AUDIO_FREQUENCY: RegisterAddress = RegisterAddress::known(0x07);
const REG_CDCSS_CODE: RegisterAddress = RegisterAddress::known(0x08);
const REG_DEMODULATOR_2A: RegisterAddress = RegisterAddress::known(0x2A);
const REG_DEMODULATOR_2B: RegisterAddress = RegisterAddress::known(0x2B);
const REG_DEMODULATOR_2F: RegisterAddress = RegisterAddress::known(0x2F);
const REG_AM_DEMODULATOR: RegisterAddress = RegisterAddress::known(0x31);
const REG_GPIO_OUT: RegisterAddress = RegisterAddress::known(0x33);
const REG_POWER_BLOCKS: RegisterAddress = RegisterAddress::known(0x37);
const REG_AF_TAIL: RegisterAddress = RegisterAddress::known(0x3D);
const REG_INTERRUPT_MASK: RegisterAddress = RegisterAddress::known(0x3F);
const REG_DEMODULATOR_42: RegisterAddress = RegisterAddress::known(0x42);
const REG_FILTER_BANDWIDTH: RegisterAddress = RegisterAddress::known(0x43);
const REG_AF_OUTPUT: RegisterAddress = RegisterAddress::known(0x47);
const REG_AF_DAC_GAIN: RegisterAddress = RegisterAddress::known(0x48);
/// Low nibble of `REG_48`, the AF DAC gain applied after the receive gains.
const AF_DAC_GAIN_MASK: u16 = 0x000F;
/// Maximum AF DAC gain, the value the pinned source selects for every mode.
const AF_DAC_GAIN_MAXIMUM: u16 = 0x000F;
const REG_SQUELCH_CLOSE_GLITCH: RegisterAddress = RegisterAddress::known(0x4D);
const REG_SQUELCH_OPEN_GLITCH: RegisterAddress = RegisterAddress::known(0x4E);
const REG_SQUELCH_NOISE: RegisterAddress = RegisterAddress::known(0x4F);
const REG_SUB_AUDIO_CONTROL: RegisterAddress = RegisterAddress::known(0x51);
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
};

/// Requested audio-frequency output routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AfOutput {
    /// The demodulator runs but no audio reaches the output.
    Mute,
    /// Demodulated audio reaches the output.
    Demodulated,
}

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

/// Latched tone-squelch state decoded from the interrupt-status register.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToneStatus {
    /// The configured tone or code was found.
    Matched,
    /// The configured tone or code was lost.
    Lost,
    /// Neither a found nor a lost event is latched.
    Indeterminate,
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
        self.configure_sub_audio(setup.tone)?;
        self.write(REG_INTERRUPT_MASK, interrupt_mask(setup.tone))?;
        self.write(REG_INTERRUPT_FLAGS, 0)?;
        self.write_af_output(setup.modulation, setup.af)?;

        self.state = DriverState::Receiving {
            frequency: setup.frequency,
        };
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

    fn tone_status(&mut self, found: u16, lost: u16) -> Result<ToneStatus, DriverError<B::Error>> {
        let flags = self.read(REG_INTERRUPT_FLAGS)?;
        Ok(match (flags & found != 0, flags & lost != 0) {
            (true, false) => ToneStatus::Matched,
            (false, true) => ToneStatus::Lost,
            _ => ToneStatus::Indeterminate,
        })
    }

    fn configure_demodulator(
        &mut self,
        modulation: Modulation,
    ) -> Result<(), DriverError<B::Error>> {
        let am_control = self.read(REG_AM_DEMODULATOR)?;
        match modulation {
            Modulation::Am => {
                self.write(REG_AM_DEMODULATOR, am_control | 1)?;
                self.write(REG_DEMODULATOR_42, 0x6F5C)?;
                self.write(REG_DEMODULATOR_2A, 0x7434)?;
            }
            Modulation::Fm | Modulation::Usb => {
                self.write(REG_AM_DEMODULATOR, am_control & !1)?;
                self.write(REG_DEMODULATOR_42, 0x6B5A)?;
                self.write(REG_DEMODULATOR_2A, 0x7400)?;
                self.write(REG_DEMODULATOR_2B, 0x0000)?;
                self.write(REG_DEMODULATOR_2F, 0x9890)?;
            }
        }
        self.write(REG_AUDIO_54, 0x9009)?;
        self.write(REG_AUDIO_55, 0x31A9)?;
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

    fn configure_sub_audio(&mut self, tone: Tone) -> Result<(), DriverError<B::Error>> {
        match tone {
            Tone::None => {
                let disabled = self.profile.sub_audio_ctcss;
                self.write(REG_SUB_AUDIO_CONTROL, disabled)
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

    fn select_filter_path(&mut self, frequency: Frequency) -> Result<(), DriverError<B::Error>> {
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

    fn write_af_output(
        &mut self,
        modulation: Modulation,
        af: AfOutput,
    ) -> Result<(), DriverError<B::Error>> {
        let af_type = match af {
            AfOutput::Mute => AF_MUTE,
            AfOutput::Demodulated => match modulation {
                Modulation::Fm | Modulation::Am => AF_FM,
                Modulation::Usb => AF_BASEBAND2,
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
            .with_register(super::REG_NOISE, 0x00FF)
            .with_register(REG_INTERRUPT_FLAGS, 1 << 7);
        let mut radio = Bk4819::new(bus);
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::Ctcss(1_000), 145_500_000))
            .unwrap();
        // Configuration clears latched flags; restore the sampled fixture.
        radio
            .bus_mut()
            .set_register(REG_INTERRUPT_FLAGS, (1 << 7) | (1 << 3));

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

        radio.bus_mut().set_register(REG_INTERRUPT_FLAGS, 1 << 6);
        let lost = radio.receive_metrics(Tone::Ctcss(1_000)).unwrap();
        assert_eq!(lost.tone, Some(ToneStatus::Lost));
        assert!(!lost.should_unmute());

        let carrier_only = radio.receive_metrics(Tone::None).unwrap();
        assert_eq!(carrier_only.tone, None);
        assert!(carrier_only.should_unmute());
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
