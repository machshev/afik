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
