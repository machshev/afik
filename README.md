# uv-radio-rs

A ground-up modular Rust radio firmware and programming platform, initially
targeting the DP32G030-based Quansheng UV-K5 family.

The first ten architecture work packages are complete. They provide
hardware-independent domain types, channel plans, safe TX policy, protocol
framing, transactional storage, canonical logical images, a library-first
programmer, deterministic host simulation, the complete protocol command/error
matrix, and an evidence-backed minimal DP32G030 Rust image with a Renode
reset-path proof. They also provide an allocation-free boot-only TX-permission
UI, an evidence-bounded post-initialization BK4819 command driver, explicit-input
channel activation/scanning, and complete CLI and local-GUI programmer front
ends. Frequency Copy research defines a receive-only candidate and
experiment-gated design while deferring unverified BK4819 scan commands. No
target hardware-register adapter exists yet. These are not physical
flash, display, keypad, timer, bus, board-RF, or on-air implementations. No
hardware flashing has been added.

## Programmer CLI

The CLI exposes the complete currently supported programmer surface over a
fresh deterministic simulator or an explicit Linux serial path and baud:

```sh
nix develop
cargo run --package radio-programmer-cli --bin afik-programmer -- --help
cargo run --package radio-programmer-cli --bin afik-programmer -- --sim info
cargo run --package radio-programmer-cli --bin afik-programmer -- \
  --sim write --bank '1:PMR446:446006250:12500:16:licence-free'
```

`compile`, `write`, `backup`, and `restore` use negotiated capabilities,
canonical images, validated transactions, and read-back verification from the
programmer library. Serial use requires both `--device PATH` and `--baud BAUD`;
no physical device/baud default or interoperability claim exists yet.

## Local programmer GUI

The GUI retains one simulator or explicit serial session and serves responsive
embedded assets only on a loopback IP address:

```sh
cargo run --package radio-programmer-gui --bin afik-programmer-gui -- --sim
```

It exposes the same negotiated capability, object, compile, verified write,
backup, and restore workflows as the programmer library. Mutation requires a
per-process token plus explicit replacement confirmation. This reduces
accidental local mutation but is not authentication; the GUI must not be bound
remotely or treated as a shared service. See `docs/programmer-gui.md`.

## Host checks

```sh
nix develop
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Minimal DP32G030 target checks

```sh
nix develop
tool/build-dp32g030.sh
tool/verify-dp32g030-image.sh
tool/test-renode.sh
```

The target build uses the locked Nix compiler and standard-library sources. It
does not create a flashable radio package or access hardware.

With direnv installed, run `direnv allow` once and the same development shell
loads automatically on entry. `flake.lock` pins the Nix package set; the CI
toolchain also checks the declared Rust 1.86 minimum.

See `STATUS.md` for the current work package and verified commands,
`TASKS.md` for bounded work, and `docs/architecture.md` for crate boundaries.
