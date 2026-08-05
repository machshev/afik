#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_elf="$project_root/target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030"

if [[ ! -f "$target_elf" ]]; then
  echo "target image not found; run tool/build-dp32g030.sh first" >&2
  exit 1
fi

export AFIK_DP32_ELF="$target_elf"
exec renode-test \
  --results-dir "$project_root/target/renode-results" \
  "$project_root/renode/dp32g030_boot.robot" \
  "$@"
