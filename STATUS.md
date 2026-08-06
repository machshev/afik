# Project status

## Current work package

**Work Package 5 — Simulator-first boot UI and hidden TX permissions
(`UI-005`) is active.**

The package is limited to hardware-independent logical key input, bounded
semantic display views, the boot-only TX-permission editor, and deterministic
host simulation. It does not define physical keypad/display behavior, target
registers, non-volatile writes, or a transmit driver.

## State

- Repository foundation and first architecture milestone: complete.
- Work Package 2 programmer and simulator protocol loop: complete.
- Work Package 3 minimal target boot proof: complete.
- Work Package 4 canonical image/compiler round trip: complete.
- Work Package 5 simulator-first boot UI and hidden TX permissions: active.
- `STORE-004` allocation-free image codec, exact version/length/CRC contract,
  complete pre-iteration validation, and maximum-count bound: complete.
- `STORE-004` canonical compiler ordering, image round trip, capacity report,
  and negotiated-capability revalidation: complete.
- `DP32-003` CPU, byte-order, flash/RAM, and reset-vector evidence contract:
  complete.
- `DP32-003` target crate, minimum vector/Reset image, and static ELF bounds
  verification: complete.
- `DP32-003` minimal Renode platform and pre-start/post-start boot-sentinel
  test: complete.
- `DP32-003` Rust 1.86 target build and locked-Nix target/Renode CI gates:
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
- Next smallest task: add the hardware-independent `radio-ui` crate and prove
  the exact boot gesture, release gate, bounded views, edit/cancel/save paths,
  generation handling, and fail-closed persisted-state loading in unit tests.

## Completed Work Package 4 exit criteria

- The `no_std` storage codec has no heap dependency and encodes only into a
  caller-provided buffer.
- The `AFIK` image header, object envelopes, versions, lengths, canonical key
  order, and CRC-32 coverage are explicit and tested with an exact byte vector.
- Decoding checks the complete checksum, structure, order, and every object
  before returning an iterable image.
- Compiler output is byte-identical for equal logical projects regardless of
  insertion order; importing an image reconstructs the same objects and
  capacity report after enforcing every negotiated target bound.
- Empty and maximum-`u16`-count images pass; corrupt, truncated, trailing,
  reordered, duplicate, malformed, unsupported-version, and over-capacity
  images fail explicitly.
- Existing protocol and simulator behaviour remains green. The image remains
  offline and logical; physical layout and durability stay open in `RISK-004`.

## Last verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 35 unit tests and
  all doc tests, 0 failures.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-store-004-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 35 unit tests and all doc tests on Rust/Cargo 1.86.0.
- An initial minimum-toolchain invocation pinned `RUSTC` but not `RUSTDOC`;
  all 35 unit tests passed, then ambient Rustdoc 1.97 rejected the Rust 1.86
  artifacts as compiler-incompatible. Pinning both tools as above fixed the
  environment mismatch without a code change.

## Completed Work Package 3 exit criteria

- The target image uses only source-backed Cortex-M0 and memory-map facts.
- A pinned `thumbv6m-none-eabi` build emits a bounded, heap-free image with a
  valid initial stack pointer and reset vector.
- A minimal Renode Cortex-M0/flash/RAM platform boots that exact ELF and an
  automated test observes the expected RAM sentinel.
- Host workspace checks remain green and target/Renode commands are recorded.
- No peripheral behaviour is invented and no hardware is flashed.

## Work Package 3 verification

Verified 2026-08-05:

- `nix flake check path:. --no-build` — passed on `x86_64-linux`; incompatible
  `aarch64-linux` output was evaluation-skipped by Nix.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"; cargo clippy -Z build-std=core --package radio-firmware-dp32g030 --features firmware --bin radio-firmware-dp32g030 --target thumbv6m-none-eabi -- -D warnings'`
  — passed for the embedded target image.
- `nix develop path:. -c cargo test --workspace` — passed: 27 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c tool/build-dp32g030.sh` — passed.
- `nix develop path:. -c tool/verify-dp32g030-image.sh` — passed: initial SP
  `0x20004000`, Reset vector `0x00000101`, boot sentinel `0x20000000`, and
  flash image end `0x00000268`.
- `nix develop path:. -c tool/test-renode.sh --repeat 3` — passed all three
  reset-from-vector boot-sentinel iterations.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc CARGO_TARGET_DIR=/tmp/afik-host-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 27 unit tests and all doc tests on Rust/Cargo 1.86.0.
- `env RUSTC=/tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/rustc CARGO_HOME=/tmp/afik-cargo-home-1-86 CARGO_TARGET_DIR=/tmp/afik-rust-1-86-target /tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/cargo build --package radio-firmware-dp32g030 --features firmware --bin radio-firmware-dp32g030 --target thumbv6m-none-eabi`
  — passed on Rust/Cargo 1.86.0.
- `nix develop path:. -c tool/verify-dp32g030-image.sh /tmp/afik-rust-1-86-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030`
  — passed: initial SP `0x20004000`, Reset vector `0x000000cd`, boot sentinel
  `0x20000000`, and flash image end `0x00000220`.
- Hardware-in-loop tests — not run; flashing and physical-silicon claims were
  outside `DP32-003`, and recovery/package evidence remains open in `RISKS.md`.
