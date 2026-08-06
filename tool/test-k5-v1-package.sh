#!/usr/bin/env bash
set -euo pipefail

elf_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030}"
raw_path="${2:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030-k5-v1.raw}"

tool/verify-k5-v1-package.sh "$elf_path" "$raw_path"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

truncated="$temporary_directory/truncated.raw"
head -c $((0xefff)) "$raw_path" > "$truncated"
if tool/verify-k5-v1-package.sh "$elf_path" "$truncated" >/dev/null 2>&1; then
  echo "truncated K5 V1 package unexpectedly passed verification" >&2
  exit 1
fi

corrupt="$temporary_directory/corrupt.raw"
cp -- "$raw_path" "$corrupt"
printf '\x00' | dd of="$corrupt" bs=1 seek=4 count=1 conv=notrunc status=none
if tool/verify-k5-v1-package.sh "$elf_path" "$corrupt" >/dev/null 2>&1; then
  echo "corrupt K5 V1 package unexpectedly passed verification" >&2
  exit 1
fi

echo "K5 V1 package positive and negative checks passed"
