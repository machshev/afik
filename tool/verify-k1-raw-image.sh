#!/usr/bin/env bash
set -euo pipefail

image_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-k1.raw}"

if [[ ! -f "$image_path" ]]; then
  echo "K1 raw image not found: $image_path" >&2
  exit 1
fi

image_bytes="$(stat -c %s "$image_path")"
if (( image_bytes < 8 || image_bytes > 0x1b800 )); then
  echo "K1 raw image size is outside 8..0x1b800 bytes: $image_bytes" >&2
  exit 1
fi

read -r initial_stack reset_vector < <(od -An -v -tx4 -N8 "$image_path")
if [[ "$initial_stack" != "20004000" ]]; then
  echo "unexpected K1 raw initial stack vector: $initial_stack" >&2
  exit 1
fi
if (( (16#$reset_vector & 1) == 0 )); then
  echo "K1 raw Reset vector does not select Thumb code: $reset_vector" >&2
  exit 1
fi
if (( 16#$reset_vector < 16#08002808 || 16#$reset_vector >= 16#0801e000 )); then
  echo "K1 raw Reset vector is outside the qualified application: $reset_vector" >&2
  exit 1
fi

echo "verified K1 raw image: $image_path"
echo "  bytes: $image_bytes"
echo "  initial SP: 0x$initial_stack"
echo "  Reset vector: 0x$reset_vector"
