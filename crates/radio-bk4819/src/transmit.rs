//! BK4819/BK4829 transmit sequence, after the pinned Armel sources.
//!
//! `EVID-TX056-003` accepts the Armel K5 V1 and K1 drivers and callers as
//! proven board/register authority. The chip-specific values (TX unmute, TX
//! power blocks, bias clear before TX, red LED bit) come from [`ChipProfile`]. The operations below reimplement its normal
//! enter and exit paths, split into the shared PTT effects:
//!
//! - [`Bk4819::disable_pa`]: `RADIO_SetupRegisters` zero bias, then PA clear.
//! - [`Bk4819::disable_transmit`]: `BK4819_Idle`, `REG_30 = 0`.
//! - [`Bk4819::prepare_transmit`]: the non-energizing part of
//!   `FUNCTION_Transmit`/`RADIO_SetTxParameters`: DTMF off, audio mute, RX
//!   enable clear, bandwidth, frequency, compander and scrambler off, and the
//!   microphone gain Armel's receive setup
//!   leaves in force.
//! - [`Bk4819::enable_transmit`]: `BK4819_PrepareTransmit`, which enters chip TX.
//!
//! [`ChipProfile`]: crate::ChipProfile
//! - [`Bk4819::enable_pa`] then [`Bk4819::set_pa_bias`]: filter path, PA
//!   enable, and bias, with the source's 5 ms between them left to the caller.
//! - [`Bk4819::set_transmit_tone`]: the closing `CodeType` switch of
//!   `RADIO_SetTxParameters`: `BK4819_ExitSubAu`, `BK4819_SetCTCSSFrequency`
//!   or `BK4819_SetCDCSSCodeWord`, the same writes receive uses.
//!
//! The caller owns the source's delays: 10 ms after chip TX, 5 ms after PA
//! enable, and 10 ms after bias.

use radio_domain::{Bandwidth, Frequency, Modulation, Tone};
use radio_tx_policy::{TxAuthorisation, TxRequest};

use crate::receive::{
    AfOutput, REG_AGC_CONTROL, REG_AM_DEMODULATOR, REG_FILTER_BANDWIDTH, REG_GPIO_OUT,
    REG_POWER_BLOCKS, REG_SUB_AUDIO_CONTROL,
};
use crate::{
    Bk4819, DriverError, DriverState, FrequencyWord, RegisterAddress, RegisterBus,
    REG_FREQUENCY_HIGH, REG_FREQUENCY_LOW, REG_MODE_CONTROL,
};

const REG_PA_CONTROL: RegisterAddress = RegisterAddress::known(0x36);
const REG_TX_MUTE: RegisterAddress = RegisterAddress::known(0x50);
const REG_TX_52: RegisterAddress = RegisterAddress::known(0x52);
const REG_MIC_GAIN: RegisterAddress = RegisterAddress::known(0x7D);
const REG_DTMF_CONTROL: RegisterAddress = RegisterAddress::known(0x24);

/// `REG_30` written by `BK4819_TxOn_Beep` to enter chip TX.
pub const MODE_TRANSMIT_K5: u16 = 0xC1FE;
/// `REG_52` written by `BK4819_TxOn_Beep` on both chips.
const TX_52: u16 = 0x028F;
/// `REG_7E` bits 5:3, the TX microphone DC filter, set to 5 by `ExitBypass`.
const MIC_DC_FILTER_MASK: u16 = 0b111 << 3;
const MIC_DC_FILTER: u16 = 5 << 3;
/// `REG_31` bit 3 enables the compander.
const COMPANDER_ENABLE: u16 = 1 << 3;
/// `REG_31` bit 1 enables the scrambler (`BK4819_DisableScramble`).
const SCRAMBLE_ENABLE: u16 = 1 << 1;

/// `REG_33` bit for logical GPIO0, the RX enable output.
const GPIO_RX_ENABLE: u16 = 0x40;
/// `REG_33` bit for logical GPIO1, the PA enable output.
pub const GPIO_PA_ENABLE: u16 = 0x40 >> 1;

/// `REG_36` PA-control output enable, bit 7.
const PA_CONTROL_ENABLE: u16 = 1 << 7;
/// Below this, `BK4819_SetupPowerAmplifier` selects the VHF gain pair.
const PA_GAIN_BOUNDARY_HZ: u32 = 280_000_000;
const PA_GAIN_VHF: u16 = 1 << 3;
const PA_GAIN_UHF: u16 = (4 << 3) | 2;

/// `BK4819_SetupPowerAmplifier(bias, frequency)` as one `REG_36` word.
///
/// `RADIO_SetupRegisters` calls it with `(0, 0)` for receive: zero bias, VHF
/// gain, and the PA-control output still enabled. That is the reference
/// receive state, not a defect (`EVID-TX056-003`).
pub const fn pa_control_word(bias: u8, frequency_hz: u32) -> u16 {
    let gain = if frequency_hz < PA_GAIN_BOUNDARY_HZ {
        PA_GAIN_VHF
    } else {
        PA_GAIN_UHF
    };
    ((bias as u16) << 8) | PA_CONTROL_ENABLE | gain
}

