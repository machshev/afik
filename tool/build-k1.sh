#!/usr/bin/env bash
set -euo pipefail

export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"
export RUSTFLAGS="-C link-arg=-Tlink.x -C link-arg=-L$PWD/crates/radio-firmware-k1 -C link-arg=-Tstack-headroom.x -C link-arg=-z -C link-arg=max-page-size=4 -C panic=abort"

cargo -Z build-std=core build --offline \
  --package radio-firmware-k1 \
  --features firmware \
  --bin radio-firmware-k1 \
  --target thumbv6m-none-eabi \
  "$@"

# A release build is what gets flashed, so it also produces the raw image that
# `afik-flasher flash-afik` picks up by default.
for argument in "$@"; do
  if [[ "$argument" == "--release" ]]; then
    tool/package-k1-image.sh --force
    break
  fi
done
