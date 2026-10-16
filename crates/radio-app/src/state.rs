//! What the serial task and the interface task share.
//!
//! The serial task owns the link and the retained memory; the interface task
//! owns the display, the keypad, and the radio. Each asks the other through
//! these signals rather than reaching into it.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;
use radio_platform::battery::Calibration;
use radio_platform::operator_display::{BootTimes, MemoryState, ResetCause, SerialCounters};
use radio_platform::squelch::SquelchCalibration;
use radio_platform::transmit::TxCalibration;

use radio_storage::radio_image::Index;
use radio_tx_policy::TxPolicy;

use crate::configuration::Programmed;
use crate::operator_state::OperatorState;

/// Milliseconds the radio bus stays idle after the last serial byte.
///
/// A bit-banged radio bus blocks the executor for milliseconds, which would
/// drop inbound bytes. Keeping the bus quiet while the host is mid-exchange
/// makes programming and tuning safe to use in either order.
const LINK_QUIET_MILLISECONDS: u32 = 250;

/// Deadline, in milliseconds since boot, before which the bus must stay idle.
static LINK_QUIET_UNTIL: AtomicU32 = AtomicU32::new(0);

/// Latest configuration the serial task activated.
pub(crate) static PROGRAMMED: Signal<CriticalSectionRawMutex, Publication> = Signal::new();

/// The battery calibration the serial task read from the vendor block.
///
/// The memory is on the serial task's bus, so it reads the calibration once
/// and hands it over. Until it does, the interface reports that it does not
/// know the charge rather than a number derived from nothing.
pub(crate) static BATTERY_CALIBRATION: Signal<CriticalSectionRawMutex, Option<Calibration>> =
    Signal::new();

/// The PMR446 low-power TX bias bytes, read once by the serial task.
pub(crate) static TX_CALIBRATION: Signal<CriticalSectionRawMutex, Option<TxCalibration>> =
    Signal::new();

/// The unit's squelch tables, read once by the serial task.
pub(crate) static SQUELCH_CALIBRATION: Signal<CriticalSectionRawMutex, Option<SquelchCalibration>> =
    Signal::new();

/// Raised for every host request, so the interface task can ignore PTT while
/// the cable that shares its jack is in use.
pub(crate) static SERIAL_ACTIVITY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Where the operator has left the radio, for the task which owns the memory.
///
/// The interface knows the place and the serial task owns the memory, so the
/// interface says where it is and the serial task writes it down. A place
/// arriving while a host is mid-exchange waits its turn rather than racing it.
pub(crate) static OPERATOR_PLACE: Signal<CriticalSectionRawMutex, OperatorState> = Signal::new();

/// One activated configuration handed to the user interface task.
#[derive(Clone, Copy)]
pub(crate) struct Publication {
    pub(crate) programmed: Programmed,
    pub(crate) generation: u32,
    pub(crate) retained: bool,
    pub(crate) memory: MemoryState,
    /// Where the operator left this radio, on the one publication which knows.
    ///
    /// The place is read once, beside the configuration it refers to, and
    /// travels with it. Carrying it here rather than signalling it separately
    /// is what stops the interface from restoring a place into a configuration
    /// it has not adopted yet.
    pub(crate) place: Option<OperatorState>,
}

/// The index of the image the radio runs, written when an image is loaded.
///
/// A few bytes per bank block: enough to answer counting, bank filtering and
/// scan navigation from RAM, and to find any channel in memory.
pub(crate) static INDEX: TaskCell<Index> = TaskCell::new(Index::empty());

/// The programmed TX ranges as a policy, written when an image is loaded.
///
/// Nothing is permitted until an image says otherwise, and the serial task
/// resets it to that the moment a host starts writing, so the interface never
/// transmits under ranges read from a half-written image (`ADR-101`). The
/// interface takes it with each configuration it adopts.
pub(crate) static TX_POLICY: TaskCell<TxPolicy> = TaskCell::new(TxPolicy::deny_all());

/// State shared by the tasks of one thread-mode executor.
///
/// A critical-section mutex would mask interrupts for as long as a task spends
/// reading memory, which on the K5's bit-banged bus is long enough to lose
/// timer ticks. Tasks on one executor never preempt one another, and no task
/// holds a borrow across an `await`, so a plain `RefCell` is enough; a broken
/// rule panics rather than corrupting. A target keeps its memory in one, which
/// it hands to both tasks.
pub struct TaskCell<T>(RefCell<T>);

// SAFETY: a `TaskCell` is for the tasks of one thread-mode executor, and no
// interrupt handler may touch one, so it is never accessed from two contexts at
// once. Every use in this crate and in the targets keeps to that.
#[allow(unsafe_code)]
unsafe impl<T> Sync for TaskCell<T> {}

