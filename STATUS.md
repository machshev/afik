# Project status

## Current work package

**No implementation work package is currently active.**

Work Package 2 — Programmer and simulator protocol loop is complete. The next
work package and stable task ID must be defined before additional implementation.

## State

- Repository foundation and first architecture milestone: complete.
- Work Package 2 programmer and simulator protocol loop: complete.
- Target hardware support: not started.
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
- Next smallest task: define the next bounded work package and stable task ID in
  `TASKS.md` and activate it here before changing implementation.

## Exit criteria

- `LIST_OBJECTS` has deterministic bounded ordering.
- Multiple objects write and read back without insertion-order ambiguity.
- Explicit abort and every transaction error preserve active storage.
- Unsupported service/command and malformed payload responses are covered.
- Fragmented and malformed streams recover deterministically.

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
