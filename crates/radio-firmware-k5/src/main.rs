//! The first UV-K5 V1 application AFIK can observe running.
//!
//! It configures the clock, binds UART1 to the programming connector, and then
//! answers the read-only hello while validating the evidenced display, keypad,
//! keypad and squelch-gated BK4819 receive audio. PTT drives experimental
//! PMR446 low-power transmit through the shared controller (`TXK5-058`).

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use radio_bk4819::Bk4819;
use radio_dp32g030::clock;
use radio_dp32g030::dma::CircularReceiver;
use radio_dp32g030::gpio::{self, Port};
use radio_dp32g030::portcon;
use radio_dp32g030::syscon::{self, Peripheral};
use radio_dp32g030::systick::PeriodicTick;
use radio_dp32g030::uart::{k5_programming_divider, Uart};
use radio_dp32g030::UART1_BASE;
use radio_firmware_k5::application::{apply_effects, apply_event, UiDispatcher};
use radio_firmware_k5::audio::SpeakerGate;
use radio_firmware_k5::bk4819_bus::{K5Pins, K5RegisterBus};
use radio_firmware_k5::eeprom::{self, K5Bus};
use radio_firmware_k5::k5_display::K5BootDisplay;
use radio_firmware_k5::keypad::{self, K5Matrix, MatrixIo};
use radio_firmware_k5::protocol::{
    encode_common_response, encode_common_view_response, encode_hello_response, NormalRequest,
    NormalService, RESPONSE_FRAME_BYTES,
};
use radio_firmware_k5::receive::Pmr446Channel;
use radio_firmware_k5::transmit;
use radio_platform::configuration_change::ConfigurationCandidate;
use radio_platform::display::show_boot_sequence;
use radio_platform::operator_shell::{Capabilities, Context, Shell};
use radio_platform::receive_app::{Event, ReceiveApp};
use radio_platform::tx_control::{self, Controller, State};

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
    best_effort_transmit_off();
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
    // K5 has no AFIK persistence adapter yet. Generation zero is explicit at
    // this boundary; the target still enters the same shared activation and
    // receive state machine as K1 without inventing EEPROM semantics.
    let mut app = ReceiveApp::new_with_identity(1);
    let mut configured = None;
    let mut metrics = None;
    // The EEPROM shares PA10/PA11 with the keypad, so calibration is read once
    // before the matrix takes those pins. Any failure leaves TX denied.
    let mut calibration = [0_u8; 3];
    let calibration = eeprom::read(
        &mut K5Bus::initialise(),
        transmit::CALIBRATION_ADDRESS,
        &mut calibration,
    )
    .ok()
    .map(|()| calibration);
    let mut keypad = K5Matrix::initialise();
    let mut shell = Shell::new();
    shell.set_capabilities(Capabilities::K5);
    shell.select_memory();
    let mut ui_dispatch = UiDispatcher::new(keypad::scan(&mut keypad));
    let mut ui_now = 0_u32;
    if initialised {
        apply_event(
            &mut app,
            Event::ConfigurationCandidate(
                ConfigurationCandidate::from_channel(0, Pmr446Channel::FIRST.number(), true)
                    .expect("first PMR446 channel is valid"),
            ),
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        );
    } else {
        apply_event(
            &mut app,
            Event::ReceiverFault,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        );
    }

    let _ = display.show_operator(&shell, app.view(), metrics);

    let mut service = NormalService::new();
    let mut response = [0_u8; RESPONSE_FRAME_BYTES];

    let tx_policy = transmit::policy();
    let mut ptt_down = keypad.ptt_pressed();
    let mut now_ms = 0_u64;
    let mut tx = Controller::new(1, ptt_down, now_ms).expect("instance is non-zero");
    let mut selected = app.view().channel;
    let mut revision = 1_u32;
    loop {
        let view = app.view();
        if view.channel != selected {
            selected = view.channel;
            revision = revision.saturating_add(1);
        }
        let level = shell.tx_power();
        let bias =
            Pmr446Channel::new(view.channel)
                .zip(calibration)
                .and_then(|(channel, calibration)| {
                    transmit::level_bias(calibration, channel.frequency().as_hz(), level)
                });
        let request = transmit::request(view, revision, bias, level);
        let ptt = keypad.ptt_pressed();
        let ptt_event = match (ptt_down, ptt) {
            (false, true) => tx_control::Event::Pressed,
            (true, false) => tx_control::Event::Released,
            _ => tx_control::Event::Tick,
        };
        ptt_down = ptt;
        let _ = tx.handle(now_ms, ptt_event, &tx_policy, &request);
        let receive = Pmr446Channel::new(view.channel)
            .map(|channel| channel.setup_with_squelch(view.audio, app.squelch()));
        transmit::run_effects(&mut tx, &tx_policy, &request, now_ms, |effect| {
            transmit::execute(
                effect,
                bias,
                receive,
                &mut radio,
                &mut speaker,
                &mut transmit::TickDelay(&mut sample_tick),
            )
        });
        // Serial, keypad and receive sampling all touch the radio, so they
        // pause until the controller has restored receive.
        if !matches!(tx.state(), State::Ready | State::ReleaseRequired) {
            now_ms = now_ms.saturating_add(u64::from(SAMPLE_MILLISECONDS));
            sample_tick.wait();
            continue;
        }

        while let Some(byte) = receiver.read_byte() {
            let Some(request) = service.push(byte) else {
                continue;
            };
            let length = match request {
                NormalRequest::Hello => {
                    encode_hello_response(&mut response);
                    RESPONSE_FRAME_BYTES
                }
                NormalRequest::Common(request) => {
                    let result =
                        radio_firmware_k5::protocol::apply_shared_wire_request(&mut app, request);
                    match result {
                        radio_firmware_k5::protocol::SerialResult::Effects(effects) => {
                            let complete = apply_effects(
                                effects.iter(),
                                &mut radio,
                                &mut speaker,
                                &mut configured,
                                &mut metrics,
                            );
                            if !complete {
                                // A target effect failure must invalidate the
                                // semantic snapshot as well as muting the
                                // physical speaker.  Otherwise a later View
                                // could expose a healthy state after a
                                // refused serial key.
                                apply_event(
                                    &mut app,
                                    Event::ReceiverFault,
                                    &mut radio,
                                    &mut speaker,
                                    &mut configured,
                                    &mut metrics,
                                );
                                0
                            } else {
                                encode_common_view_response(app.view(), request, &mut response)
                            }
                        }
                        result => encode_common_response(result, request, &mut response)
                            .map_or(0, |_| RESPONSE_FRAME_BYTES),
                    }
                }
            };
            if length == 0 {
                continue;
            }
            UART1.write(&response[..length]);
            UART1.flush();
        }
        let key = keypad::scan(&mut keypad);
        let _ = ui_dispatch.dispatch_scan(
            key,
            ui_now,
            Context {
                visible_channels: radio_platform::configuration::PMR446_CHANNELS as u16,
                active_index: u16::from(app.view().channel.saturating_sub(1)),
                scanning: false,
            },
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        );
        ui_now = ui_now.wrapping_add(SAMPLE_MILLISECONDS);
        now_ms = now_ms.saturating_add(u64::from(SAMPLE_MILLISECONDS));
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
                &mut configured,
                &mut metrics,
            );
        } else {
            apply_event(
                &mut app,
                Event::ReceiverFault,
                &mut radio,
                &mut speaker,
                &mut configured,
                &mut metrics,
            );
        }
        let _ = display.show_operator(&shell, app.view(), metrics);
    }
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

/// Rebinds the BK4819 pins and writes PA off then chip idle, ignoring state.
///
/// Best effort only: it needs the bus and the core to be working.
fn best_effort_transmit_off() {
    let mut radio = Bk4819::new(K5RegisterBus::new(K5Pins::initialise()));
    let _ = radio.disable_pa();
    let _ = radio.disable_transmit();
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    best_effort_transmit_off();
    loop {
        core::hint::spin_loop();
    }
}
