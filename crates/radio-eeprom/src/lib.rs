//! Bounded external configuration memory, the radio's EEPROM.
//!
//! A radio's channels and settings belong in the memory the operator can
//! rewrite, not in the internal flash which holds the firmware. This crate
//! drives the external serial memory that role needs: a NOR device addressed by
//! read, page-program, and sector-erase commands.
//!
//! Nothing here owns a clock, a bus, or a pin. The caller supplies a [`NorBus`]
//! which frames one chip-selected transfer and waits a requested number of
//! microseconds, so the same driver runs on the target and against a host model
//! of the device.
//!
//! Every access is bounded twice: by the device capacity, and by the [`Region`]
//! the caller claimed. A region cannot start inside the area the radio's own
//! firmware uses, so an AFIK write cannot reach the vendor's channels,
//! settings, or calibration however wrong its arguments are.

#![no_std]
#![forbid(unsafe_code)]

use core::fmt;

/// Bytes in one programmable page.
///
/// A program which crosses a page boundary wraps inside the page on this device
/// family, so the driver splits writes at these boundaries rather than trusting
/// the caller.
pub const PAGE_BYTES: u32 = 256;
/// Bytes in one erase sector.
pub const SECTOR_BYTES: u32 = 4_096;
/// Capacity of the evidenced PY25Q16 device, 16 Mbit.
pub const CAPACITY_BYTES: u32 = 2 * 1024 * 1024;
/// Value every byte of an erased sector reads as.
pub const ERASED_BYTE: u8 = 0xFF;

/// First address AFIK may claim.
///
/// The radio's own firmware maps its channels, names, settings, calibration,
/// and boot logo into the bottom of this device; the pinned reference build
/// reaches approximately `0xD000`. This bound is the next whole 64 KiB above
/// that, so a claimed region cannot overlap anything that firmware uses even if
/// a later build grows into the rest of its map.
pub const VENDOR_RESERVED_BYTES: u32 = 0x1_0000;

/// Command opcodes this driver issues.
mod command {
    /// Read data at an address until chip select is released.
    pub const READ: u8 = 0x03;
    /// Program one page.
    pub const PAGE_PROGRAM: u8 = 0x02;
    /// Erase one four-kilobyte sector.
    pub const SECTOR_ERASE: u8 = 0x20;
    /// Set the write-enable latch.
    pub const WRITE_ENABLE: u8 = 0x06;
    /// Read status register one, whose bit zero is write-in-progress.
    pub const READ_STATUS: u8 = 0x05;
    /// Read the manufacturer and device identification.
    pub const READ_IDENTIFICATION: u8 = 0x9F;
}

/// Write-in-progress bit of the first status register.
const STATUS_WRITE_IN_PROGRESS: u8 = 0b0000_0001;
/// Write-enable-latch bit of the first status register.
const STATUS_WRITE_ENABLE_LATCH: u8 = 0b0000_0010;

/// One chip-selected transfer to the external memory.
pub trait NorBus {
    /// Bus-specific failure.
    type Error;

    /// Runs one transfer inside a single chip-select assertion.
    ///
    /// `header` and `payload` are shifted out in that order, then `response` is
    /// filled with the bytes shifted in afterwards. Either may be empty.
    fn transfer(
        &mut self,
        header: &[u8],
        payload: &[u8],
        response: &mut [u8],
    ) -> Result<(), Self::Error>;

    /// Waits at least the requested number of microseconds.
    ///
    /// The device reports completion through its status register, so this only
    /// paces polling; the driver never treats a delay as proof of completion.
    fn delay_us(&mut self, micros: u32);
}

/// Why an external-memory access could not be completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EepromError<E> {
    /// The underlying bus failed.
    Bus(E),
    /// The access fell outside the device or the claimed region.
    OutOfRange,
    /// An erase was asked for at an address which is not sector-aligned.
    Unaligned,
    /// The device stayed busy for longer than the bounded poll allows.
    Busy,
    /// The device did not accept the write-enable latch.
    WriteProtected,
    /// The device reported an identification this build does not accept.
    UnknownDevice(JedecId),
}

impl<E> fmt::Display for EepromError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(_) => formatter.write_str("external memory bus failure"),
            Self::OutOfRange => formatter.write_str("access is outside the claimed region"),
            Self::Unaligned => formatter.write_str("erase address is not sector-aligned"),
            Self::Busy => formatter.write_str("external memory stayed busy"),
            Self::WriteProtected => formatter.write_str("external memory refused write enable"),
            Self::UnknownDevice(_) => formatter.write_str("unrecognised external memory device"),
        }
    }
}

