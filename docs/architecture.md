# Architecture

## Embassy migration boundary

`K1ASYNC-023` introduces Embassy incrementally. The first accepted layer is a
heap-free static Cortex-M executor whose version and MSRV build in AFIK's pinned
environment. Time, UART, SPI, GPIO interrupts, and DMA remain separate adapters
until exact PY32F071 implementation and physical behavior are verified.
CPU-bound rendering must contain explicit await boundaries; cooperative
scheduling does not make an uninterrupted renderer non-blocking.

The first scheduling proof is deliberately hardware-independent: a deterministic
round-robin harness transfers a complete 1,024-byte frame in 16-byte chunks and
services serial work between adjacent chunks. The chunk size is compile-time
matched to the local SPI driver. This validates the await boundary only; it is
not evidence that the Cortex-M executor, USART interrupts/DMA, or SPI peripheral
run correctly on the K1.

The next compile-only layer composes the thread executor, USART1 with its two
bounded DMA channels, and transmit-only SPI1 into one K1-owned bundle. The
constructor receives every HAL peripheral token explicitly, making overlapping
ownership a type error. It remains absent from the firmware entry point and
does not own clocks, TIM15, display A0/CS, or keypad GPIO. A runnable async image
requires a separate bootloader-clock handoff contract.

The PY32 dependency boundary is repository-local. `vendor/py32-metapac` is a
pinned generated PAC and `vendor/py32-hal` is a bounded compatibility patch;
root Cargo patches select both without requiring upstream control. The optional
K1 inventory feature type-checks required peripheral names only. It is not used
by the firmware entry point and does not initialize clocks or peripherals.
The optional TIM15 time-driver feature extends that static boundary through the
HAL's interrupt-enabled Embassy driver. TIM15 is reserved as the candidate, but
the feature remains absent from the physical firmware path: RCC ownership,
clock handoff, and timer behavior require separate evidence before migration.

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
                         └─────────────────┴→ radio-programmer-web
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

The two target images are `radio-firmware-k5` and the K1 application in
`radio-firmware-k1`. Each binary is compiled only when its firmware feature and
the `thumbv6m-none-eabi` target are selected, which keeps ordinary host
workspace checks independent of target startup. The Work Package 3
`radio-firmware-dp32g030` reset image, the first K1 reset/serial image and the
K5 UART bisect image were removed on 2026-10-13 once the applications
superseded them.

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

APP-052 Milestone 2 adds an optional relative dual-slot model beside that
codec. It accepts the exact canonical image (bounded to 1,280 bytes), stores
two complete 1,312-byte slots (2,624 bytes total) over an eight-byte
`BlockBackend`, writes a full-generation candidate tag in the final block,
verifies every write by read-back, and selects the newest valid wrapping
generation. Slot and block numbers are relative to the adapter allocation;
there is deliberately no EEPROM address or target wiring here. Host fault
injection covers torn writes, stale atomic writes, corruption, and generation
wrap. This model is no_std and heap-free in production; its caller supplies a
slot buffer and image scratch, and the next task is simulator object transport.

Work Package 5 adds `radio-ui` as another embedded, hardware-independent leaf
over `radio-domain` and `radio-tx-policy`. It owns only logical key edges,
bounded semantic display views, and boot-scoped draft permission state. It does
not own key scanning, display drawing, persistence I/O, or live transmit policy.
`radio-sim` provides the host-only virtual-time adapter and keeps persisted
permission bytes separate from the active policy loaded at the last boot.

Work Package 6 adds `radio-bk4819` as an embedded, hardware-independent command
driver over a fallible logical register-bus trait. It owns exact frequency-word
packing, receive status decoding, state/fault latching, and the only modeled TX
mode transition. That transition requires a borrowed, complete-request-matching capability
from `radio-tx-policy`. It does not depend on a PAC, HAL, board crate, allocator,
or host crate. `radio-sim` supplies a host-only logical bus with explicit virtual
time and deterministic one-shot failures; it is not a silicon or RF model.

