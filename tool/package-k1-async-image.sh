#!/usr/bin/env bash
set -euo pipefail

force=0
if [[ "${1:-}" == "--force" ]]; then force=1; shift; fi
elf_path="${1:-target/thumbv6m-none-eabi/release/radio-firmware-k1-async}"
raw_path="${2:-target/thumbv6m-none-eabi/release/radio-firmware-k1-async.raw}"
[[ $# -le 2 ]] || { echo "usage: $0 [--force] [ELF [RAW]]" >&2; exit 2; }
[[ -f "$elf_path" ]] || { echo "K1 async ELF not found: $elf_path" >&2; exit 1; }
if [[ -e "$raw_path" && "$force" -ne 1 ]]; then
  echo "refusing to replace existing K1 async raw image without --force: $raw_path" >&2; exit 1
fi

tool/verify-k1-async-image.sh "$elf_path"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
temporary_raw="$temporary_directory/k1-async.raw"
llvm-objcopy --output-target=binary --gap-fill=0xff "$elf_path" "$temporary_raw"
tool/verify-k1-raw-image.sh "$temporary_raw"
mv -- "$temporary_raw" "$raw_path"
echo "packaged K1 async application: $raw_path"
echo "  bytes: $(stat -c %s "$raw_path")"
sha256sum -- "$raw_path"
