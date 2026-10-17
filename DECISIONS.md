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

- **Superseded by `ADR-100`** for radio programming (2026-10-14).

- **Date:** 2026-08-05
- **Status:** accepted
- Writes are staged in a candidate snapshot, validated as a whole, and made
  active only by commit. Protocol commands never expose arbitrary storage
  addresses.

## ADR-004 — Simulated transport implements the programmer contract

- **Superseded by `ADR-100`** for radio programming (2026-10-14).

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
- A read-only snapshot must show enabled/ready 16 MHz HSI, the x3 PLL sourced
  from HSI, PLL selected and active as SYSCLK, and undivided AHB/APB.
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

## ADR-045 — K1 inherited-clock contract follows the observed F071 fields

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The exact-unit snapshot is CR `0x03000500`, ICSCR `0x00e64d14`, CFGR
  `0x00000012`, and PLLCFGR `0x00000006`. The pinned F071 DIE072 register
  inventory defines `HSI_FS=2` as 16 MHz, `PLLSRC=2` as HSI, and `PLLMUL=1`
  as x3. With undivided AHB/APB and PLL selected/active, this is 48 MHz.
- The provisional contract incorrectly assumed 24 MHz x2 and masked PLLSRC to
  one bit. Correct both fields and explicitly validate PLLMUL. Keep every other
  mismatch fail-closed and keep clock publication out of the runnable image.

## ADR-046 — HAL clock publication requires the validated handoff proof

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The HAL exposes one unsafe inherited-frequency primitive because its global
  clock table cannot verify hardware. The K1 wrapper is the safe boundary: it
  reads the live RCC snapshot, requires the exact fail-closed 48 MHz contract,
  and only then publishes SYS, HCLK1, PCLK1, PCLK1_TIM, HSI, and PLL values.
- `InheritedClocks` fields are private so callers cannot forge the proof. This
  optional function is not selected by the runnable image and does not write
  RCC, take peripheral tokens, initialize TIM15, or enable interrupts or DMA.

## ADR-047 — Embassy runtime initialization preserves inherited RCC

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The K1 runtime may initialize HAL bookkeeping, GPIO, DMA, FLASH access, and
  the reserved TIM15 time driver only after the safe clock-publication wrapper
  validates the live exact-unit tuple. It must not call the HAL RCC
  configurator or select a new oscillator, PLL, or prescaler.
- This ordering is wrapped in one K1 function returning singleton peripheral
  tokens. Target compilation is not executor, interrupt, DMA, timing, serial,
  display, keypad, or physical proof; those remain runnable-image gates.

## ADR-048 — First runnable Embassy image is a receive-only witness

- **Date:** 2026-08-06
- **Status:** accepted for `K1ASYNC-023`
- The new image uses cortex-m-rt startup at the existing `0x08002800`
  application origin, with 16 core vectors and all 32 generated F071 interrupt
  vectors. Static gates require the exact DMA1, TIM15, and USART1 handlers.
- One static task owns async USART1/DMA and retains the exact hello response.
  A second owns PF8 backlight, cooperative SPI1 display transfers, and only the
  evidenced PB6..PB3 by PB15..PB12 keypad. It samples every 5 ms with 10 us
  column settling and reuses the 20 ms fail-closed debounce contract.
- Any clock, initialization, task-allocation, display, or timer failure stops
  in a non-transmitting loop. Side keys, persistence, general UI, radio/RF, and
  TX remain unreachable.

## ADR-049 — Side keys remain evidence-first and receive-only

- **Date:** 2026-08-06
- **Status:** accepted for `K1SIDE-024`
- The pinned K1 source identifies PTT on PB10 and refers to a separate special
  side-key path, but AFIK has no independently recorded exact side-key GPIO
  mapping, polarity, settling, or electrical behavior. The main 4-by-4 matrix
  mapping must not be extended by inference.
- The first side-key package may define only bounded raw observations with
  explicit validity and provenance. Ambiguous, malformed, stale, or unverified
  samples remain untrusted and produce no semantic key edge.
- No side-key/PTT observation may alter the display, persistence, channel plan,
  RF state, or TX authority. Any physical experiment must be separately
  guarded, receive-only, and retain the known-good recovery path.

## ADR-050 — Banked channel storage uses membership masks, not member lists

- **Date:** 2026-08-07
- **Status:** accepted for `STORE-026`
- Explicit channels are stored as one bounded object each, and banks are stored
  as name and flag metadata only. Membership lives in a 16-bit mask on the
  channel rather than a member list on the bank.
- A member list would either bound a bank to roughly twenty channels inside the
  64-byte object limit or require multi-object paging. A mask keeps every object
  fixed size, lets a channel belong to several banks, and keeps the canonical
  image order stable when a channel moves between banks.
- The cost is that bank membership can only address sixteen banks. The
  compiler rejects any channel referencing a bank the project does not define,
  so a mask can never point at nothing.

## ADR-051 — The receive path reproduces the pinned firmware's register values

- **Date:** 2026-08-07
- **Status:** accepted for `RX-027`
- Where primary Beken documentation does not define a field, AFIK writes the
  exact value the pinned K1 reference firmware writes, recorded in
  `EVID-BK4819-053`. This is a deliberate operator decision to treat that
  source as authoritative for registers and pinout.
- AFIK still does not copy the reference implementation: the values are facts
  recorded in evidence, and the driver is an independent state machine with its
  own ordering guarantees, fail-closed faulting, and TX authority boundary.
- Calibration data is excluded. Squelch thresholds are inputs supplied by a
  board layer from the unit's own calibration, never constants in the driver.
- The receive path writes only the documented receive mode block. The transmit
  mode word remains reachable solely through the existing central-policy token.

## ADR-052 — The native editor validates before it can reach a radio

- **Date:** 2026-08-07
- **Status:** accepted for `NGUI-028`
- The native editor keeps operator input as drafts and converts them to typed
  records only through validation. An invalid field cannot reach a canonical
  image, a device transaction, or a flashing workflow.
- Firmware and EEPROM operations reuse the recovery-gated flasher library
  unchanged, including every confirmation phrase, the retained recovery image,
  and the retained EEPROM backup. The editor adds no shortcut and weakens no
  gate; it only collects input and reports progress.
- The editor is a local tool for a directly connected radio. It is not a
  service, exposes no network surface, and holds no authentication.

## ADR-053 — Chip variants are explicit profiles, not conditional code

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-029`
- The BK4819 and BK4829 share a bus and register map but differ in roughly a
  dozen values. AFIK models each variant as one `ChipProfile` constant holding
  exactly the differing values, recorded in `EVID-BK4829-055`.
- The driver logic stays single-path: ordering, state, and the transmit
  authority boundary are identical for both variants, so a variant can never
  introduce a different safety story.
- The K1 target selects the BK4829 profile because the pinned K1 build compiles
  that driver. A board must choose its profile explicitly; there is no default
  guess and no runtime chip detection.

## ADR-054 — Radio transfers run inside a serial request, not beside one

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-029`
- The three-wire bus is bit-banged and blocks the executor for milliseconds.
  The serial responder reads one byte at a time, so concurrent bit-banging
  drops inbound bytes and the application stops answering, as recorded in
  `EVID-K1-058`.
- The K1 image therefore owns the receiver inside the serial task and performs
  bring-up and sampling between a decoded request and its response. This is a
  deliberate constraint of the current witness image, not a general design rule
  for a radio with interrupt-driven or buffered serial input.
- A future image which needs free-running reception must first give the serial
  path a continuous receive buffer, or move the bus to a peripheral which does
  not hold the CPU.

## ADR-055 — Audio is an operator control on the keypad, not a serial command

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-029`
- The programming cable occupies the speaker jack, so routing audio removes the
  serial link and the internal speaker cannot be heard while the cable is in.
  `EVID-K1-059` records the evidence.
- Audio is therefore toggled by side key one and its state, the raw RSSI, and
  the squelch link are shown on the display. The operator unplugs the cable,
  listens, and reads the screen; no host is involved.
- The task which owns the radio also owns the audio pin and publishes a
  snapshot the serial responder reads. Serial never touches the bus, so
  answering a request cannot bit-bang beside an inbound frame, and metering
  runs only while audio is routed, which is when the cable is expected to be
  out.
- Audio remains receive-only in every path: AFIK constructs no transmit
  authority, so neither the keypad nor the serial link can key the radio.

## ADR-056 — One device-side protocol implementation, shared by target and simulator

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-031`
- The device half of the AFIK protocol lived only in the deterministic
  simulator. Giving the K1 its own copy would have created two answers to every
  transaction, listing, replay, and error question, with only one of them
  covered by the existing tests.
- `radio-device` now holds that implementation: the stream decoder, the bounded
  transactional store, the single-exchange replay cache, the per-kind activation
  limits, and the stable storage-error mapping. It is `no_std`, allocation-free,
  and bounded by const generics, so the target runs the same code the simulator
  does.
- Observable steps are reported through a caller-supplied observer instead of
  being recorded inside the service. The simulator keeps its timed trace and the
  firmware passes an observer which does nothing, so neither pays for the
  other's needs and the service knows nothing about time.
- The simulator was refactored onto it rather than left in place: its complete
  existing protocol test suite passed unchanged, which is the evidence that the
  shared implementation preserved every observable behaviour.

