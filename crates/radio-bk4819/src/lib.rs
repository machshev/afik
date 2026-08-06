//! Evidence-bounded BK4819 register commands and TX authority boundary.
//!
//! This crate models only the post-initialization command contract recorded in
//! `docs/hardware-evidence.md`. It is not a physical bus or complete chip
//! initialization implementation.

#![no_std]
#![forbid(unsafe_code)]

mod receive;

pub use receive::{
    cdcss_code_word, ctcss_control_word, AfOutput, ReceiveMetrics, ReceiveSetup, SquelchError,
    SquelchThresholds, ToneStatus, RECEIVE_FILTER_PATH_BOUNDARY_HZ,
};

use core::fmt;
use radio_domain::{ActiveChannel, Frequency, TxClass};
use radio_tx_policy::TxAuthorisation;

const REG_MODE_CONTROL: RegisterAddress = RegisterAddress::known(0x30);
const REG_FREQUENCY_LOW: RegisterAddress = RegisterAddress::known(0x38);
const REG_FREQUENCY_HIGH: RegisterAddress = RegisterAddress::known(0x39);
const REG_SQUELCH_STATUS: RegisterAddress = RegisterAddress::known(0x0C);
const REG_RSSI: RegisterAddress = RegisterAddress::known(0x67);

const MODE_STANDBY: u16 = 0x0000;
const MODE_RECEIVE: u16 = 0xBEF1;
const MODE_TRANSMIT: u16 = 0x80FE;
const RSSI_MASK: u16 = 0x01FF;
const SQUELCH_OPEN: u16 = 1 << 1;

/// A validated BK4819 seven-bit register address.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegisterAddress(u8);

impl RegisterAddress {
    /// Validates a seven-bit register address.
    pub const fn new(value: u8) -> Result<Self, AddressError> {
        if value <= 0x7F {
            Ok(Self(value))
        } else {
            Err(AddressError)
        }
    }

    /// Returns the encoded seven-bit address value.
    pub const fn get(self) -> u8 {
        self.0
    }

    const fn known(value: u8) -> Self {
        Self(value)
    }
}

/// A register address exceeded the source-backed seven-bit envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressError;

impl fmt::Display for AddressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BK4819 register address exceeds seven bits")
    }
}

/// Fallible logical register bus implemented later by physical and simulated adapters.
///
/// Returning `Err` means the requested read or write was not completed. The
/// driver treats every error as an unknown hardware state and latches fault.
pub trait RegisterBus {
    /// Adapter-specific bus failure.
    type Error;

    /// Writes one complete 16-bit register value.
    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error>;

    /// Reads one complete 16-bit register value.
    fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error>;
}

/// A BK4819 10-Hz frequency control word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrequencyWord(u32);

impl FrequencyWord {
    /// Converts an exact integer frequency without rounding.
    #[allow(clippy::manual_is_multiple_of)] // Keep the workspace Rust 1.86 MSRV.
    pub fn from_frequency(frequency: Frequency) -> Result<Self, FrequencyError> {
        let hertz = frequency.as_hz();
        if hertz % 10 != 0 {
            return Err(FrequencyError::NotTenHertzAligned);
        }
        Ok(Self(hertz / 10))
    }

    /// Returns the low word written to `REG_38`.
    pub const fn low(self) -> u16 {
        let bytes = self.0.to_le_bytes();
        u16::from_le_bytes([bytes[0], bytes[1]])
    }

    /// Returns the high word written to `REG_39`.
    pub const fn high(self) -> u16 {
        let bytes = self.0.to_le_bytes();
        u16::from_le_bytes([bytes[2], bytes[3]])
    }

    /// Returns the complete 10-Hz control word.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Frequency conversion failed before any bus operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrequencyError {
    /// The frequency cannot be represented exactly in 10-Hz units.
    NotTenHertzAligned,
}

impl fmt::Display for FrequencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTenHertzAligned => {
                formatter.write_str("BK4819 frequency is not aligned to 10 Hz")
            }
        }
    }
}

