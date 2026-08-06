# UV-K1 evidence and bring-up

`K1EVID-013` prepares one exact UV-K1/PY32F071 unit for an independently
implemented AFIK target. It does not authorize an AFIK image write or RF
transmission.

## Pinned evidence baseline

- Armel repository: `armel/uv-k1-k5v3-firmware-custom`
- Upstream branch selected by the user: latest default branch (`main`; upstream
  has no `master` branch)
- Commit: `fe9c4e9432694b50aea651084a043aae0b58673d`
- Commit date: 2026-08-04 17:45:07 +02:00
- Retrieved: 2026-08-06 into a read-only evidence checkout outside this
  repository
- Project preset at this revision: Fusion `v5.8.0`
- Exact unit's installed application reported by the user: Fusion `v5.5`

Relevant pinned-file SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `Core/py32f071xb.ld` | `aaa55edac0cf738d548073095179577b6e664130ba0088752d470f159649ae0d` |
| `Core/startup_py32f071xx.s` | `aba7b695a3099439609edb47e8c6684601b5ce209153e145bcc46d93bcbd8f11` |
| `Core/Inc/main.h` | `502dd23019b03531518f70ba2269d8a8619f87721a72df6b0ce810ccbe24e707` |
| `Core/Src/main.c` | `e1295d8e6f81ff9e560c0d1fbee6f5b14a182449166d1a4900207c8974427021` |
| `App/version.h` | `b6f9faddf71f95be7f1ddce36f555d3a24ba6252c84a546610e95b414c0726c3` |

The project is trusted as hardware-tested evidence. These hashes support
reproduction and review; no source file is copied into AFIK.

## Initial evidence matrix

| Surface | Puya primary fact | Pinned Armel board observation | Confidence and next check |
| --- | --- | --- | --- |
| CPU | PY32F071-E is Arm Cortex-M0+, up to 72 MHz | Repository and startup target PY32F071 | High for project; read exact package marking on unit |
| Main flash | Up to 128 KiB | Application origin `0x08002800`, length 118 KiB | High for selected source; establish what owns first 10 KiB before writing |
| SRAM | Up to 16 KiB | RAM origin `0x20000000`, length 16 KiB | High; exact MCU suffix must select the 16 KiB variant |
| Reset/startup | Cortex vector/reset support and boot modes exist | Vector table is linked at application origin and startup initializes data/BSS | High for project; independent AFIK vector contract still required |
| USB | One USB 2.0 full-speed interface exists | Fusion enables USB and documents USB-C serial/flashing | Medium-high; capture enumerated IDs and normal/DFU behavior on unit |
| LCD | MCU includes a segment-LCD controller | Board uses a separate ST7565 path; A0 PA6 and chip-select PB2 are declared | High for project mapping; controller marking, clock/data/reset wiring remain to verify |
| Keypad/PTT | GPIO capability | Rows PB12..PB15, columns PB3..PB6, PTT PB10 | High for project mapping; verify levels and matrix without transmitting |
| BK4819 control | GPIO/SPI-class capability only | Clock PB8, data PB9, chip-select PF9 | High for project mapping; measure idle levels and transactions receive-only |
| Audio/backlight | GPIO/analog capability only | Audio PA enable PA8; backlight PF8 | Medium-high; verify polarity, voltage, and safe sequencing |
| External storage | Not an MCU fact | PY25Q16 initialization and flash chip-select PA3 | Medium-high; identify fitted chip and preserve calibration/configuration first |
| Recovery | MCU documents SWD and system boot modes | Project documents Web Serial DFU and calibration dump/restore | Medium-high; record exact unit behavior and prove known-good recovery before AFIK |
| TX controls | Not established by MCU documentation | Existing project contains BK4819 and board-control behavior | Insufficient for AFIK TX; PA, RF switch, filter, calibration and fault experiments remain required |

The official Puya PY32F071-E product page and datasheet v1.4 are the primary
MCU sources. They establish the architectural envelope, not the K1 board
binding. The pinned Armel source supplies the trusted board observations, and
the exact unit supplies the final binding.

## Exact-unit record still required

Record locally without committing serial numbers, calibration bytes, or device
secrets:

1. Full displayed build identity beyond the user-reported Fusion `v5.5`.
2. Product and under-battery model/revision markings.
3. PCB revision and complete readable MCU, BK4819, display-controller, and
   external-flash markings.
4. Normal-mode USB VID/PID, product string, and exposed interfaces.
5. DFU/recovery-mode USB identity and the exact entry procedure.
6. Known-good firmware filename, byte length, SHA-256, and two retained copies.
7. Calibration/configuration backup filename, byte length, SHA-256, and two
   retained copies.

## Exact-unit passive bootloader observation

Observed 2026-08-06 with the user's K1 in bootloader mode:

- Host interface: QinHeng CH340/CH341 USB serial adapter, USB `1a86:7523`,
  Linux `ch341-uart`, exposed as `/dev/ttyUSB0` at the time of inspection.
- Serial configuration: 38,400 baud, raw, no flow control, matching the pinned
  evidence. This changes only host adapter settings.
- Method: three-second read-only capture of unsolicited bootloader bytes. The
  host transmitted no handshake, command, payload, reset, or flash data.
- Capture: 140 bytes containing one complete decoded `0x0518` device-info
  frame with a 36-byte decoded message and 32-byte data field.
- Bootloader version field: printable ASCII `7.03.01`.
- Device UID: a 16-byte field was present. Its value is deliberately redacted,
  is not reported here, and must not be committed.

This establishes the exact unit's passive beacon shape and version. It does not
establish a safe write protocol, flash read-back, board identity, calibration
backup, recovery image, or application boot.

## Safe experiment order

1. Observe and record the running version and normal USB descriptors.
2. Create and retain the calibration/configuration backup using the currently
   trusted firmware workflow. Do not commit its contents.
3. Validate and retain the known-good Armel recovery image.
4. Enter and leave DFU without writing; record descriptors and the procedure.
5. Reflash the same known-good Armel image, confirm normal boot, and confirm
   that recovery remains available.
6. Only a later work package may build a minimal AFIK target. Its first physical
   image must provide a harmless visible or USB boot witness, contain no RF
   operation, and be followed immediately by proven Armel recovery.

The serial device is visible only through elevated device access in the current
agent environment. Passive bootloader observation is complete; normal-mode
backup and physical markings remain pending.
