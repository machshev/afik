# Architecture decisions

Append-only decision log. Supersede earlier entries rather than editing their
meaning.

## ADR-001 — Separate host and embedded responsibilities

- **Date:** 2026-08-05
- **Status:** accepted
- Embedded model, plan, storage, TX-policy, and protocol crates are `no_std`
  and heap-free. `radio-programmer` and `radio-sim` are host crates and may use
  allocation.
- This keeps device representations bounded while allowing richer offline
  project data and test tooling.

## ADR-002 — Fixed protocol envelope with COBS stream framing

- **Date:** 2026-08-05
- **Status:** accepted for milestone, wire format provisional
- Frames use a fixed header, bounded payload, CRC-16/CCITT-FALSE, COBS encoding,
  and a zero delimiter. COBS provides deterministic resynchronisation without
  requiring the transport to preserve packet boundaries.

## ADR-003 — Object-level candidate transactions

- **Date:** 2026-08-05
- **Status:** accepted
- Writes are staged in a candidate snapshot, validated as a whole, and made
  active only by commit. Protocol commands never expose arbitrary storage
  addresses.

## ADR-004 — Simulated transport implements the programmer contract

- **Date:** 2026-08-05
- **Status:** accepted
- The transport contract is byte-oriented `send`/`receive`. The in-memory
  transport drives the same encoded frames expected from later UART and replay
  transports.

## ADR-005 — Locked Nix flake is the primary development environment

- **Date:** 2026-08-05
- **Status:** accepted
- `flake.nix` and `flake.lock` provide the Rust host tools on supported NixOS
  systems; `.envrc` loads that shell through direnv. CI separately runs the
  declared Rust 1.86 minimum so the locked Nix package set may safely carry a
  newer compiler.

## ADR-006 — Negotiated maxima do not have to equal host maxima

- **Date:** 2026-08-05
- **Status:** accepted
- A host with smaller fixed buffers may operate within a device's larger
  advertised maximum. The programmer compiler enforces both negotiated target
  limits and its own local bounds instead of rejecting a device merely because
  that device can accept larger frames or objects.

## ADR-007 — Object listing is bounded, paged, and generation-tagged

- **Date:** 2026-08-05
- **Status:** accepted for Work Package 2, wire format provisional
- `LIST_OBJECTS` pages use a zero-based object offset and include active
  generation, total count, echoed offset, and fixed-size object descriptors.
- Descriptors are strictly ordered by stable `(kind, ID)` key. The programmer
  rejects inconsistent generations, totals, offsets, bounds, or ordering rather
  than combining an ambiguous listing.
- Paging decouples complete listings from both the fixed frame payload and a
  device's negotiated object capacity.

## ADR-008 — Immediate duplicate requests replay one cached response

- **Date:** 2026-08-05
- **Status:** accepted for Work Package 2, wire format provisional
- The device caches exactly its most recent decoded request and response. An
  immediate byte-identical retry replays the response without re-executing the
  command.
- Reusing that sequence for different request bytes returns `SequenceConflict`
  without mutation or cache replacement. Older sequences outside the one-entry
  window are ordinary new requests.
- This gives synchronous transports safe response-loss retries with fixed
  memory and no unbounded replay history.

## ADR-009 — The first target image has an explicit minimum reset contract

- **Date:** 2026-08-05
- **Status:** accepted for `DP32-003`
- The image defines exactly the initial stack pointer and Reset vector required
  by Cortex-M0, then writes one simulation-only atomic sentinel and spins.
- The linker owns flash/RAM placement and asserts the vector size, sentinel
  position, memory bounds, and absence of `.data`/`.bss` that would require
  unimplemented runtime initialisation.
- Rust linking attributes are the only locally allowed unsafe language surface;
  there are no unsafe operations or raw hardware accesses.

## ADR-010 — Target builds are explicit and use pinned core sources

- **Date:** 2026-08-05
- **Status:** accepted for `DP32-003`
- The target binary requires an explicit Cargo feature so default host
  workspace formatting, linting, and tests do not attempt to link firmware.
