//! BK1080 FM broadcast receiver driver (`ADR-105`).
//!
//! The register values and sequences are the Armel firmware's, which both the
//! K5 and the K1 run (`EVID-BCFM-066-001`). The chip is reached through a
//! logical 16-bit [`RegisterBus`]; the I2C framing and any enable pin belong
//! to the target's adapter.

#![no_std]
#![forbid(unsafe_code)]

use radio_domain::{Frequency, Modulation, BROADCAST_RASTER_HZ};

/// Power configuration: enabled, audio on.
const REG_POWER: u8 = 0x02;
/// Channel and tune strobe.
const REG_CHANNEL: u8 = 0x03;
/// System configuration 2: band in bits 7:6, volume and seek threshold.
const REG_SYSTEM2: u8 = 0x05;
/// Frequency deviation (bits 15:4) and SNR (bits 3:0).
const REG_DEVIATION_SNR: u8 = 0x07;
/// Status: AFC railed (bit 12) and RSSI (bits 7:0).
const REG_STATUS: u8 = 0x0A;
/// Internal calibration register toggled once after power-up.
const REG_INTERNAL: u8 = 0x19;

const POWER_ON: u16 = 0x0201;
const POWER_MUTED: u16 = 0x4201;
const POWER_DOWN: u16 = 0x0241;
const SYSTEM2: u16 = 0x0A1F;
const BAND_SHIFT: u16 = 6;
const TUNE: u16 = 0x8000;
const AFC_RAILED: u16 = 1 << 12;

/// Registers 0 to 33 as written once after the chip is first enabled.
pub const REGISTER_TABLE: [u16; 34] = [
    0x0008, 0x1080, 0x0201, 0x0000, 0x40C0, 0x0A1F, 0x002E, 0x02FF, 0x5B11, 0x0000, 0x411E, 0x0000,
    0xCE00, 0x0000, 0x0000, 0x1000, 0x3197, 0x0000, 0x13FF, 0x9852, 0x0000, 0x0000, 0x0008, 0x0000,
    0x51E1, 0xA8BC, 0x2645, 0x00E4, 0x1CD8, 0x3A50, 0xEAE0, 0x3000, 0x0200, 0x0000,
];

/// Logical BK1080 register access.
///
/// Returning `Err` means the operation did not complete; the driver then
/// forgets its state so the next tune starts from the register table again.
pub trait RegisterBus {
    /// Adapter-specific failure.
    type Error;

    /// Writes one 16-bit register.
    fn write(&mut self, register: u8, value: u16) -> Result<(), Self::Error>;

    /// Reads one 16-bit register.
    fn read(&mut self, register: u8) -> Result<u16, Self::Error>;

    /// Drives the chip's enable line, where the board has one.
    fn set_enabled(&mut self, _enabled: bool) {}
}

/// Blocking millisecond delay used by the power-up and tune sequences.
pub trait Delay {
    /// Waits at least `ms` milliseconds.
    fn delay_ms(&mut self, ms: u32);
}

/// Why the driver refused or failed an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error<E> {
    /// The frequency is not one the broadcast receiver tunes.
    Frequency,
    /// The bus failed.
    Bus(E),
}

/// One of the chip's bands, chosen by the frequency tuned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Band {
    /// 76–108 MHz, band 1.
    Wide,
    /// 64–76 MHz, band 3.
    East,
}

impl Band {
    const fn of(hz: u32) -> Self {
        if hz < 76_000_000 {
            Self::East
        } else {
            Self::Wide
        }
    }

    const fn bits(self) -> u16 {
        match self {
            Self::Wide => 1,
            Self::East => 3,
        }
    }

    const fn floor_hz(self) -> u32 {
        match self {
            Self::Wide => 76_000_000,
            Self::East => 64_000_000,
        }
    }
}

/// One reading of the received signal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Signal {
    /// Received signal strength, chip units.
    pub rssi: u8,
    /// Signal-to-noise ratio, chip units 0–15.
    pub snr: u8,
    /// Whether the AFC has run to its limit, as off a station.
    pub afc_railed: bool,
    /// Frequency deviation, 12-bit two's complement.
    pub deviation: u16,
}