## ADR-057 — A radio retains its configuration in a reserved flash sector

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-031`
- A radio which forgets its channels at power-off is not programmable in any
  useful sense, so a committed configuration must survive a power cycle. The
  K1's external SPI flash holds the vendor's calibration and is not yet read by
  AFIK, so internal flash is the only region AFIK can claim honestly.
- The last 8 KiB erase sector, `0x0801E000` to `0x08020000`, is reserved. The
  application region ends there in the linker memory map, in the raw-image size
  gate, and in the ELF LOAD gate, so an application image cannot overwrite a
  retained configuration and an over-large image fails packaging instead.
- The retained bytes are the existing canonical configuration image, not a new
  on-flash format. The container header supplies the exact length, so the
  reserved region needs no separate record, and the complete checksum, ordering,
  and object validation still run before anything is activated. Anything else
  found there, including an erased sector, means "nothing retained".
- Retaining happens before the commit response is sent. The host is waiting for
  that response, so the interrupt masking required by the flash controller
  cannot drop an inbound byte. A failed retain is reported to the operator on
  the information screen as a built-in configuration; it never invents a state.
- The canonical image encoder gained an incremental writer so a device can emit
  its objects one at a time from a fixed table, without a second object-sized
  buffer. The slice encoder is implemented on it, so both paths produce
  identical bytes by construction.

## ADR-058 — The operator shell is a pure state machine

- **Date:** 2026-08-07
- **Status:** accepted for `RFK1-031`
- Channel selection now has real behaviour: screens, a list cursor, timed
  numeric entry, and a bank filter. Written inline in the target task loop, none
  of it could be tested without a display, a keypad, and a radio.
- `shell` therefore consumes debounced key presses plus explicit milliseconds
  and returns an intent for the caller to apply. Every transition is covered by
  host tests, and the intent set contains selection, bank filtering, monitoring,
  and receive-audio routing only, so no key press can produce a transmit
  request even by mistake.
- View positions belong to the receive controller beside the bank filter it
  already owns, not to the shell. The numbers the operator types are the
  positions the screen shows, resolved by the same code that filters the view.

## ADR-059 — A generated plan is a stored channel source, not a shorthand

- **Date:** 2026-08-07
- **Status:** accepted for `PLAN-034`
- A generated bank stored a base frequency, spacing, count, and transmit class
  only. That is enough to name frequencies and not enough to be channels, so no
  image expanded one: the K1 activated explicit records only and dropped the
  object. The space saving existed in the storage format and nowhere else, while
  the studio showed banks as containers for channel rows, which is the model the
  plan was supposed to replace.
- A plan now stores one `ChannelTemplate` beside its arithmetic: tones,
  modulation, bandwidth, power, manual step, squelch, and behaviour flags, held
  once for the whole bank. That is what makes it a channel source rather than a
  frequency list, and it is the whole saving: 46 bytes hold a bank of any size
  against 42 bytes for each explicit channel record. The object format version
  is 2 and version 1 objects are rejected rather than reinterpreted, because
  guessing an absent template would invent a radio's receive settings.
- Expansion produces complete `ChannelRecord`s, so selection, bank filtering,
  dual watch, and scanning cannot tell an expanded channel from a stored one and
  need no second path. Expanded channels take identifiers from a reserved range
  at or above `0x8000` which packs the bank and index; `ChannelRecord::new`
  refuses that range, so a stored channel and an expanded one can never collide.
  Names are derived as the truncated plan name plus the one-based position, so
  the number the operator reads is the number in the plan's documentation.
- `ProgrammedMemory` composes stored channels with installed plans and expands
  per lookup. Nothing is materialised, so a plan of a thousand channels costs
  the same RAM as a plan of ten, and the K1 bounds what it will select rather
  than what a plan may contain: four plans by the retained-image budget and 128
  expanded channels by what the interface can walk responsively.
- The studio edits the template once per plan and expands the plan in place, so
  the operator sees the channels the radio will build before writing them, and
  both tabs report what the configuration costs and what the plans saved. A
  channel row cannot join a plan's bank, because the plan already owns every
  channel in it.

## ADR-060 — Channels and settings live in external memory; internal flash is firmware only

- **Date:** 2026-08-08
- **Status:** accepted for `EEPROM-035`
- ADR-057 reserved the last internal flash sector for a retained configuration
  because the external memory was unread. That put the operator's data inside
  the region the firmware occupies: programming a radio competed with the
  space its own code needed, 1,280 bytes bounded the whole configuration, and
  reflashing risked what the operator had entered. `EVID-K1-061` removed the
  reason for it by identifying the fitted device.
- Configuration now lives in the external serial NOR memory the radio already
  carries, and internal flash holds firmware and nothing else. The reserved
  sector is gone and the packaging gates give the application the whole region
  through `0x08020000`. `EVID-K1-062` records one plan surviving a power cycle
  with no internal-flash store present.
- AFIK claims one sector-aligned region at 1 MiB, half the device and far above
  the approximately 52 KiB the radio's own firmware maps. `radio-eeprom` refuses
  any region below a fixed bound, so a wrong constant cannot reach the vendor's
  channels, settings, or calibration; the claim is checked before any access.
- The driver owns no bus, clock, or pin, and every access is bounded twice, by
  the device capacity and by the claimed region. A write erases the whole region
  before programming, so a shorter configuration cannot leave the tail of an
  older one behind to be read back as valid.
- The device declares the region size in its capability profile, so a host can
  say how much room a project leaves rather than guessing. The studio shows that
  as bytes used and free, and says nothing at all when no radio is connected.
- The memory is opened read-only first and its identification is reported on the
  information screen. A memory which does not answer leaves the radio a working
  receiver with nothing retained, because a configuration store is not worth a
  radio which will not start.

## ADR-061 — The operator interface never waits on another task

- **Date:** 2026-08-08
- **Status:** accepted for `EEPROM-035`
- The interface task waited for the serial task's first publication before it
  read a key, so that the display would not show the VFO to an operator whose
  radio was programmed. That coupling cost four images: when the serial task
  died, the radio drew its boot information screen and then ignored every key,
  which reads as a dead radio and hides which task actually failed.
- The interface now starts from an empty configuration and adopts a publication
  when one arrives. A radio whose serial or storage path is broken is still a
  receiver the operator can tune, and the failure is visible instead of total.
- The information screen carries the evidence an operator can read without a
  host: the external memory's state and identification, and serial received and
  answered counters. Those counters distinguished a deaf interface from an
  unanswered frame in one power cycle, having previously cost several images of
  speculation.

## ADR-062 — The radio holds its configuration once, encoded

- **Date:** 2026-08-08
- **Status:** accepted for `PLAN-037`
- A generated plan does not materialise its channels. The radio nonetheless
  materialised its whole configuration: the device service held it encoded in an
  active and a candidate snapshot, `Programmed` held it again decoded, and that
  decoded copy passed by value into the publication signal, the interface task
  and the receive controller. The same channels occupied SRAM about four times.
  Laziness stopped one level too low.
- `Programmed` is now an index and holds no configuration — each stored
  channel's identifier and bank mask, each plan's bank and channel count, which
  identifiers a named bank defines, and the global receive settings. The objects
  live once, encoded, in one shared snapshot.
- Counting, bank filtering and scan navigation are answered from the index
  alone, with no lock and no decode, because they only need to know how many
  channels there are and which bank each belongs to. Only materialising a record
  reaches the snapshot, once per channel actually shown or tuned.
- The consequence is the point: object bounds size stored bytes and a small
  index rather than a decoded cache. `MAX_CHANNELS` was a RAM decision and
  should not have been. `ARENA-038` finishes the job by making declared bytes
  the only bound.

## ADR-063 — An encoding is declared, and costs what it is

- **Date:** 2026-08-08
- **Status:** accepted for `PLAN-037`, superseded in part by `ARENA-038`
- `LinearFixedOffset` was added by giving `GeneratedBank` a transmit offset and
  deriving the encoding from whether that offset is zero. That was wrong twice
  over: every simplex plan now carries four bytes it never uses, and the
  encoding became an inference from data rather than a declared property.
- It survived review because a fixed 70-byte object slot charges a 55-byte plan
  and a 59-byte plan the same, so the waste was invisible. That is the same
  blindness the slot store imposes everywhere, and the reason to remove it.
- The intended shape is a shared plan core with a per-encoding tail: nothing for
  simplex, an offset for a repeater, a tone for a toned plan, a table for the
  table encodings. Expansion stays one implementation, because names,
  identifiers, membership and the template are all core and only the transmit
  frequency consults the tail. `TableSimplex`, `TableMixedDuplex` and
  `SparseExceptions` are variable length and cannot be expressed any other way
  without sizing every plan for the worst case.

## ADR-064 — Frames are delimited at both ends

- **Date:** 2026-08-08
- **Status:** accepted for `PLAN-037`
- Frames are COBS packets terminated by a zero byte, and were delimited at the
  end only. Opening a USB serial port puts a byte or two on the line as it
  settles, so a receiver holds rubbish when the frame begins and the packet it
  decodes is the rubbish and the frame together. It fails COBS, or its length,
  or its CRC, and never becomes a request. A radio received sixteen bytes for a
  fourteen-byte hello and refused every frame it was ever sent.
- The transport sends a zero byte before each frame. A receiver holding
  something decodes and discards it separately, then decodes the frame from a
  clean start; a receiver holding nothing ignores it, because a delimiter with
  an empty buffer is not a packet. The cost is one byte per frame.
- The device decoder already recovered on the next delimiter and needed no
  change. What was missing was a delimiter before the first frame rather than
  after the last one.


## ADR-065 — A store is bytes, and a device declares one number

- **Date:** 2026-08-09
- **Status:** accepted for `ARENA-038`; completes `ADR-062` and `ADR-063`
- A store held a fixed table of fixed slots, each 70 bytes whatever it carried:
  42 for a channel, 22 for a named bank, 16 for the configuration, 59 for a
  plan. A device therefore had four bounds — the slot, the slot count, a count
  per kind, and a declared byte capacity nothing enforced — and refused projects
  it had the room for while reserving room it could not use. The K1 held 3,220
  bytes across its two snapshots to store at most about 880.
- Objects are now packed end to end as `(kind, id, length, payload)` entries in
  strict `(kind, id)` order, and the bytes are the whole bound. Writing an
  object which is present replaces it and compacts around it; removing one
  closes its gap. Both are a single move within a transaction's own copy, so a
  failed transaction leaves the active bytes untouched by construction.
- That the entries are in canonical order and in the image's own layout is not a
  coincidence but the design: an arena is an image payload. Retaining the active
  snapshot became a copy rather than a sorted rebuild through a key index; the
  snapshot the interface reads became the same copy; a listing became a page of
  an order the store already holds. Three pieces of bookkeeping disappeared
  because the bytes were arranged the way they are consumed.
- A device declares `configuration_bytes` and nothing else binds. `max_objects`
  is derived — the count those bytes imply given the shortest object any kind
  encodes to — so it is an upper bound a host can trust rather than a second
  limit it must satisfy. A host refuses a project for the bytes it needs and
  names both numbers. A device reporting zero declares nothing rather than a
  full store, and refuses what it cannot hold as the bytes arrive.
- What the operator gets for those bytes is the operator's decision. The K1's
  1,264 bytes are about twenty-six explicit channels, or two dozen band plans
  and the tens of thousands of channels they expand to, or any mixture. The
  firmware has no opinion, which is the difference between a bound and a policy.
- Decoders take a borrowed object, so the store lends its bytes out rather than
  copying them into a worst-case buffer to be read. `MAX_OBJECT_DATA` survives
  as what one object may carry over the wire and in an image, which is a
  protocol fact rather than a storage cost.

## ADR-066 — The plan tail, and what a zero offset may not mean

- **Date:** 2026-08-09
- **Status:** accepted for `ARENA-038`; supersedes the inference in `ADR-063`
- A generated bank is a 56-byte core plus its declared family's tail: nothing
  for `LinearSimplex`, four bytes of transmit offset for `LinearFixedOffset`.
  Version 3 charged both 59. The core carries everything expansion needs for
  names, identifiers, membership and the template, so only the transmit
  frequency consults the tail and expansion stays one implementation.
- The family is stored, not derived. A repeater sub-band parked at a zero offset
  is a repeater sub-band, and stays one across a write and a read-back; under
  the inference it silently became a simplex plan and a host negotiating
  capabilities was told it needed a bit it did not.
- An editor asks the operator for an offset rather than for an encoding family,
  so `linear_from_offset_with` is the one place the two meet: a zero declares
  simplex and anything else declares fixed offset. After that the declaration
  travels with the plan and storage charges it for what it declared.
- Encodings which are declared but unimplemented are refused by name rather than
  given a length. `TableSimplex`, `TableMixedDuplex` and `SparseExceptions` are
  variable length; the packed store is what makes them expressible at all.

## ADR-067 — The operator's place is not configuration

- **Date:** 2026-08-09
- **Status:** accepted
- Where the operator left the radio — source, bank, channel, VFO frequency and
  step — is stored in its own erase sector, as a sixteen-byte CRC-16 record
  programmed into the next erased slot. It is not an object in the
  configuration store and does not appear in a configuration image.
- A configuration is what a host programmed: a canonical image, erased and
  rewritten as one thing, changing when someone deliberately reprograms the
  radio. A place is what the operator did afterwards, and it changes every time
  they turn the channel knob. Sharing storage would spend the channel list's
  erase cycles on a channel change, and would put the channel list at risk in
  the window a place is being written.
- Programming an erased slot rather than rewriting one is what makes the
  ordinary save a single page program. A sector holds two hundred and fifty-six
  records, so it is erased once every two hundred and fifty-six saves.
- A place is therefore also allowed to be wrong. It carries the channel
  identifier beside the index, and a restore which cannot match them starts at
  the top of the view. Nothing downstream may treat a place as authority: it
  names a selection, and the configuration remains the only thing that says
  what that selection is.
- The radio-wide squelch level stays in the configuration, where `EEPROM-035`
  put it, because it is a setting a host may legitimately program.

## ADR-068 — A walk asks membership before it asks for a channel

- **Date:** 2026-08-09
- **Status:** accepted
- Selection, bank filtering, and scanning ask a `ChannelSource` whether an index
  is in the active bank before asking it to build the record. A record is built
  only where a scan must read the skip flag, or where a channel is selected.
- A generated plan answers membership from arithmetic. Asking for the record
  first meant a filtered walk expanded every channel of every other bank it
  stepped over and threw all of them away, which is work proportional to the
  whole plan for a step through one bank of it.
- This is a cost rule, not a memory rule: nothing was ever materialised, and the
  record a walk does build is still dropped again. What it buys is that a scan
  over a band-sized plan costs a decode per channel it lands on.

## ADR-069 — The handset carries what the field changes, the programmer the rest

- **Date:** 2026-08-09
- **Status:** accepted
- A settings row on the radio has to earn its place by being something an
  operator changes with the radio in their hand and no host in reach. Squelch
  qualifies: the right level depends on where they are standing. The scan dwell
  does not: it is set once for a unit and then left.
- Everything else a radio holds is programmed. The configuration store already
  carries the whole `RadioConfig`, so a setting without a menu is not a setting
  without a control — it is one reached from the programmer, at full
  resolution, and retained in external memory like any other.
- This is why the scan-dwell menu was removed after one release rather than
  kept: it had done its job, which was to find the number on the unit. The
  handset list could only offer the rows it was compiled with; a host offers
  whole milliseconds.
- The cost of a menu is not only code. It is a screen the operator has to walk
  past, and a second place a value can be changed from, which is a second thing
  that can disagree with the store.

## ADR-070 — The operator says what the radio is; the beacon may only contradict

- **Date:** 2026-08-11
- **Status:** accepted
- AFIK does not classify Quansheng hardware from its bootloader beacon. The
  operator declares the target, and the observed beacon is recorded and used only
  to refuse a declaration it positively contradicts.
- The attempt to classify failed on contact with three radios. `EVID-K5-012` and
  `EVID-K5-013` record one generation, one working vendor firmware, and two
  bootloader versions — `2.00.06` and `4.00.01`. `EVID-K5-016` records that no
  reviewed external project can query the processor either: K5TOOL separates V1,
  V2 and V3 by physical marking, its hello carries no processor identifier, and
  the browser flashers ask the operator which radio they have. A `4.00.*`
  bootloader appears in none of those sources, so the published tables cannot
  settle what this unit is in either direction.
- A version-prefix gate is also the wrong test even where a taxonomy exists.
  `EVID-K5-015` establishes that the protocol is announced by the beacon command
  — `0x0518` against `0x057A` for the AES path — and that the printable version
  describes the build. AFIK gated on the version and special-cased the command,
  which is why a protocol-compatible radio was refused.
- So the beacon becomes data. `detect_bootloader` reports the command, the
  version and the identity field as observed, and a separate step compares that
  observation against the declared target. The comparison may fail only on a
  known incompatibility, never on an unfamiliar one: if an unrecognised version
  blocks an operation, the taxonomy has been rebuilt implicitly and the next
  radio Quansheng ships is refused for no reason anyone can defend.
- Unknown is therefore not mismatch. This is the whole discipline of the
  decision, and it is what makes the design survive hardware nobody has seen.
- A declaration is cheap to assert and may be wrong, so it settles what the radio
  is and never whether it may be written to. The recovery rehearsal, the EEPROM
  backup and the exact confirmation phrases stay as they are. `ADR-067` already
  states this shape for operator state: it names something, and nothing
  downstream may treat it as authority.
- The confirmation phrase is derived from the declared target rather than
  compiled in. `RISK-037` recorded the real defect in the old arrangement: a
  constant describing one bootloader on one unit read as a claim about a family.
  A claim belongs to the operator, per unit, per session, where it can be
  corroborated or contradicted.
- What the operator declares is one profile, and it is the same profile that
  parameterises a build. Geometry and memory capacity are per-unit facts, so a
  hand-fitted memory part is a supported case rather than an exception.
- Because a declaration can be wrong, the image verifies what it can reach:
  `SCB CPUID` for the core, and address aliasing for the fitted memory. A
  declared capacity larger than the part destroys calibration on the first
  erase, which is precisely the failure a host-side gate cannot catch.
## ADR-071 — K1 and K5 share application services above target adapters

- **Date:** 2026-08-12
- **Status:** accepted for `PLAT-050`
- `radio-platform` is a `no_std`, heap-free leaf containing the bounded serial
  hello service and application-facing boot-display contract. Target firmware
  supplies identity and bytes, and owns all UART, DMA, interrupt, pin, register,
  display-transport, and timing behavior.
- K1-only diagnostic commands remain in K1; sharing the common hello does not
  widen K5's read-only application protocol. The proven K5 synchronous-GPIO
  display stays its adapter and the unproven SPI0 path gains no status.

## ADR-072 — First K5 receive write remains muted and operator-sampled

- **Date:** 2026-08-13
- **Status:** accepted for `K5APP-051`
- The first physical BK4819 write candidate initializes and meters receive only.
  It independently mutes the chip AF route and the PC4 speaker gate, carries no
  TX authority, and does not touch PTT, the PA, or an RF switch.
- The sixteen analogue PMR446 centre frequencies are recognizable nearby-signal
  examples, not transmit permission or equipment-compliance evidence. The image
  uses narrow FM, no tone, and squelch-off thresholds solely to expose raw
  metering before calibration is interpreted.
- No receiver timing is invented. Up/Down retune and Menu resamples once on a
  new key edge; an operator who wants a post-settling sample waits before
  pressing Menu. Periodic polling and audible output require later evidence.

## ADR-073 — K5 audio defaults on but the amplifier follows sampled squelch

- **Date:** 2026-08-13
- **Status:** accepted for `K5APP-051`
- Audio-on is the operator default, but it does not mean PC4 is held on. The
  BK4819 receives with demodulated AF selected while a 50 ms polled Cortex-M0
  `SysTick` updates PC4 from the chip's squelch link. A retune starts closed and
  a read failure mutes immediately.
- Menu is the immediate escape: it disables PC4 before changing the chip AF
  route, and the inverse action still waits for a squelch sample before opening
  the amplifier. PTT remains a displayed input with no behavior.
- Level three is AFIK's existing carrier-only provisional threshold, not this
  unit's factory calibration. Physical audio can be validated with it, while
  calibrated squelch and selectivity remain separate work.

## ADR-074 — Unified application extraction proceeds with K1 risk deferred

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `RISK-036` remains an honest K1 defect but does not block extracting pure
  application behavior. The now-working K5 receive/audio path provides a second
  adapter which can expose hidden K1 assumptions instead of repeatedly
  bisecting one target before architecture work begins.
- The shared boundary is event/effect and semantic state, not peripheral traits
  which pretend unlike MCUs are alike. Target crates continue to own scheduling,
  register buses, chip profiles, persistence transport, rendering, packaging,
  and recovery.
- The first unified milestone is receive-only. TX, calibrated RF claims, battery
  parity, and nonessential peripheral parity do not delay it.

## ADR-075 — Keypad meaning and receive semantic view are shared, scanning and rendering are not

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `radio-platform::receive_app::Key` is the canonical post-debounce logical
  key vocabulary for K1 and K5. `Up`, `Down`, and `Menu` map to the shared
  receive application; other keys remain available to target-owned shells and
  have no receive-only action. Physical matrix scans, timing, debounce, GPIO,
  and PTT input handling remain below the boundary in each target adapter.
- The shared `View` remains authoritative for channel, operator audio,
  sampled squelch, and receiver health. Target adapters retain raw RSSI,
  glitch/noise, filter read-back, chip profiles, and pixel rendering. K5 must
  clear or suppress those raw fields when the shared view retunes or latches a
  receiver fault, so stale hardware observations cannot contradict semantic
  state.
- This step does not import the K1 shell into K5, alter K1 persistence,
  diagnostics, serial control, BK4829 profile, image layout, or recovery, and
  does not authorize a hardware flash. The K5 package remains bounded by the
  existing `0xF000` region. Adapter failures are routed back through the
  shared receiver-fault event so the semantic view and speaker gate fail
  silent even when a target operation fails during a key or sample event.

## ADR-076 — Configuration activation shares values, not persistence

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `radio-platform::configuration` owns only the bounded common PMR446 channel
  identity/raster and `ActivatedConfiguration`, a copyable value carrying an
  opaque active generation, selected channel, and operator audio preference.
  It is `no_std`, heap-free, and has no storage, serial, EEPROM, or target
  dependency.
- K1 constructs the value from its retained `Programmed` publication and
  generation. K1 continues to own object decoding, external-memory writes,
  operator-place restore, arbitrary channels, VFO, scanning, and host protocol.
  Replacing a K1 publication drops the old shared activation so semantic state
  cannot cross generations.
- K5 constructs generation zero from its built-in PMR446 receive fixture. This
  is an explicit absence of persistence behavior, not a claim that K5 EEPROM
  stores AFIK configuration. K5 gains no `radio-device`, storage, or EEPROM
  import from this step.
- Shared activation fixtures compare ordered application effects and semantic
  views while separately asserting K1 and K5 generation identity. This keeps
  the common behavior identical without treating unlike persistence systems as
  interchangeable.

## ADR-077 — Configuration changes commit only after an exact adapter acknowledgement

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `radio-platform::configuration_change` is a `no_std`, heap-free, bounded
  value contract. A candidate carries the common channel/audio value and the
  opaque target/storage generation; a change adds a monotonically allocated
  application-local identity; an acknowledgement carries that exact change
  and an applied/rejected result.
- `ReceiveApp` keeps the active semantic snapshot unchanged while a change is
  pending. It emits one identified change request, accepts only a matching
  acknowledgement, ignores mismatched or stale acknowledgements, and on a
  matching rejection keeps the old snapshot while muting chip audio and the
  speaker and marking the receiver unknown. IDs fail closed at exhaustion and
  are never wrapped or reused.
- K1's target adapter acknowledges only after its existing receive operation
  has not faulted; K5 acknowledges only after its existing bounded tune,
  read-back, and metric operations succeed. The contract does not own K1
  object transactions, EEPROM persistence, serial framing, or any K5 EEPROM
  behavior.

## ADR-078 — Pending shared changes are isolated and adapter completion is explicit

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- While a `ReceiveApp` configuration candidate is pending, `Start`, channel
  navigation, direct selection, and audio toggles are ignored. The old active
  semantic snapshot therefore cannot be overwritten before the adapter has
  acknowledged the exact candidate. A receiver fault clears the pending
  change, so a delayed applied acknowledgement cannot resurrect it.
- Change identities retain a bounded application-instance token in addition to
  their per-instance sequence. Target adapters supply only an opaque token;
  no MCU, storage, or transport type crosses the boundary. Host tests prove
  that acknowledgements from two live app instances with the same candidate do
  not cross-commit.
- K1 adapter application returns an explicit complete-success result and
  treats translation failure, pre-existing fault, and any operation fault as
  rejection. It continues safe speaker-off and redraw effects after a failure.
  K5 checks its display result and drains all safe effects before returning its
  acknowledgement result; both adapters force the speaker closed on failure.

## ADR-079 — Serial control shares application meaning, not wire or storage

- **Superseded by `ADR-100`** for radio programming (2026-10-14).

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `radio-platform::serial_control` accepts only requests which have already
  crossed a target-owned wire boundary. It contains bounded semantic-view and
  capability queries, logical-key commands, and configuration candidates. It
  emits the same `ReceiveApp` effects as keypad events; candidates retain the
  existing exact change acknowledgement contract.
- K1 keeps UART/DMA, the object/runtime protocol, `DeviceService`, EEPROM
  persistence and commit/read-back, arbitrary channels, VFO, scan, and raw
  metrics in its target adapter. K5 keeps circular-DMA UART and exposes only
  shared receive/view/key capabilities with generation-zero volatile
  activation; it does not gain invented persistence or a K1 object protocol.
- Capability sets are target-owned and deterministic host fixtures assert the
  divergence. This boundary is not wire interoperability, EEPROM durability,
  physical serial evidence, or proof that K5 supports K1-only controls.

## ADR-080 — Common serial payloads are bounded, while framing remains target-owned

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052`
- `radio-platform::serial_control::wire` defines only a fixed two-byte request
  payload for semantic-view/capability queries and canonical logical keys.
  Responses are fixed, bounded view or capability payloads. The shared key
  decoder maps to the same `ReceiveApp` request as a keypad edge.