/// Manufacturer and device identification bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JedecId {
    /// JEDEC manufacturer identifier.
    pub manufacturer: u8,
    /// Device memory type.
    pub memory_type: u8,
    /// Device capacity code; capacity is `1 << code` bytes.
    pub capacity_code: u8,
}

impl JedecId {
    /// Returns the capacity the identification claims, when it is representable.
    #[must_use]
    pub const fn capacity_bytes(self) -> Option<u32> {
        if self.capacity_code < 32 {
            Some(1_u32 << self.capacity_code)
        } else {
            None
        }
    }

    /// Reports whether the device is present and answering at all.
    ///
    /// A bus with no device pulls every byte to one value, so an all-zero or
    /// all-ones identification is absence rather than a device.
    #[must_use]
    pub const fn is_present(self) -> bool {
        !self.reads_as(0x00) && !self.reads_as(0xFF)
    }

    /// Reports whether every identification byte holds one value.
    const fn reads_as(self, value: u8) -> bool {
        self.manufacturer == value && self.memory_type == value && self.capacity_code == value
    }
}

/// A bounded, sector-aligned area of the external memory claimed by a caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Region {
    origin: u32,
    len: u32,
}

impl Region {
    /// Claims a sector-aligned region above the area the radio firmware uses.
    ///
    /// Claiming is the only way to obtain write access, so the checks here are
    /// what stop an AFIK write reaching the vendor's data.
    pub const fn new(origin: u32, len: u32) -> Result<Self, RegionError> {
        if origin % SECTOR_BYTES != 0 || len % SECTOR_BYTES != 0 {
            return Err(RegionError::Unaligned);
        }
        if len == 0 {
            return Err(RegionError::Empty);
        }
        if origin < VENDOR_RESERVED_BYTES {
            return Err(RegionError::VendorReserved);
        }
        match origin.checked_add(len) {
            Some(end) if end <= CAPACITY_BYTES => Ok(Self { origin, len }),
            _ => Err(RegionError::OutOfRange),
        }
    }

    /// Returns the first address of the region.
    #[must_use]
    pub const fn origin(self) -> u32 {
        self.origin
    }

    /// Returns the length of the region in bytes.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Reports whether the region has no bytes. A claimed region never has.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Returns the number of erase sectors the region spans.
    #[must_use]
    pub const fn sectors(self) -> u32 {
        self.len / SECTOR_BYTES
    }

    /// Resolves an offset and length inside the region to a device address.
    const fn address(self, offset: u32, len: u32) -> Option<u32> {
        match (offset.checked_add(len), self.origin.checked_add(offset)) {
            (Some(end), Some(address)) if end <= self.len => Some(address),
            _ => None,
        }
    }
}

/// Why a region could not be claimed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionError {
    /// The origin or length was not a whole number of erase sectors.
    Unaligned,
    /// The region had no length.
    Empty,
    /// The region started inside the area the radio firmware uses.
    VendorReserved,
    /// The region ran past the end of the device.
    OutOfRange,
}

impl fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unaligned => formatter.write_str("region is not sector-aligned"),
            Self::Empty => formatter.write_str("region has no length"),
            Self::VendorReserved => {
                formatter.write_str("region overlaps the area the radio firmware uses")
            }
            Self::OutOfRange => formatter.write_str("region runs past the end of the device"),
        }
    }
}

/// Bounded polling attempts before a busy device is called failed.
///
/// A sector erase is the longest operation this driver issues. The bound is
/// generous against the device's specified maximum and is fixed, so a device
/// which never reports completion fails instead of hanging the caller.
const BUSY_POLL_ATTEMPTS: u32 = 100_000;
/// Microseconds between busy polls.
const BUSY_POLL_INTERVAL_US: u32 = 10;

/// The radio's external configuration memory.
pub struct Eeprom<B: NorBus> {
    bus: B,
}

impl<B: NorBus> Eeprom<B> {
    /// Wraps a bus without touching the device.
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Returns the bus, so a caller can release the pins it owns.
    pub fn release(self) -> B {
        self.bus
    }

    /// Reads the manufacturer and device identification.
    pub fn identify(&mut self) -> Result<JedecId, EepromError<B::Error>> {
        let mut response = [0_u8; 3];
        self.bus
            .transfer(&[command::READ_IDENTIFICATION], &[], &mut response)
            .map_err(EepromError::Bus)?;
        Ok(JedecId {
            manufacturer: response[0],
            memory_type: response[1],
            capacity_code: response[2],
        })
    }

