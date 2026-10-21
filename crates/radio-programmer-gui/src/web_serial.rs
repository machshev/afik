//! A radio link over the browser's Web Serial (`ADR-119`).
//!
//! The link behaves as the desktop serial port does: 8N1 without flow
//! control, and a read that returns nothing once
//! [`radio_programmer_serial::READ_TIMEOUT`] passes. A read the window cuts
//! short is kept and finished by the next call, so no byte is lost.
//!
//! The ports the operator has granted the page are kept by name, so the
//! Flash tab can list and reopen them as it does desktop device paths.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::{poll_fn, Future},
    io,
    path::{Path, PathBuf},
    pin::Pin,
    task::Poll,
};

use js_sys::{Function, Object, Promise, Reflect, Uint8Array};
use radio_flasher::{LinkRead, LinkWrite};
use radio_programmer_serial::{is_supported_baud, READ_TIMEOUT};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ReadableStreamDefaultReader, ReadableStreamReadResult, SerialOptions, SerialPort,
    WritableStreamDefaultWriter,
};

/// Bytes the browser buffers from the port between reads.
const BUFFER_BYTES: u32 = 4_096;

thread_local! {
    /// The ports granted to the page, each with the name it is listed by.
    static GRANTED: RefCell<Vec<(PathBuf, SerialPort)>> = const { RefCell::new(Vec::new()) };
    /// Counts changes to [`GRANTED`], so a list of it knows when it is stale.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// The names of the ports granted to the page, in the browser's order.
pub fn granted() -> Vec<PathBuf> {
    GRANTED.with_borrow(|ports| ports.iter().map(|(name, _)| name.clone()).collect())
}

/// Changes whenever the granted ports do.
pub fn generation() -> u64 {
    GENERATION.get()
}

/// Asks the browser again which ports the page may open; [`granted`] has
/// the answer once it arrives.
pub fn refresh() {
    wasm_bindgen_futures::spawn_local(async {
        if let Err(error) = refresh_now().await {
            log::warn!("listing granted serial ports: {error}");
        }
    });
}

/// Shows the browser's port chooser, so this must follow a click. A chosen
/// port joins [`granted`].
pub fn choose() {
    wasm_bindgen_futures::spawn_local(async {
        let chosen = match serial() {
            Ok(serial) => serial
                .request_port()
                .await
                .map(drop)
                .map_err(|error| format!("no port chosen: {}", message(&error))),
            Err(error) => Err(error),
        };
        if let Err(error) = chosen {
            log::warn!("{error}");
        }
        if let Err(error) = refresh_now().await {
            log::warn!("listing granted serial ports: {error}");
        }
    });
}

async fn refresh_now() -> Result<(), String> {
    let ports: Vec<SerialPort> = serial()?
        .get_ports()
        .await
        .map_err(|error| message(&error))?
        .iter()
        .collect();
    let unchanged = GRANTED.with_borrow(|known| {
        known.len() == ports.len()
            && known
                .iter()
                .zip(&ports)
                .all(|((_, old), new)| Object::is(old, new))
    });
    if !unchanged {
        let named = ports
            .into_iter()
            .enumerate()
            .map(|(index, port)| (port_name(index, &port), port))
            .collect();
        GRANTED.set(named);
        GENERATION.set(GENERATION.get().wrapping_add(1));
    }
    Ok(())
}

/// Names a granted port by its place in the list and its USB identity.
fn port_name(index: usize, port: &SerialPort) -> PathBuf {
    PathBuf::from(format!("Port {}: {}", index + 1, describe(port)))
}

/// What the browser says the port is.
fn describe(port: &SerialPort) -> String {
    let info = port.get_info();
    match (info.get_usb_vendor_id(), info.get_usb_product_id()) {
        (Some(vendor), Some(product)) => format!("USB serial {vendor:04x}:{product:04x}"),
        _ => "serial port".to_owned(),
    }
}

fn serial() -> Result<web_sys::Serial, String> {
    if !WebSerialLink::is_available() {
        return Err(
            "this browser has no Web Serial; use a recent Chrome, Edge or Firefox".to_owned(),
        );
    }
    Ok(web_sys::window()
        .ok_or("no browser window")?
        .navigator()
        .serial())
}

/// One open Web Serial port, locked for reading and writing.
pub struct WebSerialLink {
    port: SerialPort,
    reader: ReadableStreamDefaultReader,
    writer: WritableStreamDefaultWriter,
    /// A read the last window cut short.
    pending: Option<Pin<Box<JsFuture>>>,
    /// Bytes read but not yet taken.
    buffered: VecDeque<u8>,
    description: String,
    /// [`Self::close`] has released the port.
    closed: bool,
}

impl WebSerialLink {
    /// Whether this browser offers Web Serial at all.
    pub fn is_available() -> bool {
        web_sys::window().is_some_and(|window| {
            Reflect::has(&window.navigator(), &JsValue::from_str("serial")).unwrap_or(false)
        })
    }

    /// Asks the browser for a port, which shows its chooser, and opens the
    /// chosen one at `baud`.
    pub async fn request(baud: u32) -> Result<Self, String> {
        if !is_supported_baud(baud) {
            return Err(format!("unsupported baud: {baud}"));
        }
        let port = serial()?
            .request_port()
            .await
            .map_err(|error| format!("no port chosen: {}", message(&error)))?;
        // The Flash tab lists the port from now on.
        refresh();
        Self::open(port, baud).await
    }

    /// Opens the granted port [`granted`] lists as `name`, at `baud`.
    pub async fn open_granted(name: &Path, baud: u32) -> Result<Self, String> {
        if !is_supported_baud(baud) {
            return Err(format!("unsupported baud: {baud}"));
        }
        let port = GRANTED
            .with_borrow(|ports| {
                ports
                    .iter()
                    .find(|(known, _)| known == name)
                    .map(|(_, port)| port.clone())
            })
            .ok_or_else(|| format!("{} is not a port granted to this page", name.display()))?;
        Self::open(port, baud).await
    }

    async fn open(port: SerialPort, baud: u32) -> Result<Self, String> {
        let options = SerialOptions::new(baud);
        options.set_buffer_size(BUFFER_BYTES);
        port.open(&options)
            .await
            .map_err(|error| format!("could not open the port: {}", message(&error)))?;
        let reader = port
            .readable()
            .get_reader()
            .unchecked_into::<ReadableStreamDefaultReader>();
        let writer = port
            .writable()
            .get_writer()
            .map_err(|error| format!("could not write to the port: {}", message(&error)))?;
        let description = describe(&port);
        log::info!("opened {description} at {baud} baud");
        Ok(Self {
            port,
            reader,
            writer,
            pending: None,
            buffered: VecDeque::new(),
            description,
            closed: false,
        })
    }

    /// Closes the port and waits until the browser has, so it can be
    /// opened again at once.
    pub async fn close(mut self) {
        self.closed = true;
        release(&self.port, &self.reader, &self.writer).await;
    }

    /// What the browser says the port is.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Waits for the next chunk from the port, or the read window to pass.
    async fn fill(&mut self) -> io::Result<()> {
        let reader = &self.reader;
        let read = self
            .pending
            .get_or_insert_with(|| Box::pin(JsFuture::from(reader.read())));
        let mut timer = Box::pin(JsFuture::from(sleep(READ_TIMEOUT.as_millis())));
        let chunk = poll_fn(|context| {
            if let Poll::Ready(result) = read.as_mut().poll(context) {
                return Poll::Ready(Some(result));
            }
            timer.as_mut().poll(context).map(|_| None)
        })
        .await;
        let Some(result) = chunk else {
            return Ok(());
        };
        self.pending = None;
        let result = result
            .map_err(|error| io::Error::other(format!("serial read: {}", message(&error))))?
            .unchecked_into::<ReadableStreamReadResult>();
        if result.get_done().unwrap_or(false) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the serial port closed",
            ));
        }
        let bytes = result.get_value().unchecked_into::<Uint8Array>().to_vec();
        self.buffered.extend(bytes);
        Ok(())
    }
}