- In the Nix shell, `tool/build-dp32g030.sh` builds `core` from the locked
  Nixpkgs Rust sources and uses the pinned unwrapped LLD. This avoids rustup
  state, unpinned downloads, and host-linker wrapper flags.
- The verifier rejects non-Arm, non-little-endian, non-Armv6-M, dynamic, or
  out-of-range ELF output before the image can be used by simulation.

## ADR-011 — The first Renode platform models only the reset prerequisites

- **Date:** 2026-08-05
- **Status:** accepted for `DP32-003`
- The platform contains the evidenced flash and RAM ranges plus Renode's
  Cortex-M0/NVIC core plumbing. It contains no DP32G030 or board peripheral
  behaviour and assigns no guessed peripheral reset values.
- The boot test observes RAM before and after starting the loaded ELF. It does
  not set PC, SP, or vector-table offset, so the vector/linker contract remains
  part of the behaviour under test.
- A passing model proves software behaviour against declared assumptions, not
  the physical UV-K5 reset map or bootloader packaging.

## ADR-012 — Configuration images are canonical logical containers

- **Date:** 2026-08-06
- **Status:** accepted for `STORE-004`
- `radio-storage` owns a versioned, CRC-32-protected image containing a complete
  logical object set in strict `(kind, ID)` order. Its codec is `no_std`, uses
  borrowed input and caller-provided output, and validates the complete image
  before exposing objects.
- `radio-programmer` canonicalises compiled objects and revalidates all
  negotiated object, frame, storage-version, and plan-encoding limits when
  importing an image. Image import remains an offline compiler operation.
- The image format is not a DP32G030 flash layout, transactional journal,
  project-file syntax, or proof of power-loss durability. Those concerns
  require separate evidence and decisions.

## ADR-013 — TX-permission editing is boot-only and activates on reboot

- **Date:** 2026-08-06
- **Status:** accepted for `UI-005`
- The hardware-independent UI enters its hidden editor only when the initial
  logical key set is exactly `Menu+Back`, then waits for all held keys to be
  released. Incomplete, additional, or post-boot keys cannot enter it. The
  logical gesture is a deliberate-presence workflow, not authentication or a
  claim about physical key wiring.
- Editing changes only a bounded draft. Cancel emits no record; save increments
  the generation and emits the existing redundant CRC-protected permission
  record, or refuses at generation exhaustion. `Never` is not selectable.
- Saving never mutates the active policy. The simulator and future target
  adapters must keep persistence separate from the `TxPolicy` loaded at boot;
  saved permissions take effect only after subsequent validated loading.

## ADR-014 — BK4819 commands are evidence-bounded and fail closed

- **Date:** 2026-08-06
- **Status:** accepted for `RF-006`, physical integration prohibited
- `radio-bk4819` is a `no_std`, heap-free command layer over a fallible logical
  7-bit-address/16-bit-value bus. It assumes a separately initialized chip and
  encodes only the frequency, mode, RSSI, and squelch fields recorded in
  `docs/hardware-evidence.md`.
- Receive and transmit plans first write neutral mode, then the low and high
  10-Hz frequency words, and write the inferred final mode last. Any failed bus
  operation latches an unknown physical state; only a subsequent successful
  neutral-mode write recovers it.
- `TxAuthorisation` carries its approved class. The driver's sole transition to
  the inferred TX mode requires a borrowed token whose class matches the active
  channel before any bus operation. This proves a software authority boundary,
  not safe physical transmission or correct silicon/board behavior.

## ADR-015 — Scanning is an explicit-input state machine

- **Date:** 2026-08-06
- **Status:** accepted for `SCAN-007`
- `radio-channel-control` owns no clock, scheduler, RF driver, or target adapter.
  It changes only through manual selection, start/stop, normalized signal, and
  opaque timer-expiry inputs. Non-zero dwell and hold milliseconds are explicit
  workflow policy rather than hardware facts.
- Every timer arm has a fresh bounded token. Replaced, cancelled, early, and
  stale expiry behavior is explicit, so a delayed adapter event cannot silently
  advance the current scan state.
