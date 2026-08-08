//! K1 external configuration memory on the PY32F071's second SPI peripheral.
//!
//! `EVID-K1-060` records the device and its wiring: a PY25Q16 serial NOR memory
//! on `SPI2`, `SCK` on `PA0`, `MOSI` on `PA1`, `MISO` on `PA2`, and an
//! active-low chip select on `PA3`. The vendored HAL generates those exact
//! alternate functions, so the peripheral shifts the bytes; only chip select is
//! an ordinary output, because one selection must span a whole command.
//!
//! This is where a radio's channels and settings live. The internal flash holds
//! the firmware and nothing else, so programming a radio cannot consume the
//! space its own code occupies, and the configuration survives reflashing.

use embassy_time::{block_for, Duration, Timer};
use py32_hal::gpio::{Level, Output, Speed};
use py32_hal::pac::spi::vals::Br;
use py32_hal::peripherals::{PA0, PA1, PA2, PA3, SPI2};
use py32_hal::spi::{Error as SpiError, Spi};
use radio_eeprom::{page_span, Eeprom, EepromError, JedecId, Region, RegionError};
use radio_storage::{configuration_image_len_from_header, CONFIGURATION_IMAGE_HEADER_LEN};

use crate::battery::{Calibration, CALIBRATION_ADDRESS, CALIBRATION_BYTES};
use crate::configuration::RETAINED_IMAGE_BYTES;
use crate::eeprom_bus::{EepromPort, SpiEepromBus};

/// First address of the region AFIK claims for its configuration.
///
/// The radio's own firmware maps its channels, names, settings, calibration,
/// and boot logo into the bottom of this device, reaching approximately
/// `0xD000`. One megabyte is half of the evidenced part and far above anything
/// that map can grow into, so an AFIK configuration and the vendor's data
/// cannot meet. `radio-eeprom` refuses a region below its own bound as well.
pub const CONFIGURATION_ORIGIN: u32 = 0x10_0000;
/// Bytes claimed for the configuration, one erase sector.
pub const CONFIGURATION_BYTES: u32 = 4_096;

// A region which cannot hold the largest programmable configuration would fail
// only after the operator had already programmed the radio.
const _: () =
    assert!(crate::configuration::MAX_CONFIGURATION_IMAGE_BYTES <= CONFIGURATION_BYTES as usize);

/// Divider applied to the peripheral clock for the memory.
///
/// The evidenced part accepts far more than this; the conservative divider is
/// chosen because no timing has been observed on this unit yet.
const CLOCK_DIVIDER: Br = Br::DIV16;

/// Why a configuration could not be retained or restored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainError {
    /// The image does not fit the claimed region.
    TooLarge,
    /// The external memory reported a failure.
    Memory(EepromError<SpiError>),
    /// The claimed region is not a valid region of the device.
    Region(RegionError),
    /// No memory answered on the bus.
    Absent(JedecId),
    /// The memory stayed busy for longer than the bounded wait allows.
    Busy,
}

/// Polls before a memory which never finishes is called failed.
///
/// A sector erase is the longest operation here. The bound is generous against
/// the device's specified maximum and is fixed, so a memory which never reports
/// completion fails instead of retaining the radio forever.
const READY_POLL_ATTEMPTS: u32 = 20_000;
/// Microseconds yielded between readiness polls.
const READY_POLL_INTERVAL_MICROSECONDS: u64 = 50;

/// The port the K1 presents to the external memory driver.
pub struct K1EepromPort {
    spi: Spi<'static, SPI2>,
    chip_select: Output<'static>,
}

impl EepromPort for K1EepromPort {
    type Error = SpiError;

    fn select(&mut self, asserted: bool) -> Result<(), Self::Error> {
        // The memory selects on a low level.
        self.chip_select
            .set_level(if asserted { Level::Low } else { Level::High });
        Ok(())
    }

    fn transfer_byte(&mut self, value: u8) -> Result<u8, Self::Error> {
        self.spi.transfer_byte(value)
    }

    fn delay_microseconds(&mut self, microseconds: u32) {
        // Only the driver's blocking paths reach this, and retention does not
        // use them: a stored configuration is written through the yielding path
        // above. Waits here are the driver's own short inter-poll pacing.
        block_for(Duration::from_micros(u64::from(microseconds)));
    }
}

/// Bounded accessor for the retained configuration in external memory.
pub struct RetainedConfiguration {
    eeprom: Eeprom<SpiEepromBus<K1EepromPort>>,
    region: Region,
}

