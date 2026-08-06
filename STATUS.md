# Project status

## Current work package

**Work Package 11 — APRS receive feasibility and repeater discovery
(`APRS-011`) is active.**

The package will establish an evidence-backed physical receive-chain verdict
and implement only the hardware-independent, bounded parsing/discovery layers
supported by primary AX.25/APRS specifications. RF/audio/bit recovery remains
outside the code boundary unless separately evidenced. Discovered repeater data
is untrusted receive-only information and cannot create TX authority.

## State

- Repository foundation and first architecture milestone: complete.
- Work Package 2 programmer and simulator protocol loop: complete.
- Work Package 3 minimal target boot proof: complete.
- Work Package 4 canonical image/compiler round trip: complete.
- Work Package 5 simulator-first boot UI and hidden TX permissions: complete.
- Work Package 6 BK4819 receive path and token-gated TX boundary: complete.
- Work Package 7 channel activation and deterministic scanning: complete.
- Work Package 8 programmer CLI: complete.
- Work Package 9 programmer GUI: complete.
- Work Package 10 Frequency Copy research: complete.
- Work Package 11 APRS receive feasibility and repeater discovery: active.
- `UI-005` logical key edges, bounded semantic views, exact boot-only entry,
  release gate, draft editor, and checked persistence action: complete.
- `UI-005` separate persisted/active policy simulation, deterministic timed
  trace, corrupt-state denial, and reboot-only activation proof: complete.
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
- `RF-006` official product/datasheet provenance, mirrored-application-note
  boundary, interface/frequency/status/mode facts, low-confidence command-plan
  inference, published-band contradiction, and required experiments: complete.
- `RF-006` heap-free driver, exact command ordering/status decoding,
  class-bound capability token, fail-closed state recovery, deterministic RF
  simulation, and mismatch/failure trace proofs: complete.
- `SCAN-007` checked activation/navigation, explicit timer-token dwell/hold
  state, stale expiry safety, scan-time TX denial, selected-state policy bundle,
  and repeatable integrated control/RF traces: complete.
- `CLI-008` snapshot backup encoding, strict simulator/serial command front end,
  bounded safe files, stable output/status, transactional write/restore with
  read-back, and binary tests: complete.
- `GUI-009` shared verified workflows/serial transport, persistent local
  session, bounded loopback HTTP, responsive object workflow, canonical
  downloads/uploads, confirmed token-gated mutation, and binary tests:
  complete.
- `FREQ-010` FCC workflow provenance, Air Copy separation, bounded observation
  matrix, receive-only candidate/state proposal, storage/TX boundary,
  experiment plan, and hardware-command defer verdict: complete.
- Next smallest task: record primary AX.25/APRS framing and voice-repeater
  advertisement facts, then map the complete physical receive chain and its
  unverified BK4819/board/MCU boundaries before adding parser behavior.

## Completed Work Package 10 exit criteria

- The FCC-filed manual's exhibit identity, checksum, exact Fast Copy controls,
  displayed/saved outputs, known-frequency CTCSS/DCS scan, and distinct
  transmitting Air Copy workflow are recorded with scope and confidence.
- Beken's advertised scan/signalling capabilities are separated from the
  machine-translated revision-unverified register description and one
  non-independent descendant firmware observation. Unexplained constants are
  named and remain prohibited from production or physical simulation.
- The feasibility design inventories observable and non-observable properties,
  specifies a heap-free bounded receive-only candidate and explicit-input/token
  state flow, preserves signalling uncertainty, and treats cleanup failure as a
  fault latch.
- Capture cannot become `ActiveChannel` or mint TX authority. Any future save is
  separately confirmed, requires a new receive-only storage representation,
  and remains `TxClass::Never`; no RX-to-TX inference is permitted.
- Receive-only equipment/recovery, register, frequency/level, false-lock,
  CTCSS/DCS, cancellation, stale-result, bus-fault, and cleanup experiments plus
  future deterministic tests are specified. The explicit verdict is
  design-ready but hardware-command blocked under `RISK-011`.
- No behavioral code, register command, target adapter, register-level
  simulator, automatic storage mutation, transmission, flashing, or physical
  success claim was added.

## Work Package 10 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 82 unit tests and all
  doc tests, 0 failures.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-freq-010-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 82 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 9 exit criteria

- `radio-programmer` owns verified write, canonical backup, validated restore,
  and exact generation/object read-back mismatch handling. Both front ends use
  the shared `radio-programmer-serial` Linux path/baud adapter.
- `afik-programmer-gui` retains one explicitly selected simulator or serial
  session. Capability, generation, and object views refresh from that same
  session; simulator write/backup/restore behavior is repeatable.
- The dependency-free server accepts only loopback IP addresses, caps headers
  at 16 KiB and bodies at 8 MiB, rejects ambiguous/chunked framing, and survives
  client I/O failures without ending the selected device session.
- Generated-bank text is strict and capped at 64 KiB. Compile and backup return
  canonical downloads; restore accepts bounded uploaded bytes. No endpoint
  accepts a server filesystem path or exposes a raw write.
- Responsive embedded assets provide readable capabilities, object listing,
  project editing, status, downloads, and deliberate write/restore confirmation.
  Mutation also requires a random 256-bit per-process token and an explicit
  replacement header; CSP/no-store/no-sniff responses preserve the local
  same-origin boundary without claiming authentication.
- Model, endpoint, parser, asset, launcher, shared-workflow, mismatch, serial,
  and CLI-regression tests pass. `RISK-009` and `RISK-010` remain open; no target
  UART, physical programming, remote service, firmware flashing, or security
  capability is claimed.

