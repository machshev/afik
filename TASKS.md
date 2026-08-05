# Tasks

## FOUND-001 — First architecture milestone

- **Status:** complete (2026-08-05)
- **Objective:** establish the workspace and demonstrate one generated-bank
  configuration round trip through the programmer and simulated device.
- **Scope:** `radio-domain`, `radio-channel-plan`, `radio-storage`,
  `radio-tx-policy`, `radio-protocol`, `radio-programmer`, `radio-sim`, CI,
  Nix/direnv development environment, and foundation documents.
- **Dependencies:** none.
- **Assumptions:** host Rust is sufficient; no target register or packaging
  facts are required; simulated storage limits are declared capabilities.
- **Likely files:** root project files, `crates/radio-*`, `.github/workflows/ci.yml`,
  and architecture/protocol/storage/simulator/TX-policy documents.
- **Tests required:** checked domain arithmetic, plan bounds, storage transaction
  isolation, COBS/CRC framing and recovery, programmer/simulator negotiation and
  bank round trip, deterministic traces, default-deny/corrupt-policy tests.
- **Acceptance criteria:** workspace and host tests pass; embedded core crates
  are heap-free and `no_std`; framing recovers after malformed packets;
  capability negotiation and generated-bank transaction/read-back succeed;
  identical runs produce identical traces; invalid TX state fails closed.
- **Completion notes:** created the locked Nix/direnv workspace and seven
  crates; implemented checked domain types, linear generated banks, bounded
  object encoding, isolated candidate transactions, COBS/CRC framing,
  fail-closed TX permissions, offline compilation, capability negotiation,
  in-memory byte transport, simulated validation/commit/read-back, fragmented
  receive handling, and deterministic traces. All acceptance checks recorded
  in `STATUS.md` pass, including a separate Rust 1.86 test run.

## PROTO-002 — Expand object and transaction protocol coverage

- **Status:** complete (2026-08-05)
- **Objective:** add list, multiple-object read/write, abort, and explicit error
  response coverage without changing the transport contract.
- **Scope:** host protocol/programmer/simulator only.
- **Dependencies:** `FOUND-001`.
- **Assumptions:** first milestone wire-format decisions remain provisional.
- **Likely files:** protocol, programmer, simulator crates and protocol docs.
- **Tests required:** multi-object ordering, abort isolation, duplicate sequence,
  unsupported service/command, and malformed-stream recovery.
- **Acceptance criteria:** deterministic complete command/error matrix.
- **Completion notes:** bounded `LIST_OBJECTS` paging is implemented with
  generation and total consistency, strict stable-key ordering, negotiated
  host validation, deterministic simulator ordering independent of insertion,
  and protocol/simulator integration tests. `read_configuration` now reads all
  listed objects in stable-key order, checks descriptor lengths, and confirms
  the generation/listing afterward; an out-of-order three-object write is
  verified end to end. Explicit encoded abort discards a staged replacement,
  preserves active data/generation, and allows a subsequent transaction. The
  transaction-state matrix covers already-open, missing/wrong transaction, and
  commit-before-validation responses without active mutation. Candidate
  validation and capacity failures are abortable and preserve complete active
  snapshots. A table-driven encoded request matrix covers unsupported services,
  wrong-service commands, invalid flags, malformed payloads for every command
  family, out-of-range listing, and missing objects without active mutation.
  Immediate identical retries replay one cached response without repeating
  mutation, while conflicting sequence reuse is rejected explicitly. Combined
  stream tests recover deterministically from unknown wire values, invalid CRC,
  malformed COBS, and overflow before accepting a one-byte-fragmented frame.
  All acceptance checks recorded in `STATUS.md` pass on the pinned environment
  and Rust 1.86 minimum toolchain.

## DP32-003 — Minimal DP32G030 Rust image and Renode boot

- **Status:** complete (2026-08-05)
- **Objective:** establish the smallest evidence-backed DP32G030 target image
  and prove its reset path in a deterministic Renode machine.
- **Scope:** source-backed CPU and memory facts, one `no_std`/heap-free
  `thumbv6m-none-eabi` image, its linker/vector-table contract, a CPU-and-memory
  Renode platform, an automated boot-sentinel test, and pinned build/test
  integration. No radio peripheral, board I/O, flashing, or packaging logic.
- **Dependencies:** `PROTO-002`.
- **Assumptions:** the DP32G030 v1.23 reference manual is sufficient evidence
  for the core and address ranges; Arm's Cortex-M reset contract is applicable;
  a RAM sentinel is a simulation-only boot observation and not a hardware
  register claim.
- **Likely files:** `Cargo.toml`, `flake.nix`, `.cargo/`, a new DP32G030 target
  crate, `renode/`, CI, and architecture/hardware/simulator documents.
- **Tests required:** host workspace checks remain green; the target image
  builds with the pinned Rust toolchain; ELF/vector/section bounds are checked;
  Renode starts from the vector table and observes the exact RAM sentinel; a
  negative or pre-start check demonstrates the assertion is behavioural.
- **Acceptance criteria:** every encoded hardware fact has a cited source and
  confidence; the target crate is embedded-only, heap-free, and independent of
  host crates; the minimal image fits the evidenced flash/RAM ranges; the
  Renode model contains no invented peripheral behaviour; the automated boot
  proof is deterministic and runs from the pinned environment; no hardware is
  flashed.
- **Completion notes:** recorded the DP32G030 v1.23 core, byte-order, memory-map,
  and Arm reset-vector evidence boundary; added a standalone feature-gated
  `no_std`/heap-free target crate with an exact two-entry vector table, Reset
  handler, bounded linker script, and static ELF contract verifier; added a
  CPU/flash/RAM-only Renode platform whose automated test proves the sentinel
  is zero before execution and written only after reset-from-vector startup;
  and gated the minimum Rust target build plus locked-Nix target verification
  and Renode proof in CI. Pinned host checks, Rust 1.86 host and target builds,
  static image verification, and three repeated Renode runs all pass as
  recorded in `STATUS.md`. No peripheral behaviour was modeled and no hardware
  was flashed.
