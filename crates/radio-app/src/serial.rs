//! The serial task: the configuration service, the link, and the memory.
//!
//! It runs the shared AFIK configuration protocol, so the host tooling programs
//! a radio exactly as it programs the simulator. It owns the retained memory,
//! restores the configuration from it at boot, and writes it back whenever the
//! host or the operator changes it.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{select, select3, Either, Either3};
use embassy_time::Timer;
use radio_device::{ControlAnswer, DeviceEvent, DeviceService, Push};
use radio_platform::operator_display::MemoryState;
use radio_protocol::{DeviceErrorCode, MAX_ENCODED_FRAME};

use crate::configuration::{
    device_service, store_setting, AppDeviceService, Programmed, RETAINED_IMAGE_BYTES,
};
use crate::drivers::{RetainedMemory, SerialLink};
use crate::operator_state::OperatorState;
use crate::state::{
    count, hold_bus_idle, store_objects, Publication, BATTERY_CALIBRATION, CONTROL_ANSWER,
    CONTROL_REQUEST, OPERATOR_PLACE, PROGRAMMED, SERIAL_ACTIVITY, SERIAL_ANSWERED,
    SERIAL_DISCARDED, SERIAL_ERRORS, SERIAL_RECEIVED, SETTING_CHOICE, TX_CALIBRATION,
};

/// Milliseconds a host waits for the interface task to answer one request.
///
/// The interface task answers from the controller alone and needs no bus, so
/// this is generous rather than tight. It exists so a host cannot be left
/// waiting forever by an interface task which has stopped: the link keeps
/// working and says the radio did not answer, rather than going silent.
const CONTROL_ANSWER_MILLISECONDS: u64 = 200;

/// The serial task's two large values, kept out of its future (`RISK-033`).
///
/// Built in a task, a by-value service or image buffer lands in the task's
/// poll frame before it is moved, and the size-optimised build kept a 4 KiB
/// poll frame that overflowed the stack. These statics are filled once, by
/// [`state`], before the executor starts.
static mut SERVICE: MaybeUninit<AppDeviceService> = MaybeUninit::uninit();
static mut IMAGE: [u8; RETAINED_IMAGE_BYTES] = [0; RETAINED_IMAGE_BYTES];

/// The serial task's service and image buffer.
pub struct State {
    service: &'static mut AppDeviceService,
    image: &'static mut [u8; RETAINED_IMAGE_BYTES],
}

/// Fills the serial task's statics and hands them out.
///
/// Call it once, before the executor starts. It is never inlined, so the
/// service it builds on the stack is gone again before any task runs.
///
/// # Panics
///
/// On a second call, which would alias the statics.
#[allow(unsafe_code)]
#[inline(never)]
pub fn state() -> State {
    // A load and a store: Cortex-M0 has no atomic swap, and this runs before
    // the executor, so nothing can interleave.
    static TAKEN: AtomicBool = AtomicBool::new(false);
    assert!(!TAKEN.load(Ordering::Relaxed), "serial state is taken once");
    TAKEN.store(true, Ordering::Relaxed);
    // SAFETY: the flag above lets this run once, so these are the only
    // references to either static for the image's life.
    unsafe {
        let service = &raw mut SERVICE;
        let image = &raw mut IMAGE;
        State {
            service: (*service).write(device_service()),
            image: &mut *image,
        }
    }
}

