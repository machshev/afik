//! Deterministic no-hardware proof of the display/serial cooperative boundary.

use core::future::{poll_fn, Future};
use core::pin::pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};

use crate::display::{ASYNC_WRITE_CHUNK_BYTES, FRAME_BYTES};

static DISPLAY_CHUNKS: AtomicUsize = AtomicUsize::new(0);
static SERIAL_POLLS: AtomicUsize = AtomicUsize::new(0);

async fn yield_once() {
    let mut yielded = false;
    poll_fn(|context| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

async fn transfer_frame() {
    for _ in 0..FRAME_BYTES / ASYNC_WRITE_CHUNK_BYTES {
        DISPLAY_CHUNKS.fetch_add(1, Ordering::SeqCst);
        yield_once().await;
    }
}

async fn service_serial() {
    while DISPLAY_CHUNKS.load(Ordering::SeqCst) < FRAME_BYTES / ASYNC_WRITE_CHUNK_BYTES {
        SERIAL_POLLS.fetch_add(1, Ordering::SeqCst);
        yield_once().await;
    }
}

fn poll_round_robin(
    display: impl Future<Output = ()>,
    serial: impl Future<Output = ()>,
) -> (usize, usize) {
    let mut display = pin!(display);
    let mut serial = pin!(serial);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut display_done = false;
    let mut serial_done = false;

    while !display_done || !serial_done {
        if !display_done {
            display_done = display.as_mut().poll(&mut context).is_ready();
        }
        if !serial_done {
            serial_done = serial.as_mut().poll(&mut context).is_ready();
        }
    }

    (
        DISPLAY_CHUNKS.load(Ordering::SeqCst),
        SERIAL_POLLS.load(Ordering::SeqCst),
    )
}

#[test]
fn full_display_frame_allows_serial_progress_between_chunks() {
    DISPLAY_CHUNKS.store(0, Ordering::SeqCst);
    SERIAL_POLLS.store(0, Ordering::SeqCst);

    let (display_chunks, serial_polls) = poll_round_robin(transfer_frame(), service_serial());

    assert_eq!(display_chunks, FRAME_BYTES / ASYNC_WRITE_CHUNK_BYTES);
    assert_eq!(display_chunks, 64);
    assert_eq!(serial_polls, display_chunks - 1);
}