- Scanning cannot request TX authority. Selected state pairs the exact channel
  with a capability minted by `TxPolicy`; the BK4819 boundary independently
  checks that class. Host simulation composes these layers and never substitutes
  simulated success for physical timing or RF evidence.

## ADR-016 — The CLI is a thin, capability-negotiated front end

- **Date:** 2026-08-06
- **Status:** accepted for `CLI-008`
- `radio-programmer-cli` owns strict argument parsing, bounded file I/O, stable
  text rendering, exit codes, and transport selection. Compilation, canonical
  images, object transactions, backup snapshots, restore, and read-back
  verification remain library operations in `radio-programmer`.
- Every operational command explicitly selects a fresh simulator or a Linux
  serial device path plus supported baud. The serial adapter uses `stty` and a
  file byte stream without unsafe code; it is not target-UART evidence and has
  no implicit physical default.
- Compile and backup refuse to replace an existing output unless `--force` is
  present. Restore validates a bounded canonical image before transaction
  mutation. No command exposes raw memory, raw object writes, or firmware
  flashing.

## ADR-017 — The programmer GUI is one bounded loopback session

- **Date:** 2026-08-06
- **Status:** accepted for `GUI-009`, not a remote-service security model
- `radio-programmer-gui` retains exactly one explicitly selected simulator or
  serial backend. Compilation, canonical images, transactions, backup, restore,
  and read-back verification remain in `radio-programmer`; Linux serial setup
  remains in the shared `radio-programmer-serial` adapter.
- Its dependency-free HTTP listener accepts only an explicit loopback IP,
  bounds headers and bodies, rejects ambiguous framing, embeds all assets, and
  exposes uploaded/downloaded bytes rather than arbitrary server paths.
- Configuration mutation requires a random per-process token plus an explicit
  replacement-confirmation header. Same-origin delivery, CSP, and these checks
  reduce accidental local mutation but are not authentication or authorization.
  Non-loopback, multi-user, or deployed use requires a separate threat model.

## ADR-018 — Frequency Copy yields only a reviewed receive observation

- **Date:** 2026-08-06
- **Status:** accepted for `FREQ-010`; production hardware command deferred
- Fast Copy is local measurement of one received transmission, not the
  separately documented transmitting Air Copy configuration protocol. Its only
  candidate outputs are an observed receive frequency, optional signalling
  evidence, and bounded quality metadata.
- A capture candidate is not an `ActiveChannel` and cannot contain or infer a
  transmit frequency, trusted `TxClass`, or `TxAuthorisation`. Decoder timeout
  is recorded as signalling not observed, not silently converted to a trusted
  no-tone setting. Saving is a separate confirmed transaction and any future
  saved receive-only object must remain `TxClass::Never`.
- Beken confirms scan capability existence, but the accessible register note is
  machine-translated and revision-unverified, while descendant firmware uses
  explicitly unexplained bits and non-independent implementation choices.
  Production commands, register simulation, target binding, and physical claims
  remain blocked until the documented experiments establish every bit, unit,
  transition, false-lock case, timeout, fault, and cleanup path.

## ADR-019 — APRS discovery begins at a complete validated frame

- **Date:** 2026-08-06
- **Status:** accepted for `APRS-011`; physical receive chain deferred
- `radio-aprs` accepts one bounded, de-stuffed, octet-aligned AX.25 UI frame
  including FCS. It validates addresses, zero through eight APRS path entries,
  UI control, PID, information length, and FCS before parsing supported
  uncompressed APRS Object/Item voice-repeater advertisements. RF/audio
  demodulation, clock/NRZI/HDLC recovery, target peripherals, and BK4819 modem
  commands remain separate blocked lower layers.
- Discovery keeps advertised output/alternate-input frequency, CTCSS/DCS text,
  offset, range, position ambiguity, source, and receive time as untrusted
  receive data. It cannot create `ActiveChannel`, trusted `Tone`, plan
  membership, `TxClass`, or `TxAuthorisation`, and it has no automatic tune,
  save, or transmit path.
