//! The first UV-K5 V1 application AFIK can observe running.
//!
//! It configures the clock, binds UART1 to the programming connector, and then
//! answers the read-only hello while validating the evidenced display, keypad,
//! keypad and squelch-gated BK4819 receive audio. It has no transmit path.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use radio_bk4819::{AfOutput, Bk4819, ReadbackRegister, ReceiveMetrics, RegisterBus};
use radio_dp32g030::clock;
use radio_dp32g030::dma::CircularReceiver;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::systick::PeriodicTick;
use radio_dp32g030::uart::{k5_programming_divider, Uart};
use radio_dp32g030::UART1_BASE;
use radio_firmware_k5::audio::SpeakerGate;
use radio_firmware_k5::bk4819_bus::{K5Pins, K5RegisterBus};
use radio_firmware_k5::k5_display::K5BootDisplay;
use radio_firmware_k5::keypad::{self, K5Matrix};
use radio_firmware_k5::protocol::{HelloService, APPLICATION_IDENTITY, RESPONSE_FRAME_BYTES};
use radio_firmware_k5::receive::Pmr446Channel;
use radio_platform::display::show_boot_sequence;
use radio_platform::receive_app::{Effect, Event, ReceiveApp};

/// Initial stack pointer, per `EVID-K5-019`: the top of the evidenced RAM less
/// the sixteen bytes the firmware running on these units leaves alone.
const INITIAL_STACK_POINTER: u32 = 0x2000_3FF0;

/// The UART bound to the programming connector, per `EVID-K5-019`.
const UART1: Uart = Uart::new(UART1_BASE);
const DMA_BYTES: usize = 256;
const SAMPLE_MILLISECONDS: u32 = 50;

static mut DMA_BUFFER: [u8; DMA_BYTES] = [0; DMA_BYTES];

#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
    non_maskable_interrupt: extern "C" fn() -> !,
    hard_fault: extern "C" fn() -> !,
}

// SAFETY: the linker script owns this section, retains this one table, asserts
// its exact size, and places it at the evidenced Cortex-M0 vector address.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".vector_table")]
static VECTOR_TABLE: VectorTable = VectorTable {
    initial_stack_pointer: INITIAL_STACK_POINTER,
    reset,
    non_maskable_interrupt: fault,
    hard_fault: fault,
};

// SAFETY: this is the single linker entry symbol named by link.x. The name is
// scoped to this standalone image, and the function has the Cortex-M ABI.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    startup::initialise_ram();
    main()
}

// SAFETY: as `reset`. No interrupt is ever enabled, so these two vectors can
// only be reached by a fault, which this image cannot recover from and must not
// pretend to.
#[allow(unsafe_code)]
#[unsafe(export_name = "Fault")]
extern "C" fn fault() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn main() -> ! {
    // The ports are gated on beside the UART, and the two pins are given an
    // explicit direction, because `AFIK-K5-1.0` did neither and was silent:
    // this GPIO block takes its pad direction from `GPIODIR`, so a pin whose
    // port has no clock is not driven however PORTCON selects it.
    syscon::enable(&[
        Peripheral::GpioA,
        Peripheral::GpioB,
        Peripheral::GpioC,
        Peripheral::Uart1,
    ]);
    gpio::set_output(Port::A, 7);
    gpio::set_input(Port::A, 8);
    portcon::enable_input(Port::A, 8);
    portcon::enable_pull_up(Port::A, 8);

    let clock = clock::configure();
    let mut sample_tick = PeriodicTick::start(clock.hertz(), SAMPLE_MILLISECONDS)
        .expect("50 ms fits the Cortex-M0 SysTick reload at the configured clock");
    portcon::select_pa7_uart1_tx();
    portcon::select_pa8_uart1_rx();
    UART1.prepare_receive_dma_with_divider(k5_programming_divider(clock.hertz()));
    // SAFETY: this single-core image gives the static buffer exclusively to
    // this receiver for the rest of its lifetime.
    #[allow(unsafe_code)]
    let mut receiver = unsafe {
        CircularReceiver::<DMA_BYTES>::new(&raw mut DMA_BUFFER as *mut u8, UART1.receive_address())
    };
    UART1.start_receive_dma();

    let mut display = K5BootDisplay::initialise();
    let _ = show_boot_sequence(&mut display);

    // PC4 starts low even though receive audio is the operator default. It is
    // opened only after the first complete sample says squelch is open.
    let mut speaker = SpeakerGate::initialise();
    let mut radio = Bk4819::new(K5RegisterBus::new(K5Pins::initialise()));
    let initialised = radio.initialise().is_ok();
    let mut app = ReceiveApp::new();
    let mut configured = None;
    let mut metrics = None;
    let mut keypad = K5Matrix::initialise();
    let mut shown_key = keypad::scan(&mut keypad);
    if initialised {
        apply_event(
            &mut app,
            Event::Start,
            &mut radio,
            &mut speaker,
            &mut display,
            &mut configured,
            &mut metrics,
        );
    } else {
        apply_event(
            &mut app,
            Event::ReceiverFault,
            &mut radio,
            &mut speaker,
            &mut display,
            &mut configured,
            &mut metrics,
        );
    }

    let mut service = HelloService::new(APPLICATION_IDENTITY);
    let mut response = [0_u8; RESPONSE_FRAME_BYTES];
    loop {
        while let Some(byte) = receiver.read_byte() {
            let Some(length) = service.push(byte, &mut response) else {
                continue;
            };
            UART1.write(&response[..length]);
            UART1.flush();
        }
        let key = keypad::scan(&mut keypad);
        if key != shown_key {
            shown_key = key;
            if let Some(key) = key {
                apply_event(
                    &mut app,
                    Event::KeyPress(key.shared()),
                    &mut radio,
                    &mut speaker,
                    &mut display,
                    &mut configured,
                    &mut metrics,
                );
            }
        }
        sample_tick.wait();
        if let Ok(sample) = radio.receive_metrics(radio_domain::Tone::None) {
            metrics = Some(sample);
            apply_event(
                &mut app,
                Event::ReceiveSample {
                    squelch_open: sample.squelch_open,
                },
                &mut radio,
                &mut speaker,
                &mut display,
                &mut configured,
                &mut metrics,
            );
        } else {
            apply_event(
                &mut app,
                Event::ReceiverFault,
                &mut radio,
                &mut speaker,
                &mut display,
                &mut configured,
                &mut metrics,
            );
        }
    }
}

