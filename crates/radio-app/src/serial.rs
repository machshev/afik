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
use radio_storage::radio_image::{Index, HEADER_LEN};

use crate::configuration::Programmed;
use crate::drivers::{Memory, SerialLink};
use crate::state::{
    hold_bus_idle, mark_boot, Publication, TaskCell, BATTERY_CALIBRATION, BOOT_LOADED_MS,
    BOOT_SERIAL_START_MS, INDEX, OPERATOR_PLACE, PROGRAMMED, SERIAL_ACTIVITY, SQUELCH_CALIBRATION,
    TX_CALIBRATION, TX_POLICY,
};
use radio_tx_policy::TxPolicy;

/// Milliseconds the link must stay quiet after a write before the image is
/// loaded again.
///
/// A host writes an image as hundreds of blocks and reads it back to verify,
/// all in one burst. Loading after every block would show the operator a
/// configuration that is half old and half new; waiting for the burst to end
/// loads only the finished one.
const RELOAD_QUIET_MILLISECONDS: u64 = 1_000;

/// Bytes collected per burst. A host sends one request and waits for its
/// answer, and the largest request is 28 bytes.
const RECEIVE_BYTES: usize = 64;

/// Runs the serial task for ever.
///
/// `open` takes the memory and reports what it found; it is kept in `memory`,
/// which the interface task reads channels from. It runs after the first
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
    memory: &'static TaskCell<Option<M>>,
) {
    mark_boot(&BOOT_SERIAL_START_MS);
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

    let (opened, memory_state) = open();
    memory.with_mut(|cell| *cell = opened);

    // The calibration is the radio's own data and never changes, so it is read
    // once, here.
    BATTERY_CALIBRATION
        .signal(memory.with_mut(|cell| cell.as_mut().and_then(M::read_battery_calibration)));
    TX_CALIBRATION.signal(memory.with_mut(|cell| cell.as_mut().and_then(M::read_tx_calibration)));
    SQUELCH_CALIBRATION
        .signal(memory.with_mut(|cell| cell.as_mut().and_then(M::read_squelch_calibration)));

    // The place is read beside the configuration it refers to and published
    // with it, so the interface never restores a channel into a channel list it
    // has not adopted. Its squelch is the operator's own choice and replaces
    // the image's default.
    let place = memory.with_mut(|cell| cell.as_mut().and_then(M::read_operator_state));
    let mut generation = 1_u32;
    let (programmed, loaded) = memory.with_mut(|cell| load(cell.as_mut()));
    let programmed = match place.and_then(|place| place.squelch) {
        Some(squelch) => programmed.with_squelch(squelch),
        None => programmed,
    };
    mark_boot(&BOOT_LOADED_MS);
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
                // Yield before retrying so a persistent receiver error can never
                // starve the interface task.
                Timer::after_millis(1).await;
                continue;
            }
            Either3::Second(place) => {
                memory.with_mut(|cell| {
                    if let Some(memory) = cell.as_mut() {
                        let _ = memory.write_operator_state(place);
                    }
                });
                continue;
            }
            Either3::Third(()) => {
                // The host has finished. What it wrote is the configuration
                // now, squelch included, so nothing of the old one is kept.
                written = false;
                generation = generation.wrapping_add(1);
                let (programmed, loaded) = memory.with_mut(|cell| {
                    // A commit which failed leaves the old image, or none, and
                    // the load says which.
                    if let Some(memory) = cell.as_mut() {
                        let _ = memory.commit_host_write();
                    }
                    load(cell.as_mut())
                });
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
            hold_bus_idle();
            // `RISK-036`: with this feature the byte is dropped, and nothing
            // below this line runs.
            if cfg!(feature = "serial-drop-bytes") {
                continue;
            }
            let request = match decoder.push(byte) {
                Push::Idle | Push::Rejected => continue,
                Push::Request(request) => request,
            };
            SERIAL_ACTIVITY.signal(());
            let length = match request {
                Request::Hello => Some(encode_hello_response(identity, &mut response)),
                Request::Read { address, length } => memory.with_mut(|cell| {
                    let memory = cell.as_mut()?;
                    encode_read_response(address, length, &mut response, |data| {
                        memory.read(address, data)
                    })
                }),
                Request::Write { address, data } => {
                    if host_may_write(address) && !written {
                        // The image is about to change under the index the
                        // interface reads it through, so the interface lets go
                        // of it until the new one is loaded.
                        written = true;
                        TX_POLICY.with_mut(|policy| *policy = TxPolicy::deny_all());
                        generation = generation.wrapping_add(1);
                        PROGRAMMED.signal(Publication {
                            programmed: Programmed::empty(),
                            generation,
                            retained: false,
                            memory: memory_state,
                            place: None,
                        });
                    }
                    let stored = host_may_write(address)
                        && memory.with_mut(|cell| {
                            cell.as_mut()
                                .is_some_and(|memory| memory.write(address, &data))
                        });
                    // A refused or failed write is not acknowledged, so the
                    // host's write times out and it stops.
                    stored.then(|| encode_write_response(address, &mut response))
                }
            };
            if let Some(length) = length {
                link.write(&response[..length]).await;
            }
            hold_bus_idle();
        }
    }
}