- The fixed-capacity key is report kind, case-sensitive name, and source/SSID.
  Newer same-key data wins, equal-time differences conflict, older inputs are
  stale, and no full-table eviction occurs. Same-origin kills retain bounded
  freshness until explicit expiry, preventing an out-of-order older live report
  from resurrecting a removed entry. This is a conservative local safety rule,
  not APRS authentication or network ownership semantics.

## ADR-020 — K5 deployment preserves the stock bootloader and requires recovery

- **Date:** 2026-08-06
- **Status:** accepted for `FLASH-012`; physical completion pending
- AFIK supports only an operator-confirmed UV-K5 V1 fitted with DP32G030 and an
  exact version-2 bootloader beacon. A beacon does not identify the board. V2,
  V3, bootloader v5, clones, and unknown variants fail closed.
- The target owns only `0x0000..=0xEFFF`. Packaging emits exactly 60 KiB and
  fills unused application bytes with `0xFF`; neither linker nor flasher can
  address the stock bootloader at `0xF000..=0xFFFF`. The flasher exposes no
  address, length, partial-write, EEPROM-write, or wildcard-version option.
- The host library, not its CLI, owns legacy framing, full read-only EEPROM
  backup, image/recovery validation, sequential page writes, and exact response
  checking. The legacy bootloader protocol remains separate from AFIK's
  object-level runtime configuration protocol.
- A destructive run requires the qualified-target phrase, exact application
  image CRC-32, a validated 8 KiB EEPROM backup, and a vector-valid known-good
  raw recovery image. CRC-32 is an accidental-selection guard, not a security
  signature. Files use create-new behavior unless force is explicit.
- Page acknowledgement proves only bootloader acceptance. No retry follows an
  ambiguous write, and no success claim follows without physical recovery and
  independent boot observations on the exact unit.

## ADR-021 — K1 evidence may be trusted without importing its implementation

- **Date:** 2026-08-06
- **Status:** accepted for `K1EVID-013`
- The exact Armel firmware demonstrated on the available UV-K1 is accepted as
  trusted, hardware-tested evidence. Its direct manufacturer support and
  sponsorship materially raise confidence in its supported target and board
  observations.
- AFIK will pin exact source revisions and cite exact locations for facts, then
  independently implement those facts in Rust. It will not copy, link, port,
  or incrementally translate existing application or driver code.
- The user-selected latest Armel development line resolves to the repository's
  default `main` branch (there is no `master` branch), pinned on 2026-08-06 as
  commit `fe9c4e9432694b50aea651084a043aae0b58673d`.
- Puya documentation remains the primary source for the PY32F071 architectural
  and peripheral contract. Armel evidence binds those capabilities to the
  exact radio board; physical observations on the available unit validate the
  binding and recovery path.
- `FLASH-012` is deferred, not completed or superseded. Its K5-specific image,
  bootloader, backup, and recovery assumptions do not apply to K1.

## ADR-022 — K1 evidence package accepts two verified local copies for now

- **Date:** 2026-08-06
- **Status:** accepted for `K1EVID-013`
- The exact unit's complete read-only backup and the pinned v5.5.0 recovery
  candidate each have two verified local copies under ignored `.private/k1/`.
- For the current evidence package, those two local copies are sufficient; an
  independent-storage requirement is deliberately deferred at the user's
  request. The shared-filesystem durability risk remains documented.
- This decision does not prove physical recovery and does not authorize an
  AFIK image write, a firmware restore, or RF transmission. Exact-unit physical
  markings, USB identities, and a non-destructive recovery procedure remain
  required before those actions.
- `K1HIL-015` subsequently authorized only the unchanged, independently
  validated stock recovery image; its 375 page acknowledgements and matching
  post-flash backup are now recorded in `EVID-K1-023`. This does not authorize
  a K1 AFIK application image or RF operation.

## ADR-023 — K1/K5 automatic detection is protocol classification only

