#!/usr/bin/env bash
set -euo pipefail

image_path="${1:-target/thumbv6m-none-eabi/release/radio-firmware-k1-async}"

[[ -f "$image_path" ]] || { echo "K1 async ELF not found: $image_path" >&2; exit 1; }
for required_tool in llvm-readelf llvm-objcopy llvm-nm llvm-size; do
  command -v "$required_tool" >/dev/null || { echo "required tool not found: $required_tool" >&2; exit 1; }
done

header="$(llvm-readelf --file-header "$image_path")"
grep -Eq 'Data:.*little endian' <<<"$header" || { echo "K1 async ELF is not little-endian" >&2; exit 1; }
grep -Eq 'Machine:.*ARM' <<<"$header" || { echo "K1 async ELF is not Arm" >&2; exit 1; }

segments="$(llvm-readelf --segments --wide "$image_path")"
grep -Eq '^[[:space:]]*(INTERP|DYNAMIC)[[:space:]]' <<<"$segments" && {
  echo "K1 async ELF contains a host interpreter or dynamic segment" >&2; exit 1;
}
while read -r virtual_address physical_address memory_size; do
  virtual_start=$((virtual_address)); virtual_end=$((virtual_address + memory_size))
  physical_start=$((physical_address)); physical_end=$((physical_address + memory_size))
  if (( virtual_start >= 0x08002800 && virtual_end <= 0x08020000 )); then continue; fi
  if (( virtual_start >= 0x20000000 && virtual_end <= 0x20004000 && physical_start >= 0x08002800 && physical_end <= 0x08020000 )); then continue; fi
  if (( virtual_start >= 0x20000000 && virtual_end <= 0x20004000 && physical_start >= 0x20000000 && physical_end <= 0x20004000 )); then continue; fi
  echo "K1 async LOAD is outside evidenced flash/RAM: $virtual_address/$physical_address" >&2; exit 1
done < <(awk '$1 == "LOAD" { print $3, $4, $6 }' <<<"$segments")

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
vectors="$temporary_directory/vectors.bin"
llvm-objcopy --dump-section ".vector_table=$vectors" "$image_path"
[[ "$(stat -c %s "$vectors")" -eq 192 ]] || { echo "K1 async vector table is not 192 bytes" >&2; exit 1; }

read -r initial_stack reset_vector < <(od -An -v -tx4 -N8 "$vectors")
[[ "$initial_stack" == "20004000" ]] || { echo "unexpected async initial SP: $initial_stack" >&2; exit 1; }
(( (16#$reset_vector & 1) == 1 )) || { echo "async Reset is not Thumb: $reset_vector" >&2; exit 1; }
(( 16#$reset_vector >= 16#080028c0 && 16#$reset_vector < 16#08020000 )) || { echo "async Reset is out of range: $reset_vector" >&2; exit 1; }

# Static RAM must leave the executor, the interrupt frames, and every by-value
# configuration copy a working stack. A build which fits flash but not RAM
# faults on the unit and shows nothing, so it is refused here rather than
# packaged: `EVID-K1-020` evidences the 16 KiB SRAM and `link.x` places the
# initial stack at its top.
ram_bytes=16384
minimum_stack_bytes=4096
static_bytes=$(llvm-size -A "$image_path" | awk '$1 == ".data" || $1 == ".bss" || $1 == ".uninit" { total += $2 } END { print total + 0 }')
stack_headroom=$((ram_bytes - static_bytes))
(( stack_headroom >= minimum_stack_bytes )) || {
  echo "K1 async static RAM leaves too little stack: ${static_bytes} bytes static, ${stack_headroom} free, ${minimum_stack_bytes} required" >&2
  exit 1
}

symbols="$(llvm-nm --defined-only "$image_path")"
symbol_address() { awk -v name="$1" '$3 == name { print $1 }' <<<"$symbols"; }
[[ -n "$(symbol_address k1_relocate_vectors)" ]] || {
  echo "missing source-backed K1 VTOR relocation boundary" >&2; exit 1;
}
for mapping in "9:DMA1_CHANNEL1" "10:DMA1_CHANNEL2_3" "11:DMA1_CHANNEL4_5_6_7" "20:TIM15" "27:USART1"; do
  irq="${mapping%%:*}"; symbol="${mapping#*:}"
  address="$(symbol_address "$symbol")"
  [[ -n "$address" ]] || { echo "missing async handler: $symbol" >&2; exit 1; }
  vector="$(od -An -v -tx4 -j $(((16 + irq) * 4)) -N4 "$vectors" | tr -d ' ')"
  expected=$(printf '%08x' $((16#$address | 1)))
  [[ "$vector" == "$expected" ]] || { echo "$symbol vector mismatch: $vector != $expected" >&2; exit 1; }
done

echo "verified K1 async application image: $image_path"
echo "  vector bytes: 192"
echo "  initial SP: 0x$initial_stack"
echo "  Reset vector: 0x$reset_vector"
echo "  required IRQ handlers: DMA1 ch1/ch2-3/ch4-7, TIM15, USART1"
echo "  VTOR relocation boundary: k1_relocate_vectors"
echo "  static RAM: $static_bytes bytes, stack headroom: $stack_headroom bytes"
