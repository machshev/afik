//! The byte link a radio is reached over, blocking or not (`ADR-119`).
//!
//! Every exchange in this crate is an `async fn` over [`LinkRead`] and
//! [`LinkWrite`], so the same code drives a blocking serial port on the
//! desktop and Web Serial in the browser. Time is measured only in empty
//! reads: a link's read returns zero once its read window passes with
//! nothing, as a serial port's read timeout does.

use std::{
    future::Future,
    io::{self, Read, Write},
    pin::pin,
    task::{Context, Poll, Waker},
};

/// The receiving half of a link to a radio.
// No `Send` bound is wanted: a browser link's futures hold JavaScript values.
#[allow(async_fn_in_trait)]
pub trait LinkRead {
    /// Reads whatever has arrived into `buffer`, returning zero once the
    /// link's read window passes with nothing.
    async fn receive(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
}

/// The sending half of a link to a radio.
#[allow(async_fn_in_trait)]
pub trait LinkWrite {
    /// Sends every byte of `bytes` in order and flushes them to the radio.
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()>;
}

// A blocking port finishes each call before returning, so nothing is awaited.
impl<T: Read> LinkRead for T {
    #[allow(clippy::unused_async_trait_impl)]
    async fn receive(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        Read::read(self, buffer)
    }
}

impl<T: Write> LinkWrite for T {
    #[allow(clippy::unused_async_trait_impl)]
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        Write::write_all(self, bytes)?;
        Write::flush(self)
    }
}

/// Runs an exchange over a blocking link to completion on this thread.
///
/// A blocking link never leaves an exchange waiting, so every poll makes
/// progress; a future that does wait on something else is polled again until
/// it finishes.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{block_on, LinkRead, LinkWrite};

    #[test]
    fn a_blocking_reader_reads_through_the_link() {
        let mut link = Cursor::new(vec![1_u8, 2, 3]);
        let mut buffer = [0_u8; 2];
        assert_eq!(block_on(link.receive(&mut buffer)).unwrap(), 2);
        assert_eq!(buffer, [1, 2]);
        assert_eq!(block_on(link.receive(&mut buffer)).unwrap(), 1);
        assert_eq!(block_on(link.receive(&mut buffer)).unwrap(), 0);
    }

    #[test]
    fn a_blocking_writer_sends_every_byte() {
        let mut link = Vec::new();
        block_on(link.send(&[7, 8, 9])).unwrap();
        assert_eq!(link, [7, 8, 9]);
    }

    #[test]
    fn block_on_finishes_a_future_that_waits_once() {
        let mut waited = false;
        let output = block_on(std::future::poll_fn(|_| {
            if waited {
                std::task::Poll::Ready(5)
            } else {
                waited = true;
                std::task::Poll::Pending
            }
        }));
        assert_eq!(output, 5);
    }
}
