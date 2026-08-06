//! Receive-only K1 Embassy keypad/display/UART witness.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;

use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_time::{Duration, Instant, Timer};
use py32_hal::gpio::{Input, Level, Output, Pull, Speed};
use py32_hal::mode::Async;
use py32_hal::peripherals::SPI1;
use py32_hal::spi::SpiTx;
use py32_hal::usart::Uart;
use radio_firmware_k1::display::{
    render_key_witness, render_witness, COLUMN_OFFSET, FRAME_BYTES, PAGES, SETUP_COMMANDS, WIDTH,
};
use radio_firmware_k1::keypad::{decode, Debouncer, Edge, Sample};
use radio_firmware_k1::protocol::{
    decode_request, encode_hello_response, Request, REQUEST_BODY_BYTES, RESPONSE_FRAME_BYTES,
};
use radio_firmware_k1::py32f071_runtime::{compose, K1RuntimePeripherals};
use radio_firmware_k1::py32f071_runtime_init::init;

const _: [(); 8] = [(); PAGES];

#[entry]
fn main() -> ! {
    let Ok(runtime_init) = init() else {
        fail_closed();
    };
    let p = runtime_init.peripherals;
    let Ok(runtime) = compose(K1RuntimePeripherals {
        usart: p.USART1,
        usart_rx: p.PA10,
        usart_tx: p.PA9,
        usart_tx_dma: p.DMA1_CH1,
        usart_rx_dma: p.DMA1_CH2,
        display_spi: p.SPI1,
        display_sck: p.PA5,
        display_mosi: p.PA7,
    }) else {
        fail_closed();
    };

    let mut executor = runtime.executor;
    let serial = runtime.usart;
    let display = DisplayPins {
        spi: runtime.display_spi,
        a0: Output::new(p.PA6, Level::High, Speed::High),
        cs: Output::new(p.PB2, Level::High, Speed::High),
        backlight: Output::new(p.PF8, Level::High, Speed::High),
    };
    let keypad = KeypadPins {
        columns: [
            Output::new(p.PB6, Level::High, Speed::High),
            Output::new(p.PB5, Level::High, Speed::High),
            Output::new(p.PB4, Level::High, Speed::High),
            Output::new(p.PB3, Level::High, Speed::High),
        ],
        rows: [
            Input::new(p.PB15, Pull::Up),
            Input::new(p.PB14, Pull::Up),
            Input::new(p.PB13, Pull::Up),
            Input::new(p.PB12, Pull::Up),
        ],
    };

    // SAFETY: `main` never returns, so this stack allocation lives for the
    // executor's complete lifetime and no other mutable reference is retained.
    #[allow(unsafe_code)]
    let executor: &'static mut Executor = unsafe { core::mem::transmute(&mut executor) };
    executor.run(|spawner| {
        let Ok(serial) = serial_task(serial) else {
            fail_closed();
        };
        let Ok(ui) = ui_task(display, keypad) else {
            fail_closed();
        };
        spawner.spawn(serial);
        spawner.spawn(ui);
    });
}

struct DisplayPins {
    spi: SpiTx<'static, SPI1>,
    a0: Output<'static>,
    cs: Output<'static>,
    backlight: Output<'static>,
}

impl DisplayPins {
    async fn transfer(&mut self, data: bool, bytes: &[u8]) -> bool {
        self.a0
            .set_level(if data { Level::High } else { Level::Low });
        self.cs.set_low();
        let result = self.spi.write(bytes).await.is_ok();
        self.cs.set_high();
        result
    }

    async fn initialise(&mut self) -> bool {
        self.backlight.set_high();
        if !self.transfer(false, &[0xE2]).await {
            return false;
        }
        Timer::after_millis(120).await;
        if !self.transfer(false, &SETUP_COMMANDS).await || !self.transfer(false, &[0x2B]).await {
            return false;
        }
        Timer::after_millis(1).await;
        if !self.transfer(false, &[0x2E]).await {
            return false;
        }
        Timer::after_millis(1).await;
        if !self.transfer(false, &[0x2F]).await {
            return false;
        }
        Timer::after_millis(40).await;
        self.transfer(false, &[0x40, 0xAF]).await
    }

    async fn frame(&mut self, frame: &[u8; FRAME_BYTES]) -> bool {
        if !self.transfer(false, &[0x40]).await {
            return false;
        }
        for page in 0_u8..8 {
            let address = [
                0xB0 | page,
                0x10 | (COLUMN_OFFSET >> 4),
                COLUMN_OFFSET & 0x0F,
            ];
            if !self.transfer(false, &address).await {
                return false;
            }
            let start = usize::from(page) * WIDTH;
            if !self.transfer(true, &frame[start..start + WIDTH]).await {
                return false;
            }
        }
        true
    }
}

struct KeypadPins {
    columns: [Output<'static>; 4],
    rows: [Input<'static>; 4],
}

impl KeypadPins {
    async fn scan(&mut self) -> [u8; 4] {
        let mut result = [0_u8; 4];
        for column in &mut self.columns {
            column.set_high();
        }
        for (index, column) in self.columns.iter_mut().enumerate() {
            column.set_low();
            Timer::after_micros(10).await;
            for (row, input) in self.rows.iter().enumerate() {
                if input.is_low() {
                    result[index] |= 1 << row;
                }
            }
            column.set_high();
        }
        result
    }
}

#[embassy_executor::task]
async fn serial_task(mut uart: Uart<'static, Async>) {
    let mut window = [0_u8; 16];
    let mut used = 0_usize;
    loop {
        if uart.read(&mut window[used..=used]).await.is_err() {
            continue;
        }
        used += 1;
        if used >= 2 && window[used - 2..used] == [0xAB, 0xCD] {
            window.copy_within(used - 2..used, 0);
            used = 2;
        }
        if used != window.len() {
            continue;
        }
        if window[2..4] == 8_u16.to_le_bytes() && window[14..16] == [0xDC, 0xBA] {
            let mut body = [0_u8; REQUEST_BODY_BYTES];
            body.copy_from_slice(&window[4..14]);
            if decode_request(&mut body) == Some(Request::Hello) {
                let mut response = [0_u8; RESPONSE_FRAME_BYTES];
                encode_hello_response(&mut response);
                let _ = uart.write(&response).await;
            }
        }
        used = 0;
    }
}

#[embassy_executor::task]
async fn ui_task(mut display: DisplayPins, mut keypad: KeypadPins) {
    let mut frame = [0_u8; FRAME_BYTES];
    render_witness(&mut frame);
    if !display.initialise().await || !display.frame(&frame).await {
        fail_closed();
    }
    let mut debounce = Debouncer::new();
    loop {
        let sample = match decode(keypad.scan().await) {
            Ok(Some(key)) => Sample::Key(key),
            Ok(None) => Sample::Released,
            Err(_) => Sample::Invalid,
        };
        let now = u32::try_from(Instant::now().as_millis()).unwrap_or(u32::MAX);
        if let Edge::Pressed(key) = debounce.update(now, sample) {
            render_key_witness(&mut frame, key);
            if !display.frame(&frame).await {
                fail_closed();
            }
        }
        Timer::after(Duration::from_millis(5)).await;
    }
}

fn fail_closed() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    fail_closed()
}
