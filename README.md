# uv-radio-rs

A ground-up modular Rust radio firmware and programming platform, initially
targeting the DP32G030-based Quansheng UV-K5 family.

The first two architecture work packages are complete: hardware-independent
domain types, channel plans, safe TX policy, protocol framing, transactional
storage, a library-first programmer, deterministic host simulation, bounded
multi-object operations, and the complete protocol command/error matrix. No
hardware register implementation exists yet. Work Package 3 is active and is
limited to an evidence-backed minimal DP32G030 Rust image and Renode reset-path
proof; it does not add radio peripheral behaviour or hardware flashing.

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
```

The target build uses the locked Nix compiler and standard-library sources. It
does not create a flashable radio package or access hardware.

With direnv installed, run `direnv allow` once and the same development shell
loads automatically on entry. `flake.lock` pins the Nix package set; the CI
toolchain also checks the declared Rust 1.86 minimum.

See `STATUS.md` for the current work package and verified commands,
`TASKS.md` for bounded work, and `docs/architecture.md` for crate boundaries.
