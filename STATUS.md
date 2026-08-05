# Project status

## Current work package

**Work Package 3 — Minimal DP32G030 Rust image and Renode boot (`DP32-003`) is
active.**

The package is limited to an evidence-backed Cortex-M0 reset path, bounded
flash/RAM image, and deterministic Renode boot sentinel. Peripheral models,
board I/O, packaging, and hardware flashing are out of scope.

## State

- Repository foundation and first architecture milestone: complete.
- Work Package 2 programmer and simulator protocol loop: complete.
- Work Package 3 minimal target boot proof: in progress.
- `DP32-003` CPU, byte-order, flash/RAM, and reset-vector evidence contract:
  complete.
- `PROTO-002`: complete.
- Bounded, paged `LIST_OBJECTS`: complete.
- Out-of-order multi-object write/list/read-back: complete.
- Explicit abort isolation and subsequent transaction recovery: complete.
- Transaction state errors preserve active data: complete.
- Candidate validation and capacity errors preserve active data: complete.
- Unsupported service/command, malformed payload, and missing-object matrix:
  complete.
- Bounded duplicate-sequence replay and conflict rejection: complete.
- Fragmented and malformed stream recovery: complete.
- Next smallest task: add a pinned, embedded-only `thumbv6m-none-eabi` target
  crate whose linker/vector-table contract is statically checked against the
  accepted flash and RAM ranges.

## Exit criteria

- The target image uses only source-backed Cortex-M0 and memory-map facts.
- A pinned `thumbv6m-none-eabi` build emits a bounded, heap-free image with a
  valid initial stack pointer and reset vector.
- A minimal Renode Cortex-M0/flash/RAM platform boots that exact ELF and an
  automated test observes the expected RAM sentinel.
- Host workspace checks remain green and target/Renode commands are recorded.
- No peripheral behaviour is invented and no hardware is flashed.

## Last verification

Verified 2026-08-05:

- `nix flake check path:. --no-build` — passed on `x86_64-linux`; incompatible
  `aarch64-linux` output was evaluation-skipped by Nix.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all seven crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 27 unit tests and
  all doc tests, 0 failures.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc CARGO_TARGET_DIR=/tmp/afik-rust-1.86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 27 unit tests and all doc tests on Rust/Cargo 1.86.0.
- Renode and hardware-in-loop tests — not run because target and Renode models
  do not exist in this work package.