- K1 and K5 adapters retain their existing framing and transport: K1's COBS
  object/runtime service and K5's observed normal-mode hello envelope and
  circular-DMA UART. They may carry the common payload only after their own
  framing validates it; no target or host wire type enters `radio-platform`.
- Candidate configuration changes are excluded from this payload because their
  exact acknowledgement, K1 persistence, and K5 volatile generation-zero
  semantics are not common. Deterministic K1, K5, and simulator fixtures prove
  request/response bytes and shared key effects; this remains host/static
  evidence, not physical serial interoperability.

## ADR-081 — Common serial commands are assigned inside each target envelope

- **Date:** 2026-08-13
- **Status:** historical boundary; key-command portion superseded for `2.0P`
  by ADR-085/ADR-086. The framing and K1 `0x18` assignment remain applicable.
- K1 keeps the existing `RuntimeControl` service and assigns command `0x18` to
  an exactly two-byte `CommonApplication` payload. `DeviceService` retains the
  normal response flags, sequence replay, sequence-conflict rejection, and
  target-only command matrix; the new command is decoded only after those
  checks and its result is applied through the existing `ReceiveApp` instance.
- K5's observed normal-mode request body has no spare payload bytes after its
  fixed session word. This historical record assigned `0x0600` to View,
  `0x0601` to Capabilities, and `0x0602..=0x0614` to the canonical
  logical-key vocabulary, with common response command `0x0620`. For the
  corrected unflashed `AFIK-K5-2.0P`, ADR-086 supersedes that key range:
  `0x0603` is Side2, `0x0606` is Up, `0x0607` is Down, and `0x0615` is the
  explicit AUDIO operation; unsupported shell keys are rejected before app
  dispatch. The fixed hello command and observed bootloader/version
  assignments remain unchanged. Invalid CRC/session/trailer, malformed
  length, and unassigned command values are discarded before the shared app
  is reached.
