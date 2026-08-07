//! Evidence-bounded BK4819 receive configuration and metering.
//!
//! Every register address and value in this module is taken from the pinned
//! K1 reference firmware recorded in `docs/hardware-evidence.md`. Nothing here
//! can request transmit mode: the only mode-control word written is the
//! documented receive block.

use core::fmt;

use radio_domain::{Bandwidth, Frequency, Modulation, Tone};

use crate::{
    Bk4819, DriverError, DriverState, FrequencyWord, RegisterAddress, RegisterBus, MODE_RECEIVE,
    MODE_STANDBY, REG_FREQUENCY_HIGH, REG_FREQUENCY_LOW, REG_MODE_CONTROL,
};

const REG_INTERRUPT_FLAGS: RegisterAddress = RegisterAddress::known(0x02);
const REG_SUB_AUDIO_FREQUENCY: RegisterAddress = RegisterAddress::known(0x07);
const REG_CDCSS_CODE: RegisterAddress = RegisterAddress::known(0x08);
const REG_AGC_GAIN_0: RegisterAddress = RegisterAddress::known(0x10);
const REG_AGC_GAIN_1: RegisterAddress = RegisterAddress::known(0x11);
const REG_AGC_GAIN_2: RegisterAddress = RegisterAddress::known(0x12);
const REG_AGC_GAIN_3: RegisterAddress = RegisterAddress::known(0x13);
const REG_AGC_GAIN_MINUS_1: RegisterAddress = RegisterAddress::known(0x14);
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
const REG_AGC_THRESHOLD: RegisterAddress = RegisterAddress::known(0x49);
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
const REG_AGC_MODE: RegisterAddress = RegisterAddress::known(0x7B);

const POWER_BLOCKS_RECEIVE: u16 = 0x1F0F;

const AF_MUTE: u16 = 0;
const AF_FM: u16 = 1;
const AF_BASEBAND2: u16 = 5;
const AF_OUTPUT_FIXED_BITS: u16 = (6 << 12) | (1 << 6);

const BANDWIDTH_WIDE: u16 = 0x3628;
const BANDWIDTH_NARROW: u16 = 0x3648;

const SUB_AUDIO_DISABLED: u16 = 0x904A;
const SUB_AUDIO_CTCSS: u16 = 0x904A;
const SUB_AUDIO_CDCSS: u16 = 0x8033;
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
pub const RECEIVE_FILTER_PATH_BOUNDARY_HZ: u32 = 28_000_000;

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

        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(REG_POWER_BLOCKS, POWER_BLOCKS_RECEIVE)?;
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(REG_MODE_CONTROL, MODE_RECEIVE)?;

        self.configure_demodulator(setup.modulation)?;
        self.write(
            REG_FILTER_BANDWIDTH,
            match setup.bandwidth {
                Bandwidth::Wide => BANDWIDTH_WIDE,
                Bandwidth::Narrow => BANDWIDTH_NARROW,
            },
        )?;
        self.configure_agc(setup.modulation)?;

        self.write(REG_FREQUENCY_LOW, word.low())?;
        self.write(REG_FREQUENCY_HIGH, word.high())?;

        self.configure_squelch(setup.squelch)?;
        self.configure_sub_audio(setup.tone)?;
        self.write(REG_INTERRUPT_MASK, interrupt_mask(setup.tone))?;
        self.write(REG_INTERRUPT_FLAGS, 0)?;
        self.select_filter_path(setup.frequency)?;
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
        self.write(REG_AF_DAC_GAIN, 0x000F)?;
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
        self.write(REG_AGC_GAIN_3, 0x03BE)?;
        self.write(REG_AGC_GAIN_2, 0x037B)?;
        self.write(REG_AGC_GAIN_1, 0x027B)?;
        self.write(REG_AGC_GAIN_0, 0x007A)?;
        if matches!(modulation, Modulation::Am) {
            self.write(REG_AGC_GAIN_MINUS_1, 0x0000)?;
            self.write(REG_AGC_THRESHOLD, (50 << 7) | 0x20)?;
        } else {
            self.write(REG_AGC_GAIN_MINUS_1, 0x0019)?;
            self.write(REG_AGC_THRESHOLD, (84 << 7) | 0x38)?;
        }
        self.write(REG_AGC_MODE, 0x8420)
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
            Tone::None => self.write(REG_SUB_AUDIO_CONTROL, SUB_AUDIO_DISABLED),
            Tone::Ctcss(tenths_hz) => {
                self.write(REG_SUB_AUDIO_CONTROL, SUB_AUDIO_CTCSS)?;
                self.write(
                    REG_SUB_AUDIO_FREQUENCY,
                    SUB_AUDIO_MODE_CTC1 | ctcss_control_word(tenths_hz),
                )
            }
            Tone::Dcs { code, inverted } => {
                let word = cdcss_code_word(code, inverted);
                self.write(REG_SUB_AUDIO_CONTROL, SUB_AUDIO_CDCSS)?;
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
        let gpio = if frequency.as_hz() < RECEIVE_FILTER_PATH_BOUNDARY_HZ {
            GPIO_VHF_LNA
        } else {
            GPIO_UHF_LNA
        };
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
        self.write(REG_AF_OUTPUT, AF_OUTPUT_FIXED_BITS | (af_type << 8))
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
        SquelchThresholds, ToneStatus, REG_AF_OUTPUT, REG_FILTER_BANDWIDTH, REG_GPIO_OUT,
        REG_INTERRUPT_FLAGS, REG_SQUELCH_RSSI, REG_SUB_AUDIO_CONTROL, REG_SUB_AUDIO_FREQUENCY,
    };
    use crate::tests_support::{FakeBus, Operation};
    use crate::{Bk4819, DriverError, DriverState};
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
        assert!(operations.contains(&Operation::Write(REG_FILTER_BANDWIDTH, 0x3648)));
        assert!(operations.contains(&Operation::Write(REG_SQUELCH_RSSI, 0x4846)));
        assert!(operations.contains(&Operation::Write(REG_SUB_AUDIO_CONTROL, 0x904A)));
        assert!(operations.contains(&Operation::Write(REG_SUB_AUDIO_FREQUENCY, 2065)));
        assert!(operations.contains(&Operation::Write(REG_AF_OUTPUT, 0x6140)));
        assert!(operations.contains(&Operation::Write(REG_GPIO_OUT, 0x08)));
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
    fn vhf_and_uhf_paths_use_distinct_low_noise_amplifier_bits() {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio
            .configure_receive(&setup(Modulation::Fm, Tone::None, 27_000_000))
            .unwrap();
        assert!(radio
            .bus()
            .operations
            .contains(&Operation::Write(REG_GPIO_OUT, 0x04)));
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
