#!/usr/bin/env bash
set -euo pipefail

image_path="${1:-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030}"

if [[ ! -f "$image_path" ]]; then
  echo "target image not found: $image_path" >&2
  exit 1
fi

for required_tool in llvm-readelf llvm-objcopy llvm-nm; do
  if ! command -v "$required_tool" >/dev/null; then
    echo "required tool not found in pinned environment: $required_tool" >&2
    exit 1
  fi
done

header="$(llvm-readelf --file-header "$image_path")"
if ! grep -Eq 'Data:.*little endian' <<<"$header"; then
  echo "target ELF is not little-endian" >&2
  exit 1
fi
if ! grep -Eq 'Machine:.*ARM' <<<"$header"; then
  echo "target ELF is not Arm" >&2
  exit 1
fi

attributes="$(llvm-readelf --arch-specific "$image_path")"
if ! grep -q 'Description: ARM v6S-M' <<<"$attributes"; then
  echo "target ELF does not declare the Armv6-M architecture" >&2
  exit 1
fi

segments="$(llvm-readelf --segments --wide "$image_path")"
if grep -Eq '^[[:space:]]*(INTERP|DYNAMIC)[[:space:]]' <<<"$segments"; then
  echo "target ELF contains a host interpreter or dynamic segment" >&2
  exit 1
fi
while read -r virtual_address memory_size; do
  segment_start=$((virtual_address))
  segment_end=$((virtual_address + memory_size))
  if (( segment_start >= 0 && segment_end <= 0x0000f000 )); then
    continue
  fi
  if (( segment_start >= 0x20000000 && segment_end <= 0x20004000 )); then
    continue
  fi
  echo "load segment is outside evidenced flash/RAM: $virtual_address" >&2
  exit 1
done < <(awk '$1 == "LOAD" { print $3, $6 }' <<<"$segments")

symbols="$(llvm-nm --defined-only --numeric-sort "$image_path")"
symbol_address() {
  local symbol_name="$1"
  awk -v name="$symbol_name" '$3 == name { print $1 }' <<<"$symbols"
}

stack_top="$(symbol_address __stack_top)"
sentinel_start="$(symbol_address __boot_sentinel_start)"
flash_end="$(symbol_address __flash_image_end)"
application_end="$(symbol_address __application_end)"

[[ "$stack_top" == "20004000" ]] || {
  echo "unexpected stack top: $stack_top" >&2
  exit 1
}
[[ "$sentinel_start" == "20000000" ]] || {
  echo "unexpected boot sentinel address: $sentinel_start" >&2
  exit 1
}
[[ "$application_end" == "0000f000" ]] || {
  echo "unexpected qualified K5 V1 application end: $application_end" >&2
  exit 1
}
if (( 16#$flash_end > 16#0000f000 )); then
  echo "flash image reaches the reserved K5 V1 bootloader region: $flash_end" >&2
  exit 1
fi

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
vector_bytes="$temporary_directory/vector-table.bin"
llvm-objcopy --dump-section ".vector_table=$vector_bytes" "$image_path"

if [[ "$(stat -c %s "$vector_bytes")" -ne 8 ]]; then
  echo "vector table is not exactly eight bytes" >&2
  exit 1
fi

read -r initial_stack reset_vector < <(od -An -v -tx4 "$vector_bytes")
if [[ "$initial_stack" != "20004000" ]]; then
  echo "unexpected initial stack vector: $initial_stack" >&2
  exit 1
fi
if (( (16#$reset_vector & 1) == 0 )); then
  echo "Reset vector does not select Thumb code: $reset_vector" >&2
  exit 1
fi
if (( 16#$reset_vector < 8 || 16#$reset_vector >= 16#0000f000 )); then
  echo "Reset vector is outside the qualified K5 V1 application: $reset_vector" >&2
  exit 1
fi

echo "verified DP32G030 image: $image_path"
echo "  initial SP: 0x$initial_stack"
echo "  Reset vector: 0x$reset_vector"
echo "  boot sentinel: 0x$sentinel_start"
echo "  flash image end: 0x$flash_end"
echo "  application end: 0x$application_end"