- The assignments establish software command ownership and simulator parity,
  not physical K1/K5 serial interoperability. K5 has no sequence/replay field
  in this observed envelope, and neither target gains common candidate
  persistence or K1 object operations.
- **Historical envelope review statement:** the K5 response keeps the observed
  fixed outer envelope and uses the common response data length in its inner
  length field. The earlier shared sentence said that a failed common key in
  either adapter fed `ReceiverFault` back before refusal. For the corrected
  `2.0P` boundary, K1 refuses common key/audio requests before dispatch because
  its advertised common serial capability is view-only; K5 feeds
  `ReceiverFault` back into `ReceiveApp` when a dispatched target effect fails,
  preserving fail-silent semantic state and the physical speaker gate.

## ADR-082 — K5 host control is one bounded normal-mode exchange

- **Date:** 2026-08-13
- **Status:** accepted for `APP-052` release preparation
- The host flasher reuses the existing K5 envelope codec for exactly one
  fixed-session request and one response. It exposes only View `0x0600`,
  Capabilities `0x0601`, and the common Menu/Up/Down commands
  `0x0605`/`0x0606`/`0x0607`.
- A response is accepted only when the existing footer/marker and response
  CRC checks pass, the command is exactly `0x0620`, the inner length matches the
  requested result, reserved bytes are zero, and common view/capability flags
  are bounded. There is no retry or sequence cache because the observed K5
  envelope has no sequence field.
