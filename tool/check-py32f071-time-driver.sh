#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "$0")/.." && pwd)"

: "${RUST_SRC_PATH:?run this check inside the pinned Nix development shell}"
export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"

exec cargo clippy --offline -Z build-std=core \
  --manifest-path "$project_root/Cargo.toml" \
  --package radio-firmware-k1 \
  --lib \
  --features py32f071-time-driver \
  --target thumbv6m-none-eabi \
  -- \
  -D warnings