impl<B: RegisterBus> Bk4819<B> {
    /// Zeroes PA bias, then clears PA enable and the TX LED, from any state.
    ///
    /// The logical chip state is unchanged: chip TX may still be on.
    pub fn disable_pa(&mut self) -> Result<(), DriverError<B::Error>> {
        self.write(REG_PA_CONTROL, pa_control_word(0, 0))?;
        self.gpio_out &= !(GPIO_PA_ENABLE | self.profile.gpio_red_led);
        self.write(REG_GPIO_OUT, self.gpio_out)
    }

    /// Writes the idle mode from any state, leaving the chip in standby.
    pub fn disable_transmit(&mut self) -> Result<(), DriverError<B::Error>> {
        self.recover_to_standby()
    }

    /// Mutes receive audio and tunes for TX without entering chip TX.
    ///
    /// Requires standby or receive, and current authority for `request`.
    pub fn prepare_transmit(
        &mut self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
    ) -> Result<(), DriverError<B::Error>> {
        check_authority(request, authorisation)?;
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        let frequency = request.channel.transmit;
        let word = FrequencyWord::from_frequency(frequency)?;
        // `FUNCTION_Transmit` disables DTMF first: "if DTMF is enabled when
        // TX'ing, it changes the TX audio filtering".
        self.write(REG_DTMF_CONTROL, 0)?;
        self.write_af_output(Modulation::Fm, AfOutput::Mute)?;
        self.gpio_out &= !GPIO_RX_ENABLE;
        self.write(REG_GPIO_OUT, self.gpio_out)?;
        self.write(
            REG_FILTER_BANDWIDTH,
            match request.bandwidth {
                Bandwidth::Wide => self.profile.bandwidth_wide,
                Bandwidth::Narrow => self.profile.bandwidth_narrow,
            },
        )?;
        self.write(REG_FREQUENCY_LOW, word.low())?;
        self.write(REG_FREQUENCY_HIGH, word.high())?;
        let compander = self.read(REG_AM_DEMODULATOR)?;
        self.write(
            REG_AM_DEMODULATOR,
            compander & !(COMPANDER_ENABLE | SCRAMBLE_ENABLE),
        )?;
        self.write(REG_MIC_GAIN, self.profile.tx_mic_gain)?;
        self.state = DriverState::PreparedTransmit { frequency };
        Ok(())
    }

