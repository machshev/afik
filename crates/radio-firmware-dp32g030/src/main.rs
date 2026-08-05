//! Minimal DP32G030 Cortex-M0 reset image.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

const INITIAL_STACK_POINTER: u32 = 0x2000_4000;
const BOOT_SENTINEL_VALUE: u32 = 0xD032_B007;

#[repr(C)]
struct VectorTable {
    initial_stack_pointer: u32,
    reset: extern "C" fn() -> !,
}

// SAFETY: the linker script owns this section, retains this one table, asserts
// its exact size, and places it at the evidenced Cortex-M0 vector address.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".vector_table")]
static VECTOR_TABLE: VectorTable = VectorTable {
    initial_stack_pointer: INITIAL_STACK_POINTER,
    reset,
};

// SAFETY: the linker script owns this section, asserts its one-word size, and
// places it inside evidenced RAM. Atomic access avoids raw-pointer operations.
#[allow(unsafe_code)]
#[used]
#[unsafe(link_section = ".boot_sentinel")]
static BOOT_SENTINEL: AtomicU32 = AtomicU32::new(0);

// SAFETY: this is the single linker entry symbol named by link.x. The name is
// scoped to this standalone image, and the function has the Cortex-M ABI.
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