- **Date:** 2026-08-06
- **Status:** accepted for `K1FLASH-014`
- The generic flasher may classify a live bootloader beacon as the qualified
  K5 V1 protocol only for validated `2.*` beacons, or as the qualified K1
  protocol only for the pinned `7.03.*` beacon shape. Unknown and unsupported
  versions fail closed.
- USB metadata narrows candidate serial paths but never proves the physical
  board, MCU, or RF revision. Automatic mode must fail on zero or multiple
  viable candidates and must report protocol classification separately from
  hardware identity.
- K1 support includes the independently implemented raw recovery/backup path
  and a separate AFIK application command. The K1 command must never route
  through the K5 image path and remains subject to its own target, recovery,
  image, and version confirmations.

## ADR-024 — K1 device trailers are not the K5 response marker

- **Date:** 2026-08-06
- **Status:** accepted for `K1HIL-015`
- The captured K1 `0x0518` device-info frame has the expected bounded legacy
  envelope, footer, command, declared length, UID/version payload, and a
  decoded trailer of `0x6ed1`. The K5-oriented AFIK decoder expected the
  decoded device trailer to be `0xffff`, so it rejected the live K1 beacon.
- The K1 path may therefore skip only the decoded trailer-value check while
  retaining envelope/footer, payload length, command, version, transaction,
  page, and result validation. This is an evidence-bounded interoperability
  rule, not a claim that the K1 serial link has integrity protection.
- K1 AFIK application flashing is exposed only by a separately guarded command;
  the recovery command remains restricted to the independently validated,
  unchanged stock image. Page acknowledgement is not read-back or boot proof.

## ADR-025 — The first K1 AFIK image is a reset-and-serial witness image

- **Date:** 2026-08-06
- **Status:** accepted for `K1BOOT-016`
- The first independently implemented K1 image uses only the pinned
  Cortex-M0+, application-origin, SRAM, and source-backed USART1 facts. It
  places a two-word vector table at `0x08002800`, writes one RAM witness from
  Reset, and answers one bounded normal-mode hello.
- The image does not initialise a guessed clock, access board peripherals, send
  USB data, draw the display, scan keys, control the BK4819, access external
  flash, enable TX, or issue a reset or bootloader command.
- The RAM witness is a development observation. The exact physical witness is
  the independent `AFIK-K1-0.1` response over the external CH340/UART path.

## ADR-026 — K1 physical boot witness requires exact board observation

- **Date:** 2026-08-06
- **Status:** accepted for `K1WIT-017`
- Puya's PY32F071 documentation establishes that the MCU family contains a
  USB 2.0 full-speed peripheral, but the exact K1 does not expose native USB
  for this workflow. The observed `1a86:7523` CH340 is the intended external
  USB-to-UART adapter on the bootloader and normal-mode serial path.
- The pinned exact-board source records USART1 on PA9/PA10 AF1 at 38,400 baud
  and a bootloader-provided 48 MHz clock. AFIK may use those facts in an
  independent bounded serial witness; it must not copy or link the source
  driver.
- A CH340 enumeration alone is not an application witness. The AFIK host must
  receive the exact `AFIK-K1-0.1` normal-mode hello response over the same
  serial path after the bounded witness-image write; only then is application
  boot considered proven.
- This gate passed on 2026-08-06 after 172 acknowledged pages and a user
  power-cycle. The result proves only the bounded serial application slice;
  future radio surfaces require their own evidence and recovery gate.

## ADR-027 — K1 AFIK application writes require a recovery rehearsal

- **Date:** 2026-08-06
- **Status:** accepted for `K1WIT-017`
- The generic flasher exposes `flash-afik-k1` separately from recovery. It
  requires the exact AFIK target phrase, exact detected `7.03.01` version,
  image CRC-32, a complete validated EEPROM backup, a distinct retained stock
  recovery image, and the exact recovery-rehearsal phrase.
- The K1 writer validates all guards before consuming the post-detection
  handshake or sending a page. It sends only the AFIK image; the recovery
  image is a guard and is never written by this command.
