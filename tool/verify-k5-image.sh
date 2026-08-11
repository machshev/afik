#!/usr/bin/env bash
set -euo pipefail

image_path="${1:-target/thumbv6m-none-eabi/release/radio-firmware-k5}"

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
flash_end="$(symbol_address __flash_image_end)"
ram_end="$(symbol_address __ram_image_end)"
application_end="$(symbol_address __application_end)"

# EVID-K5-019: the stack starts sixteen bytes below the top of evidenced RAM.
[[ "$stack_top" == "20003ff0" ]] || {
  echo "unexpected stack top: $stack_top" >&2
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
if (( 16#$ram_end > 16#$stack_top )); then
  echo "static RAM reaches the stack: $ram_end" >&2
  exit 1
fi

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
vector_bytes="$temporary_directory/vector-table.bin"
llvm-objcopy --dump-section ".vector_table=$vector_bytes" "$image_path"

if [[ "$(stat -c %s "$vector_bytes")" -ne 16 ]]; then
  echo "vector table is not exactly sixteen bytes" >&2
  exit 1
fi

read -r initial_stack reset_vector nmi_vector hard_fault_vector \
  < <(od -An -v -tx4 "$vector_bytes")
if [[ "$initial_stack" != "20003ff0" ]]; then
  echo "unexpected initial stack vector: $initial_stack" >&2
  exit 1
fi
for vector_name in reset nmi hard_fault; do
  case "$vector_name" in
    reset) vector="$reset_vector" ;;
    nmi) vector="$nmi_vector" ;;
    hard_fault) vector="$hard_fault_vector" ;;
  esac
  if (( (16#$vector & 1) == 0 )); then
    echo "$vector_name vector does not select Thumb code: $vector" >&2
    exit 1
  fi
  if (( 16#$vector < 16 || 16#$vector >= 16#0000f000 )); then
    echo "$vector_name vector is outside the qualified K5 V1 application: $vector" >&2
    exit 1
  fi
done

echo "verified K5 V1 image: $image_path"
echo "  initial SP: 0x$initial_stack"
echo "  Reset vector: 0x$reset_vector"
echo "  NMI vector: 0x$nmi_vector"
echo "  HardFault vector: 0x$hard_fault_vector"
echo "  flash image end: 0x$flash_end"
echo "  static RAM end: 0x$ram_end"
echo "  stack top: 0x$stack_top"
echo "  application end: 0x$application_end"