Work Package 7 adds `radio-channel-control` as an embedded leaf over the domain,
generated-plan, and TX-policy crates. It owns checked one-bank activation,
manual navigation, explicit dwell/hold state, opaque logical timer tokens, and
selected-state policy authorization. It owns neither a clock nor an RF driver.
`radio-sim` is the host composition root that applies activation outputs to the
logical BK4819 driver and schedules tokens on explicit virtual time.

`TXAUTH-053` makes the TX-policy leaf the sole complete-request authority.
Its bounded request owns exact settings, configuration/selection identity,
operator/regional inputs, and explicit hardware and per-unit calibration
envelopes. A token borrows the immutable request and policy; controller bundles
also borrow selection state. The logical driver compares the entire current
request before writing. Test-only synthetic envelopes live under
`radio-tx-policy/tests/support` and are excluded from firmware. No target main,
PTT lifecycle, PA control, or physical TX capability is added.

`TXCTRL-054` adds `radio-platform::tx_control` above that authority, depending
only on the hardware-independent TX-policy contract. It owns bounded PTT
press/release interlocks, exact-request entry sequencing, explicit-time timeout,
identified adapter acknowledgements, and PA/TX shutdown before receive restore.
`radio-sim::tx_control` supplies the logical adapter and fault injection; no
target runtime invokes it. Effects borrow the controller and current authority,
and faults latch without automatic retry. See [the PTT contract](tx-control.md).

Work Package 8 adds `radio-programmer-cli` as a host-only front-end leaf. It
depends on `radio-programmer` for compilation, image, protocol, transaction,
listing, backup, restore, and verification logic, and on `radio-sim` only to
offer the same commands against a deterministic backend. Its binary owns
argument parsing, files, rendering, process status, and explicit transport
selection; no programming logic moves out of the library.

Work Package 9 moves verified write, backup, and restore orchestration fully
into `radio-programmer` and moves the explicit Linux file/`stty` adapter into
the reusable host-only `radio-programmer-serial` crate. Both CLI and GUI are
thin leaves over those layers. `radio-programmer-web` owns one persistent
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
flashing library owns the legacy packet codec, read-only EEPROM backup, the
explicitly selected guarded EEPROM restore workflow, bootloader-v2 probe,
raw-image validation, recovery gates, and page sequence. Restore preflight
requires a complete expected-current image and a target/image confirmation;
the host sends only source-backed eight-byte `0x051D` writes after an immediate
compare and requires both per-block and complete-image read-back. It is not
an allocation decision or target persistence adapter.
Its CLI is a thin explicit-path adapter over the shared Linux serial port. This
protocol is the stock UV-K5 deployment protocol and does not enter
`radio-protocol`, which remains AFIK's object-level runtime configuration
protocol.

## Shared K1/K5 application platform

`PLAT-050` adds `radio-platform`, a `no_std`, heap-free application boundary
above both MCUs. Its bounded serial service owns legacy normal-mode hello frame
validation, stream resynchronisation, and identity response dispatch. K1 and K5
supply only a validated printable identity and feed bytes received by their own
UART adapters; K1's Embassy USART/DMA/idle-line behavior and K5's circular DMA
and UART registers remain target-specific.

The same crate owns the boot-display request vocabulary and ordering. The K1
adapter renders those requests into its framebuffer and existing display bus;
the K5 adapter retains the physically proven synchronous-GPIO implementation.
Neither the K5 SPI0 experiment nor any DMA, interrupt, pin, register, timing, or
framebuffer implementation is part of the shared contract.

`APP-052` extends this boundary with a receive application state machine. It
accepts target-neutral start, navigation, mute, volatile squelch selection,
target acknowledgements, receiver-sample, and fault events and emits a bounded
ordered list of tune, chip-audio, speaker-gate, and semantic-redraw effects.
PMR446 example selection and the squelch/audio rule therefore compile once;
timing, metric acquisition, BK4819/BK4829 selection, GPIO, keypad scanning,
and rendering remain target adapters. A squelch choice is staged until the
target acknowledges the exact threshold operation, so the shell, semantic
state, physical threshold, and K1 persistence signal cannot diverge on a
fault. Target replacement cancels the staged choice, and the adapter reports
applied, rejected/no-effect, or faulted outcomes explicitly. K5's setting is
volatile generation-zero behavior, not persistence.