- Every page acknowledgement is checked for transaction, page, and zero
  result. There is no K1 flash read-back or automatic reset; the operator must
  power-cycle and run `probe-normal`, then retain the known-good recovery path.

## ADR-028 — The next K1 application slice is display-only

- **Date:** 2026-08-06
- **Status:** accepted for `K1APP-018` and `K1DISP-019`
- The first step beyond the serial witness is one fixed AFIK identity screen on
  the 128-by-64 ST7565-compatible display path observed in the pinned board
  source. It retains the existing USART1 hello as an independent boot and
  rollback observation.
- Display command generation and fixed-screen rendering are hardware-independent,
  `no_std`, heap-free, bounded, and tested by exact traces. PY32F071 RCC, GPIO,
  and SPI1 MMIO remain in the K1 target leaf.
- This choice does not authorize keypad/PTT scanning, backlight or audio,
  storage, BK4819 access, RF receive, TX, USB, or a general application. A
  physical image write remains separately confirmation-gated after static
  verification and must be followed by both visible-screen and serial probes.

## ADR-029 — First K1 illumination is constant GPIO, not PWM

- **Date:** 2026-08-06
- **Status:** accepted for `K1BL-020`
- Bright-light observation proved the fixed LCD pixels while the separately
  mapped backlight remained off. The next slice configures PF8 as the pinned
  active-high push-pull output and holds it high for the boot witness.
- AFIK does not reproduce the existing firmware's TIM7/DMA PWM, fades,
  brightness settings, or persistence. Those are larger behaviors requiring
  their own timing, power, UI, and durability contracts.
- The constant output adds illumination only. It cannot access keypad/PTT,
  audio, storage, BK4819, RF/TX, USB, interrupts, or EEPROM.

## ADR-030 — First K1 contrast correction is one fixed evidenced byte

- **Date:** 2026-08-06
- **Status:** accepted for `K1CON-021`
- The physically visible but faint AFIK words use electronic volume 21. The
  pinned board startup uses 31, so the next calibration changes only `0x15` to
  `0x1f` in the exact init trace.
- This does not create a contrast menu, keypad dependency, EEPROM setting,
  automatic calibration, or final UI policy. Those require later bounded tasks.

## ADR-031 — First K1 keypad slice is a fail-closed 4-by-4 display witness

- **Date:** 2026-08-06
- **Status:** accepted for `K1KEY-022`
- The first keypad binding covers only the 16-key PB12..PB15 by PB3..PB6 main
  matrix. PTT PB10 and the side-key special case are not matrix inputs and are
  excluded even though the pinned evidence handles them near the keypad code.
- Rows are pull-up inputs and columns are push-pull outputs held high at idle.
  One selected column may be driven low at a time, after which all columns must
  return high. Only one stable low row in one selected column can become a key;
  absent, ambiguous, changing, invalid, or failed samples produce no action.
- Debounce is an AFIK hardware-independent explicit-time state machine, not a
  copied polling loop or assumed target tick rate. Its only application effect
  is replacing a fixed display label after a debounced press. It cannot invoke
  general menus, persistence, radio control, or TX authority.

## ADR-032 — K1 keypad diagnosis uses a simulation-only execution hook

- **Date:** 2026-08-06
- **Status:** accepted for `K1KEY-022`
- A K1 Renode harness may supply bounded register storage and one synthetic
  active-low main-key cell, then hook the existing ELF symbol
  `render_key_witness`. Reaching that symbol proves the compiled scan/debounce
  control flow accepted the injected cell; it does not prove electrical levels,
  timing, controller behavior, or visible pixels.
- The harness must use explicit test-only conventions rather than presenting
  them as PY32 registers. No production firmware diagnostic sentinel, copied
  peripheral implementation, RF/TX behavior, or physical-success claim is
  added.

## ADR-033 — K1 physical keypad diagnosis reports raw matrix samples

- **Date:** 2026-08-06
- **Status:** accepted for `K1KEY-022`
- The retained normal-mode serial responder may service one read-only request
  by performing one existing main-matrix scan and returning the four raw row
  masks plus scan validity. The response must be exact-length, CRC-protected,
  bounded to four bits per mask, and rejected on malformed status or reserved
  fields.
