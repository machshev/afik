#!/usr/bin/env bash
set -euo pipefail

force=0
if [[ "${1:-}" == "--force" ]]; then
  force=1
  shift
fi

elf_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-k1}"
raw_path="${2:-target/thumbv6m-none-eabi/debug/radio-firmware-k1.raw}"

if [[ $# -gt 2 ]]; then
  echo "usage: $0 [--force] [ELF [RAW]]" >&2
  exit 2
fi
if [[ ! -f "$elf_path" ]]; then
  echo "K1 target ELF not found: $elf_path" >&2
  exit 1
fi
if [[ -e "$raw_path" && "$force" -ne 1 ]]; then
  echo "refusing to replace existing K1 raw image without --force: $raw_path" >&2
  exit 1
fi
if ! command -v llvm-objcopy >/dev/null; then
  echo "required tool not found in pinned environment: llvm-objcopy" >&2
  exit 1
fi

tool/verify-k1-image.sh "$elf_path"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
temporary_raw="$temporary_directory/k1.raw"

llvm-objcopy \
  --output-target=binary \
  --gap-fill=0xff \
  "$elf_path" \
  "$temporary_raw"

tool/verify-k1-raw-image.sh "$temporary_raw"

mv -- "$temporary_raw" "$raw_path"
echo "packaged K1 application: $raw_path"
echo "  bytes: $(stat -c %s "$raw_path")"
sha256sum -- "$raw_path"
