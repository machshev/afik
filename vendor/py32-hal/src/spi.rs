//! Bounded SPI support.
//!
//! This local AFIK extension implements the cooperative transmit-only surface
//! required by the evidenced K1 display path, and the blocking full-duplex
//! surface required by the evidenced K1 external configuration memory. The
//! memory answers on its own data line, so it cannot be driven by the
//! transmit-only interface.

use core::marker::PhantomData;

use embassy_futures::yield_now;
use embassy_hal_internal::PeripheralRef;

use crate::gpio::{AfType, AnyPin, OutputType, SealedPin as _, Speed};
use crate::pac::spi::vals::{Bidimode, Bidioe, Br, Cpha, Cpol, Lsbfirst, Mstr, Rxonly};
use crate::pac::spi::Spi as Regs;
use crate::{Peripheral, peripherals};

/// Maximum bytes transferred before yielding to the executor.
pub const ASYNC_WRITE_CHUNK_BYTES: usize = 16;
const MAX_STATUS_POLLS: usize = 65_535;
/// Status polls the blocking driver makes before giving up on one byte.
///
/// The blocking driver runs inside an executor task, so a device which never
/// answers must cost a small bounded delay rather than a visible stall.
const MAX_BLOCKING_STATUS_POLLS: usize = 4_096;

const fn should_yield(bytes_or_polls: usize) -> bool {
    bytes_or_polls != 0 && bytes_or_polls.is_multiple_of(ASYNC_WRITE_CHUNK_BYTES)
}

/// Transmit failure reported by the bounded SPI interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A peripheral mode-fault flag was observed.
    ModeFault,
    /// A peripheral overrun flag was observed.
    Overrun,
    /// A peripheral CRC-error flag was observed.
    Crc,
    /// The expected hardware status did not arrive within the bounded poll limit.
    Timeout,
}

/// Transmit-only SPI driver with cooperative async writes.
pub struct SpiTx<'d, T: Instance> {
    _peri: PeripheralRef<'d, T>,
    sck: PeripheralRef<'d, AnyPin>,
    mosi: PeripheralRef<'d, AnyPin>,
    _phantom: PhantomData<T>,
}

impl<'d, T: Instance> SpiTx<'d, T> {
    /// Creates a mode-3, MSB-first, divide-by-64 transmit-only SPI interface.
    pub fn new(
        peri: impl Peripheral<P = T> + 'd,
        sck: impl Peripheral<P = impl SckPin<T>> + 'd,
        mosi: impl Peripheral<P = impl MosiPin<T>> + 'd,
    ) -> Self {
        let peri = peri.into_ref();
        let sck = new_pin!(sck, AfType::output(OutputType::PushPull, Speed::High)).unwrap();
        let mosi = new_pin!(mosi, AfType::output(OutputType::PushPull, Speed::High)).unwrap();

        T::RCC_INFO.enable_and_reset();
        let regs = T::regs();
        regs.cr1().write(|w| {
            w.set_cpha(Cpha::SECONDEDGE);
            w.set_cpol(Cpol::IDLEHIGH);
            w.set_mstr(Mstr::MASTER);
            w.set_br(Br::DIV64);
            w.set_lsbfirst(Lsbfirst::MSBFIRST);
            w.set_ssi(true);
            w.set_ssm(true);
            w.set_bidioe(Bidioe::TRANSMIT);
            w.set_bidimode(Bidimode::BIDIRECTIONAL);
            w.set_spe(true);
        });

        Self {
            _peri: peri,
            sck,
            mosi,
            _phantom: PhantomData,
        }
    }

    /// Writes all bytes and yields after each fixed-size chunk.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for (index, byte) in bytes.iter().copied().enumerate() {
            self.wait_for(|regs| regs.sr().read().txe()).await?;
            T::regs().dr().write(|w| w.set_dr(u16::from(byte)));
            if should_yield(index + 1) {
                yield_now().await;
            }
        }
        self.wait_for(|regs| !regs.sr().read().bsy()).await
    }

    async fn wait_for(&self, ready: impl Fn(Regs) -> bool) -> Result<(), Error> {
        for poll in 0..MAX_STATUS_POLLS {
            let regs = T::regs();
            let status = regs.sr().read();
            if status.modf() {
                return Err(Error::ModeFault);
            }
            if status.ovr() {
                return Err(Error::Overrun);
            }
            if status.crcerr() {
                return Err(Error::Crc);
            }
            if ready(regs) {
                return Ok(());
            }
            if should_yield(poll + 1) {
                yield_now().await;
            }
        }
        Err(Error::Timeout)
    }
}

impl<T: Instance> Drop for SpiTx<'_, T> {
    fn drop(&mut self) {
        T::regs().cr1().modify(|w| w.set_spe(false));
        self.sck.set_as_disconnected();
        self.mosi.set_as_disconnected();
        T::RCC_INFO.disable();
    }
}

#[allow(private_interfaces)]
pub(crate) trait SealedInstance {
    fn regs() -> Regs;
}

/// SPI peripheral instance.
#[allow(private_bounds)]
pub trait Instance: Peripheral<P = Self> + SealedInstance + crate::rcc::RccPeripheral {}