- The command is read/control-only: it has no EEPROM, persistence, TX, or flash
  path. Fake-transport and parser tests are software readiness evidence, not
  physical serial interoperability or normal-mode proof.
- This release-prep record preserves the 1.9S/Menu facts at the time; ADR-085
  and ADR-086 supersede its advertised mutation boundary for the 2.0P image.
- **ADR-083 — Operator shell semantics are shared and target capabilities gate screens (2026-08-13):** `radio-platform::operator_shell` owns the pure K1/K5 operator shell: logical key meaning, operating/channel/source/settings/info screen transitions, list cursors, numeric channel selection, hold/release handling, and bounded intents. K1 keeps only a target-module re-export, so it cannot silently grow a second shell implementation. K1 enables VFO, settings, sources, scan, and Info. K5 enables the common sixteen-channel source/list, operating, settings/squelch, and Info screens; VFO and scanning remain disabled. K5 handset squelch is a volatile target-owned apply path, not persistence. K1 and K5 share display framebuffer layout, glyphs, and render semantics; target adapters retain framebuffer storage, page/command transport, timing, receiver metrics, storage, and MCU mechanics. K5's renderer consumes the shared shell state and shared semantic `View`; its richer screen output is static/simulator evidence, not physical display proof.
- **ADR-084 — Operator pixels are shared; transport remains target-owned (2026-08-13):** the K1 128x64 layout, glyphs, list rows, operating state, capability-gated selector, and Info model live in `radio-platform::operator_display`. Both targets call these render functions and only adapters own controller commands, page timing, and framebuffer writes. K5 used the historical `AFIK-K5-2.0P`, PMR names/frequencies derived from the bounded shared raster, and explicit unprobed persistence/generation-zero values. ADR-087 supersedes that image identity and transport result: the current corrective identity is `AFIK-K5-2.1D`, with the same shared pixels but the evidenced controller column window. A K5 Side2 press maps to the shared monitor override and opens the same receive speaker gate semantics; Menu remains list confirmation. This is deterministic host/simulator evidence, not physical K5 display or side-key acceptance.
- **ADR-085 — Monitor and audio commands have separate boundaries (2026-08-13):** `Intent::ToggleMonitor` is the shell-facing operator intent. On K5 it enters `ReceiveApp`; on K1’s common PMR adapter the target controller applies the requested monitor mechanics and the shared app receives the same desired value through `Event::SetMonitor`. This preserves one semantic monitor state at the application boundary without pretending arbitrary K1 channels use that app. `Key::Menu` is never a receive-app audio shortcut: shell Menu confirms lists, and serial audio uses explicit common operation `0x04` / K5 command `0x0615`. Receiver health is part of the shared operating view and faults render visibly. Direct target display writers invalidate any cached shared frame.
- **ADR-086 — Common serial key ownership follows the target controller (2026-08-13):** K5's `ReceiveApp` is its bounded receive/controller boundary, so K5 advertises and accepts common Up/Down/Side2 key commands plus explicit AUDIO. Their direction matches the shared shell: Up is previous and Down is next. K1's banked controller and shell live in a separate UI task; its serial adapter advertises view-only common access and refuses key/audio mutations rather than mutating a second `ReceiveApp` state. The shared wire vocabulary and bit-2 monitoring flag remain available to adapters that can own the corresponding target mechanics.
- **ADR-087 — K5 shared-frame transport uses the evidenced controller window (2026-08-13):** The physical `2.0P` image booted and answered normal serial, but its shared framebuffer was visibly cropped on the left with stray pixels on the right. The target adapter trace found that direct text already selected controller column `+4`, while shared `clear`/frame writes selected column `0`. K5 now keeps the shared pixels unchanged and sends each eight-page 128-byte framebuffer row at controller columns `4..=131`; tests prove every visible column is written once with no edge wrap. The corrective image is `AFIK-K5-2.1D`. This is software evidence until a separately authorized physical write and display observation.
- **ADR-088 — K5 handset squelch is a volatile acknowledged application setting (2026-08-13):** K5 enables the shared Settings/SquelchList shell, but the shell does not commit its displayed value until `ReceiveApp` stages the selected `SquelchLevel`, the target applies `SquelchThresholds::for_level`, and the target acknowledges success. The target seam reports explicit applied, rejected/no-effect, and faulted outcomes; activation, source, publication, and pending replacement paths cancel staged squelch before replacing the target generation. A rejected, faulted, or pending operation leaves the shell and shared level unchanged, closes audio, and emits no K1 persistence signal. The accepted K5 path is volatile generation-zero behavior; no EEPROM, serial persistence, or capability claim is added. `AFIK-K5-2.2Q` is an unflashed software/package identity. The guarded physical `AFIK-K5-2.1D` evidence remains separate and is not read-back or `2.2Q` evidence.

## ADR-089 — Persistence is a relative dual-slot canonical-image model (2026-08-14)

- **Status:** accepted for APP-052 Milestone 2 software modelling only
- `radio-storage::dual_slot` owns no application state and defines no EEPROM
  address. A target adapter later supplies relative eight-byte block reads and
  writes. Each of two fixed slots contains explicit metadata, the exact
  canonical configuration image, and a final commit marker written last.
- Every candidate block is protected by expected-byte comparison and verified
  read-back. An uncommitted or corrupt candidate is ignored; the highest valid
  wrapping generation wins, with equal and half-range ambiguity resolved by
  the lower relative slot index. An update never writes the active slot.
- Before replacing the selected inactive slot, its current commit block is
  atomically expected-checked and written as an all-FF tombstone, with
  independent read-back. Header/payload blocks follow, and only the final
  full-generation digest marker can publish the new candidate. Thus a torn
  update cannot leave an old marker authenticating new bytes; a failure after
  the final marker write may legitimately recover the new candidate. The
  all-FF commit block is rejected before generation/tag parsing, and marker
  encoding remaps the otherwise reserved all-FF result.
- The bounded 1280-byte image limit mirrors the existing K1 retained-image
  bound and is a model geometry, not evidence of the K5 EEPROM map. The two
  slots occupy 2624 bytes within an 8192-byte documented model region. The
  retained complete backup and pinned eight-byte write
  shape do not establish a disposable address, power-loss behaviour, or safe
  restore route. Therefore K5 capabilities remain volatile generation zero,
  no firmware identity is bumped, and no physical write is authorized. The
  next task is simulator object transport, not physical allocation.

## ADR-090 — K5 object transport is an explicit simulator boundary

- **Superseded by `ADR-100`** for radio programming (2026-10-14).

- **Date:** 2026-08-14
- **Status:** accepted for APP-052 Milestone 3 simulator/host evidence only
- `radio-protocol::k5_extended` wraps one complete existing COBS/CRC frame in
  a bounded envelope with generic magic/version, explicit command/session,
  exact length, and CRC32. It is deliberately not advertised, wired into a
  target main loop, or assigned over the existing `Hello` and common
  `0x0600..` controls.
- `radio-sim::K5ObjectTransport` is selected explicitly by its constructor;
  it does not auto-detect a target. It unwraps into the existing
  `DeviceService<1264>` and fragments responses deterministically so the
  existing `radio-programmer` transaction, listing, and read-back code is the
  exercised path.
- A successful simulated commit serializes the exact canonical
  `radio-storage` image into the relative dual-slot model. Reboot validates
  the newest image and restores its generation. The fake backend demonstrates
  interruption and corruption fallback only; it is not evidence of a K5
  EEPROM allocation, normal-mode transport, persistence capability, or
  physical restore route. TX remains absent and default-deny policy is not
  widened.
- Persistence is triggered only by a fresh `TransactionCommitted` event, not
  by inspecting a successful response, so service replay cannot repeat a
  durable write. If the dual-slot update fails, the simulator restores the
  complete pre-commit `DeviceService` snapshot and returns the existing
  protocol `Internal` error. That error is cached as a bounded frame for exact
  same-request replay; conflicting sequence bytes receive `SequenceConflict`,
  while a new-sequence Abort can close the restored transaction. Reboot
  exposes typed persistence errors and does not replace the service on a
  backend read failure. Relative block range errors and malformed direct
  transport input are explicit failures, never panics.
- The next decision gate is safe allocation evidence and target-adapter
  sizing/static analysis, not a flash or physical persistence operation.

## ADR-091 — K5 EEPROM restore is explicit, backup-gated, and host-verified

- **Date:** 2026-08-14
- **Status:** accepted for APP-052 Milestone 4a software tooling only
- `restore-eeprom` is a separate explicit K5 CLI command and is absent from
  auto-detection. It requires a complete non-uniform restore image, a separate
  exact expected-current image, a vector-checked 60,612-byte recovery
  application with a separately supplied trusted SHA-256 plus version/CRC
  confirmation, the qualified target phrase, and a
  `TARGET:CRC32` restore token.
- Preflight reads hello and all 8,192 live bytes, refuses any mismatch, and
  creates a create-new mode-0600 pre-write backup before a write. The host
  then performs two stable immediate reads for each expected eight-byte block
  before sending the vendor `0x051D` request, validates the `0x051E` offset
  echo, reads the block back, and requires a final complete-image read-back.
  This narrows but does not eliminate TOCTOU because the vendor command has no
  atomic expected-byte compare; exclusive serial ownership and a quiescent
  stock UI are required. Missing or ambiguous responses stop without retry.
