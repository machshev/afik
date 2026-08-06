#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$project_root/vendor/py32-hal/Cargo.toml"

: "${RUST_SRC_PATH:?run this check inside the pinned Nix development shell}"
export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$project_root/target/py32-hal-check}"

for chip in py32f071c1b py32f071k18 py32f071k1b py32f071r1b; do
  cargo check --offline -Z build-std=core \
    --manifest-path "$manifest" \
    --target thumbv6m-none-eabi \
    --no-default-features \
    --features "$chip"
done