/// SPI clock pin.
pub trait SckPin<T: Instance>: crate::gpio::Pin {
    /// Alternate-function number for this SPI instance.
    fn af_num(&self) -> u8;
}

/// SPI controller-output pin.
pub trait MosiPin<T: Instance>: crate::gpio::Pin {
    /// Alternate-function number for this SPI instance.
    fn af_num(&self) -> u8;
}

/// SPI controller-input pin.
pub trait MisoPin<T: Instance>: crate::gpio::Pin {
    /// Alternate-function number for this SPI instance.
    fn af_num(&self) -> u8;
}

macro_rules! impl_spi {
    ($instance:ident) => {
        impl SealedInstance for peripherals::$instance {
            fn regs() -> Regs {
                unsafe { Regs::from_ptr(crate::pac::$instance.as_ptr()) }
            }
        }

        impl Instance for peripherals::$instance {}
    };
}

foreach_peripheral!(
    (spi, $instance:ident) => {
        impl_spi!($instance);
    };
);

/// Blocking full-duplex SPI driver.
///
/// Mode zero, most significant bit first. Chip select is left to the caller as
/// an ordinary output, because a device such as a serial memory needs one
/// selection to span several transfers.
pub struct Spi<'d, T: Instance> {
    _peri: PeripheralRef<'d, T>,
    sck: PeripheralRef<'d, AnyPin>,
    mosi: PeripheralRef<'d, AnyPin>,
    miso: PeripheralRef<'d, AnyPin>,
    _phantom: PhantomData<T>,
}

impl<'d, T: Instance> Spi<'d, T> {
    /// Creates a mode-zero, MSB-first, full-duplex controller interface.
    pub fn new(
        peri: impl Peripheral<P = T> + 'd,
        sck: impl Peripheral<P = impl SckPin<T>> + 'd,
        mosi: impl Peripheral<P = impl MosiPin<T>> + 'd,
        miso: impl Peripheral<P = impl MisoPin<T>> + 'd,
        divider: Br,
    ) -> Self {
        let peri = peri.into_ref();
        let sck = new_pin!(sck, AfType::output(OutputType::PushPull, Speed::High)).unwrap();
        let mosi = new_pin!(mosi, AfType::output(OutputType::PushPull, Speed::High)).unwrap();
        let miso = new_pin!(miso, AfType::input(crate::gpio::Pull::None)).unwrap();

        T::RCC_INFO.enable_and_reset();
        let regs = T::regs();
        regs.cr1().write(|w| {
            w.set_cpha(Cpha::FIRSTEDGE);
            w.set_cpol(Cpol::IDLELOW);
            w.set_mstr(Mstr::MASTER);
            w.set_br(divider);
            w.set_lsbfirst(Lsbfirst::MSBFIRST);
            w.set_ssi(true);
            w.set_ssm(true);
            w.set_rxonly(Rxonly::FULLDUPLEX);
            w.set_bidimode(Bidimode::UNIDIRECTIONAL);
            w.set_spe(true);
        });

        Self {
            _peri: peri,
            sck,
            mosi,
            miso,
            _phantom: PhantomData,
        }
    }

    /// Shifts one byte out and returns the byte shifted in beside it.
    pub fn transfer_byte(&mut self, value: u8) -> Result<u8, Error> {
        let regs = T::regs();
        self.wait_for(|regs| regs.sr().read().txe())?;
        // A byte at the slowest supported divider takes a few hundred
        // peripheral clocks, so a bound in the thousands is generous for a
        // working peripheral and quick to give up on a dead one.
        regs.dr().write(|w| w.set_dr(u16::from(value)));
        self.wait_for(|regs| regs.sr().read().rxne())?;
        Ok(u8::try_from(regs.dr().read().dr() & 0xFF).unwrap_or(0))
    }

    /// Shifts every byte of `write` out, discarding what arrives beside them.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for byte in bytes {
            self.transfer_byte(*byte)?;
        }
        Ok(())
    }

    /// Fills `buffer` with the bytes arriving while zeros are shifted out.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<(), Error> {
        for slot in buffer.iter_mut() {
            *slot = self.transfer_byte(0)?;
        }
        Ok(())
    }

    fn wait_for(&self, ready: impl Fn(Regs) -> bool) -> Result<(), Error> {
        for _ in 0..MAX_BLOCKING_STATUS_POLLS {
            let regs = T::regs();
            let status = regs.sr().read();
            if status.modf() {
                return Err(Error::ModeFault);
            }
            if status.ovr() {
                return Err(Error::Overrun);
            }
            if status.crcerr() {
                return Err(Error::Crc);
            }
            if ready(regs) {
                return Ok(());
            }
        }
        Err(Error::Timeout)
    }
}

impl<T: Instance> Drop for Spi<'_, T> {
    fn drop(&mut self) {
        T::regs().cr1().modify(|w| w.set_spe(false));
        self.sck.set_as_disconnected();
        self.mosi.set_as_disconnected();
        self.miso.set_as_disconnected();
        T::RCC_INFO.disable();
    }
}
