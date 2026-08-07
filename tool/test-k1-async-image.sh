#!/usr/bin/env bash
set -euo pipefail

elf_path="${1:-target/thumbv6m-none-eabi/release/radio-firmware-k1-async}"
raw_path="${2:-target/thumbv6m-none-eabi/release/radio-firmware-k1-async.raw}"
tool/verify-k1-async-image.sh "$elf_path"
tool/verify-k1-raw-image.sh "$raw_path"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
expected="$temporary_directory/expected.raw"
llvm-objcopy --output-target=binary --gap-fill=0xff "$elf_path" "$expected"
cmp --silent -- "$expected" "$raw_path" || { echo "K1 async raw does not match ELF" >&2; exit 1; }

truncated="$temporary_directory/truncated.raw"
head -c 7 "$raw_path" > "$truncated"
if tool/verify-k1-raw-image.sh "$truncated" >/dev/null 2>&1; then
  echo "truncated async vector image unexpectedly passed" >&2; exit 1
fi
oversized="$temporary_directory/oversized.raw"
truncate -s $((0x1b801)) "$oversized"
if tool/verify-k1-raw-image.sh "$oversized" >/dev/null 2>&1; then
  echo "oversized async image unexpectedly passed" >&2; exit 1
fi
echo "K1 async package positive and negative checks passed"