/// Validates and indexes the configuration image in memory.
///
/// A missing, erased, torn, or corrupt image indexes as empty, so the radio
/// runs on the VFO rather than on half a configuration.
fn load<M: Memory>(memory: Option<&mut M>) -> (Programmed, bool) {
    // The index is rebuilt where it lives: a kilobyte is too much to build
    // on the stack and copy.
    let policy = INDEX.with_mut(|index| {
        *index = Index::empty();
        let memory = memory?;
        let mut header = [0_u8; HEADER_LEN];
        if !memory.read(IMAGE_ORIGIN, &mut header) {
            return None;
        }
        index
            .rebuild(&header, |offset, buffer| read_image(memory, offset, buffer))
            .ok()?;
        Some(index.tx_policy(|offset, buffer| read_image(memory, offset, buffer)))
    });
    TX_POLICY.with_mut(|stored| *stored = policy.unwrap_or_else(TxPolicy::deny_all));
    let programmed = INDEX.with(Programmed::of);
    (programmed, !programmed.is_empty())
}

/// Reads image payload bytes from the memory view.
pub(crate) fn read_image<M: Memory>(memory: &mut M, offset: usize, buffer: &mut [u8]) -> bool {
    u16::try_from(HEADER_LEN + offset)
        .is_ok_and(|offset| memory.read(IMAGE_ORIGIN + offset, buffer))
}

#[cfg(test)]
mod tests {
    use super::load;
    use crate::drivers::Memory;
    use crate::state::TX_POLICY;
    use radio_channel_plan::{ChannelDefinition, ChannelFlags, ChannelName, ChannelRecord};
    use radio_domain::{
        Bandwidth, ChannelId, Frequency, FrequencyStep, Modulation, PowerLevel, RadioConfig, Tone,
        TxClass,
    };
    use radio_platform::battery::Calibration;
    use radio_platform::memory_link::{VIEW_BYTES, WRITE_BYTES};
    use radio_platform::transmit::TxCalibration;
    use radio_storage::radio_image::compile;
    use radio_tx_policy::{TxPolicy, TxRange};

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

        fn write(&mut self, address: u16, data: &[u8; WRITE_BYTES]) -> bool {
            let start = usize::from(address);
            self.0[start..start + WRITE_BYTES].copy_from_slice(data);
            true
        }

        fn read_battery_calibration(&mut self) -> Option<Calibration> {
            None
        }

        fn read_tx_calibration(&mut self) -> Option<TxCalibration> {
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
            power: PowerLevel::Low1,
            step: FrequencyStep::from_hz(12_500).expect("step"),
            flags: ChannelFlags::default(),
            bank: Some(radio_domain::BankId::new(0)),
        })
        .expect("channel")
    }

    /// One test, because the copy it loads into is the image's one static.
    #[test]
    fn an_image_in_memory_loads_and_a_damaged_or_erased_one_does_not() {
        let pmr = TxRange {
            class: TxClass::LicenceFreePlan,
            first_hz: 446_006_250,
            last_hz: 446_193_750,
            max_mw: 500,
            wide_fm: false,
            raster_hz: 12_500,
        };
        let mut memory = FakeMemory([0xFF; VIEW_BYTES as usize]);
        let length = compile(
            RadioConfig::conservative(),
            &[pmr],
            &[],
            &[channel(1), channel(2)],
            &[],
            &[],
            &mut memory.0,
        )
        .expect("image");

        let (programmed, loaded) = load(Some(&mut memory));
        assert!(loaded);
        assert_eq!(programmed.channel_count(), 2);
        assert_eq!(
            TX_POLICY.with(|policy| *policy),
            TxPolicy::new(&[pmr]).unwrap(),
            "the programmed ranges are the TX policy"
        );

        memory.0[length - 1] ^= 0x01;
        let (programmed, loaded) = load(Some(&mut memory));
        assert!(!loaded, "a damaged image is refused");
        assert!(programmed.is_empty(), "and nothing of it is kept");
        assert_eq!(
            TX_POLICY.with(|policy| *policy),
            TxPolicy::deny_all(),
            "nor its TX ranges"
        );

        let (programmed, loaded) = load(Some(&mut FakeMemory([0xFF; VIEW_BYTES as usize])));
        assert!(
            !loaded && programmed.is_empty(),
            "erased memory is no image"
        );
        let (_, loaded) = load::<FakeMemory>(None);
        assert!(!loaded);
    }
}