/// Driver state known only from successfully completed logical bus operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverState {
    /// No successful neutralizing write has established chip mode.
    Unknown,
    /// Every documented mode-control block is disabled.
    Standby,
    /// The inferred receive mode was written for this frequency.
    Receiving {
        /// Requested receive frequency.
        frequency: Frequency,
    },
    /// The inferred transmit mode was written for this authorized class/frequency.
    Transmitting {
        /// Requested transmit frequency.
        frequency: Frequency,
        /// Exact policy class approved by the token.
        class: TxClass,
    },
    /// A bus operation failed and the physical state is unknown.
    Faulted,
}

/// One source-backed receive status sample using integer half-dBm units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveStatus {
    /// Approximate RSSI multiplied by two, preserving the documented 0.5 dB step.
    pub rssi_dbm_x2: i16,
    /// Read-only squelch link result.
    pub squelch_open: bool,
}

/// BK4819 command or authority-boundary failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DriverError<E> {
    /// The logical register adapter failed; driver state is now faulted.
    Bus(E),
    /// The operation is forbidden from the current known state.
    InvalidState(DriverState),
    /// The exact integer frequency cannot be encoded.
    Frequency(FrequencyError),
    /// The capability token approves a different class than the channel.
    AuthorisationClassMismatch {
        /// Class carried by the central-policy token.
        approved: TxClass,
        /// Class required by the requested active channel.
        requested: TxClass,
    },
}

impl<E: fmt::Display> fmt::Display for DriverError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(formatter, "BK4819 register bus failed: {error}"),
            Self::InvalidState(state) => write!(formatter, "invalid BK4819 state: {state:?}"),
            Self::Frequency(error) => write!(formatter, "{error}"),
            Self::AuthorisationClassMismatch {
                approved,
                requested,
            } => write!(
                formatter,
                "TX authorization class {approved:?} does not match {requested:?}"
            ),
        }
    }
}

impl<E> From<FrequencyError> for DriverError<E> {
    fn from(error: FrequencyError) -> Self {
        Self::Frequency(error)
    }
}

/// Post-initialization BK4819 command driver over an unbound register bus.
pub struct Bk4819<B: RegisterBus> {
    bus: B,
    state: DriverState,
}

