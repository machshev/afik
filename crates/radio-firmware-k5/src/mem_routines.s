@ Byte-at-a-time memcpy, memmove and memset, and the ARM EABI entry points
@ Rust calls, for the K5 (Thumb-1). This is a Rust `global_asm!` template, so
@ braces are doubled. Included by the firmware (`main.rs`, `mod mem`) and by
@ the emulator test (`src/bin/mem_test.rs`, `tool/test-k5-mem.sh`).
.section .text.afik_mem,"ax",%progbits
.balign 2
.global memcpy, __aeabi_memcpy, __aeabi_memcpy4, __aeabi_memcpy8
.global memmove, __aeabi_memmove, __aeabi_memmove4, __aeabi_memmove8
.global memset, __aeabi_memset, __aeabi_memset4, __aeabi_memset8
.global __aeabi_memclr, __aeabi_memclr4, __aeabi_memclr8
.type memcpy,%function
.type __aeabi_memcpy,%function
.type __aeabi_memcpy4,%function
.type __aeabi_memcpy8,%function
.type memmove,%function
.type __aeabi_memmove,%function
.type __aeabi_memmove4,%function
.type __aeabi_memmove8,%function
.type memset,%function
.type __aeabi_memset,%function
.type __aeabi_memset4,%function
.type __aeabi_memset8,%function
.type __aeabi_memclr,%function
.type __aeabi_memclr4,%function
.type __aeabi_memclr8,%function
.thumb_func
memmove:
.thumb_func
__aeabi_memmove:
.thumb_func
__aeabi_memmove4:
.thumb_func
__aeabi_memmove8:
    cmp r0, r1
    bls memcpy
    push {{r4, lr}}
3:  subs r2, #1
    bcc 4f
    ldrb r4, [r1, r2]
    strb r4, [r0, r2]
    b 3b
4:  pop {{r4, pc}}
.thumb_func
memcpy:
.thumb_func
__aeabi_memcpy:
.thumb_func
__aeabi_memcpy4:
.thumb_func
__aeabi_memcpy8:
    push {{r4, lr}}
    movs r3, #0
1:  cmp r3, r2
    beq 2f
    ldrb r4, [r1, r3]
    strb r4, [r0, r3]
    adds r3, #1
    b 1b
2:  pop {{r4, pc}}
.thumb_func
__aeabi_memclr:
.thumb_func
__aeabi_memclr4:
.thumb_func
__aeabi_memclr8:
    movs r2, #0
.thumb_func
__aeabi_memset:
.thumb_func
__aeabi_memset4:
.thumb_func
__aeabi_memset8:
    mov r3, r1
    mov r1, r2
    mov r2, r3
.thumb_func
memset:
    movs r3, #0
5:  cmp r3, r2
    beq 6f
    strb r1, [r0, r3]
    adds r3, #1
    b 5b
6:  bx lr
