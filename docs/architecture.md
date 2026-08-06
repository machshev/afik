# Architecture

The current hardware-independent crates have this dependency direction:

```text
radio-domain ──→ radio-channel-plan ──→ radio-storage
      │
      └────────→ radio-tx-policy ─────→ radio-ui

radio-protocol ──→ radio-programmer ──→ radio-sim
radio-ui ─────────────────────────────→ radio-sim
```

`radio-domain` owns checked integer radio types and classifications.
`radio-channel-plan` owns bounded generated-bank definitions and expansion.
`radio-storage` owns stable object envelopes and candidate transactions.
`radio-tx-policy` is an independent authority boundary. `radio-protocol` owns
only bytes and wire semantics. `radio-programmer` compiles host projects and
speaks the protocol through a transport. `radio-sim` supplies the deterministic
device and in-memory transport.

No host crate is a dependency of an embedded crate. At the first milestone,
board, PAC, HAL, display, keypad, BK4819, firmware, Renode, CLI, and GUI crates
were intentionally deferred.

Work Package 3 adds `radio-firmware-dp32g030` as a standalone embedded target
leaf. It has no crate dependencies, is `no_std`, and is compiled only when its
explicit `firmware` feature and `thumbv6m-none-eabi` target are selected. This
keeps ordinary host workspace checks independent of target startup.

The minimum target image contains only a Cortex-M0 vector table, Reset handler,
and simulation observation word. Its linker script owns all allocated sections,
asserts the evidenced flash/RAM bounds, and rejects `.data` or `.bss` until a
later evidenced startup step deliberately implements their initialisation.

Work Package 4 keeps the canonical configuration-image codec in the embedded
`radio-storage` crate. It reads borrowed bytes, writes caller-provided buffers,
and allocates no heap. `radio-programmer` owns host-side canonical ordering,
image import/export, negotiated-capability checks, and allocated project data.
The image is independent of transport and target hardware; no physical flash
layout or durability semantics cross into the hardware-independent crate.

Work Package 5 adds `radio-ui` as another embedded, hardware-independent leaf
over `radio-domain` and `radio-tx-policy`. It owns only logical key edges,
bounded semantic display views, and boot-scoped draft permission state. It does
not own key scanning, display drawing, persistence I/O, or live transmit policy.
`radio-sim` provides the host-only virtual-time adapter and keeps persisted
permission bytes separate from the active policy loaded at the last boot.