impl<B: RegisterBus> Bk4819<B> {
    /// Wraps a bus in unknown mode; no hardware operation is performed.
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            state: DriverState::Unknown,
        }
    }

    /// Returns the last state established entirely by successful operations.
    pub const fn state(&self) -> DriverState {
        self.state
    }

    /// Returns an immutable reference to the underlying adapter for observation.
    pub const fn bus(&self) -> &B {
        &self.bus
    }

    /// Returns a mutable reference to the underlying adapter for test fixtures.
    #[cfg(test)]
    pub(crate) const fn bus_mut(&mut self) -> &mut B {
        &mut self.bus
    }

    /// Writes the neutral mode from any state, including unknown or faulted.
    pub fn recover_to_standby(&mut self) -> Result<(), DriverError<B::Error>> {
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.state = DriverState::Standby;
        Ok(())
    }

    /// Tunes and enters the inferred receive mode from standby or receive.
    pub fn start_receive(&mut self, frequency: Frequency) -> Result<(), DriverError<B::Error>> {
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        let word = FrequencyWord::from_frequency(frequency)?;
        self.program_mode(word, MODE_RECEIVE)?;
        self.state = DriverState::Receiving { frequency };
        Ok(())
    }

    /// Reads approximate RSSI and squelch only while known to be receiving.
    pub fn receive_status(&mut self) -> Result<ReceiveStatus, DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Receiving { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        let rssi_raw = self.read(REG_RSSI)? & RSSI_MASK;
        let squelch = self.read(REG_SQUELCH_STATUS)?;
        let rssi_dbm_x2 = i16::try_from(rssi_raw).unwrap_or(0) - 320;
        Ok(ReceiveStatus {
            rssi_dbm_x2,
            squelch_open: squelch & SQUELCH_OPEN != 0,
        })
    }

    /// Tunes and enters inferred TX mode only with matching central authority.
    ///
    /// This method is the crate's only path to the TX mode-control word.
    pub fn start_transmit(
        &mut self,
        channel: ActiveChannel,
        authorisation: &TxAuthorisation,
    ) -> Result<(), DriverError<B::Error>> {
        if authorisation.class() != channel.tx_class {
            return Err(DriverError::AuthorisationClassMismatch {
                approved: authorisation.class(),
                requested: channel.tx_class,
            });
        }
        if !matches!(
            self.state,
            DriverState::Standby | DriverState::Receiving { .. }
        ) {
            return Err(DriverError::InvalidState(self.state));
        }
        let word = FrequencyWord::from_frequency(channel.transmit)?;
        self.program_mode(word, MODE_TRANSMIT)?;
        self.state = DriverState::Transmitting {
            frequency: channel.transmit,
            class: channel.tx_class,
        };
        Ok(())
    }

    /// Stops a known transmit session by writing neutral mode.
    pub fn stop_transmit(&mut self) -> Result<(), DriverError<B::Error>> {
        if !matches!(self.state, DriverState::Transmitting { .. }) {
            return Err(DriverError::InvalidState(self.state));
        }
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.state = DriverState::Standby;
        Ok(())
    }

    fn program_mode(
        &mut self,
        frequency: FrequencyWord,
        final_mode: u16,
    ) -> Result<(), DriverError<B::Error>> {
        self.write(REG_MODE_CONTROL, MODE_STANDBY)?;
        self.write(REG_FREQUENCY_LOW, frequency.low())?;
        self.write(REG_FREQUENCY_HIGH, frequency.high())?;
        self.write(REG_MODE_CONTROL, final_mode)
    }

    fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), DriverError<B::Error>> {
        if let Err(error) = self.bus.write(address, value) {
            self.state = DriverState::Faulted;
            return Err(DriverError::Bus(error));
        }
        Ok(())
    }

    fn read(&mut self, address: RegisterAddress) -> Result<u16, DriverError<B::Error>> {
        match self.bus.read(address) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.state = DriverState::Faulted;
                Err(DriverError::Bus(error))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    extern crate std;

    use super::{RegisterAddress, RegisterBus};
    use std::vec::Vec;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum Operation {
        Write(RegisterAddress, u16),
        Read(RegisterAddress),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct FakeBusError;

    impl core::fmt::Display for FakeBusError {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter.write_str("injected bus failure")
        }
    }

    pub(crate) struct FakeBus {
        registers: [u16; 128],
        pub(crate) operations: Vec<Operation>,
        fail_on_call: Option<usize>,
        calls: usize,
    }

    impl FakeBus {
        pub(crate) fn new(fail_on_call: Option<usize>) -> Self {
            Self {
                registers: [0; 128],
                operations: Vec::new(),
                fail_on_call,
                calls: 0,
            }
        }

        pub(crate) fn with_register(mut self, address: RegisterAddress, value: u16) -> Self {
            self.registers[usize::from(address.get())] = value;
            self
        }

        pub(crate) fn set_register(&mut self, address: RegisterAddress, value: u16) {
            self.registers[usize::from(address.get())] = value;
        }

        fn before_operation(&mut self) -> Result<(), FakeBusError> {
            let call = self.calls;
            self.calls += 1;
            if self.fail_on_call == Some(call) {
                Err(FakeBusError)
            } else {
                Ok(())
            }
        }
    }

    impl RegisterBus for FakeBus {
        type Error = FakeBusError;

        fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
            self.before_operation()?;
            self.registers[usize::from(address.get())] = value;
            self.operations.push(Operation::Write(address, value));
            Ok(())
        }

        fn read(&mut self, address: RegisterAddress) -> Result<u16, Self::Error> {
            self.before_operation()?;
            self.operations.push(Operation::Read(address));
            Ok(self.registers[usize::from(address.get())])
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::tests_support::{FakeBus, FakeBusError, Operation};
    use super::{
        Bk4819, DriverError, DriverState, FrequencyError, FrequencyWord, ReceiveStatus,
        RegisterAddress, MODE_RECEIVE, MODE_STANDBY, MODE_TRANSMIT, REG_FREQUENCY_HIGH,
        REG_FREQUENCY_LOW, REG_MODE_CONTROL, REG_RSSI, REG_SQUELCH_STATUS, SQUELCH_OPEN,
    };
    use radio_domain::{ActiveChannel, Frequency, TxClass};
    use radio_tx_policy::{PermissionSet, StoredPermissions, TxPolicy};
    use std::vec;

    fn frequency(hertz: u32) -> Frequency {
        Frequency::from_hz(hertz).unwrap()
    }

    fn channel(class: TxClass) -> ActiveChannel {
        ActiveChannel {
            receive: frequency(145_500_000),
            transmit: frequency(145_500_000),
            tx_class: class,
        }
    }

    fn policy(classes: &[TxClass]) -> TxPolicy {
        let mut permissions = PermissionSet::none();
        for class in classes {
            permissions = permissions.with(*class, true);
        }
        TxPolicy::load(&StoredPermissions::new(permissions, 1).encode()).0
    }

    #[test]
    fn register_and_frequency_encodings_are_exact_and_bounded() {
        assert_eq!(RegisterAddress::new(0x7F).unwrap().get(), 0x7F);
        assert!(RegisterAddress::new(0x80).is_err());
        let word = FrequencyWord::from_frequency(frequency(409_750_000)).unwrap();
        assert_eq!(word.get(), 40_975_000);
        assert_eq!(word.low(), 0x3A98);
        assert_eq!(word.high(), 0x0271);
        assert_eq!(
            FrequencyWord::from_frequency(frequency(409_750_001)),
            Err(FrequencyError::NotTenHertzAligned)
        );
    }

    #[test]
    fn receive_plan_is_standby_first_and_status_uses_sourced_fields() {
        let bus = FakeBus::new(None)
            .with_register(REG_RSSI, 0xFE64)
            .with_register(REG_SQUELCH_STATUS, 0x0002);
        let mut radio = Bk4819::new(bus);
        radio.recover_to_standby().unwrap();
        radio.start_receive(frequency(409_750_000)).unwrap();
        assert_eq!(
            radio.bus().operations,
            vec![
                Operation::Write(REG_MODE_CONTROL, MODE_STANDBY),
                Operation::Write(REG_MODE_CONTROL, MODE_STANDBY),
                Operation::Write(REG_FREQUENCY_LOW, 0x3A98),
                Operation::Write(REG_FREQUENCY_HIGH, 0x0271),
                Operation::Write(REG_MODE_CONTROL, MODE_RECEIVE),
            ]
        );
        assert_eq!(
            radio.receive_status().unwrap(),
            ReceiveStatus {
                rssi_dbm_x2: -220,
                squelch_open: true,
            }
        );
        assert_eq!(
            &radio.bus().operations[5..],
            &[
                Operation::Read(REG_RSSI),
                Operation::Read(REG_SQUELCH_STATUS)
            ]
        );
    }

    #[test]
    fn only_a_matching_class_token_reaches_transmit_mode() {
        let tx_policy = policy(&[TxClass::Amateur, TxClass::Business]);
        let amateur = tx_policy.authorise(TxClass::Amateur).unwrap();
        let business = tx_policy.authorise(TxClass::Business).unwrap();
        let mut radio = Bk4819::new(FakeBus::new(None));
        radio.recover_to_standby().unwrap();

        let operation_count = radio.bus().operations.len();
        assert_eq!(
            radio.start_transmit(channel(TxClass::Business), &amateur),
            Err(DriverError::AuthorisationClassMismatch {
                approved: TxClass::Amateur,
                requested: TxClass::Business,
            })
        );
        assert_eq!(radio.bus().operations.len(), operation_count);

        radio
            .start_transmit(channel(TxClass::Business), &business)
            .unwrap();
        assert_eq!(
            &radio.bus().operations[operation_count..],
            &[
                Operation::Write(REG_MODE_CONTROL, MODE_STANDBY),
                Operation::Write(REG_FREQUENCY_LOW, 0x03F0),
                Operation::Write(REG_FREQUENCY_HIGH, 0x00DE),
                Operation::Write(REG_MODE_CONTROL, MODE_TRANSMIT),
            ]
        );
        assert_eq!(
            radio.state(),
            DriverState::Transmitting {
                frequency: frequency(145_500_000),
                class: TxClass::Business,
            }
        );
        radio.stop_transmit().unwrap();
        assert_eq!(radio.state(), DriverState::Standby);
        assert_eq!(
            radio.bus().operations.last(),
            Some(&Operation::Write(REG_MODE_CONTROL, MODE_STANDBY))
        );
    }

    #[test]
    fn unknown_and_invalid_states_deny_without_bus_operations() {
        let tx_policy = policy(&[TxClass::Amateur]);
        let authorisation = tx_policy.authorise(TxClass::Amateur).unwrap();
        let mut radio = Bk4819::new(FakeBus::new(None));
        assert_eq!(
            radio.start_receive(frequency(145_500_000)),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        assert_eq!(
            radio.start_transmit(channel(TxClass::Amateur), &authorisation),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        assert_eq!(
            radio.receive_status(),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        assert_eq!(
            radio.stop_transmit(),
            Err(DriverError::InvalidState(DriverState::Unknown))
        );
        assert!(radio.bus().operations.is_empty());
    }

    fn assert_mode_failure_latches_fault(final_mode: u16, fail_step: usize) {
        let tx_policy = policy(&[TxClass::Amateur]);
        let authorisation = tx_policy.authorise(TxClass::Amateur).unwrap();
        let mut radio = Bk4819::new(FakeBus::new(Some(fail_step + 1)));
        radio.recover_to_standby().unwrap();
        let result = if final_mode == MODE_RECEIVE {
            radio.start_receive(frequency(145_500_000))
        } else {
            radio.start_transmit(channel(TxClass::Amateur), &authorisation)
        };
        assert_eq!(result, Err(DriverError::Bus(FakeBusError)));
        assert_eq!(radio.state(), DriverState::Faulted);
        let operation_count = radio.bus().operations.len();
        assert_eq!(
            radio.start_transmit(channel(TxClass::Amateur), &authorisation),
            Err(DriverError::InvalidState(DriverState::Faulted))
        );
        assert_eq!(radio.bus().operations.len(), operation_count);
        radio.recover_to_standby().unwrap();
        assert_eq!(radio.state(), DriverState::Standby);
    }

    #[test]
    fn every_mode_write_failure_faults_and_denies_later_tx_until_recovery() {
        for final_mode in [MODE_RECEIVE, MODE_TRANSMIT] {
            for fail_step in 0..4 {
                assert_mode_failure_latches_fault(final_mode, fail_step);
            }
        }
    }

    #[test]
    fn either_status_read_failure_faults_the_driver() {
        for failing_call in [5, 6] {
            let bus = FakeBus::new(Some(failing_call))
                .with_register(REG_RSSI, 100)
                .with_register(REG_SQUELCH_STATUS, SQUELCH_OPEN);
            let mut radio = Bk4819::new(bus);
            radio.recover_to_standby().unwrap();
            radio.start_receive(frequency(145_500_000)).unwrap();
            assert_eq!(radio.receive_status(), Err(DriverError::Bus(FakeBusError)));
            assert_eq!(radio.state(), DriverState::Faulted);
        }
    }
}
