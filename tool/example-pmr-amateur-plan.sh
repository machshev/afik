#!/usr/bin/env bash
# Example channel plan: PMR446 plus 2 m and 70 cm amateur FM simplex.
#
# Twelve explicit channels in three named banks, which is exactly what the K1
# receive image can hold (twelve channels, sixteen banks). Frequencies are
# licence-free PMR446 and IARU Region 1 amateur FM simplex segments; confirm
# them against your own national band plan before transmitting on any radio
# which can. The K1 image is receive-only whatever a channel's class says.
#
# Usage:
#   tool/example-pmr-amateur-plan.sh --device PATH write
#   tool/example-pmr-amateur-plan.sh --device PATH compile OUTPUT [--force]
#   tool/example-pmr-amateur-plan.sh --sim compile OUTPUT [--force]
#
# The backend supplies the capabilities the plan is compiled against, so
# compiling for a radio needs that radio's own advertised capacity.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 (--sim | --device PATH) (write | compile OUTPUT [--force])" >&2
  exit 2
fi

backend=()
case "$1" in
  --sim)
    backend=(--sim)
    shift
    ;;
  --device)
    if [ "$#" -lt 3 ]; then
      echo "$0: --device requires PATH" >&2
      exit 2
    fi
    backend=(--device "$2" --baud 38400)
    shift 2
    ;;
  *)
    echo "$0: first argument must be --sim or --device PATH" >&2
    exit 2
    ;;
esac

# Bank 0: the eight-channel analogue PMR446 allocation, licence free, 12.5 kHz
# spacing from 446.00625 MHz. Four are programmed to leave room for amateur use.
pmr=(
  "1:PMR 1:446006250:0:licence-free"
  "2:PMR 2:446018750:0:licence-free"
  "3:PMR 3:446031250:0:licence-free"
  "4:PMR 4:446043750:0:licence-free"
)

# Bank 1: 2 m FM simplex. 145.500 MHz is the calling channel and the rest sit in
# the FM simplex segment at 25 kHz spacing.
two_metres=(
  "5:2M CALL:145500000:1:amateur"
  "6:2M 145.525:145525000:1:amateur"
  "7:2M 145.550:145550000:1:amateur"
  "8:2M 145.575:145575000:1:amateur"
)

# Bank 2: 70 cm FM simplex. 433.500 MHz is the calling channel and the rest sit
# in the 433.400 to 433.575 MHz simplex segment at 25 kHz spacing.
seventy_centimetres=(
  "9:70CM CALL:433500000:2:amateur"
  "10:70CM 433.40:433400000:2:amateur"
  "11:70CM 433.45:433450000:2:amateur"
  "12:70CM 433.55:433550000:2:amateur"
)

arguments=()
for bank in "0:PMR446:scan" "1:2M SIMPLEX:scan" "2:70CM SIMPLEX:scan"; do
  arguments+=(--channel-bank "$bank")
done
for channel in "${pmr[@]}" "${two_metres[@]}" "${seventy_centimetres[@]}"; do
  arguments+=(--channel "$channel")
done

exec cargo run --quiet --package radio-programmer-cli --bin afik-programmer -- \
  "${backend[@]}" "$@" "${arguments[@]}"
