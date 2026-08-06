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
- K1 support is limited to the independently implemented raw recovery/backup
  path. No K1 AFIK application image or target contract exists, so the generic
  flasher must reject K1 AFIK flashing rather than route it to the K5 image
  path.

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
- K1 application flashing remains unavailable. The only physical image
  permitted by `K1HIL-015` is the independently validated, unchanged stock
  recovery image, followed by normal-mode backup comparison.
