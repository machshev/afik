#!/usr/bin/env bash
set -euo pipefail

elf_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-k1}"
raw_path="${2:-target/thumbv6m-none-eabi/debug/radio-firmware-k1.raw}"

tool/verify-k1-image.sh "$elf_path"
tool/verify-k1-raw-image.sh "$raw_path"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

expected_raw="$temporary_directory/expected.raw"
llvm-objcopy --output-target=binary --gap-fill=0xff "$elf_path" "$expected_raw"
if ! cmp --silent -- "$expected_raw" "$raw_path"; then
  echo "K1 raw package does not exactly match the verified ELF" >&2
  exit 1
fi

truncated="$temporary_directory/truncated.raw"
head -c 7 "$raw_path" > "$truncated"
if tool/verify-k1-raw-image.sh "$truncated" >/dev/null 2>&1; then
  echo "truncated K1 raw image unexpectedly passed verification" >&2
  exit 1
fi

corrupt="$temporary_directory/corrupt.raw"
cp -- "$raw_path" "$corrupt"
printf '\x00' | dd of="$corrupt" bs=1 seek=4 count=1 conv=notrunc status=none
if tool/verify-k1-raw-image.sh "$corrupt" >/dev/null 2>&1; then
  echo "non-Thumb K1 raw image unexpectedly passed verification" >&2
  exit 1
fi

too_long="$temporary_directory/too-long.raw"
truncate -s $((0x1d801)) "$too_long"
if tool/verify-k1-raw-image.sh "$too_long" >/dev/null 2>&1; then
  echo "oversized K1 raw image unexpectedly passed verification" >&2
  exit 1
fi

echo "K1 raw package positive checks passed"
echo "K1 negative fixture sizes: truncated=$(stat -c %s "$truncated"), oversized=$(stat -c %s "$too_long")"