impl<T> TaskCell<T> {
    /// Wraps a value for the tasks of one executor.
    pub const fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }

    /// Runs `f` with shared access.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.0.borrow())
    }

    /// Runs `f` with exclusive access.
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

pub(crate) fn now_ms() -> u32 {
    u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX)
}

pub(crate) fn hold_bus_idle() {
    LINK_QUIET_UNTIL.store(
        now_ms().saturating_add(LINK_QUIET_MILLISECONDS),
        Ordering::Relaxed,
    );
}

pub(crate) fn bus_available() -> bool {
    now_ms() >= LINK_QUIET_UNTIL.load(Ordering::Relaxed)
}

/// Bytes the serial task has received, for the information screen.
pub(crate) static SERIAL_RECEIVED: AtomicU32 = AtomicU32::new(0);
/// Frames the serial task has answered, for the information screen.
pub(crate) static SERIAL_ANSWERED: AtomicU32 = AtomicU32::new(0);
/// Complete packets the serial task rejected as malformed.
///
/// A frame heard and rejected otherwise looks the same as a frame never
/// received. Counting them is what tells those apart without a host.
pub(crate) static SERIAL_DISCARDED: AtomicU32 = AtomicU32::new(0);
/// Receiver errors: bytes which arrived and could not be framed.
///
/// A byte the link cannot frame is never delivered, so it cannot be counted as
/// received. Without this, a link running at the wrong baud rate looks exactly
/// like a link with nothing on it (`RISK-036`).
pub(crate) static SERIAL_ERRORS: AtomicU32 = AtomicU32::new(0);

/// Adds one to a counter only its owning task writes; this core has no atomic
/// read-modify-write, so a load and store is sufficient.
pub(crate) fn count(counter: &AtomicU32) {
    counter.store(
        counter.load(Ordering::Relaxed).wrapping_add(1),
        Ordering::Relaxed,
    );
}

/// Returns the current serial counters.
pub(crate) fn serial_counters() -> SerialCounters {
    let shown =
        |counter: &AtomicU32| u16::try_from(counter.load(Ordering::Relaxed) % 1_000).unwrap_or(0);
    SerialCounters {
        received: shown(&SERIAL_RECEIVED),
        answered: shown(&SERIAL_ANSWERED),
        discarded: shown(&SERIAL_DISCARDED),
        errors: shown(&SERIAL_ERRORS),
    }
}

/// Milliseconds since reset at which the serial task started.
pub(crate) static BOOT_SERIAL_START_MS: AtomicU32 = AtomicU32::new(0);
/// Milliseconds since reset at which the stored configuration was loaded.
pub(crate) static BOOT_LOADED_MS: AtomicU32 = AtomicU32::new(0);
/// Milliseconds since reset at which the interface drew its first frame.
pub(crate) static BOOT_FIRST_FRAME_MS: AtomicU32 = AtomicU32::new(0);

/// Records that boot reached a stage now.
pub(crate) fn mark_boot(stage: &AtomicU32) {
    stage.store(now_ms(), Ordering::Relaxed);
}

/// Returns when boot reached each stage, for the information screen.
pub(crate) fn boot_times() -> BootTimes {
    BootTimes {
        serial_start_ms: BOOT_SERIAL_START_MS.load(Ordering::Relaxed),
        loaded_ms: BOOT_LOADED_MS.load(Ordering::Relaxed),
        first_frame_ms: BOOT_FIRST_FRAME_MS.load(Ordering::Relaxed),
    }
}

/// Peripheral clock the target runs its links from, in kilohertz.
static PERIPHERAL_CLOCK_KHZ: AtomicU32 = AtomicU32::new(0);
/// Reset flags the target read at boot.
static RESET_CAUSE: AtomicU32 = AtomicU32::new(0);

/// Records, for the information screen, the clock every baud rate is derived
/// from. A wrong clock is otherwise visible only as a link which hears bytes
/// and rejects every packet.
pub fn set_peripheral_clock_khz(khz: u32) {
    PERIPHERAL_CLOCK_KHZ.store(khz, Ordering::Relaxed);
}

/// Records, for the information screen, why the radio last reset.
pub fn set_reset_cause(cause: ResetCause) {
    RESET_CAUSE.store(u32::from(cause.0), Ordering::Relaxed);
}

pub(crate) fn peripheral_clock_khz() -> u32 {
    PERIPHERAL_CLOCK_KHZ.load(Ordering::Relaxed)
}

pub(crate) fn reset_cause() -> ResetCause {
    ResetCause(u8::try_from(RESET_CAUSE.load(Ordering::Relaxed) & 0xff).unwrap_or(0))
}