- This does not select an EEPROM address, create a persistence capability, or
  authorize a physical run. The relative 2,624-byte dual-slot model remains
  unallocated; deliberate vendor-memory repurposing requires a separate
  evidence decision.

## ADR-092 — TX authority approves the complete immutable request

- **Date:** 2026-10-13
- **Status:** accepted for `TXAUTH-053` first milestone
- Class permission alone no longer constructs a `TxAuthorisation`. The sole
  constructor checks the exact channel, modulation, bandwidth, conducted
  milliwatts, bounded timeout, healthy selected state, configuration identity,
  regional/operator facts, and independent hardware/calibration envelopes.
- Tokens borrow the policy and complete request, so their approved inputs
  cannot be mutated while borrowed. Downstream logical TX entry compares the
  complete current request before any bus operation. Application instance and
  selection/configuration revision identifiers must not be reused; generation
  zero is allowed for volatile targets and is not the sole identity.
- Envelopes are caller-supplied evidence inputs, not built-in target profiles.
  No synthesized fixture is hardware evidence. The existing regional snapshot
  retains its version/date; no claim of refreshed regulatory compliance is
  made. First-milestone tests do not establish PTT lifecycle or physical TX.
- **Verified result:** both envelopes must match the non-zero opaque unit
  identity, every matching overlap contributes its strictest limit, and
  optional grants restrict built-in rules. Timeout is an application ceiling
  of 60,000 ms, not a hardware measurement. The token borrows policy/request,
  and selected-controller bundles also borrow the controller. The complete
  request is compared again before logical TX entry. Workspace and both target
  build gates pass; neither firmware wires TX or includes synthetic envelopes.

## ADR-093 — PTT effects are borrowed, sequential, and explicitly acknowledged

- **Date:** 2026-10-13
- **Status:** accepted for `TXCTRL-054`, host/simulator lifecycle only
- The shared `radio-platform::tx_control` core owns no clock, driver, target,
  register, allocator, or scheduler. Every input carries explicit monotonic
  milliseconds and current facts. The timeout starts at the accepted press,
  includes entry time, and cancels entry at the exact deadline. Held boot,
  denied press, timeout, and presses during cleanup require a fresh release.
- Startup establishes PA off and TX off before receive. Authorized entry is
  PA off, preparation of exact approved settings/power, chip TX, then PA on.
  The sole pending effect borrows controller/policy/request; every entry effect
  rechecks complete authority. Adapters execute once synchronously, then report
  the exact instance/cycle/phase identity. Old acknowledgements cannot advance
  a later operation. Cycles never wrap; adapter instances must never be reused.
- Release, invalidation, timeout, and fault cleanup request PA off before
  unconditional TX off, including unknown/faulted chip state. Receive restoration
  uses the current authoritative setup only after both disables succeed. Failed
  PA off still attempts TX off. A failed receive restore runs a distinct second
  cleanup pair. Any failure latches a fault and prevents further TX; replacement
  requires a fresh instance and safe startup rather than automatic retry.
- Simulator PA/power state and applied acknowledgements are logical contracts,
  not target evidence. Deadline delivery and bounded adapter execution remain
  adapter obligations. Neither target is wired to this controller; `RISK-007`,
  `RISK-038`, and `RISK-039` remain open for physical integration.

## ADR-094 — TX source evidence qualifies each target effect separately

- **Date:** 2026-10-13
- **Status:** accepted for the evidence-only `TXEVID-055` package
- Every shared PTT effect requires a separate exact-target contract. Begin
  with unconditional PA/chip TX off from unknown, reset, faulted and partially
  applied state. Source/register statements, exact-unit measurements, software
  tests and unsupported assumptions remain distinct in the hardware ledger.
- K1 evidence follows the pinned build's selected BK4829 driver; K5 evidence
  follows the separate V1/BK4819 source. Shared function names and register
  addresses do not establish interchangeable mode words, power maps or safety
  prerequisites. Existing receive observations do not qualify post-TX cleanup.
- Both reviewed sources control the PA over the chip bus with an output shadow.
  Their named preparation enters chip TX, and bias follows PA enable. No safe
  whole-register word, independent PA inhibit, non-energizing preparation split,
  applied acknowledgement, power conversion or timing bound is inferred from
  those source sequences. Physical bus `Ok` is software completion alone.
- Target hardware and calibration envelopes remain absent until supported by
  independently recorded unit-bound evidence; source bias tables, host fixtures
  and regulatory ceilings cannot create them. Qualification of shutdown and
  fail-off precedes energizing experiments. Unknown inputs continue to deny TX.
- The matrices and proposed experiments establish a reviewable boundary only.
  No production code, firmware identity, adapter wiring, calibration bytes or
  physical experiment changes. `TXPLAN-056` prepares the next non-transmitting
  shutdown acceptance plan; its activation and any physical work are separate.

## ADR-095 — Use proven Armel controls for a receive-only hardware witness

- **Date:** 2026-10-13
- **Status:** accepted for `TXPLAN-056`, following operator hardware steering
- Correlate every hardware conclusion with the selected Armel driver and its
  callers. Accept their proven operating paths as board/register authority;
  remaining AFIK fault-state and timing questions do not invalidate that work.
- Distinguish logical GPIO indices from inherited package-number labels.
  K1's `PIN29` symbol denotes enum index 1; the selected BK4829 document places
  GPIO1 on package pin 1. This is not an Armel driver defect. Probe pads need
  the matched physical package/net map.
- Proceed first with a source-backed K5 receive-only off/idle/restore witness,
  host fault/order tests, a verified image, and real boot/register/receive
  observations. Treat those observations at their actual scope rather than
  requiring later RF measurements to precede a non-transmitting smoke test.
- The operator authorized required flashing and requested prompt hardware
  testing. Carry that permission into `TXOFF-057`; no repeat approval is needed
  for the authorized required flash. No physical TX integration or invented
  hardware/calibration envelope follows from this permission or witness.

## ADR-096 — Proportionate TX bring-up: key up first, harden after

- **Date:** 2026-10-13
- **Status:** accepted, operator course correction. Supersedes the gating
  order in `ADR-094` and the witness-first ordering in `ADR-095`.
- The proven normal TX enter/exit and receive-restore paths in Armel's
  selected drivers (`EVID-TX056-003`) are sufficient board/register authority
  for a first transmission. They run on thousands of units. AFIK reimplements
  the sequence from that evidence; it does not copy the code.
- **Gates for the first K5 key-up are only:** the shared `tx_control` lifecycle
  and policy in the path; host/simulator tests of order and failure cleanup;
  the lowest reference power level derived from the unit's own EEPROM
  calibration; a short timeout (≤ 5 s); a dummy load or a frequency the
  operator may use (PMR446 for the K5 test); and carrier on/off seen on an
  SDR or second radio at press, release and timeout.
- These are **follow-up hardening, not gates**: instrument-measured off in
  every unknown/reset/fault state, an independent PA fail-off route, watchdog
  coverage of RF outputs, worst-case deadline-latency bounds, and a full
  power/modulation envelope. Neither the stock firmware nor Armel provides
  them. Track them as ordinary tasks once TX works.
- The TX hardware/calibration envelope for K5 is declared in firmware: the
  lowest reference power level and the operator's channels only. Its milliwatt
  figure is nominal until measured, and is labelled as such.
- **Process:** no further evidence-only or plan-only work packages for TX.
  Each package changes code, adds tests, and ends with a hardware observation
  where hardware is involved. Record findings in a few lines at their real
  scope. Physical transmission still needs the operator's explicit
  authorization, recorded in `STATUS.md`; once given for the bring-up sequence
  it is not re-requested per step.

## ADR-097 — One version for every target and tool

- **Date:** 2026-10-13
- **Status:** accepted, operator request.
- Every firmware identity (`AFIK-K1-…`, `AFIK-K5-…`, screen and serial alike)
  and every host tool's `--version` derive from the workspace version in the
  root `Cargo.toml`. Bump it once for each image that will be flashed.
- Starts at `7.0.0`, above both earlier per-target numbers (K1 6.5, K5 2.5).
  This also fixes the K1 serial hello, which still reported `AFIK-K1-1.2`.

## ADR-098 — One flasher binary that detects the radio

- **Date:** 2026-10-13
- **Status:** accepted, operator request. Supersedes the separate `afik-k5`
  binary and the `flash-afik-k1` command from `ADR-023` and `ADR-027`.
- `afik-flasher` is the only flasher. It carries the former `afik-k5`
  commands (`inspect`, `control`, `restore-eeprom`, recovery-gated
  `flash-afik --recovery`) with `--device PATH|auto` like every other command;
  `afik-k5 probe` is covered by `identify`.
- `flash-afik IMAGE` reads the bootloader beacon and writes K5 V1 (`2.*`) or K1
  (`7.03.*`), refusing an image whose vectors belong to the other radio before
  the prompt. The K1 form no longer takes the optional, validation-only
  `--recovery`/`--backup` files; `--version` is refused for a K1.

## ADR-099 — One firmware application, targets differ only in drivers

- **Date:** 2026-10-14
- **Status:** accepted, operator request.
- K5 and K1 converge on one shared application. The application logic (UI,
  receive, TX, serial service, persistence, scanning) lives in shared crates and
  runs as the same Embassy tasks on both radios. Each target crate supplies
  drivers behind shared traits and a `main` that starts them.
- K5 runs Embassy too: the same pinned executor as the K1, with a 1 MHz time
  driver on SysTick (`radio-dp32g030::time_driver`, 1 ms exception). K1 keeps
  the `py32-hal` TIM15 driver. Neither target sets an Embassy `tick-hz`
  feature, so a workspace build cannot unify two different rates.
