/*
 * Reserved stack for the K1 async application, asserted at link time.
 *
 * RISK-033: statics and task futures grow silently, and an overflow corrupts
 * the top of .bss rather than failing. `AFIK-K1-2.5` reached the operator
 * having exhausted the stack and never started, and `AFIK-K1-4.0` did the same
 * after a slot-budget change added 512 bytes of statics. A scripted floor
 * existed in `tool/verify-k1-async-image.sh` and both images cleared it,
 * because it was set at 4 KiB.
 *
 * This is the same bound in the linker, where it cannot be skipped by building
 * without the packaging script. It is a policy floor, not a measurement: 5,396
 * bytes demonstrably did not start, so the floor sits above it with margin.
 * `RISK-033` stays open until a painted-stack high-water reading on the exact
 * unit says what peak use actually is.
 */
_min_stack_size = 6144;

ASSERT(_stack_start - __sheap >= _min_stack_size,
       "K1 async image leaves less stack than the reserved minimum: reduce statics or task futures");
