# UV-K5 V1 firmware deployment

`FLASH-012` is a recovery-gated deployment path for one exact hardware target:
an inspected UV-K5 V1 fitted with DP32G030 and the stock version-2 serial
bootloader. It is not general K5-family support and does not make the current
minimal AFIK image a usable radio firmware.

## Boundaries

- Application flash is `0x0000..=0xEFFF`. The raw file is exactly `0xF000`
  bytes and unused application bytes are `0xFF`.
- The existing stock bootloader at `0xF000..=0xFFFF` is never linked, packaged,
  erased, or written by AFIK.
- Serial is one explicit Linux path at 38,400 baud, 8-N-1. There is no device
  discovery or default path.
- Normal-mode access is read-only and limited to a complete 8 KiB EEPROM
  backup. No EEPROM or calibration write exists.
- Bootloader access accepts only the exact version-2 shape and printable `2.*`
  version. Bootloader v5 and unknown variants are rejected.
- Flashing is one full 240-page write. There is no address, length, partial,
  resume, retry, bootloader-write, or wildcard-version option.
- Each run uses a new explicit nonzero transaction identifier and requires the
  bootloader to echo it with the exact page index and zero result.
- A bootloader acknowledgement is not flash read-back and is not proof that the
  application booted.

## Required physical record

Before any destructive command, record all of the following for the exact test
unit in a local experiment log. Do not add calibration bytes, device secrets,
or personally identifying serial numbers to the repository.

1. Product label/model, under-battery V1 marking, PCB revision, and photographs
   of a readable DP32G030 MCU marking.
2. Programming cable identity and logic voltage, a stable charged supply, and a
   cable-retention plan for the complete write.
3. Normal-mode firmware version and a successful complete EEPROM backup. Record
   its file size and SHA-256 outside the repository and retain two copies.
4. A known-good raw recovery application for this exact V1/bootloader family.
   Validate its Cortex-M vector words, byte length, and SHA-256 and retain two
   copies. Vendor packed/encrypted files are not accepted directly.
5. The observed bootloader beacon and full raw transcript. It must be the exact
   supported version-2 shape; the beacon supplements rather than replaces board
   inspection.

## Recovery rehearsal

The first physical write is the known-good recovery application, not AFIK.
Enter the stock programming mode using the unit's verified procedure, probe the
beacon, rewrite the recovery application through the AFIK tool, power-cycle,
and demonstrate normal stock boot. Re-enter programming mode a second time to
show the preserved bootloader remains available.

Stop if any page acknowledgement is absent, malformed, mismatched, or reports
an error. Do not retry the page automatically. Keep power stable and use the
preserved bootloader plus known-good recovery image for a deliberate recovery
attempt after inspecting the transcript.

## AFIK image attempt

Only after the recovery rehearsal succeeds may the complete AFIK raw image be
selected. Inspect the image first and copy its printed CRC-32 into the
destructive confirmation argument; this guards against selecting a different
file but is not a signature. Re-enter programming mode, probe again, and write
all pages without interruption.

The current Reset image only writes a RAM sentinel used by Renode and then
spins. It has no evidenced display, keypad, UART, RF, or GPIO adapter, so a
successful write is not yet a useful or independently observable hardware boot.
`FLASH-012` remains incomplete until a safe read-only debug observation or a
separately evidenced physical output proves Reset reached AFIK code, followed
by successful stock recovery.

## Deliberate exclusions

This package does not use SWD to write or replace the bootloader, decrypt vendor
packages, flash bootloader v5/AES, support V2/V3 radios, infer MCU identity from
serial traffic, transmit RF, or import application/driver source from existing
UV-K5 firmware.
