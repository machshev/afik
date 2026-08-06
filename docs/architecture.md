# Architecture

The current hardware-independent crates have this dependency direction:

```text
radio-domain ──→ radio-channel-plan ──→ radio-storage
      │                   │
      ├────────→ radio-tx-policy ─────→ radio-ui
      │                   │
      └───────────────────┴───────────→ radio-channel-control
      │                   │
      └────────→ radio-bk4819

radio-protocol ──→ radio-programmer ──→ radio-sim
                         │                 │
                         └─────────────────┴→ radio-programmer-cli
radio-ui ─────────────────────────────→ radio-sim
radio-bk4819 ─────────────────────────→ radio-sim
radio-channel-control ────────────────→ radio-sim
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

Work Package 6 adds `radio-bk4819` as an embedded, hardware-independent command
driver over a fallible logical register-bus trait. It owns exact frequency-word
packing, receive status decoding, state/fault latching, and the only modeled TX
mode transition. That transition requires a borrowed, class-matching capability
from `radio-tx-policy`. It does not depend on a PAC, HAL, board crate, allocator,
or host crate. `radio-sim` supplies a host-only logical bus with explicit virtual
time and deterministic one-shot failures; it is not a silicon or RF model.

Work Package 7 adds `radio-channel-control` as an embedded leaf over the domain,
generated-plan, and TX-policy crates. It owns checked one-bank activation,
manual navigation, explicit dwell/hold state, opaque logical timer tokens, and
selected-state policy authorization. It owns neither a clock nor an RF driver.
`radio-sim` is the host composition root that applies activation outputs to the
logical BK4819 driver and schedules tokens on explicit virtual time.

Work Package 8 adds `radio-programmer-cli` as a host-only front-end leaf. It
depends on `radio-programmer` for compilation, image, protocol, transaction,
listing, backup, restore, and verification logic, and on `radio-sim` only to
offer the same commands against a deterministic backend. Its binary owns
argument parsing, files, rendering, process status, and explicit transport
selection; no programming logic moves out of the library.
