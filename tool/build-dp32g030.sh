#!/usr/bin/env bash
set -euo pipefail

export RUSTC_BOOTSTRAP=1
# Nix exposes the pinned standard-library workspace directly rather than in
# rustup's sysroot layout expected by Cargo's unstable build-std implementation.
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
# Use unwrapped LLD: Nix's generic ld.lld wrapper injects host Linux flags.
# Rustup toolchains use their bundled default linker outside this local wrapper.
export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"
exec cargo -Z build-std=core build \
  --package radio-firmware-dp32g030 \
  --features firmware \
  --bin radio-firmware-dp32g030 \
  --target thumbv6m-none-eabi \
  "$@"