- Firmware is built for size (`opt-level = "s"`, fat LTO, one codegen unit,
  set in `tool/build-k{5,1}.sh`), because the K5 application region is 60 KiB.
- Order: K5 executor; shared application crate from the K1's tasks; K5 drivers
  behind its traits; then features such as the EEPROM backup are written once.
- Step 2 (`7.0.14`): `radio-app` holds the interface and serial tasks
  (`interface::run`, `serial::run`), the configuration service, the operator
  place, host control, and key debounce. Targets implement `radio-app::drivers`
  (`Display`, `Keypad`, `Speaker`, `BatterySense`, `SerialLink`, `Memory`)
  and pass a `Target` with their identity and PMR446 TX
  constants. The K1 is the first target on it.

## ADR-100 — Radios are programmed as memory; the studio validates

- **Date:** 2026-10-14
- **Status:** accepted, operator decision. Supersedes `ADR-003`, `ADR-004`,
  `ADR-079` and `ADR-090` for radio programming.
- The on-radio object protocol (framing, transactions, encoding, live control)
  cost about 21 KB of K1 text, which the 60 KiB K5 cannot carry beside the
  application. Radios now speak the stock memory protocol only: hello
  `0x0514`, read `0x051B` (up to 128 bytes), write `0x051D` (8 aligned bytes,
  acknowledged with `0x051E` once stored), over an 8 KiB memory view
  (`radio-platform::memory_link`).
- The compact storage format stays: the image is the existing header plus
  packed objects, with generated banks as runtime generators. The studio
  compiles and validates it and writes it at `0x0000`; the region is
  `0x1D00` bytes. The radio keeps no copy: it validates the image in memory
  at boot and 1 s after a host's last write, and keeps only its index. Channels are read from memory when shown or tuned. Any
  failed check leaves no channels rather than part of them, and the interface
  drops the old index as soon as a host starts writing.
- Storage format 5 (operator direction), `radio-storage::radio_image`: every
  channel is stored in one bank block. A list block keeps its channels' shared
  values once, with a `fields` bitfield for what each fixed-size record carries;
  a linear block is a generated plan. The radio's RAM index is one entry per
  block (at most 48). The project and editing model stays the format 4 object
  set, compiled to format 5 for a radio; a channel in two banks is refused.
- Studio (operator direction, done in `7.0.17`): one Banks tab; a bank holds
  channels or is a generator, and every channel is in exactly one named bank.
  Projects are kept by name under `$XDG_DATA_HOME/afik/projects` and the last
  one opened is reopened. Project files stay the format 4 object set; radios
  are written the format 5 image, only by AFIK `7.0.17` or later.
- `0x1D00` holds the radio's own record of the operator's place and handset
  squelch, so the handset never rewrites the host's image. `0x1E00..0x2000`
  is vendor calibration, readable but never writable by a host.
- The studio's live tune, scan and metrics controls are dropped.


## ADR-101 — TX permission is programmed frequency ranges

- **Date:** 2026-10-14
- **Status:** accepted, operator decision. Supersedes the class bitset,
  built-in regional snapshot, grants and attestations of `TXAUTH-053`, and
  the hidden boot permission editor of Work Package 5.
- The studio programs up to 16 TX ranges: a class label, an inclusive span,
  a power ceiling in milliwatts, wide FM allowed or not, and an optional
  raster. Presets cover common US, EU/CEPT and UK services. These ranges are
  the authority: `radio-tx-policy` permits a request only inside one, and
  every range containing the frequency must permit it.
- The check is on the TX frequency, so channel and VFO mode are protected
  alike. Channels no longer carry a TX class.
- The handset can switch programmed classes off and on again, kept in the
  radio's own record, to restrict a radio lent to someone else. It can only
  narrow what was programmed.
- Hardware and calibration envelopes still apply beneath the ranges.

## ADR-102 — Two slots and dual watch, keyed as the stock firmware

- **Date:** 2026-10-14
- **Status:** accepted, operator request (`DUALW-064`).
- The radio has slots A and B, each a source of its own (channel in any bank,
  or VFO). Keys act on the selected slot and PTT always transmits on its
  frequency, whichever slot dual watch has the receiver on.
- F arms the next key as on stock and Armel: F 2 is A/B, F 3 is VFO/MR. Any
  other key after F only clears it. F twice opens Info (F alone used to).
- Dual watch is a handset setting kept in the place record. It moves the
  receiver every scan dwell between quiet slots, holds a busy one and for
  3 s after, and returns to the selected slot after any action or TX. The
  hang is chosen, not taken from a reference. The host's `dual_watch` option
  and the controller's same-bank partner are not used by the firmware.
- The place record grows to 32 bytes (version 3); versions 1 and 2 still read.

## ADR-103 — The studio syncs satellites from SatNOGS on request

- **Date:** 2026-10-14
- **Status:** accepted, operator request.
- The studio's only network access: Sync on the satellite set fetches the
  SatNOGS DB transmitter and satellite lists over HTTPS (`ureq`, rustls) on a
  worker, and caches both raw responses in `$XDG_CACHE_HOME/afik/satnogs`.
  Start-up and adding the set read the cache only; nothing fetches on its own.
- A response that does not parse leaves the cache untouched. With no cache
  the built-in satellite bank applies.
- Selection: active, FM both ways, satellite in orbit, both frequencies in
  the BK4819 datasheet coverage; the uplink tone is read from the
  description. The result is an ordinary bank, validated before any write,
  and the TX ranges still decide what may transmit.

## ADR-104 — The studio adds UK repeaters near a locator from the ETCC list

- **Date:** 2026-10-15
- **Status:** accepted, operator request.
- Sync fetches the RSGB ETCC systems list over HTTPS on a worker and caches
  the raw response in `$XDG_CACHE_HOME/afik/etcc`, as `ADR-103` does for
  satellites; nothing fetches on its own, and a response that does not parse
  leaves the cache untouched.
- Selection: repeater types AV, DM and RL, operational or at reduced output,
  carrying analogue FM, output and input both in the BK4819 datasheet
  coverage. Gateways are left out. Distance is great-circle from the centre
  of the operator's locator square to the centre of the repeater's; many
  ETCC locators are four characters, so distances are approximate.
- "Likely range" is an operator-chosen radius, not a propagation model; the
  bank takes the nearest N (default 40, about a tenth of the image), and the
  studio shows what the bank and the project would take before adding,
  refusing one that does not fit. The TX ranges still decide what may
  transmit.

## ADR-105 — FM broadcast is a bank modulation, played by the BK1080

- **Date:** 2026-10-15
- **Status:** accepted, operator request (`BCFM-066`).
- Broadcast stations are ordinary channels and banks, programmed in the
  studio like any other. Modulation value 3, `Broadcast`, marks them; it fits
  the existing two mode bits, so the image format is unchanged and an older
  radio refuses such an image rather than misreading it.
- A broadcast channel is simplex, 64–108 MHz, on the 100 kHz raster the
  reference firmware tunes the BK1080 on (`EVID-BCFM-066-001`); channel
  records and generated plans refuse anything else, so the studio refuses it
  before writing and the radio again on load. The receiver is chosen by
  modulation, not frequency, so 4 m NFM near 70 MHz stays on the BK4819.
- TX on a broadcast channel is refused by `radio-tx-policy`, which permits
  only analog FM.
- Both radios have a BK1080 (K5: shared EEPROM I2C on PA10/PA11, enable PB15
  active low; K1: its own I2C on PF5/PF6, no enable), driven from the Armel
  sources' register table.

## ADR-106 — The studio adds airport frequencies near a locator from OurAirports

- **Date:** 2026-10-15
- **Status:** accepted, operator request (`AIRPORT-068`).
- Source: OurAirports' `airports.csv` and `airport-frequencies.csv`
  (public domain, worldwide, volunteer-maintained) from its GitHub Pages
  mirror, synced on request and cached raw in
  `$XDG_CACHE_HOME/afik/ourairports` as `ADR-104` does. OpenAIP needs a key
  and is non-commercial; FAA NASR is US only.
- Lists give channel names, so a remainder of 5, 10 or 15 kHz in a 25 kHz
  block is the 8.33 kHz carrier at +0, +8.33 or +16.67 kHz (to 10 Hz), and
  20 is a two-decimal 25 kHz cut (`123.07` is 123.075). Off-grid values
  (about 30 of 27 000) and anything outside 118–136.975 MHz are dropped.
- Location is the repeaters' shared locator; airports not closed within an
  operator-chosen radius, nearest first, a shared frequency kept once for the
  nearest airport. Channels are simplex AM narrow; no TX range covers the
  airband, so they never transmit.

## ADR-107 — Only FM channels have a transmit side in the studio

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`AIRPORT-068`).
- `radio-tx-policy` transmits analog FM alone, so an AM, USB or broadcast
  channel, bank or plan only receives (`Modulation::is_receive_only`). The
  studio neither asks for nor stores its transmit frequency, offset, tones,
  power or TX flags, as `ADR-105` already did for broadcast; AM and USB keep
  their bandwidth. The image is unchanged: the record still carries the
  receive frequency as its transmit one.

## ADR-108 — A duplex channel the radio may not transmit on is heard on both sides

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`DUPLEX-069`).
- No channel type or flag: a memory channel whose transmit frequency differs
  from its receive one, and which the programmed TX ranges (after the
  handset class switch) would not let key up, listens on both. A licensed
  operator's range, such as the Marine VHF preset, makes the same channel
  an ordinary transmitting one. A flag tried first (`79c9a3b`) was reverted.
- `TxPolicy::may_transmit` answers the range, raster, bandwidth and FM
  checks, power aside. It grants nothing; PTT is still authorised in full.