impl RetainedConfiguration {
    /// Takes exclusive ownership of the memory bus and claims the region.
    ///
    /// Nothing is written here. The region claim is checked before any access,
    /// so a wrong constant fails at start-up rather than over the vendor's data.
    pub fn new(spi: SPI2, sck: PA0, mosi: PA1, miso: PA2, cs: PA3) -> Result<Self, RetainError> {
        let region =
            Region::new(CONFIGURATION_ORIGIN, CONFIGURATION_BYTES).map_err(RetainError::Region)?;
        let port = K1EepromPort {
            spi: Spi::new(spi, sck, mosi, miso, CLOCK_DIVIDER),
            // Deselected until a transfer asserts it.
            chip_select: Output::new(cs, Level::High, Speed::High),
        };
        Ok(Self {
            eeprom: Eeprom::new(SpiEepromBus::new(port)),
            region,
        })
    }

    /// Reads the memory's identification.
    ///
    /// This is the only access made before the operator has seen the radio, so
    /// a missing or unresponsive memory is reported rather than assumed.
    pub fn identify(&mut self) -> Result<JedecId, RetainError> {
        let id = self.eeprom.identify().map_err(RetainError::Memory)?;
        if id.is_present() {
            Ok(id)
        } else {
            Err(RetainError::Absent(id))
        }
    }

    /// Reads the radio's own battery calibration, if it holds a usable one.
    ///
    /// This is the one thing AFIK reads from the vendor's data: without the
    /// count the sense input reads at a known voltage, a conversion is a number
    /// and not a battery level. The read takes no region, so it cannot become a
    /// write, and a memory which does not answer simply leaves the radio without
    /// a battery reading.
    pub fn read_battery_calibration(&mut self) -> Option<Calibration> {
        let mut block = [0_u8; CALIBRATION_BYTES];
        self.eeprom
            .read_vendor(CALIBRATION_ADDRESS, &mut block)
            .ok()?;
        Calibration::from_vendor_block(&block)
    }

    /// Reads a retained image into `buffer` and returns its exact length.
    ///
    /// Only the container header is inspected here so the exact image length
    /// can be read back; the complete checksum, ordering, and object validation
    /// happen when the caller loads it. An erased or foreign region yields
    /// `None` rather than an error, because an unprogrammed radio is normal.
    pub fn read(&mut self, buffer: &mut [u8; RETAINED_IMAGE_BYTES]) -> Option<usize> {
        // The header alone gives the exact length, so an erased region or an
        // unresponsive memory is discovered after sixteen bytes rather than
        // after a whole image of timeouts.
        self.eeprom
            .read(
                self.region,
                0,
                &mut buffer[..CONFIGURATION_IMAGE_HEADER_LEN],
            )
            .ok()?;
        let length = configuration_image_len_from_header(buffer).ok()?;
        if length > RETAINED_IMAGE_BYTES {
            return None;
        }
        self.eeprom
            .read(
                self.region,
                u32::try_from(CONFIGURATION_IMAGE_HEADER_LEN).ok()?,
                &mut buffer[CONFIGURATION_IMAGE_HEADER_LEN..length],
            )
            .ok()?;
        Some(length)
    }

    /// Replaces the retained image with the first `length` bytes of `buffer`,
    /// yielding to the executor while the memory works.
    ///
    /// The whole claimed region is erased before programming, so no part of a
    /// previous image can survive to be read back beside this one. An erase
    /// takes far longer than a program, and this core runs one task at a time,
    /// so the wait is a yield rather than a spin: the receive path and the
    /// operator interface keep running while a configuration is stored.
    pub async fn write(
        &mut self,
        buffer: &[u8; RETAINED_IMAGE_BYTES],
        length: usize,
    ) -> Result<(), RetainError> {
        if length > RETAINED_IMAGE_BYTES || length as u32 > CONFIGURATION_BYTES {
            return Err(RetainError::TooLarge);
        }

        for sector in 0..self.region.sectors() {
            self.eeprom
                .issue_erase(self.region, sector)
                .map_err(RetainError::Memory)?;
            self.await_ready().await?;
        }

        let mut offset = 0_usize;
        while offset < length {
            let span = page_span(self.region.origin() + u32::try_from(offset).unwrap_or(u32::MAX))
                .min(length - offset);
            self.eeprom
                .issue_program(
                    self.region,
                    u32::try_from(offset).map_err(|_| RetainError::TooLarge)?,
                    &buffer[offset..offset + span],
                )
                .map_err(RetainError::Memory)?;
            self.await_ready().await?;
            offset += span;
        }
        Ok(())
    }

    /// Waits for the memory to finish, yielding between bounded polls.
    async fn await_ready(&mut self) -> Result<(), RetainError> {
        for _ in 0..READY_POLL_ATTEMPTS {
            if self.eeprom.is_ready().map_err(RetainError::Memory)? {
                return Ok(());
            }
            Timer::after(Duration::from_micros(READY_POLL_INTERVAL_MICROSECONDS)).await;
        }
        Err(RetainError::Busy)
    }
}
