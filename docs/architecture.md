# Architecture

## Embassy migration boundary

`K1ASYNC-023` introduces Embassy incrementally. The first accepted layer is a
heap-free static Cortex-M executor whose version and MSRV build in AFIK's pinned
environment. Time, UART, SPI, GPIO interrupts, and DMA remain separate adapters
until exact PY32F071 implementation and physical behavior are verified.
CPU-bound rendering must contain explicit await boundaries; cooperative
scheduling does not make an uninterrupted renderer non-blocking.

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
                         ├────────────→ radio-programmer-serial
                         │                 │
                         └─────────────────┴→ radio-programmer-cli
                         │                 │
                         └─────────────────┴→ radio-programmer-gui
radio-ui ─────────────────────────────→ radio-sim
radio-bk4819 ─────────────────────────→ radio-sim
radio-channel-control ────────────────→ radio-sim
radio-domain ─────────→ radio-aprs ───→ radio-sim
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

`K1BOOT-016` adds `radio-firmware-k1` as a separate standalone embedded target
leaf. It has no host crate dependencies and uses only the pinned K1/PY32F071
application origin (`0x08002800`), 118 KiB application bound, Cortex-M0+
Thumb-compatible target, and 16 KiB SRAM facts. Its Reset handler writes one
development-only RAM witness, configures the evidenced USART1 serial path, and
answers one bounded normal-mode hello with the AFIK application identity. The
image has no USB, display, keypad, radio, TX, EEPROM, or reset behavior. The
K1 build script supplies its own linker script and small ELF page size so
program segments cannot masquerade as bytes below the application origin. The
raw package is a bounded application payload, not a physical flashing
permission.

`K1DISP-019` keeps the K1 target as the board composition leaf. Its display
command generation and fixed-screen rendering remain hardware-independent,
`no_std`, heap-free, and exact-trace tested inside `radio-firmware-k1`; only the
target binary may bind those outputs to sourced PY32F071 GPIO/SPI1 registers.
The existing serial hello remains live as an independent observation. This
slice adds no keypad, storage, BK4819, RF/TX, audio, backlight, USB, interrupt,
or DMA behavior.

`K1KEY-022` adds a hardware-independent 4-by-4 main-matrix decoder, explicit-
time debounce state, fixed key labels, and deterministic scan sequencing in
the standalone K1 crate. The target leaf alone binds it to PB12..PB15 inputs
and PB3..PB6 outputs and composes debounced presses with the already verified
display. PTT, side keys, menus, persistence, radio control, and TX authority
remain outside the composition.

The `K1KEY-022` Renode harness is a separate execution diagnostic, not a PY32
peripheral model. It uses test-only register storage and synthetic row input to
prove the built ELF can return from initial display setup and carry one MENU
cell through scan/debounce to the render function. Physical GPIO timing,
electrical behavior, and LCD output remain hardware observations.

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

Work Package 9 moves verified write, backup, and restore orchestration fully
into `radio-programmer` and moves the explicit Linux file/`stty` adapter into
the reusable host-only `radio-programmer-serial` crate. Both CLI and GUI are
thin leaves over those layers. `radio-programmer-gui` owns one persistent
selected session, strict generated-bank form parsing, a bounded loopback HTTP
boundary, embedded browser assets, downloads/uploads, and presentation. It
does not introduce a remote service, raw object writes, arbitrary server file
paths, target behavior, or a physical-programming claim.

Work Package 10 adds no crate or behavior. Its Frequency Copy feasibility
design keeps a measurement candidate separate from `ActiveChannel`, storage,
and `TxAuthorisation`; observation cannot infer a transmit frequency or trusted
class. Any future save must be deliberate and receive-only with
`TxClass::Never`. Register commands and a physical adapter remain blocked by
revision, board-path, crystal, false-lock, timing, and cleanup evidence gates.

Work Package 11 adds `radio-aprs` as an embedded, hardware-independent leaf
over `radio-domain`. It accepts only complete de-stuffed AX.25 UI frames with
FCS, parses the supported APRS Object/Item voice-repeater fields, and owns a
fixed-capacity explicit-time discovery table. Advertised frequencies use the
domain's checked integer `Frequency`, but no advertisement can become an
`ActiveChannel`, trusted tone, plan class, or TX capability. `radio-sim`
supplies complete frames and virtual receive times without composing discovery
with channel or RF control. Physical RF/audio demodulation, NRZI/HDLC recovery,
target peripherals, persistence, and automatic tuning remain outside the
architecture until their evidence gates are satisfied.

Work Package 12 adds a separate host-only K5 deployment boundary. A new
flashing library owns the legacy packet codec, read-only EEPROM backup,
bootloader-v2 probe, raw-image validation, recovery gates, and page sequence.
Its CLI is a thin explicit-path adapter over the shared Linux serial port. This
protocol is the stock UV-K5 deployment protocol and does not enter
`radio-protocol`, which remains AFIK's object-level runtime configuration
protocol.

The DP32G030 target linker and package tools own the V1 application-region
contract. They emit a full `0x0000..0xEFFF` raw image and never include the
stock `0xF000..0xFFFF` bootloader region. Hardware-independent radio crates do
not gain target, serial, or legacy-protocol dependencies. Acknowledged pages do
not establish target boot or peripheral support.
