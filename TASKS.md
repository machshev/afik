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

## STORE-004 — Canonical configuration image and compiler round trip

- **Status:** complete (2026-08-06)
- **Objective:** define a canonical, versioned, checksummed configuration image
  and make the authoritative host compiler emit and consume it without device
  mutation.
- **Scope:** a caller-buffer image codec in `radio-storage`, stable object-key
  ordering, image integrity and structural validation, and negotiated-capability
  validation in `radio-programmer`. Documentation and host tests are included.
  Serial protocol changes, physical flash layout, power-loss durability,
  project-file parsing, and additional channel-plan encodings are excluded.
- **Dependencies:** `DP32-003`.
- **Assumptions:** the image is an offline interchange and backup container for
  a complete logical object set, not evidence for a DP32G030 non-volatile
  storage layout; existing object payload version 1 remains unchanged.
- **Likely files:** `crates/radio-storage`, `crates/radio-programmer`, storage
  and programmer documents, `DECISIONS.md`, `RISKS.md`, and handoff files.
- **Tests required:** exact image-format vectors; byte-identical output for the
  same logical objects supplied in different orders; compiler image round trip;
  empty and maximum-bounded inputs; rejection of bad magic/version/checksum,
  truncation/trailing bytes, unordered or duplicate keys, malformed objects,
  and target-capability violations.
- **Acceptance criteria:** the `no_std` storage codec allocates no heap and
  writes only to a caller-provided buffer; one canonical image has an explicit
  length and integrity contract; decoding validates the complete image before
  exposing objects; the compiler produces stable-key order and will not accept
  an image outside negotiated target bounds; existing protocol/simulator
  behaviour remains green; physical durability claims remain explicitly open.
- **Completion notes:** added a caller-buffer `AFIK` image codec with explicit
  container and object-format versions, exact lengths, canonical object
  envelopes, and CRC-32 integrity; decoding validates the complete image,
  strict keys, and all object payloads before iteration. The compiler now sorts
  by stable key, emits images, imports them back to the same objects and
  capacity report, and rechecks storage version, object count/size, write-frame
  size, and plan-encoding support. Exact vectors, order independence, empty and
  maximum-count images, corruption and structure rejection, capability
  rejection, simulator compatibility, pinned workspace gates, and Rust 1.86
  tests all pass as recorded in `STATUS.md`. Physical persistence remains open
  in `RISK-004`.