- This diagnostic does not interpret keys, change the display, persist state,
  include PTT or side keys, or create any RF/TX path. Its sole purpose is to
  separate physical GPIO sampling from the already simulated render path.
- When held-key serial response is unavailable, the target may retain only the
  latest nonzero four-mask sample in volatile bounded RAM. The next successful
  read-only probe identifies, returns, and clears that capture; no timer,
  interrupt, persistence, or semantic key action is introduced.
- If decoded rows remain zero, the diagnostic may report the exact low 16 bits
  of GPIOB IDR for each selected column. A released baseline excludes only the
  known scanner-owned PB3..PB6 changes; any other changed bits are observations,
  not authorization to adopt an alternate mapping.

## ADR-034 — K1 async migration is incremental and evidence-gated

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Embassy is the intended concurrency direction. Cooperative tasks do not
  preempt CPU-bound work, so rendering must still be chunked or explicitly
  yield; async syntax alone is not an acceptance criterion.
- Prefer the safe Cortex-M thread executor first. Interrupt executors, time,
  UART, SPI, and DMA require separate PY32F071 evidence and tests. The current
  raw-MMIO image and guarded recovery path remain available until each migrated
  boundary is independently proven.
- AFIK retains `no_std`, heap-free static tasks, integer units, bounded queues,
  fail-closed TX policy, and no invented register behavior.

## ADR-035 — PY32 support is pinned and patched locally

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- AFIK vendors the exact generated `py32-metapac` and `py32-hal` sources needed
  for reproducible F071 work. Cargo patches resolve them locally; AFIK does not
  require an upstream fork, release, pull request, or network fetch to build the
  F071 inventory.
- The generated PAC provenance and HAL delta are recorded in `vendor/README.md`.
  F071 is an explicit chip/series selection; AFIK must not substitute an F072
  chip feature. Shared die data is accepted only where the pinned data source
  explicitly assigns it to the F071 series.
- A compiling peripheral name proves only the generated software inventory.
  Driver behavior, clocks, interrupts, time, UART, SPI, DMA, and physical
  operation remain separately evidenced boundaries. Unsupported ADC bindings
  stay disabled rather than inheriting unevidenced constants merely to compile.

## ADR-036 — TIM15 is the bounded K1 Embassy time candidate

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Reserve TIM15 for the prospective Embassy timebase because the pinned F071
  inventory supplies its RCC control, `PCLK1_TIM`, dedicated interrupt, and two
  compare channels, while the vendored driver needs CC1 for rollover accounting
  and CC2 for one alarm.
- Compile the complete interrupt-enabled driver behind a K1-only optional
  feature, but do not select it from the firmware image or call HAL init. The
  existing raw-MMIO startup continues to inherit the observed bootloader clock.
- Runtime adoption requires an explicit, evidenced clock handoff and separate
  timing/interrupt verification. A successful target compile is not a claim
  that the physical timer runs or that the HAL may reconfigure the board clock.

## ADR-037 — K1 USART1 migration begins as a no-entry-point async proof

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The prospective async serial path is exactly USART1 at 38,400 baud on PA9 TX
  AF1 and PA10 RX AF1. It uses the generated dedicated USART1 interrupt and two
  bounded DMA1 channels; it does not broaden the normal-mode protocol.
- Compile the real HAL constructor behind a K1-only optional feature, but do not
  select it from the firmware image or call it from startup. The current
  polling serial witness remains the physical recovery and responsiveness
  observation until async clock, interrupt, and DMA behavior pass separately
  guarded target tests.
- A successful compile proves API and generated-metadata compatibility only. It
  does not prove that HAL clock ownership preserves the bootloader-provided
  clock or that USART1 remains responsive while display work yields.

