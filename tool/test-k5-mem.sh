#!/usr/bin/env bash
# Runs the K5's byte-loop memcpy/memmove/memset and their EABI entry points
# (crates/radio-firmware-k5/src/mem_routines.s) under qemu-arm, since no host
# test can execute Thumb-1 code. Exits non-zero, naming the failed check, if
# any check fails.
set -euo pipefail

export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"
export RUSTFLAGS="-C panic=abort"
cargo -Z build-std=core build \
  --package radio-firmware-k5 \
  --features mem-test \
  --bin k5-mem-test \
  --target thumbv6m-none-eabi \
  --release

status=0
qemu-arm target/thumbv6m-none-eabi/release/k5-mem-test || status=$?
if [[ "$status" -ne 0 ]]; then
  echo "k5 memory routines: check $status failed" >&2
  exit 1
fi
echo "k5 memory routines: all checks passed"
