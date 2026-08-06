#!/usr/bin/env bash
set -euo pipefail

elf_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030}"
raw_path="${2:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030-k5-v1.raw}"

if [[ $# -gt 2 ]]; then
  echo "usage: $0 [ELF [RAW]]" >&2
  exit 2
fi
if [[ ! -f "$raw_path" ]]; then
  echo "packaged K5 V1 image not found: $raw_path" >&2
  exit 1
fi

tool/verify-dp32g030-image.sh "$elf_path"

if [[ "$(stat -c %s "$raw_path")" -ne $((0xf000)) ]]; then
  echo "packaged K5 V1 image is not exactly 0xF000 bytes" >&2
  exit 1
fi

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
expected_raw="$temporary_directory/expected.raw"
llvm-objcopy \
  --output-target=binary \
  --gap-fill=0xff \
  --pad-to=0xf000 \
  "$elf_path" \
  "$expected_raw"

if ! cmp --silent -- "$expected_raw" "$raw_path"; then
  echo "packaged K5 V1 image does not exactly match the verified ELF" >&2
  exit 1
fi

read -r initial_stack reset_vector < <(od -An -v -tx4 -N8 "$raw_path")
if [[ "$initial_stack" != "20004000" ]]; then
  echo "unexpected packaged initial stack vector: $initial_stack" >&2
  exit 1
fi
if (( (16#$reset_vector & 1) == 0 )); then
  echo "packaged Reset vector does not select Thumb code: $reset_vector" >&2
  exit 1
fi
if (( 16#$reset_vector < 8 || 16#$reset_vector >= 16#0000f000 )); then
  echo "packaged Reset vector is outside the K5 V1 application: $reset_vector" >&2
  exit 1
fi

echo "verified K5 V1 application package: $raw_path"
echo "  bytes: 0xF000"
echo "  initial SP: 0x$initial_stack"
echo "  Reset vector: 0x$reset_vector"
sha256sum -- "$raw_path"