## ADR-038 — First K1 async SPI surface is transmit-only and cooperative

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Extend the pinned local HAL only with the display's evidenced SPI1, PA5 SCK
  AF0, PA7 MOSI AF0, mode-3, MSB-first, divide-by-64 surface. MISO, hardware
  NSS, other pins/instances, DMA, and receive behavior remain excluded.
- The first async write may poll each short hardware byte transfer, but it must
  yield after a fixed bounded byte chunk so executor tasks can progress during
  a display frame. Async syntax without those yield points is insufficient.
- Compile this interface without selecting it from the physical image. Runtime
  scheduler progress, UART responsiveness, and visible display behavior remain
  later Renode and separately guarded physical gates.

## ADR-039 — Display scheduling is proven independently of hardware adoption

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- One complete visible frame is 1,024 bytes and the cooperative display boundary
  yields after every 16 bytes. A deterministic round-robin future harness must
  demonstrate serial-service progress between adjacent display chunks.
- The hardware-independent schedule and local HAL driver use compile-time-equal
  chunk constants. This host proof establishes only that the await placement
  permits interleaving; it does not prove Cortex-M executor startup, interrupt
  delivery, DMA, peripheral timing, or physical UART/display coexistence.

## ADR-040 — First async runtime composition is compile-only ownership

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Compose the thread executor, USART1/PA9/PA10 with two bounded DMA channels,
  and SPI1/PA5/PA7 into one optional K1-owned bundle. Construction requires
  explicit caller-supplied HAL tokens so ownership conflicts are type-checked.
- Do not call HAL initialization, adopt a clock tree, bind TIM15, create a
  runnable entry point, or include A0/CS and keypad GPIO in this step. Static
  tasks and physical migration follow only after clock, interrupt, and DMA
  behavior have their own guarded contract.

## ADR-041 — K1 inherited clocks must be observed before HAL adoption

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The pinned application source assigns `SystemCoreClock = 48_000_000` but does
  not establish the bootloader's oscillator, PLL, or prescaler register state.
  AFIK therefore treats 48 MHz as a required handoff contract, not sufficient
  evidence to publish HAL clocks.
- A read-only snapshot must show enabled/ready 24 MHz HSI, the fixed x2 PLL
  sourced from HSI, PLL selected and active as SYSCLK, and undivided AHB/APB.
  Every mismatch denies adoption. Reading and validating these fields neither
  changes RCC nor initializes the HAL.
- Publishing the validated frequencies, taking peripheral tokens, enabling
  interrupts/DMA/TIM15, and changing the runnable image require a later guarded
  step after the exact-unit snapshot is observed.

## ADR-042 — Exact-unit clock observation reuses the bounded serial witness

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Add one fixed-session, read-only normal-mode request returning RCC CR, ICSCR,
  CFGR, and PLLCFGR plus the target's fail-closed contract result. The response
  is exact-length, CRC-protected, and reserves zeroed bytes for strict parsing.
- The request performs four volatile reads only. It cannot write RCC, publish
  HAL clocks, initialize TIM15/DMA/interrupts, mutate keypad/display state,
  access persistence, or reach RF/TX.
- A physical run remains separately guarded by the existing K1 recovery,
  image-validation, explicit-confirmation, power-cycle, and serial-probe path.

## ADR-043 — Clock-probe timeout is isolated with one register per request

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- Four new fixed-session requests read CR, ICSCR, CFGR, and PLLCFGR separately.
  Each exact response identifies its register, reserves zeroed fields, and
  returns one raw 32-bit value. A failure therefore names the first boundary
  that did not complete rather than losing all progress in a combined response.
- The diagnostic retains the combined request for comparison but changes no
  clock, HAL, keypad/display, persistence, RF, or TX state.

## ADR-044 — RCC diagnosis runs in a serial-only image

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The physical clock request timed out while the normal hello remained
  recoverable, and the image still initialized display, keypad, and backlight.
  The next diagnostic removes those runtime paths rather than assuming they are
  harmless to serial timing.
- Only the boot RAM witness, polling USART, normal hello, no-MMIO control, and
  read-only RCC requests may execute. This is an isolation image, not a keypad
  or display regression claim.