impl LinkRead for WebSerialLink {
    async fn receive(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.buffered.is_empty() {
            self.fill().await?;
        }
        let count = buffer.len().min(self.buffered.len());
        for (slot, byte) in buffer.iter_mut().zip(self.buffered.drain(..count)) {
            *slot = byte;
        }
        Ok(count)
    }
}

impl LinkWrite for WebSerialLink {
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        JsFuture::from(self.writer.write_with_chunk(&Uint8Array::from(bytes)))
            .await
            .map(drop)
            .map_err(|error| io::Error::other(format!("serial write: {}", message(&error))))
    }
}

impl Drop for WebSerialLink {
    /// Unlocks and closes the port, so it can be chosen again.
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let (port, reader, writer) = (self.port.clone(), self.reader.clone(), self.writer.clone());
        wasm_bindgen_futures::spawn_local(async move { release(&port, &reader, &writer).await });
    }
}

async fn release(
    port: &SerialPort,
    reader: &ReadableStreamDefaultReader,
    writer: &WritableStreamDefaultWriter,
) {
    let _cancelled = JsFuture::from(reader.cancel()).await;
    reader.release_lock();
    writer.release_lock();
    if let Err(error) = port.close().await {
        log::warn!("closing the serial port: {}", message(&error));
    }
}

/// A promise the page's timer resolves after `millis`.
fn sleep(millis: u128) -> Promise {
    let millis = i32::try_from(millis).unwrap_or(i32::MAX);
    Promise::new(&mut |resolve: Function, _reject: Function| {
        if let Some(window) = web_sys::window() {
            let _id =
                window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
        }
    })
}

/// The message of a JavaScript error, or the value as text.
fn message(error: &JsValue) -> String {
    Reflect::get(error, &JsValue::from_str("message"))
        .ok()
        .and_then(|text| text.as_string())
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}