    /// Enters chip TX at the prepared frequency; the PA stays disabled.
    pub fn enable_transmit(
        &mut self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
    ) -> Result<(), DriverError<B::Error>> {
        check_authority(request, authorisation)?;
        let frequency = request.channel.transmit;
        if self.state != (DriverState::PreparedTransmit { frequency }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write_af_output(Modulation::Fm, AfOutput::Mute)?;
        let agc = self.read(REG_AGC_CONTROL)?;
        self.write(REG_AGC_CONTROL, (agc & !MIC_DC_FILTER_MASK) | MIC_DC_FILTER)?;
        self.write(REG_TX_MUTE, self.profile.tx_unmute)?;
        if self.profile.tx_clears_bias_first {
            self.write(REG_PA_CONTROL, 0)?;
        }
        self.write(REG_POWER_BLOCKS, self.profile.tx_power_blocks)?;
        self.write(REG_TX_52, TX_52)?;
        self.write(REG_MODE_CONTROL, 0)?;
        self.write(REG_MODE_CONTROL, MODE_TRANSMIT_K5)?;
        self.state = DriverState::Transmitting {
            frequency,
            class: authorisation.class(),
        };
        Ok(())
    }

    /// Selects the TX filter path, then sets PA enable and the TX LED.
    pub fn enable_pa(
        &mut self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
    ) -> Result<(), DriverError<B::Error>> {
        let frequency = self.check_transmitting(request, authorisation)?;
        self.select_filter_path(frequency)?;
        self.gpio_out |= GPIO_PA_ENABLE | self.profile.gpio_red_led;
        self.write(REG_GPIO_OUT, self.gpio_out)
    }

    /// Applies PA bias, after PA enable.
    pub fn set_pa_bias(
        &mut self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
        bias: u8,
    ) -> Result<(), DriverError<B::Error>> {
        let frequency = self.check_transmitting(request, authorisation)?;
        self.write(REG_PA_CONTROL, pa_control_word(bias, frequency.as_hz()))
    }

    /// Sends `tone` with the transmission, after the bias has settled: no
    /// tone switches sub-audio off (`BK4819_ExitSubAu`, `REG_51 = 0`).
    pub fn set_transmit_tone(
        &mut self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
        tone: Tone,
    ) -> Result<(), DriverError<B::Error>> {
        self.check_transmitting(request, authorisation)?;
        match tone {
            Tone::None => self.write(REG_SUB_AUDIO_CONTROL, 0),
            tone => self.configure_sub_audio(tone),
        }
    }

    fn check_transmitting(
        &self,
        request: &TxRequest,
        authorisation: &TxAuthorisation<'_>,
    ) -> Result<Frequency, DriverError<B::Error>> {
        check_authority(request, authorisation)?;
        let frequency = request.channel.transmit;
        if self.state
            != (DriverState::Transmitting {
                frequency,
                class: authorisation.class(),
            })
        {
            return Err(DriverError::InvalidState(self.state));
        }
        Ok(frequency)
    }
}

fn check_authority<E>(
    request: &TxRequest,
    authorisation: &TxAuthorisation<'_>,
) -> Result<(), DriverError<E>> {
    if !authorisation.matches(request) {
        return Err(DriverError::AuthorisationRequestMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{pa_control_word, GPIO_PA_ENABLE, MODE_TRANSMIT_K5};
    use crate::tests_support::{FakeBus, Operation};
    use crate::tx_fixture::{channel, request};
    use crate::{Bk4819, DriverError, DriverState, RegisterAddress, GPIO_OUT_DEFAULT};
    use radio_domain::{Tone, TxClass};
    use radio_tx_policy::{TxPolicy, TxRequest};
    use std::vec::Vec;

    fn policy() -> TxPolicy {
        crate::tx_fixture::policy(true)
    }

    fn pmr() -> TxRequest {
        request(channel(TxClass::LicenceFreePlan))
    }

    fn reg(address: u8) -> RegisterAddress {
        RegisterAddress::new(address).unwrap()
    }

    fn writes(radio: &Bk4819<FakeBus>) -> Vec<(u8, u16)> {
        radio
            .bus()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Write(address, value) => Some((address.get(), *value)),
                Operation::Read(_) => None,
            })
            .collect()
    }

    fn receiving() -> Bk4819<FakeBus> {
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();
        radio.bus_mut().operations.clear();
        radio
    }

    #[test]
    fn pa_words_match_the_source_gain_pairs() {
        assert_eq!(pa_control_word(0, 0), 0x0088);
        assert_eq!(pa_control_word(0x12, 446_006_250), 0x12A2);
        assert_eq!(pa_control_word(0x12, 145_500_000), 0x1288);
    }

    #[test]
    fn full_entry_follows_the_armel_order_and_exit_clears_pa_first() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        let mut radio = receiving();
        radio.bus_mut().set_register(reg(0x31), 0x000F);
        radio.bus_mut().set_register(reg(0x7E), 0x3000);

        radio.prepare_transmit(&request, &token).unwrap();
        // 446.00625 MHz is 44_600_625 in 10 Hz units: 0x02A8_8D31.
        assert_eq!(
            writes(&radio),
            [
                (0x24, 0x0000),
                (0x47, (6 << 12) | (1 << 6)),
                (0x33, GPIO_OUT_DEFAULT),
                (0x43, 0x3648),
                (0x38, 0x8D31),
                (0x39, 0x02A8),
                (0x31, 0x0005),
                (0x7D, 0xE95F),
            ]
        );
        radio.bus_mut().operations.clear();

        radio.enable_transmit(&request, &token).unwrap();
        assert_eq!(
            writes(&radio),
            [
                (0x47, (6 << 12) | (1 << 6)),
                (0x7E, 0x3028),
                (0x50, 0x3B20),
                (0x37, 0x1D0F),
                (0x52, 0x028F),
                (0x30, 0x0000),
                (0x30, MODE_TRANSMIT_K5),
            ]
        );
        radio.bus_mut().operations.clear();

        radio.enable_pa(&request, &token).unwrap();
        radio.set_pa_bias(&request, &token, 0x0B).unwrap();
        radio
            .set_transmit_tone(&request, &token, Tone::None)
            .unwrap();
        let uhf_lna = 0x40 >> 3;
        let path = GPIO_OUT_DEFAULT | uhf_lna;
        assert_eq!(
            writes(&radio),
            [
                (0x33, path),
                (0x33, path | GPIO_PA_ENABLE | 0x02),
                (0x36, 0x0BA2),
                (0x51, 0x0000),
            ]
        );
        radio.bus_mut().operations.clear();

        radio.disable_pa().unwrap();
        radio.disable_transmit().unwrap();
        assert_eq!(
            writes(&radio),
            [(0x36, 0x0088), (0x33, path), (0x30, 0x0000)]
        );
        assert_eq!(radio.state(), DriverState::Standby);
    }

