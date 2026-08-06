#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_elf="$project_root/target/thumbv6m-none-eabi/debug/radio-firmware-k1"

if [[ ! -f "$target_elf" ]]; then
  echo "K1 target image not found; run tool/build-k1.sh first" >&2
  exit 1
fi

reset_address="$(llvm-readelf -sW "$target_elf" | awk '$8 == "Reset" { address = "0x" $2 } END { print address }')"
render_address="$(llvm-nm "$target_elf" | awk '$3 ~ /display18render_key_witness$/ { address = "0x" $1 } END { print address }')"
keypad_init_address="$(llvm-nm "$target_elf" | awk '$3 ~ /11keypad_init$/ { address = "0x" $1 } END { print address }')"

if [[ -z "$reset_address" || -z "$render_address" || -z "$keypad_init_address" ]]; then
  echo "required K1 Reset/keypad-init/render symbols not found" >&2
  exit 1
fi

export AFIK_K1_ELF="$target_elf"
export AFIK_K1_RESET_ADDRESS="$reset_address"
export AFIK_K1_RENDER_ADDRESS="$render_address"
export AFIK_K1_KEYPAD_INIT_ADDRESS="$keypad_init_address"

exec renode-test \
  --results-dir "$project_root/target/k1-renode-results" \
  "$project_root/renode/k1_keypad.robot" \
  "$@"