    /// Reads bytes from one claimed region.
    pub fn read(
        &mut self,
        region: Region,
        offset: u32,
        buffer: &mut [u8],
    ) -> Result<(), EepromError<B::Error>> {
        let len = u32::try_from(buffer.len()).map_err(|_| EepromError::OutOfRange)?;
        let address = region.address(offset, len).ok_or(EepromError::OutOfRange)?;
        if buffer.is_empty() {
            return Ok(());
        }
        self.bus
            .transfer(&read_header(address), &[], buffer)
            .map_err(EepromError::Bus)
    }

    /// Reads bytes the radio's own firmware wrote, below the claimable regions.
    ///
    /// Some facts about a unit are only recorded in the vendor's own data, and
    /// its battery calibration is one of them: without it a voltage reading is
    /// counts, not volts. This therefore exists, and is deliberately read-only
    /// and takes no [`Region`]. A region is the thing that grants write access,
    /// so a read which cannot obtain one cannot be turned into a write, and
    /// there is no vendor-addressed erase or program to pair with this.
    pub fn read_vendor(
        &mut self,
        address: u32,
        buffer: &mut [u8],
    ) -> Result<(), EepromError<B::Error>> {
        if buffer.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(buffer.len()).map_err(|_| EepromError::OutOfRange)?;
        match address.checked_add(len) {
            Some(end) if end <= VENDOR_RESERVED_BYTES => {}
            _ => return Err(EepromError::OutOfRange),
        }
        self.bus
            .transfer(&read_header(address), &[], buffer)
            .map_err(EepromError::Bus)
    }

    /// Erases every sector of one claimed region.
    pub fn erase(&mut self, region: Region) -> Result<(), EepromError<B::Error>> {
        for sector in 0..region.sectors() {
            let address = region.origin + sector * SECTOR_BYTES;
            self.erase_sector(address)?;
        }
        Ok(())
    }

    /// Writes bytes into one claimed region, erasing the sectors they occupy.
    ///
    /// The whole region is erased first, so a shorter write always leaves the
    /// remainder erased rather than a mixture of this write and the last one.
    /// A reader therefore cannot mistake stale trailing bytes for current data.
    pub fn write(&mut self, region: Region, bytes: &[u8]) -> Result<(), EepromError<B::Error>> {
        let len = u32::try_from(bytes.len()).map_err(|_| EepromError::OutOfRange)?;
        if len > region.len {
            return Err(EepromError::OutOfRange);
        }
        self.erase(region)?;
        let mut written = 0_u32;
        for chunk in bytes.chunks(page_split(region.origin)) {
            self.program_page(region.origin + written, chunk)?;
            written += u32::try_from(chunk.len()).map_err(|_| EepromError::OutOfRange)?;
        }
        Ok(())
    }

    /// Reports whether the device has finished its last write.
    ///
    /// A caller which cannot block, such as one inside a cooperative executor,
    /// drives an operation by issuing it and then polling this between yields,
    /// so an erase does not stop everything else the radio is doing.
    pub fn is_ready(&mut self) -> Result<bool, EepromError<B::Error>> {
        Ok(self.status()? & STATUS_WRITE_IN_PROGRESS == 0)
    }

    /// Issues one sector erase without waiting for it to finish.
    pub fn issue_erase(
        &mut self,
        region: Region,
        sector: u32,
    ) -> Result<(), EepromError<B::Error>> {
        if sector >= region.sectors() {
            return Err(EepromError::OutOfRange);
        }
        self.begin_erase_sector(region.origin() + sector * SECTOR_BYTES)
    }

    /// Issues one page program without waiting for it to finish.
    ///
    /// The bytes must lie inside one page of one claimed region; use
    /// [`page_span`] to split a buffer.
    pub fn issue_program(
        &mut self,
        region: Region,
        offset: u32,
        bytes: &[u8],
    ) -> Result<(), EepromError<B::Error>> {
        let len = u32::try_from(bytes.len()).map_err(|_| EepromError::OutOfRange)?;
        let address = region.address(offset, len).ok_or(EepromError::OutOfRange)?;
        self.begin_program_page(address, bytes)
    }

    /// Erases one sector by its device address.
    fn erase_sector(&mut self, address: u32) -> Result<(), EepromError<B::Error>> {
        self.begin_erase_sector(address)?;
        self.await_ready()
    }