    #[test]
    fn bk4829_uses_its_own_unmute_power_blocks_bias_clear_and_led() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        let mut radio = Bk4819::with_profile(FakeBus::new(None), crate::BK4829_PROFILE);
        radio.recover_to_standby().unwrap();
        radio.prepare_transmit(&request, &token).unwrap();
        assert!(
            writes(&radio).contains(&(0x7D, 0xE960)),
            "Armel K1 mic gain"
        );
        radio.bus_mut().operations.clear();
        radio.enable_transmit(&request, &token).unwrap();
        let entry = writes(&radio);
        assert_eq!(
            &entry[2..],
            [
                (0x50, 0x3B18),
                (0x36, 0x0000),
                (0x37, 0x9D1F),
                (0x52, 0x028F),
                (0x30, 0x0000),
                (0x30, MODE_TRANSMIT_K5),
            ]
        );
        radio.bus_mut().operations.clear();
        radio.enable_pa(&request, &token).unwrap();
        let led_and_pa = writes(&radio).last().unwrap().1;
        assert_eq!(led_and_pa & (GPIO_PA_ENABLE | 0x01), GPIO_PA_ENABLE | 0x01);
        radio.disable_pa().unwrap();
        assert_eq!(
            writes(&radio).last().unwrap().1 & (GPIO_PA_ENABLE | 0x01),
            0
        );
    }

    #[test]
    fn tones_use_each_chips_own_sub_audio_words_after_the_bias() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        // (profile, CTCSS REG_51, CDCSS REG_51) from Armel K5 `bk4819.c` and
        // K1 `bk4829.c`, as receive uses them.
        for (profile, ctcss, cdcss) in [
            (crate::BK4819_PROFILE, 0x904A, 0x8033),
            (crate::BK4829_PROFILE, 0x9040, 0xA033),
        ] {
            let mut radio = Bk4819::with_profile(FakeBus::new(None), profile);
            radio.recover_to_standby().unwrap();
            radio.prepare_transmit(&request, &token).unwrap();
            radio.enable_transmit(&request, &token).unwrap();
            radio.enable_pa(&request, &token).unwrap();
            radio.set_pa_bias(&request, &token, 0x0B).unwrap();
            radio.bus_mut().operations.clear();

            // 88.5 Hz: (885 * 206488 + 50000) / 100000 = 1827.
            radio
                .set_transmit_tone(&request, &token, Tone::Ctcss(885))
                .unwrap();
            assert_eq!(writes(&radio), [(0x51, ctcss), (0x07, 1827)]);
            radio.bus_mut().operations.clear();

            // DCS 023N is the Golay word 0x763813, sent low half first.
            radio
                .set_transmit_tone(
                    &request,
                    &token,
                    Tone::Dcs {
                        code: 23,
                        inverted: false,
                    },
                )
                .unwrap();
            assert_eq!(
                writes(&radio),
                [(0x51, cdcss), (0x07, 2775), (0x08, 0x0813), (0x08, 0x8763)]
            );
        }
    }

    #[test]
    fn a_tone_needs_chip_tx_and_matching_authority() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        let mut radio = receiving();
        assert!(matches!(
            radio.set_transmit_tone(&request, &token, Tone::Ctcss(885)),
            Err(DriverError::InvalidState(DriverState::Standby))
        ));
        assert!(radio.bus().operations.is_empty());
    }

    #[test]
    fn energizing_steps_require_their_predecessor_and_matching_authority() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        let mut radio = receiving();
        assert!(matches!(
            radio.enable_transmit(&request, &token),
            Err(DriverError::InvalidState(DriverState::Standby))
        ));
        assert!(matches!(
            radio.enable_pa(&request, &token),
            Err(DriverError::InvalidState(DriverState::Standby))
        ));
        assert!(matches!(
            radio.set_pa_bias(&request, &token, 1),
            Err(DriverError::InvalidState(DriverState::Standby))
        ));

        let mut changed = request;
        changed.conducted_mw -= 1;
        assert_eq!(
            radio.prepare_transmit(&changed, &token),
            Err(DriverError::AuthorisationRequestMismatch)
        );
        assert!(radio.bus().operations.is_empty());
    }

    #[test]
    fn a_failed_write_faults_and_pa_disable_still_runs_afterwards() {
        let policy = policy();
        let request = pmr();
        let token = policy.authorise(&request).unwrap();
        let mut radio = Bk4819::new(FakeBus::new(Some(1)));
        radio.recover_to_standby().unwrap();
        assert!(matches!(
            radio.prepare_transmit(&request, &token),
            Err(DriverError::Bus(_))
        ));
        assert_eq!(radio.state(), DriverState::Faulted);
        radio.disable_pa().unwrap();
        radio.disable_transmit().unwrap();
        assert_eq!(radio.state(), DriverState::Standby);
    }
}