## Work Package 9 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 82 unit/integration
  tests and all doc tests, 0 failures.
- `nix develop --command cargo run -q -p radio-programmer-gui --bin afik-programmer-gui -- --help`
  — passed with the stable help document.
- `nix develop --command cargo run -q -p radio-programmer-gui --bin afik-programmer-gui -- --version`
  — passed with `afik-programmer-gui 0.1.0`.
- The process-level binary test also confirms exact help/version output and
  exit status 2 for a rejected `0.0.0.0:9000` listener.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-gui-009-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 82 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.
- The first minimum-toolchain audit exposed that its sandbox prohibited two
  test-created TCP sockets. HTTP parsing/serialization tests were made generic
  over deterministic byte streams, while production remains a loopback
  `TcpListener`; the final command above then passed. One intermediate retry
  pointed `RUSTDOC` at a nonexistent store path and stopped before doc tests;
  correcting that invocation required no code change.

## Completed Work Package 8 exit criteria

- `ConfigurationSnapshot` validates canonical order and supported objects,
  reports exact capacity, and emits the shared canonical image without front-end
  reimplementation.
- `afik-programmer` supports info, list, compile, write, backup, and restore over
  an explicitly selected fresh simulator or serial device path plus baud.
- CLI parsing validates backend exclusivity, supported baud, bounded bank
  fields, class names, command arity, and force semantics. Usage and operation
  errors have distinct stable exit codes.
- Input streaming is capped at 8 MiB. Compile/backup refuse existing outputs by
  default and replace only under explicit `--force`.
- Write and restore compile/decode in `radio-programmer`, use object-level
  transactions, then require exact generation and object read-back.
- The serial adapter adds no unsafe code, raw command, discovery/default, target
  UART assumption, or physical interoperability claim; `RISK-009` remains open.

## Work Package 8 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 68 unit/integration
  tests and all doc tests, 0 failures.
- `nix develop path:. -c cargo run --quiet --package radio-programmer-cli --bin afik-programmer -- --sim info`
  — passed with the exact six negotiated simulator capability fields.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-cli-008-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 68 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 7 exit criteria

- `radio-channel-control` is hardware-independent, `no_std`, heap-free,
  allocation-free, bounded, and passes a `thumbv6m-none-eabi` warning-denied
  lint with its embedded dependencies.
- Initial/manual indexes are checked before mutation; navigation and scan
  advancement wrap exactly; each update emits at most one activation.
- The controller owns no clock. Non-zero integer dwell/hold configuration,
  fresh bounded timer tokens, early-deadline enforcement in the host adapter,
  and stale/cancelled token tests make scheduling explicit and deterministic.
- Open squelch restarts/rearms hold without retuning; a closed hold expiry
  advances once and rearms dwell. Signal values remain logical inputs.
- Scanning cannot obtain TX authority. Selected state goes through `TxPolicy`,
  carries the exact class-bound token, and reaches logical TX only through the
  BK4819 driver; denial leaves the RF trace unchanged.
- Identical timed scan, hold, stop, and TX scripts produce identical control and
  RF traces. No physical timing, signal, target peripheral, or RF claim was
  added.

## Work Package 7 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 60 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-channel-control --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-channel-control` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-scan-007-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 60 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 6 exit criteria

- `radio-bk4819` is hardware-adapter-independent, `no_std`, heap-free, uses
  checked integer units, and passes a `thumbv6m-none-eabi` warning-denied lint.
- Register addresses, fields, formulas, combined-mode inferences, provenance,
  confidence, contradictory bands, and required physical experiments are
  recorded before and alongside the implementation.
- Exact frequency packing, standby-first receive/TX ordering, status decoding,
  state rejection, stop/recovery, and failure at every logical read/write step
  are tested.
- `TxAuthorisation` carries its approved class. The driver's only TX-mode path
  borrows a token, checks an exact channel-class match before any write, and
  cannot complete after a fault without explicit neutral-mode recovery.
- Identical virtual-time RF scripts produce identical traces. Mismatched
  authority emits no register operation or TX event, and a failed final
  TX-mode write emits no completed TX event.
- No physical bus, initialization sequence, board RF control, external PA,
  physical receive result, flashing, or on-air transmission was added.

## Work Package 6 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 50 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-bk4819 --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-bk4819` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-rf-006-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 50 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 5 exit criteria

- `radio-ui` is `no_std`, heap-free, allocation-free, hardware-independent, and
  passes a `thumbv6m-none-eabi` lint build.
- Only the exact initial logical `Menu+Back` set enters the hidden editor;
  incomplete, additional, and post-boot keys cannot enter, and all held keys
  must be released before editing.
- The fixed selectable order contains all six authorisable classes and excludes
  `Never`; bounded views expose selection, enabled/changed state, save errors,
  and saved generation without physical display assumptions.
- Cancel emits no record. Deliberate save emits one next-generation redundant
  CRC-protected record, while generation exhaustion emits none.
- The UI never owns live policy or constructs authorization. Simulator save
  changes only persisted bytes; only a validated reboot changes active policy.
- Corrupt persistence defaults both editor draft and active policy to deny-all;
  identical timed scripts produce identical traces and bytes.
- No physical key/display behavior, non-volatile write, serial permission
  object, hardware register access, or TX driver was added.

## Last verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 42 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-ui --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-ui` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-ui-005-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 42 unit tests and all doc tests on Rust/Cargo 1.86.0.

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

## Work Package 4 verification

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