The configuration-activation portion of this boundary is deliberately a value
boundary rather than a persistence abstraction. `radio-platform::configuration`
owns the exact bounded PMR446 raster and `ActivatedConfiguration`, carrying an
opaque active generation, selected channel, and audio preference. K1 adapts its
retained object index into that value while keeping EEPROM, object decoding,
operator-place restore, arbitrary channels, VFO, scanning, and serial control
local. K5 supplies generation zero from its receive fixture and has no invented
AFIK persistence behavior. Thus both targets execute one activation/receive
state machine without importing unlike storage or transport dependencies into
the K5 image.

Configuration changes use a separate bounded candidate/change/acknowledgement
contract in `radio-platform::configuration_change`. A change carries both the
candidate and a monotonically allocated application-local identity; the
opaque storage generation remains part of the candidate and is never treated as
the change identity. `ReceiveApp` keeps the old semantic snapshot until the
adapter acknowledges that exact change, ignores mismatched or stale
acknowledgements, and fails silent on a matching rejection. K1 creates these
changes only after its retained object path has selected a common receive
setup; K5 applies them to its immutable built-in receive fixture. Neither
target's serial protocol, EEPROM transport, object commit/read-back, or
volatile capability is represented in this shared contract.

While a candidate is pending, start/navigation/audio events cannot overwrite
the active snapshot, and a receiver fault invalidates the pending change before
any later acknowledgement can apply it. Change identities carry a bounded
application-instance token so queued acknowledgements cannot cross an app
replacement when callers provide a fresh token. Target adapters return an
explicit complete-effect result: translation, tune, chip-audio, read-back,
metric, and display failures reject the change, while failure cleanup still
closes the speaker and emits the semantic fault redraw. These are in-memory
software guarantees, not persistence or hardware evidence.

The serial-control extension is a smaller boundary above those target-owned
wire services. `radio-platform::serial_control` receives only a decoded query
or common command and returns a semantic view, an explicit capability set, or
the same bounded effects that a keypad event produces. Its `wire` submodule
defines a fixed two-byte request payload for view/capability queries and the
canonical logical-key vocabulary, plus bounded view/capability response
payloads. K1 and K5 adapters may put those payloads inside their existing
framing, but do not share a UART or framing implementation. Candidate commands
are intentionally absent from this payload mapping: they still emit an
identified configuration change and can become active only after the exact
adapter acknowledgement. K1 and K5 construct capability sets locally: K1
retains its object/runtime protocol, persistence, arbitrary channels, VFO,
scan, and raw metrics, while K5 declares only shared receive/view/key behavior
and volatile generation-zero activation. No UART, DMA, framing, EEPROM,
storage, or target controller type crosses this seam.

APP-052 Milestone 3 adds a separate simulator-only preparation seam in
`radio-protocol::k5_extended`. It wraps one existing bounded COBS/CRC frame
with explicit magic/version/session/command/length/CRC fields and rejects
malformed, oversized, trailing, wrong-session, and wrong-command input before
the object service. The envelope is generic host/future-firmware codec code;
it is not advertised or wired into either target's normal-mode loop and does
not alter `Hello` or the common `0x0600..` controls.

`radio-sim::K5ObjectTransport` explicitly selects that envelope and drives the
existing `DeviceService<1264>` and `radio-programmer` transaction/list/read
workflow. A simulated commit writes the exact canonical image to the relative
`radio-storage::dual_slot` model, and reboot restores the newest validated
generation. This proves protocol/service/storage composition only. It does
not provide a K5 EEPROM address, target persistence capability, hardware
power-loss result, physical normal-mode transport, or TX path. A failed
durable commit restores the complete bounded service snapshot and caches its
protocol error for replay; allocation evidence and target adapter sizing
remain separate gates.

The DP32G030 target linker and package tools own the V1 application-region
contract. They emit a full `0x0000..0xEFFF` raw image and never include the
stock `0xF000..0xFFFF` bootloader region. Hardware-independent radio crates do
not gain target, serial, or legacy-protocol dependencies. Acknowledged pages do
not establish target boot or peripheral support.