fn apply_event<B: RegisterBus>(
    app: &mut ReceiveApp,
    event: Event,
    radio: &mut Bk4819<B>,
    speaker: &mut SpeakerGate,
    display: &mut K5BootDisplay,
    configured: &mut Option<u16>,
    metrics: &mut Option<ReceiveMetrics>,
) {
    if !apply_effects(
        app.apply(event).iter(),
        radio,
        speaker,
        display,
        configured,
        metrics,
    ) {
        let _ = apply_effects(
            app.apply(Event::ReceiverFault).iter(),
            radio,
            speaker,
            display,
            configured,
            metrics,
        );
    }
}

/// Retunes and samples only on an explicit operator key edge. No target timing
/// or receiver settling interval is inferred by this validation image.
fn tune_and_sample<B: RegisterBus>(
    radio: &mut Bk4819<B>,
    channel: Pmr446Channel,
    audio: bool,
) -> (Option<u16>, Option<ReceiveMetrics>) {
    if radio.configure_receive(&channel.setup(audio)).is_err() {
        return (None, None);
    }
    let configured = radio.read_back(ReadbackRegister::FilterBandwidth).ok();
    let metrics = radio.receive_metrics(radio_domain::Tone::None).ok();
    (configured, metrics)
}

fn apply_effects<B: RegisterBus>(
    effects: impl Iterator<Item = Effect>,
    radio: &mut Bk4819<B>,
    speaker: &mut SpeakerGate,
    display: &mut K5BootDisplay,
    configured: &mut Option<u16>,
    metrics: &mut Option<ReceiveMetrics>,
) -> bool {
    for effect in effects {
        match effect {
            Effect::Tune {
                channel,
                frequency_hz,
                audio,
            } => {
                let Some(channel) = Pmr446Channel::new(channel) else {
                    return false;
                };
                if channel.frequency().as_hz() != frequency_hz {
                    return false;
                }
                (*configured, *metrics) = tune_and_sample(radio, channel, audio);
                if configured.is_none() || metrics.is_none() {
                    return false;
                }
            }
            Effect::SetChipAudio(audio) => {
                let output = if audio {
                    AfOutput::Demodulated
                } else {
                    AfOutput::Mute
                };
                if radio
                    .set_af_output(radio_domain::Modulation::Fm, output)
                    .is_err()
                {
                    return false;
                }
            }
            Effect::SetSpeaker(enabled) => speaker.set_enabled(enabled),
            Effect::Redraw(view) => {
                if !view.receiver_ok {
                    *configured = None;
                    *metrics = None;
                }
                display.show_pmr_receive(view, *configured, *metrics)
            }
        }
    }
    true
}

/// Startup work that has to touch memory the linker owns rather than Rust.
mod startup {
    // SAFETY: every item in this module operates on the four linker-defined
    // region symbols below. The linker script places `.data` and `.bss` inside
    // the evidenced RAM, word-aligns both, and asserts that the loaded image
    // ends inside the evidenced flash, so the regions copied and cleared here
    // exist and do not overlap.
    #![allow(unsafe_code)]

    unsafe extern "C" {
        /// First word of the initialised data region in RAM.
        static mut __data_start: u32;
        /// One past the last word of the initialised data region in RAM.
        static mut __data_end: u32;
        /// First word of that region's contents, held in flash.
        static __data_load_start: u32;
        /// First word of the zero-initialised region in RAM.
        static mut __bss_start: u32;
        /// One past the last word of the zero-initialised region in RAM.
        static mut __bss_end: u32;
    }

    /// Copies initialised data out of flash and clears the zeroed region.
    ///
    /// A Cortex-M0 comes out of reset with RAM in an undefined state and no
    /// runtime behind it. Until this has run, no static in this image holds the
    /// value its source says it does.
    pub fn initialise_ram() {
        // SAFETY: see the module note. The symbols are addresses, not values,
        // and are only ever used to bound a word-aligned walk.
        unsafe {
            let mut source = &raw const __data_load_start;
            let mut destination = &raw mut __data_start;
            let data_end = &raw mut __data_end;
            while destination < data_end {
                destination.write_volatile(source.read_volatile());
                source = source.add(1);
                destination = destination.add(1);
            }

            let mut zeroed = &raw mut __bss_start;
            let bss_end = &raw mut __bss_end;
            while zeroed < bss_end {
                zeroed.write_volatile(0);
                zeroed = zeroed.add(1);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