    /// Issues one sector erase by its device address.
    fn begin_erase_sector(&mut self, address: u32) -> Result<(), EepromError<B::Error>> {
        if address % SECTOR_BYTES != 0 {
            return Err(EepromError::Unaligned);
        }
        if !(VENDOR_RESERVED_BYTES..CAPACITY_BYTES).contains(&address) {
            return Err(EepromError::OutOfRange);
        }
        self.write_enable()?;
        let header = [
            command::SECTOR_ERASE,
            address_byte(address, 2),
            address_byte(address, 1),
            address_byte(address, 0),
        ];
        self.bus
            .transfer(&header, &[], &mut [])
            .map_err(EepromError::Bus)
    }

    /// Programs bytes which lie inside one page.
    fn program_page(&mut self, address: u32, bytes: &[u8]) -> Result<(), EepromError<B::Error>> {
        self.begin_program_page(address, bytes)?;
        self.await_ready()
    }

    /// Issues a program of bytes which lie inside one page.
    fn begin_program_page(
        &mut self,
        address: u32,
        bytes: &[u8],
    ) -> Result<(), EepromError<B::Error>> {
        if bytes.is_empty() {
            return Ok(());
        }
        let len = u32::try_from(bytes.len()).map_err(|_| EepromError::OutOfRange)?;
        let end = address.checked_add(len).ok_or(EepromError::OutOfRange)?;
        if end > CAPACITY_BYTES || address < VENDOR_RESERVED_BYTES {
            return Err(EepromError::OutOfRange);
        }
        // A program which crosses a page boundary wraps inside the page, so a
        // caller which split badly would corrupt the start of the page.
        if address / PAGE_BYTES != (end - 1) / PAGE_BYTES {
            return Err(EepromError::OutOfRange);
        }
        self.write_enable()?;
        let header = [
            command::PAGE_PROGRAM,
            address_byte(address, 2),
            address_byte(address, 1),
            address_byte(address, 0),
        ];
        self.bus
            .transfer(&header, bytes, &mut [])
            .map_err(EepromError::Bus)
    }

    /// Sets the write-enable latch and confirms the device took it.
    fn write_enable(&mut self) -> Result<(), EepromError<B::Error>> {
        self.bus
            .transfer(&[command::WRITE_ENABLE], &[], &mut [])
            .map_err(EepromError::Bus)?;
        if self.status()? & STATUS_WRITE_ENABLE_LATCH == 0 {
            return Err(EepromError::WriteProtected);
        }
        Ok(())
    }

    /// Reads the first status register.
    fn status(&mut self) -> Result<u8, EepromError<B::Error>> {
        let mut response = [0_u8; 1];
        self.bus
            .transfer(&[command::READ_STATUS], &[], &mut response)
            .map_err(EepromError::Bus)?;
        Ok(response[0])
    }

    /// Polls until the device reports no write in progress.
    fn await_ready(&mut self) -> Result<(), EepromError<B::Error>> {
        for _ in 0..BUSY_POLL_ATTEMPTS {
            if self.status()? & STATUS_WRITE_IN_PROGRESS == 0 {
                return Ok(());
            }
            self.bus.delay_us(BUSY_POLL_INTERVAL_US);
        }
        Err(EepromError::Busy)
    }
}

/// Returns the largest write which stays inside one page from an offset.
///
/// A program which crosses a page boundary wraps inside the page, so a caller
/// driving the issue-and-poll path must split its buffer on these spans.
#[must_use]
pub fn page_span(offset: u32) -> usize {
    page_split(offset)
}

/// Returns the largest write which stays inside one page from an address.
fn page_split(origin: u32) -> usize {
    let into_page = origin % PAGE_BYTES;
    usize::try_from(PAGE_BYTES - into_page).unwrap_or(256)
}

/// Returns one byte of a big-endian device address.
const fn address_byte(address: u32, index: u32) -> u8 {
    ((address >> (index * 8)) & 0xFF) as u8
}