impl Signal {
    /// Whether this looks like a station, by the reference firmware's seek
    /// test: SNR above 2, AFC not railed, RSSI at least 10 and the deviation
    /// near zero.
    pub const fn is_station(self) -> bool {
        self.snr > 2
            && !self.afc_railed
            && self.rssi >= 10
            && !(self.deviation >= 280 && self.deviation <= 3815)
    }
}

/// BK1080 driver.
#[derive(Debug)]
pub struct Bk1080<B> {
    bus: B,
    initialised: bool,
    powered: bool,
    tuned: Option<Frequency>,
}

impl<B: RegisterBus> Bk1080<B> {
    /// Takes the bus; the chip is assumed powered down and uninitialised.
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            initialised: false,
            powered: false,
            tuned: None,
        }
    }

    /// The bus, for inspection.
    pub const fn bus(&self) -> &B {
        &self.bus
    }

    /// Whether the chip is powered and playing.
    pub const fn is_powered(&self) -> bool {
        self.powered
    }

    /// The frequency last tuned while powered.
    pub const fn tuned(&self) -> Option<Frequency> {
        self.tuned
    }

    /// Powers the chip up if needed and tunes `frequency`.
    ///
    /// The first power-up writes the register table and waits 310 ms in all;
    /// later ones only re-enable it. A tune waits 10 ms between the channel
    /// and its strobe.
    ///
    /// # Errors
    ///
    /// Refuses a frequency off the broadcast raster before touching the bus,
    /// or returns the bus failure.
    pub fn tune(
        &mut self,
        frequency: Frequency,
        delay: &mut impl Delay,
    ) -> Result<(), Error<B::Error>> {
        if !Modulation::is_broadcast_frequency(frequency) {
            return Err(Error::Frequency);
        }
        let result = self.try_tune(frequency, delay);
        if result.is_err() {
            self.forget();
        }
        result.map_err(Error::Bus)
    }

    fn try_tune(&mut self, frequency: Frequency, delay: &mut impl Delay) -> Result<(), B::Error> {
        let hz = frequency.as_hz();
        let band = Band::of(hz);
        if !self.powered {
            self.bus.set_enabled(true);
            if self.initialised {
                self.bus.write(REG_POWER, POWER_ON)?;
            } else {
                for (register, value) in (0_u8..).zip(REGISTER_TABLE) {
                    self.bus.write(register, value)?;
                }
                delay.delay_ms(250);
                self.bus.write(REG_INTERNAL, 0xA83C)?;
                self.bus.write(REG_INTERNAL, 0xA8BC)?;
                delay.delay_ms(60);
                self.initialised = true;
            }
            self.powered = true;
        }
        let system = (SYSTEM2 & !(0b11 << BAND_SHIFT)) | band.bits() << BAND_SHIFT;
        self.bus.write(REG_SYSTEM2, system)?;
        // At most (108 - 76) MHz / 100 kHz = 320, so this always fits.
        let channel = u16::try_from((hz - band.floor_hz()) / BROADCAST_RASTER_HZ).unwrap_or(0);
        self.bus.write(REG_CHANNEL, channel)?;
        delay.delay_ms(10);
        self.bus.write(REG_CHANNEL, channel | TUNE)?;
        self.tuned = Some(frequency);
        Ok(())
    }

    /// Mutes or unmutes the audio of a powered chip.
    ///
    /// # Errors
    ///
    /// Returns the bus failure.
    pub fn set_muted(&mut self, muted: bool) -> Result<(), Error<B::Error>> {
        if !self.powered {
            return Ok(());
        }
        let result = self
            .bus
            .write(REG_POWER, if muted { POWER_MUTED } else { POWER_ON });
        if result.is_err() {
            self.forget();
        }
        result.map_err(Error::Bus)
    }

    /// Powers the chip down. It keeps its registers, so the next tune only
    /// re-enables it.
    ///
    /// # Errors
    ///
    /// Returns the bus failure; the enable line is still released.
    pub fn power_down(&mut self) -> Result<(), Error<B::Error>> {
        let result = self.bus.write(REG_POWER, POWER_DOWN);
        self.bus.set_enabled(false);
        self.powered = false;
        self.tuned = None;
        if result.is_err() {
            self.initialised = false;
        }
        result.map_err(Error::Bus)
    }

    /// Reads the received signal.
    ///
    /// # Errors
    ///
    /// Returns the bus failure.
    pub fn signal(&mut self) -> Result<Signal, Error<B::Error>> {
        let read = |bus: &mut B| -> Result<Signal, B::Error> {
            let deviation_snr = bus.read(REG_DEVIATION_SNR)?;
            let status = bus.read(REG_STATUS)?;
            Ok(Signal {
                rssi: status.to_le_bytes()[0],
                snr: (deviation_snr & 0x000F).to_le_bytes()[0],
                afc_railed: status & AFC_RAILED != 0,
                deviation: deviation_snr >> 4,
            })
        };
        read(&mut self.bus).map_err(Error::Bus)
    }

    fn forget(&mut self) {
        self.initialised = false;
        self.powered = false;
        self.tuned = None;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{Bk1080, Delay, Error, RegisterBus, Signal, REGISTER_TABLE};
    use radio_domain::Frequency;
    use std::vec::Vec;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Op {
        Write(u8, u16),
        Read(u8),
        Enable(bool),
    }

    #[derive(Default)]
    struct Fake {
        ops: Vec<Op>,
        registers: [u16; 32],
        fail_writes_after: Option<usize>,
    }

    impl RegisterBus for Fake {
        type Error = ();

        fn write(&mut self, register: u8, value: u16) -> Result<(), ()> {
            let writes = self
                .ops
                .iter()
                .filter(|op| matches!(op, Op::Write(..)))
                .count();
            if self.fail_writes_after.is_some_and(|limit| writes >= limit) {
                return Err(());
            }
            self.ops.push(Op::Write(register, value));
            Ok(())
        }

        fn read(&mut self, register: u8) -> Result<u16, ()> {
            self.ops.push(Op::Read(register));
            Ok(self.registers[usize::from(register)])
        }

        fn set_enabled(&mut self, enabled: bool) {
            self.ops.push(Op::Enable(enabled));
        }
    }

    struct Wait<'a>(&'a mut Vec<u32>);

    impl Delay for Wait<'_> {
        fn delay_ms(&mut self, ms: u32) {
            self.0.push(ms);
        }
    }

    fn mhz_tenths(tenths: u32) -> Frequency {
        Frequency::from_hz(tenths * 100_000).unwrap()
    }

    #[test]
    fn the_first_tune_writes_the_table_then_tunes_by_channel_and_strobe() {
        let mut chip = Bk1080::new(Fake::default());
        let mut waits = Vec::new();
        chip.tune(mhz_tenths(976), &mut Wait(&mut waits)).unwrap();
        assert_eq!(waits, [250, 60, 10]);

        let mut expected = std::vec![Op::Enable(true)];
        expected.extend((0_u8..).zip(REGISTER_TABLE).map(|(r, v)| Op::Write(r, v)));
        expected.extend([
            Op::Write(0x19, 0xA83C),
            Op::Write(0x19, 0xA8BC),
            // Band 1 (76-108 MHz) in bits 7:6.
            Op::Write(0x05, 0x0A5F),
            // 97.6 MHz is 216 channels above 76.0.
            Op::Write(0x03, 216),
            Op::Write(0x03, 216 + 0x8000),
        ]);
        assert_eq!(chip.bus().ops, expected);
        assert!(chip.is_powered());
        assert_eq!(chip.tuned(), Some(mhz_tenths(976)));
    }

    #[test]
    fn a_retune_and_a_later_power_up_skip_the_table() {
        let mut chip = Bk1080::new(Fake::default());
        let mut waits = Vec::new();
        chip.tune(mhz_tenths(880), &mut Wait(&mut waits)).unwrap();
        chip.power_down().unwrap();
        let start = chip.bus().ops.len();
        chip.tune(mhz_tenths(700), &mut Wait(&mut waits)).unwrap();
        assert_eq!(
            chip.bus().ops[start..],
            [
                Op::Enable(true),
                Op::Write(0x02, 0x0201),
                // Band 3 (64-76 MHz).
                Op::Write(0x05, 0x0ADF),
                Op::Write(0x03, 60),
                Op::Write(0x03, 60 + 0x8000),
            ]
        );
        let start = chip.bus().ops.len();
        chip.tune(mhz_tenths(1080), &mut Wait(&mut waits)).unwrap();
        assert_eq!(
            chip.bus().ops[start..],
            [
                Op::Write(0x05, 0x0A5F),
                Op::Write(0x03, 320),
                Op::Write(0x03, 320 + 0x8000),
            ]
        );
    }

    #[test]
    fn power_down_mute_and_refusals_follow_the_reference_words() {
        let mut chip = Bk1080::new(Fake::default());
        let mut waits = Vec::new();
        chip.set_muted(true).unwrap();
        assert!(chip.bus().ops.is_empty(), "nothing to mute while off");
        assert_eq!(
            chip.tune(
                Frequency::from_hz(97_650_000).unwrap(),
                &mut Wait(&mut waits)
            ),
            Err(Error::Frequency)
        );
        assert_eq!(
            chip.tune(
                Frequency::from_hz(145_000_000).unwrap(),
                &mut Wait(&mut waits)
            ),
            Err(Error::Frequency)
        );
        assert!(chip.bus().ops.is_empty(), "refused before the bus");

        chip.tune(mhz_tenths(950), &mut Wait(&mut waits)).unwrap();
        let start = chip.bus().ops.len();
        chip.set_muted(true).unwrap();
        chip.set_muted(false).unwrap();
        chip.power_down().unwrap();
        assert_eq!(
            chip.bus().ops[start..],
            [
                Op::Write(0x02, 0x4201),
                Op::Write(0x02, 0x0201),
                Op::Write(0x02, 0x0241),
                Op::Enable(false),
            ]
        );
        assert!(!chip.is_powered());
        assert_eq!(chip.tuned(), None);
    }

    #[test]
    fn a_bus_failure_starts_the_next_tune_from_the_table() {
        let mut chip = Bk1080::new(Fake {
            fail_writes_after: Some(5),
            ..Fake::default()
        });
        let mut waits = Vec::new();
        assert_eq!(
            chip.tune(mhz_tenths(900), &mut Wait(&mut waits)),
            Err(Error::Bus(()))
        );
        assert!(!chip.is_powered());
        chip.bus.fail_writes_after = None;
        chip.bus.ops.clear();
        chip.tune(mhz_tenths(900), &mut Wait(&mut waits)).unwrap();
        assert_eq!(chip.bus().ops[1], Op::Write(0, REGISTER_TABLE[0]));
    }

    #[test]
    fn the_signal_reading_decodes_and_judges_a_station() {
        let mut fake = Fake::default();
        fake.registers[0x07] = (5 << 4) | 9;
        fake.registers[0x0A] = 42;
        let mut chip = Bk1080::new(fake);
        let signal = chip.signal().unwrap();
        assert_eq!(
            signal,
            Signal {
                rssi: 42,
                snr: 9,
                afc_railed: false,
                deviation: 5
            }
        );
        assert!(signal.is_station());
        assert!(!Signal { snr: 2, ..signal }.is_station());
        assert!(!Signal { rssi: 9, ..signal }.is_station());
        assert!(!Signal {
            afc_railed: true,
            ..signal
        }
        .is_station());
        assert!(!Signal {
            deviation: 1000,
            ..signal
        }
        .is_station());
        // -5 in twelve bits is close to zero.
        assert!(Signal {
            deviation: 0xFFB,
            ..signal
        }
        .is_station());
    }
}
