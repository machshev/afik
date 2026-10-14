//! A radio programmed as memory (`ADR-100`).
//!
//! The stock hello, read and write requests are decoded and answered by the
//! radio's own protocol code in `radio_platform::memory_link`, over an 8 KiB
//! view, so a host tool tested against this talks to what a radio runs.
//! [`MemoryRadio::index`] loads the image the way a radio does after a write.

use std::{collections::VecDeque, io};

use radio_platform::{
    memory_link::{
        encode_hello_response, encode_read_response, encode_write_response, host_may_write,
        Decoder, Push, Request, IMAGE_REGION_BYTES, RESPONSE_FRAME_BYTES, VIEW_BYTES,
    },
    serial::ApplicationIdentity,
};
use radio_storage::{
    radio_image::{Index, HEADER_LEN},
    StorageError,
};

/// The identity a simulated radio answers hello with.
pub const SIMULATED_IDENTITY: &[u8] = concat!("AFIK-K5-", env!("CARGO_PKG_VERSION")).as_bytes();

/// An in-memory radio answering the stock memory protocol.
#[derive(Debug)]
pub struct MemoryRadio {
    identity: &'static [u8],
    memory: Vec<u8>,
    decoder: Decoder,
    pending: VecDeque<u8>,
    writes: usize,
}

impl Default for MemoryRadio {
    fn default() -> Self {
        Self::new(SIMULATED_IDENTITY)
    }
}

impl MemoryRadio {
    /// A radio with erased memory which answers hello with `identity`.
    ///
    /// # Panics
    ///
    /// If `identity` is not a printable string of at most 32 bytes.
    pub fn new(identity: &'static [u8]) -> Self {
        assert!(
            ApplicationIdentity::new(identity).is_some(),
            "printable bounded identity"
        );
        Self {
            identity,
            memory: vec![0xFF; usize::from(VIEW_BYTES)],
            decoder: Decoder::new(),
            pending: VecDeque::new(),
            writes: 0,
        }
    }

    /// The whole memory view.
    pub fn memory(&self) -> &[u8] {
        &self.memory
    }

    /// Acknowledged writes so far.
    pub const fn writes(&self) -> usize {
        self.writes
    }

    /// Loads the image as a radio does a second after a host's last write.
    pub fn index(&self) -> Result<Index, StorageError> {
        let region = &self.memory[..usize::from(IMAGE_REGION_BYTES)];
        let mut header = [0_u8; HEADER_LEN];
        header.copy_from_slice(&region[..HEADER_LEN]);
        let payload = &region[HEADER_LEN..];
        Index::build(&header, |offset, buffer| {
            payload
                .get(offset..offset + buffer.len())
                .is_some_and(|bytes| {
                    buffer.copy_from_slice(bytes);
                    true
                })
        })
    }

    fn answer(&mut self, request: Request) {
        let mut frame = [0_u8; RESPONSE_FRAME_BYTES];
        let length = match request {
            Request::Hello => ApplicationIdentity::new(self.identity)
                .map(|identity| encode_hello_response(identity, &mut frame)),
            Request::Read { address, length } => {
                let memory = &self.memory;
                encode_read_response(address, length, &mut frame, |data| {
                    let start = usize::from(address);
                    memory.get(start..start + data.len()).is_some_and(|bytes| {
                        data.copy_from_slice(bytes);
                        true
                    })
                })
            }
            Request::Write { address, data } => {
                let start = usize::from(address);
                // A refused write is not acknowledged, as on a radio.
                (host_may_write(address) && start + data.len() <= self.memory.len()).then(|| {
                    self.memory[start..start + data.len()].copy_from_slice(&data);
                    self.writes += 1;
                    encode_write_response(address, &mut frame)
                })
            }
        };
        if let Some(length) = length {
            self.pending.extend(&frame[..length]);
        }
    }
}

impl io::Write for MemoryRadio {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        for &byte in buffer {
            if let Push::Request(request) = self.decoder.push(byte) {
                self.answer(request);
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Read for MemoryRadio {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(self.pending.len());
        for byte in &mut buffer[..count] {
            *byte = self.pending.pop_front().unwrap_or(0);
        }
        Ok(count)
    }
}