/// Returns the header of a read command at one address.
const fn read_header(address: u32) -> [u8; 4] {
    [
        command::READ,
        address_byte(address, 2),
        address_byte(address, 1),
        address_byte(address, 0),
    ]
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        Eeprom, EepromError, JedecId, NorBus, Region, RegionError, CAPACITY_BYTES, ERASED_BYTE,
        PAGE_BYTES, SECTOR_BYTES, VENDOR_RESERVED_BYTES,
    };
    use std::{vec, vec::Vec};

    /// A host model of the device, bounded to one window of its address space.
    struct ModelBus {
        window_origin: u32,
        bytes: Vec<u8>,
        latch: bool,
        writes: usize,
        erases: usize,
        /// Programs recorded as (address, length), to prove page splitting.
        programs: Vec<(u32, usize)>,
    }

    impl ModelBus {
        fn new(window_origin: u32, window_len: usize) -> Self {
            Self {
                window_origin,
                bytes: vec![ERASED_BYTE; window_len],
                latch: false,
                writes: 0,
                erases: 0,
                programs: Vec::new(),
            }
        }

        fn slot(&mut self, address: u32) -> Option<usize> {
            let offset = address.checked_sub(self.window_origin)?;
            let index = usize::try_from(offset).ok()?;
            (index < self.bytes.len()).then_some(index)
        }
    }

    impl NorBus for ModelBus {
        type Error = ();

        fn transfer(
            &mut self,
            header: &[u8],
            payload: &[u8],
            response: &mut [u8],
        ) -> Result<(), Self::Error> {
            let address = if header.len() >= 4 {
                u32::from(header[1]) << 16 | u32::from(header[2]) << 8 | u32::from(header[3])
            } else {
                0
            };
            match header.first().copied() {
                Some(0x9F) => {
                    // A PY25Q16: 16 Mbit, capacity code 21.
                    response.copy_from_slice(&[0x85, 0x60, 0x15][..response.len()]);
                }
                Some(0x05) => {
                    response[0] = u8::from(self.latch) << 1;
                }
                Some(0x06) => self.latch = true,
                Some(0x03) => {
                    for (index, byte) in response.iter_mut().enumerate() {
                        let target = address + u32::try_from(index).unwrap();
                        *byte = self
                            .slot(target)
                            .map_or(ERASED_BYTE, |slot| self.bytes[slot]);
                    }
                }
                Some(0x20) => {
                    assert!(self.latch, "erase without the write-enable latch");
                    self.erases += 1;
                    for index in 0..SECTOR_BYTES {
                        if let Some(slot) = self.slot(address + index) {
                            self.bytes[slot] = ERASED_BYTE;
                        }
                    }
                    self.latch = false;
                }
                Some(0x02) => {
                    assert!(self.latch, "program without the write-enable latch");
                    self.writes += 1;
                    self.programs.push((address, payload.len()));
                    for (index, byte) in payload.iter().enumerate() {
                        let target = address + u32::try_from(index).unwrap();
                        if let Some(slot) = self.slot(target) {
                            // NOR programming only clears bits.
                            self.bytes[slot] &= *byte;
                        }
                    }
                    self.latch = false;
                }
                _ => panic!("unexpected command"),
            }
            Ok(())
        }

        fn delay_us(&mut self, _micros: u32) {}
    }

    fn region() -> Region {
        Region::new(0x10_0000, SECTOR_BYTES * 2).expect("region")
    }

    #[test]
    fn a_region_cannot_reach_the_area_the_radio_firmware_uses() {
        assert_eq!(
            Region::new(0, SECTOR_BYTES),
            Err(RegionError::VendorReserved)
        );
        assert_eq!(
            Region::new(VENDOR_RESERVED_BYTES - SECTOR_BYTES, SECTOR_BYTES),
            Err(RegionError::VendorReserved)
        );
        assert!(Region::new(VENDOR_RESERVED_BYTES, SECTOR_BYTES).is_ok());

        assert_eq!(
            Region::new(0x10_0000 + 1, SECTOR_BYTES),
            Err(RegionError::Unaligned)
        );
        assert_eq!(
            Region::new(0x10_0000, SECTOR_BYTES + 1),
            Err(RegionError::Unaligned)
        );
        assert_eq!(Region::new(0x10_0000, 0), Err(RegionError::Empty));
        assert_eq!(
            Region::new(CAPACITY_BYTES, SECTOR_BYTES),
            Err(RegionError::OutOfRange)
        );
        assert_eq!(
            Region::new(CAPACITY_BYTES - SECTOR_BYTES, SECTOR_BYTES * 2),
            Err(RegionError::OutOfRange)
        );
    }

    #[test]
    fn a_write_round_trips_and_leaves_the_rest_of_the_region_erased() {
        let region = region();
        let mut eeprom = Eeprom::new(ModelBus::new(region.origin(), 8_192));
        let payload = (0..600_u32)
            .map(|value| u8::try_from(value % 251).unwrap())
            .collect::<Vec<_>>();
        eeprom.write(region, &payload).expect("write");

        let mut read_back = vec![0_u8; payload.len()];
        eeprom.read(region, 0, &mut read_back).expect("read");
        assert_eq!(read_back, payload);

        // Everything past the written bytes is erased, so a shorter write can
        // never leave a reader looking at the tail of an older one.
        let mut trailing = [0_u8; 16];
        eeprom
            .read(region, u32::try_from(payload.len()).unwrap(), &mut trailing)
            .expect("read");
        assert_eq!(trailing, [ERASED_BYTE; 16]);
    }

    #[test]
    fn writes_are_split_at_page_boundaries_and_every_sector_is_erased() {
        let region = region();
        let mut eeprom = Eeprom::new(ModelBus::new(region.origin(), 8_192));
        let payload = vec![0x5A_u8; 600];
        eeprom.write(region, &payload).expect("write");

        let bus = eeprom.release();
        assert_eq!(bus.erases, 2, "both sectors of the region are erased");
        assert_eq!(
            bus.programs,
            vec![
                (region.origin(), 256),
                (region.origin() + 256, 256),
                (region.origin() + 512, 88)
            ],
            "no program crosses a page boundary"
        );
    }

    #[test]
    fn access_outside_the_claimed_region_is_refused() {
        let region = region();
        let mut eeprom = Eeprom::new(ModelBus::new(region.origin(), 8_192));

        let mut buffer = [0_u8; 16];
        assert_eq!(
            eeprom.read(region, region.len() - 8, &mut buffer),
            Err(EepromError::OutOfRange),
            "a read may not run off the end of the region"
        );
        assert_eq!(
            eeprom.write(
                region,
                &vec![0_u8; usize::try_from(region.len()).unwrap() + 1]
            ),
            Err(EepromError::OutOfRange)
        );

        // The last byte of the region is reachable, so the bound is exact.
        eeprom
            .read(region, region.len() - 16, &mut buffer)
            .expect("the final bytes are readable");
    }

    #[test]
    fn the_issue_and_poll_path_writes_the_same_bytes_as_the_blocking_one() {
        let region = region();
        let payload = (0..600_u32)
            .map(|value| u8::try_from(value % 251).unwrap())
            .collect::<Vec<_>>();

        // The blocking path, for comparison.
        let mut blocking = Eeprom::new(ModelBus::new(region.origin(), 8_192));
        blocking.write(region, &payload).expect("blocking write");
        let mut expected = vec![0_u8; payload.len()];
        blocking.read(region, 0, &mut expected).expect("read");

        // The same sequence driven by a caller which polls instead of blocking,
        // which is what an executor task must do so an erase does not stop the
        // rest of the radio.
        let mut driven = Eeprom::new(ModelBus::new(region.origin(), 8_192));
        for sector in 0..region.sectors() {
            driven.issue_erase(region, sector).expect("issue erase");
            let mut polls = 0;
            while !driven.is_ready().expect("status") {
                polls += 1;
                assert!(polls < 1_000, "the model must finish an erase");
            }
        }
        let mut offset = 0_u32;
        while (offset as usize) < payload.len() {
            let span =
                super::page_span(region.origin() + offset).min(payload.len() - offset as usize);
            let end = offset as usize + span;
            driven
                .issue_program(region, offset, &payload[offset as usize..end])
                .expect("issue program");
            while !driven.is_ready().expect("status") {}
            offset += u32::try_from(span).unwrap();
        }

        let mut actual = vec![0_u8; payload.len()];
        driven.read(region, 0, &mut actual).expect("read");
        assert_eq!(actual, expected);
        assert_eq!(actual, payload);
    }

    #[test]
    fn identification_reports_capacity_and_absence() {
        let region = region();
        let mut eeprom = Eeprom::new(ModelBus::new(region.origin(), 4_096));
        let id = eeprom.identify().expect("identify");
        assert_eq!(id.capacity_bytes(), Some(CAPACITY_BYTES));
        assert!(id.is_present());

        assert!(!JedecId::default().is_present());
        assert!(!JedecId {
            manufacturer: 0xFF,
            memory_type: 0xFF,
            capacity_code: 0xFF,
        }
        .is_present());
    }

    #[test]
    fn a_page_is_the_largest_program_and_a_sector_the_erase_unit() {
        assert_eq!(PAGE_BYTES, 256);
        assert_eq!(SECTOR_BYTES, 4_096);
        assert_eq!(CAPACITY_BYTES, 2 * 1024 * 1024);
    }
}
