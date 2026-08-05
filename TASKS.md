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

- **Status:** in progress (2026-08-05)
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
  remaining transaction and command/error matrix is still open.