- Dual watch visits stops, a slot and a side: the selected slot's two sides,
  then the other slot's, a dwell each, holding on a busy one. The transmit
  side listens with the channel's TX tone, which is what stations there
  send.

## ADR-109 — The studio adds marine channels near a locator from OpenStreetMap

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`MARINE-070`).
- No open dataset lists port and marina channels with positions except
  OpenStreetMap (ODbL): `vhf`, `contact:vhf`, four `seamark:*:channel` keys
  and `waterway:ukw_channel`, about 3 000 places worldwide, thin in the UK
  (about 110). The NGA World Port Index has no channel numbers. One Overpass
  query for the world (about a minute, 1.4 MB) is synced on request and
  cached raw in `$XDG_CACHE_HOME/afik/osm-marine`, as `ADR-106` does.
- Because the map is thin, every bank starts with a fixed core: 16, 67, 6,
  8, 72, 77, M and M2. Local channels follow, nearest place first, each
  channel once under its nearest place's name; the core works unsynced.
- Values are read for Appendix 18 voice channels (not 70, 75, 76), 31 (the
  Dutch marina channel), `nA` on a duplex channel as simplex on the ship's
  frequency, `M`/`M1`/`37A` and `M2`; a part giving a frequency is skipped.
  Duplex channels keep both frequencies, so `ADR-108` applies.

## ADR-110 — A scan checks the carrier before it tunes a channel fully

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`SCAN-071`).
- Armel's fast scan (`ENABLE_FEAT_F4HWN_SCAN_FASTER`, on by default,
  `App/app/chFrScanner.c` at `0ade03c`) scans 59 channels in about 3 s. AFIK
  set up every channel in full, redrew the screen and waited the whole
  200 ms dwell, about 12 s. Proven reference behaviour (`EVID-TX056-003`),
  so implemented as found.
- Before a scanned channel's full setup the radio retunes only the
  synthesiser, reads the RSSI and compares it with the noise floor learned
  on this pass and the squelch's open threshold (`QuietGate`). A quiet
  channel is skipped at once; a candidate gets the full setup and the dwell,
  which now counts from when the channel is tuned and drawn. A squelch that
  never shuts, a modulation change and broadcast channels take the full
  setup as before.

## ADR-110 — The studio draws with its own theme

- **Date:** 2026-10-16
- **Status:** accepted, operator request.
- egui draws every widget itself, so the studio cannot look native; a new
  toolkit would mean rewriting it. Instead `theme::apply` sets Inter
  (bundled, SIL OFL, licence beside it in `assets/fonts`), roomier spacing,
  6 px rounding and an Adwaita-like light and dark palette with one blue
  accent, following the system's preference. No theme crate is added.

## ADR-111 — Gateways and beacons come from the repeaters' ETCC list

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`GEN-072`).
- The ETCC systems list the repeaters sync already holds analogue gateways
  (`AG`) and beacons (`BN`), so neither needs a new source or cache. A
  gateway is taken as a repeater is (operational, analogue FM, receivable)
  and is simplex with its CTCSS tone. A beacon is taken whatever it sends,
  if the radio can receive its carrier: of the 32 operational, 13 (10 m to
  23 cm).
- A beacon channel is USB, receive-only (`ADR-107`), tuned 700 Hz below the
  carrier so a keyed carrier is heard as a 700 Hz tone, with a 100 Hz step.
  Beacons default to a 300 km radius, gateways to the repeaters' 50 km.

## ADR-112 — Airfields: the UK sporting set, then small fields from OurAirports

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`GEN-072`).
- OpenStreetMap was the proposed source, but of 837 UK aerodromes and
  gliding sites only about 22 carry a frequency tag. `OurAirports`, already
  cached for the Airports tab, lists 168 UK small fields with frequencies,
  so no new source is synced.
- Every bank `FLD <locator>` starts with a fixed core, as the marine bank
  does: Safetycom 135.480, gliding 129.980, 130.105, 130.130, 130.405 and
  129.905, microlight 129.830, balloon 122.480 and parachute drop zone
  130.530, as CAP 1606 version 4 (May 2019) names them. The CAA has
  withdrawn CAP 1606 in favour of its frequency assignments, and no
  downloadable list was found, so a field's own frequency is the one to use.
- Local channels are the frequencies of small airports, heliports, balloon
  ports and seaplane bases in range, nearest first, each carrier once and
  none already in the core; a nearer large airport sharing a carrier does
  not hide a field. AM, receive only (`ADR-107`).

## ADR-113 — Repeaters outside the UK come from hearham

- **Date:** 2026-10-16
- **Status:** accepted, operator request (`GEN-072`).
- hearham.com's JSON list (`/api/repeaters/v1`, about 23 000 systems, 9.5 MB,
  "free to use in your application") is the open worldwide source;
  RepeaterBook needs an application token. Synced on request and cached raw
  in `$XDG_CACHE_HOME/afik/hearham`, as `ADR-106` does.
- Taken: operational, mode FM or a mix with FM (`YSF/FM`), a non-zero
  offset, output and output plus offset both receivable; `encode` is the
  access tone. hearham lists simplex gateways, APRS digipeaters and iGates
  (such as MB7UJM on 144.800) as FM, so an entry with no offset is not a
  repeater. UK callsigns (ITU prefixes G, M and 2, which every ETCC system
  has) are left out: hearham's UK entries are an old copy of ukrepeater.net
  and IRLP's list (453 of its 1 892 ukrepeater.net callsigns are not in the
  current ETCC list), and the UK tab reads ETCC. No remaining callsign is
  in the ETCC list. 11 344 operational FM systems with an offset remain,
  before the receivable check. hearham gives no bandwidth, so every channel
  is 25 kHz.
- With UK callsigns dropped the two lists do not overlap, so one Repeaters
  tab takes both (2026-10-17, operator request): one Sync fetches each,
  and `RPT <locator>` holds the most hearable from either, with one radius,
  count and band filter. hearham's are standard coverage. Banks an older
  studio named `WRPT` are kept as they are.

## ADR-114 — Airport frequencies are chosen by how far their service is heard

- **Date:** 2026-10-17
- **Status:** accepted, operator request (`AIRPORT-068`).
- Each listed use is read as a service by its words (`A/G` as `AG`, the
  widest range winning): approach/radar 150 km, ATIS 60 km, other 50 km,
  tower/AFIS/A-G 30 km, ground/delivery/ops 10 km. The ranges are estimates
  of where a ground-level handheld hears the aircraft side, not measured.
- The area bank keeps a frequency inside both the radius and its service's
  range, ordered by distance over range, a shared carrier kept for the most
  hearable. Services are chosen by checkbox; ATIS is off by default because
  it holds a scan.
- Optionally the nearest N airports each get a bank `<ident> <name>` of all
  their tower, ground and ATIS within the radius, whatever the range; the
  area bank keeps the rest. Choices persist in
  `$XDG_STATE_HOME/afik/airport-options`.

## ADR-115 — Banks are the unit of the radio image, up to 254

- **Date:** 2026-10-17
- **Status:** accepted, operator request (`BANKS-073`).
- 16 banks was ours, not the hardware's: a `u16` membership mask, a plan
  channel identifier packing the bank into 4 bits, and RAM tables sized by
  it. The image region (7 424 bytes) is the real bound: about 105–180 banks
  of one channel.
- Image format 7: every channel is in a bank and each bank is exactly one
  block, a list or a plan, carried in its bank-table entry; the compiler
  refuses a channel in no bank and a bank holding both. The radio keeps one
  RAM entry per bank (entry offset and first channel number, 4 bytes)
  instead of a summary per block, so a bank filter is a range.
- A radio channel is named by bank and position within it; the place record
  keeps that, so rewriting other banks does not move the operator.
- `MAX_BANKS` is 254 (`0xFF` is "no bank"). Project files store the bank as a
  number (object format 5); format 4 files are upgraded one way on load.

## ADR-116 — Bank groups switch the radio's context; locations live in the project

- **Date:** 2026-10-17
- **Status:** accepted, operator request (`GROUP-075`).
- A group is a name and a set of banks; a bank may be in any number of
  groups. A location is a group that also carries a Maidenhead locator; the
  locator stays in the project file (object kind 6) and is never sent to the
  radio, which gets only the name and the banks (image format 8).
- On the radio, choosing a group narrows the source list to ALL OF the group
  (one list over its banks, which a scan walks), its banks, and the other
  groups; the operator chose this over a group being one more source.
- The image stores a group as its name and a 32-byte bank set. The radio
  keeps two bytes per group (its entry offset) and the chosen group's set;
  up to 32 groups. Loading checks only a group's layout, to keep the K5
  loader small (196 bytes of flash); a group's set is checked, and narrowed
  to the banks the image holds, when it is read.
- A start place (operator request) is the radio's own place record: the
  studio encodes it with the radio's `OperatorState` (moved to
  `radio-platform`) and writes it at `0x1D00` after the image, so it needs no
  new image field. The radio goes there when the write ends and at the next
  start, until the operator moves; the K5 view is its EEPROM, and the K1 now
  keeps a host-written record and stores it on commit. Stored in the project
  as object kind 7 (group, bank, channel by bank and position).

## ADR-117 — A bank records which generator made it, in the project only

- **Date:** 2026-10-17
- **Status:** accepted, operator request.
- A bank a generator made carries its origin: the generator (a stable code,
  `BankSource`) and the locator it was made for, or none for a bank every
  location shares (FM stations, the marine and airfield cores). It is project
  object kind 8, keyed by the bank, and is never compiled into a radio image.
- The studio rewrites, cleans up and shares generated banks by origin rather
  than by name. A project saved before this records no origins; the studio
  infers them once from the generator names when it opens one.
