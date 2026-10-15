//! Emulator test for the K5's memory routines (`mem_routines.s`).
//!
//! The firmware replaces compiler-builtins' `memcpy`, `memmove` and `memset`
//! and their ARM EABI entry points with byte loops, which no host test can
//! run. `tool/test-k5-mem.sh` builds this for Thumb-1 and runs it under
//! `qemu-arm`; it exits 0 when every check passes, or the failing check's
//! number. It uses no copy, fill or comparison the compiler might lower to the
//! routines under test.

#![no_std]
#![no_main]
// The routines under test are assembly reached through raw pointers.
#![allow(unsafe_code)]

core::arch::global_asm!(include_str!("../mem_routines.s"));

#[allow(unsafe_code)]
mod routines {
    unsafe extern "C" {
        pub fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8;
        pub fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8;
        pub fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8;
        pub fn __aeabi_memcpy4(dest: *mut u8, src: *const u8, n: usize);
        pub fn __aeabi_memmove8(dest: *mut u8, src: *const u8, n: usize);
        pub fn __aeabi_memset(dest: *mut u8, n: usize, c: i32);
        pub fn __aeabi_memclr4(dest: *mut u8, n: usize);
    }
}

const LEN: usize = 48;

/// Fills `buffer` with `seed + index`, one byte at a time.
fn pattern(buffer: &mut [u8; LEN], seed: u8) {
    let mut index = 0;
    while index < LEN {
        // SAFETY: `index` is inside the buffer.
        #[allow(unsafe_code)]
        unsafe {
            core::ptr::write_volatile(
                buffer.as_mut_ptr().add(index),
                seed.wrapping_add(index as u8),
            );
        }
        index += 1;
    }
}

fn at(buffer: &[u8; LEN], index: usize) -> u8 {
    // SAFETY: callers index inside the buffer.
    #[allow(unsafe_code)]
    unsafe {
        core::ptr::read_volatile(buffer.as_ptr().add(index))
    }
}

/// Checks every byte: `expected(index)` for the bytes the call should have
/// written, the untouched pattern elsewhere.
fn check(buffer: &[u8; LEN], expected: impl Fn(usize) -> u8) -> bool {
    let mut index = 0;
    while index < LEN {
        if at(buffer, index) != expected(index) {
            return false;
        }
        index += 1;
    }
    true
}

#[allow(unsafe_code, clippy::too_many_lines)]
fn run() -> u8 {
    use routines::*;
    let mut a = [0_u8; LEN];
    let mut b = [0_u8; LEN];
    let base = |seed: u8| move |index: usize| seed.wrapping_add(index as u8);

    // 1-2: memcpy copies exactly n bytes and returns dest; n = 0 writes none.
    pattern(&mut a, 0);
    pattern(&mut b, 100);
    let returned = unsafe { memcpy(a.as_mut_ptr().add(3), b.as_ptr().add(5), 17) };
    if returned != unsafe { a.as_mut_ptr().add(3) } {
        return 1;
    }
    if !check(&a, |i| {
        if (3..20).contains(&i) {
            100 + i as u8 + 2
        } else {
            base(0)(i)
        }
    }) {
        return 2;
    }
    pattern(&mut a, 0);
    unsafe { memcpy(a.as_mut_ptr(), b.as_ptr(), 0) };
    if !check(&a, base(0)) {
        return 3;
    }

    // 4-5: memmove with the destination below an overlapping source.
    pattern(&mut a, 0);
    let returned = unsafe { memmove(a.as_mut_ptr().add(2), a.as_ptr().add(7), 20) };
    if returned != unsafe { a.as_mut_ptr().add(2) } {
        return 4;
    }
    if !check(&a, |i| {
        if (2..22).contains(&i) {
            i as u8 + 5
        } else {
            i as u8
        }
    }) {
        return 5;
    }

    // 6-7: memmove with the destination above an overlapping source.
    pattern(&mut a, 0);
    let returned = unsafe { memmove(a.as_mut_ptr().add(7), a.as_ptr().add(2), 20) };
    if returned != unsafe { a.as_mut_ptr().add(7) } {
        return 6;
    }
    if !check(&a, |i| {
        if (7..27).contains(&i) {
            i as u8 - 5
        } else {
            i as u8
        }
    }) {
        return 7;
    }

    // 8: memmove of zero bytes above the source writes none.
    pattern(&mut a, 0);
    unsafe { memmove(a.as_mut_ptr().add(9), a.as_ptr(), 0) };
    if !check(&a, base(0)) {
        return 8;
    }

    // 9-10: memset fills n bytes with the low byte of c and returns dest.
    pattern(&mut a, 0);
    let returned = unsafe { memset(a.as_mut_ptr().add(4), 0x1A5, 13) };
    if returned != unsafe { a.as_mut_ptr().add(4) } {
        return 9;
    }
    if !check(&a, |i| if (4..17).contains(&i) { 0xA5 } else { i as u8 }) {
        return 10;
    }

    // 11: __aeabi_memset takes the length before the fill.
    pattern(&mut a, 0);
    unsafe { __aeabi_memset(a.as_mut_ptr().add(1), 9, 0x3C) };
    if !check(&a, |i| if (1..10).contains(&i) { 0x3C } else { i as u8 }) {
        return 11;
    }

    // 12: __aeabi_memclr4 zeroes.
    pattern(&mut a, 1);
    unsafe { __aeabi_memclr4(a.as_mut_ptr().add(8), 16) };
    if !check(&a, |i| if (8..24).contains(&i) { 0 } else { 1 + i as u8 }) {
        return 12;
    }

    // 13: __aeabi_memcpy4 copies.
    pattern(&mut a, 0);
    pattern(&mut b, 50);
    unsafe { __aeabi_memcpy4(a.as_mut_ptr(), b.as_ptr(), LEN) };
    if !check(&a, base(50)) {
        return 13;
    }

    // 14: __aeabi_memmove8 handles overlap above the source.
    pattern(&mut a, 0);
    unsafe { __aeabi_memmove8(a.as_mut_ptr().add(1), a.as_ptr(), 40) };
    if !check(&a, |i| {
        if (1..41).contains(&i) {
            i as u8 - 1
        } else {
            i as u8
        }
    }) {
        return 14;
    }
    0
}

/// Linux `exit` under `qemu-arm`.
#[allow(unsafe_code)]
fn exit(code: u8) -> ! {
    // SAFETY: the exit system call does not return, so overwriting r7 (the
    // frame pointer, which asm may not take as an operand) is harmless.
    unsafe {
        core::arch::asm!("movs r7, #1", "svc 0", in("r0") u32::from(code), options(noreturn));
    }
}

#[allow(unsafe_code)]
#[no_mangle]
extern "C" fn _start() -> ! {
    exit(run())
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    exit(99)
}