/// Runs the serial task for ever.
///
/// `open` takes the retained memory and reports what it found. It runs after
/// the first publication, so a slow or broken memory never holds up the
/// operator interface.
pub async fn run<L: SerialLink, M: RetainedMemory>(
    mut link: L,
    open: impl FnOnce() -> (Option<M>, MemoryState),
    state: State,
) {
    let State { service, image } = state;
    // Publish before touching the memory. The interface task adopts whatever
    // arrives, so anything slow or broken in the memory path would otherwise
    // leave the operator with an empty radio for longer than it has to. An
    // empty configuration is the honest state until a restore succeeds.
    publish(service, false, MemoryState::Unknown, None);

    let (mut retained, memory_state) = open();

    // The calibration is the radio's own data and never changes, so it is read
    // once, here, while the memory is already in hand.
    BATTERY_CALIBRATION.signal(retained.as_mut().and_then(M::read_battery_calibration));
    TX_CALIBRATION.signal(retained.as_mut().and_then(M::read_tx_calibration));

    // Restore the retained configuration before the host or the operator can
    // see this radio. A missing, erased, or corrupt region simply leaves the
    // store empty and the VFO in charge.
    let restored = retained.as_mut().is_some_and(|memory| {
        memory
            .read(image)
            .is_some_and(|length| service.load_image(&image[..length]).is_ok())
    });
    // The place is read beside the configuration it refers to and published
    // with it, so the interface never restores a channel into a channel list it
    // has not adopted. A radio which has never been used simply has none.
    let place = retained.as_mut().and_then(M::read_operator_state);
    publish(service, restored, memory_state, place);

    let mut response = [0_u8; MAX_ENCODED_FRAME];
    // A whole frame at a time, collected by the link and delimited by the idle
    // line. Reading one byte per await lost bytes while the interface task held
    // the core for a bit-banged transfer (`EVID-K1-061`).
    let mut received = [0_u8; MAX_ENCODED_FRAME];
    loop {
        // The host and the operator are two sources of change for one store, so
        // this task waits on both and serves whichever arrives. Nothing here
        // interrupts a frame in progress: `read_until_idle` has already returned
        // by the time a handset choice can be taken.
        let count_received = match select3(
            link.read_until_idle(&mut received),
            SETTING_CHOICE.wait(),
            OPERATOR_PLACE.wait(),
        )
        .await
        {
            Either3::First(Ok(count_received)) => count_received,
            Either3::First(Err(_)) => {
                // This arm firing while nothing is received is what a wrong
                // baud rate looks like.
                count(&SERIAL_ERRORS);
                // Yield before retrying so a persistent receiver error can never
                // starve the interface task.
                Timer::after_millis(1).await;
                continue;
            }
            Either3::Second(change) => {
                // A handset setting which did not survive a battery change would
                // not be worth the menu, so it is stored exactly like a host
                // write and republished from the store rather than assumed.
                if store_setting(service, change).is_ok() {
                    let retained_now = retain(service, image, retained.as_mut()).await;
                    publish(service, retained_now, memory_state, None);
                }
                continue;
            }
            Either3::Third(place) => {
                if let Some(memory) = retained.as_mut() {
                    let _ = memory.write_operator_state(place).await;
                }
                continue;
            }
        };
        for &byte in &received[..count_received] {
            count(&SERIAL_RECEIVED);
            hold_bus_idle();

            // `RISK-036`: with this feature the byte is counted and dropped,
            // and nothing below this line runs.
            if cfg!(feature = "serial-drop-bytes") {
                continue;
            }

            let before = service.generation();
            let mut observe = |event| {
                if matches!(event, DeviceEvent::PacketDiscarded(_)) {
                    count(&SERIAL_DISCARDED);
                }
            };
            let pushed = service.push_control(byte, &mut response, &mut observe);
            if !matches!(pushed, Push::Idle) {
                SERIAL_ACTIVITY.signal(());
            }
            let length = match pushed {
                Push::Idle => continue,
                Push::Response(length) => length,
                // The receiver belongs to the interface task, so the answer has
                // to come from there. Nothing is held while waiting.
                Push::Control(control) => {
                    CONTROL_REQUEST.signal(control.request());
                    let answer = match select(
                        CONTROL_ANSWER.wait(),
                        Timer::after_millis(CONTROL_ANSWER_MILLISECONDS),
                    )
                    .await
                    {
                        Either::First(answer) => answer,
                        // An interface task which did not answer leaves the
                        // link working and the host told.
                        Either::Second(()) => ControlAnswer::Refused(DeviceErrorCode::Internal),
                    };
                    let Some(length) =
                        service.answer_control(control, answer, &mut response, &mut observe)
                    else {
                        continue;
                    };
                    length
                }
            };
            if service.generation() != before {
                // Retain the new configuration before answering, so a host told
                // that a transaction committed can rely on it being stored.
                let retained_now = retain(service, image, retained.as_mut()).await;
                publish(service, retained_now, memory_state, None);
            }
            count(&SERIAL_ANSWERED);
            link.write(&response[..length]).await;
            hold_bus_idle();
        }
    }
}

/// Writes the active configuration to the memory and reports whether it took.
async fn retain<M: RetainedMemory>(
    service: &AppDeviceService,
    image: &mut [u8; RETAINED_IMAGE_BYTES],
    memory: Option<&mut M>,
) -> bool {
    let Ok(length) = service.encode_active_image(&mut image[..]) else {
        return false;
    };
    match memory {
        Some(memory) => memory.write(image, length).await,
        None => false,
    }
}

/// Publishes the active configuration for the user interface task.
///
/// A snapshot the interface cannot use is published as an empty configuration
/// rather than dropped, so the display always reports what the radio really
/// holds and falls back to the VFO.
fn publish<const BYTES: usize>(
    service: &DeviceService<BYTES>,
    retained: bool,
    memory: MemoryState,
    place: Option<OperatorState>,
) {
    store_objects(service.active_payload());
    let programmed = Programmed::index(service.active_objects()).unwrap_or_default();
    PROGRAMMED.signal(Publication {
        programmed,
        generation: service.generation(),
        retained: retained && !programmed.is_empty(),
        memory,
        place,
    });
}
