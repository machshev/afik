//! Minimal UV-K1/PY32F071 Cortex-M0+ reset image.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

const INITIAL_STACK_POINTER: u32 = 0x2000_4000;
const BOOT_SENTINEL_VALUE: u32 = 0x4B31_B007;

#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
}

// SAFETY: the K1 linker script owns this section, retains this one table, and
// places it at the evidenced application origin.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".vector_table")]
static VECTOR_TABLE: VectorTable = VectorTable {
    initial_stack_pointer: INITIAL_STACK_POINTER,
    reset,
};

// SAFETY: the linker script owns this section, asserts its one-word size, and
// places it at the development-only RAM witness address.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".boot_sentinel")]
static BOOT_SENTINEL: AtomicU32 = AtomicU32::new(0);

// SAFETY: this is the single linker entry symbol named by link.x. The function
// uses the Cortex-M ABI and has no peripheral or external-memory side effect.
#[allow(unsafe_code)]
#[unsafe(export_name = "Reset")]
extern "C" fn reset() -> ! {
    BOOT_SENTINEL.store(BOOT_SENTINEL_VALUE, Ordering::Release);
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
