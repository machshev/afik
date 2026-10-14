//! The serial task: the memory protocol, the link, and the memory.
//!
//! A host programs the radio the stock way (`ADR-100`): it reads and writes
//! the 8 KiB memory view in small blocks, and it compiled and validated the
//! configuration image before writing any of it. This task answers those
//! requests, loads the image from memory at boot and again once a host has
//! finished writing, and records where the operator left the radio.

use embassy_futures::select::{select3, Either3};
use embassy_time::Timer;
use radio_platform::memory_link::{
    encode_hello_response, encode_read_response, encode_write_response, host_may_write, Decoder,
    Push, Request, IMAGE_ORIGIN, RESPONSE_FRAME_BYTES,
};
use radio_platform::operator_display::MemoryState;
use radio_platform::serial::ApplicationIdentity;
use radio_storage::CONFIGURATION_IMAGE_HEADER_LEN;

use crate::configuration::Programmed;
use crate::drivers::{Memory, SerialLink};
use crate::state::{
    count, hold_bus_idle, Publication, ACTIVE, BATTERY_CALIBRATION, OPERATOR_PLACE, PROGRAMMED,
    SERIAL_ACTIVITY, SERIAL_ANSWERED, SERIAL_DISCARDED, SERIAL_ERRORS, SERIAL_RECEIVED,
    TX_CALIBRATION,
};

/// Milliseconds the link must stay quiet after a write before the image is
/// loaded again.
///
/// A host writes an image as hundreds of blocks and reads it back to verify,
/// all in one burst. Loading after every block would show the operator a
/// configuration that is half old and half new; waiting for the burst to end
/// loads only the finished one.
const RELOAD_QUIET_MILLISECONDS: u64 = 1_000;

/// The image header as a memory offset.
#[allow(clippy::cast_possible_truncation)]
const HEADER_BYTES: u16 = CONFIGURATION_IMAGE_HEADER_LEN as u16;

/// Bytes collected per burst. A host sends one request and waits for its
/// answer, and the largest request is 28 bytes.
const RECEIVE_BYTES: usize = 64;

/// Runs the serial task for ever.
///
/// `open` takes the memory and reports what it found. It runs after the first
/// publication, so a slow or broken memory never holds up the operator
/// interface. `identity` is what hello answers.
///
/// # Panics
///
/// If `identity` is not a printable string of at most 32 bytes.
#[allow(clippy::too_many_lines)]
pub async fn run<L: SerialLink, M: Memory>(
    mut link: L,
    open: impl FnOnce() -> (Option<M>, MemoryState),
    identity: &'static [u8],
) {
    let identity = ApplicationIdentity::new(identity).expect("printable bounded identity");
    // Publish before touching the memory, so the interface starts on the VFO
    // at once rather than waiting on a slow or broken memory.
    PROGRAMMED.signal(Publication {
        programmed: Programmed::empty(),
        generation: 0,
        retained: false,
        memory: MemoryState::Unknown,
        place: None,
    });

    let (mut memory, memory_state) = open();

    // The calibration is the radio's own data and never changes, so it is read
    // once, here, while the memory is already in hand.
    BATTERY_CALIBRATION.signal(memory.as_mut().and_then(M::read_battery_calibration));
    TX_CALIBRATION.signal(memory.as_mut().and_then(M::read_tx_calibration));

    // The place is read beside the configuration it refers to and published
    // with it, so the interface never restores a channel into a channel list it
    // has not adopted. Its squelch is the operator's own choice and replaces
    // the image's default.
    let place = memory.as_mut().and_then(M::read_operator_state);
    let mut generation = 1_u32;
    let (programmed, loaded) = load(memory.as_mut());
    let programmed = match place.and_then(|place| place.squelch) {
        Some(squelch) => programmed.with_squelch(squelch),
        None => programmed,
    };
    PROGRAMMED.signal(Publication {
        programmed,
        generation,
        retained: loaded,
        memory: memory_state,
        place,
    });

    let mut decoder = Decoder::new();
    let mut response = [0_u8; RESPONSE_FRAME_BYTES];
    let mut received = [0_u8; RECEIVE_BYTES];
    // Whether a host has written since the image was last loaded.
    let mut written = false;
    loop {
        let reload = async {
            if written {
                Timer::after_millis(RELOAD_QUIET_MILLISECONDS).await;
            } else {
                core::future::pending::<()>().await;
            }
        };
        let received_count = match select3(
            link.read_until_idle(&mut received),
            OPERATOR_PLACE.wait(),
            reload,
        )
        .await
        {
            Either3::First(Ok(received_count)) => received_count,
            Either3::First(Err(_)) => {
                // This arm firing while nothing is received is what a wrong
                // baud rate looks like.
                count(&SERIAL_ERRORS);
                // Yield before retrying so a persistent receiver error can never
                // starve the interface task.
                Timer::after_millis(1).await;
                continue;
            }
            Either3::Second(place) => {
                if let Some(memory) = memory.as_mut() {
                    let _ = memory.write_operator_state(place).await;
                }
                continue;
            }
            Either3::Third(()) => {
                // The host has finished. What it wrote is the configuration
                // now, squelch included, so nothing of the old one is kept.
                written = false;
                generation = generation.wrapping_add(1);
                let (programmed, loaded) = load(memory.as_mut());
                PROGRAMMED.signal(Publication {
                    programmed,
                    generation,
                    retained: loaded,
                    memory: memory_state,
                    place: None,
                });
                continue;
            }
        };
        for &byte in &received[..received_count] {
            count(&SERIAL_RECEIVED);
            hold_bus_idle();
            // `RISK-036`: with this feature the byte is counted and dropped,
            // and nothing below this line runs.
            if cfg!(feature = "serial-drop-bytes") {
                continue;
            }
            let request = match decoder.push(byte) {
                Push::Idle => continue,
                Push::Rejected => {
                    count(&SERIAL_DISCARDED);
                    continue;
                }
                Push::Request(request) => request,
            };
            SERIAL_ACTIVITY.signal(());
            let length = match request {
                Request::Hello => Some(encode_hello_response(identity, &mut response)),
                Request::Read { address, length } => memory.as_mut().and_then(|memory| {
                    encode_read_response(address, length, &mut response, |data| {
                        memory.read(address, data)
                    })
                }),
                Request::Write { address, data } => {
                    let stored = match memory.as_mut() {
                        Some(memory) if host_may_write(address) => {
                            written = true;
                            memory.write(address, &data).await
                        }
                        _ => false,
                    };
                    // A refused or failed write is not acknowledged, so the
                    // host's write times out and it stops.
                    stored.then(|| encode_write_response(address, &mut response))
                }
            };
            if let Some(length) = length {
                count(&SERIAL_ANSWERED);
                link.write(&response[..length]).await;
            }
            hold_bus_idle();
        }
    }
}

