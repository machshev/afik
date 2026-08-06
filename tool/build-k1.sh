#!/usr/bin/env bash
set -euo pipefail

export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"
export RUSTFLAGS="-C link-arg=-Tcrates/radio-firmware-k1/link.x -C link-arg=-z -C link-arg=max-page-size=4 -C panic=abort"

exec cargo -Z build-std=core build \
  --package radio-firmware-k1 \
  --features firmware \
  --bin radio-firmware-k1 \
  --target thumbv6m-none-eabi \
  "$@"
