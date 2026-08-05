# uv-radio-rs

A ground-up modular Rust radio firmware and programming platform, initially
targeting the DP32G030-based Quansheng UV-K5 family.

The first two architecture work packages are complete: hardware-independent
domain types, channel plans, safe TX policy, protocol framing, transactional
storage, a library-first programmer, deterministic host simulation, bounded
multi-object operations, and the complete protocol command/error matrix. No
target firmware or hardware register implementation exists yet, and the next
implementation work package has not been assigned.

## Host checks

```sh
nix develop
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

With direnv installed, run `direnv allow` once and the same development shell
loads automatically on entry. `flake.lock` pins the Nix package set; the CI
toolchain also checks the declared Rust 1.86 minimum.

See `STATUS.md` for the current work package and verified commands,
`TASKS.md` for bounded work, and `docs/architecture.md` for crate boundaries.