/// Loads the configuration image from memory into the shared copy.
///
/// A missing, erased, torn, or corrupt image leaves the copy empty, so the
/// radio runs on the VFO rather than on half a configuration.
fn load<M: Memory>(memory: Option<&mut M>) -> (Programmed, bool) {
    let loaded = memory.is_some_and(|memory| {
        let mut header = [0_u8; CONFIGURATION_IMAGE_HEADER_LEN];
        memory.read(IMAGE_ORIGIN, &mut header)
            && ACTIVE.with_mut(|arena| {
                arena
                    .load_image(&header, |payload| {
                        memory.read(IMAGE_ORIGIN + HEADER_BYTES, payload)
                    })
                    .is_ok()
            })
    });
    if !loaded {
        ACTIVE.with_mut(|arena| {
            let _ = arena.load_payload(&[]);
        });
    }
    let programmed = ACTIVE.with(|arena| Programmed::index(arena).unwrap_or_default());
    (programmed, loaded && !programmed.is_empty())
}

#[cfg(test)]
mod tests {
    use super::load;
    use crate::drivers::Memory;
    use radio_channel_plan::{
        BankMask, ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord,
    };
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, SquelchLevel, Tone,
        TxClass,
    };
    use radio_platform::battery::Calibration;
    use radio_platform::memory_link::{VIEW_BYTES, WRITE_BYTES};
    use radio_storage::{encode_channel, encode_configuration_image};

    struct FakeMemory([u8; VIEW_BYTES as usize]);

    impl Memory for FakeMemory {
        fn read(&mut self, address: u16, output: &mut [u8]) -> bool {
            let start = usize::from(address);
            let Some(bytes) = self.0.get(start..start + output.len()) else {
                return false;
            };
            output.copy_from_slice(bytes);
            true
        }

        async fn write(&mut self, address: u16, data: &[u8; WRITE_BYTES]) -> bool {
            let start = usize::from(address);
            self.0[start..start + WRITE_BYTES].copy_from_slice(data);
            true
        }

        fn read_battery_calibration(&mut self) -> Option<Calibration> {
            None
        }

        fn read_tx_calibration(&mut self) -> Option<[u8; 3]> {
            None
        }
    }

    fn channel(id: u16) -> ChannelRecord {
        let frequency = Frequency::from_hz(446_006_250).expect("frequency");
        ChannelRecord::new(ChannelDefinition {
            id: ChannelId::new(id),
            name: ChannelName::new("PMR").expect("name"),
            receive: frequency,
            transmit: frequency,
            rx_tone: Tone::None,
            tx_tone: Tone::None,
            modulation: Modulation::Fm,
            bandwidth: Bandwidth::Narrow,
            power: PowerLevel::Low,
            step: FrequencyStep::from_hz(12_500).expect("step"),
            squelch: SquelchLevel::new(3).expect("level"),
            flags: ChannelFlags::default(),
            banks: BankMask::default(),
            tx_class: TxClass::Never,
        })
        .expect("channel")
    }

    /// One test, because the copy it loads into is the image's one static.
    #[test]
    fn an_image_in_memory_loads_and_a_damaged_or_erased_one_does_not() {
        let objects = [
            encode_channel(channel(1)).expect("one"),
            encode_channel(channel(2)).expect("two"),
        ];
        let mut memory = FakeMemory([0xFF; VIEW_BYTES as usize]);
        let length = encode_configuration_image(&objects, &mut memory.0).expect("image");

        let (programmed, loaded) = load(Some(&mut memory));
        assert!(loaded);
        assert_eq!(programmed.channel_count(), 2);

        memory.0[length - 1] ^= 0x01;
        let (programmed, loaded) = load(Some(&mut memory));
        assert!(!loaded, "a damaged image is refused");
        assert!(programmed.is_empty(), "and nothing of it is kept");

        let (programmed, loaded) = load(Some(&mut FakeMemory([0xFF; VIEW_BYTES as usize])));
        assert!(
            !loaded && programmed.is_empty(),
            "erased memory is no image"
        );
        let (_, loaded) = load::<FakeMemory>(None);
        assert!(!loaded);
    }
}
