#!/usr/bin/env bash
set -euo pipefail

export RUSTC_BOOTSTRAP=1
# Nix exposes the pinned standard-library workspace directly rather than in
# rustup's sysroot layout expected by Cargo's unstable build-std implementation.
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
# Use unwrapped LLD: Nix's generic ld.lld wrapper injects host Linux flags.
export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"
# This image owns its own linker script, so the workspace default is replaced
# rather than added to.
export RUSTFLAGS="-C link-arg=-Tcrates/radio-firmware-k5/link.x -C link-arg=-z -C link-arg=max-page-size=4 -C panic=abort"
cargo -Z build-std=core build \
  --package radio-firmware-k5 \
  --features firmware \
  --bin radio-firmware-k5 \
  --target thumbv6m-none-eabi \
  "$@"

# A release build is what gets flashed, so it also produces the raw image that
# `afik-flasher flash-afik` picks up by default.
for argument in "$@"; do
  if [[ "$argument" == "--release" ]]; then
    tool/package-k5-image.sh --force
    break
  fi
done
