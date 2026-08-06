#!/usr/bin/env bash
set -euo pipefail

force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
  shift
fi

elf_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030}"
raw_path="${2:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030-k5-v1.raw}"

if [[ $# -gt 2 ]]; then
  echo "usage: $0 [--force] [ELF [RAW]]" >&2
  exit 2
fi
if [[ ! -f "$elf_path" ]]; then
  echo "target ELF not found: $elf_path" >&2
  exit 1
fi
if [[ -e "$raw_path" && "$force" -ne 1 ]]; then
  echo "refusing to replace existing raw image without --force: $raw_path" >&2
  exit 1
fi
if ! command -v llvm-objcopy >/dev/null; then
  echo "required tool not found in pinned environment: llvm-objcopy" >&2
  exit 1
fi

tool/verify-dp32g030-image.sh "$elf_path"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
temporary_raw="$temporary_directory/k5-v1.raw"

llvm-objcopy \
  --output-target=binary \
  --gap-fill=0xff \
  --pad-to=0xf000 \
  "$elf_path" \
  "$temporary_raw"

if [[ "$(stat -c %s "$temporary_raw")" -ne $((0xf000)) ]]; then
  echo "packaged K5 V1 image is not exactly 0xF000 bytes" >&2
  exit 1
fi

mv -- "$temporary_raw" "$raw_path"
echo "packaged K5 V1 application: $raw_path"
echo "  bytes: 0xF000"
sha256sum -- "$raw_path"
