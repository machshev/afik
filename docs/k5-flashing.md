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

With the radio running its normal stock firmware, create the EEPROM backup
first. The output is create-new and mode `0600` unless `--force` is deliberate:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-k5 -- \
  --device /dev/ttyUSB0 backup-eeprom unit-eeprom.raw
```

After recording the backup hash and validating a raw recovery application, use
offline inspection to obtain the CRC-32 selection guard:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-k5 -- \
  inspect recovery.raw
```

Enter the verified stock programming mode, then probe before any write:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-k5 -- \
  --device /dev/ttyUSB0 probe
```

Only after the board/MCU record, backup copies, recovery bytes, power, and cable
checks are complete, run the recovery rehearsal with the exact CRC printed by
`inspect`:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-k5 -- \
  --device /dev/ttyUSB0 flash-recovery recovery.raw \
  --backup unit-eeprom.raw --version 2.01.23 \
  --confirm-target UV-K5-V1-DP32G030 \
  --confirm-image-crc32 00000000
```

`00000000` is a placeholder and must be replaced with the exact selected-image
CRC-32. The command generates a fresh nonzero transaction identifier, writes
all 240 pages, and prints `acknowledged_not_read_back` only if every exact
acknowledgement succeeds.

## AFIK image attempt

Only after the recovery rehearsal succeeds may the complete AFIK raw image be
selected. Inspect the image first and copy its printed CRC-32 into the
destructive confirmation argument; this guards against selecting a different
file but is not a signature. Re-enter programming mode, probe again, and write
all pages without interruption.

Two AFIK images exist for this target. `radio-firmware-dp32g030` is the original
`DP32-003` Reset image: it writes a RAM sentinel that only Renode can read and
then spins, so writing it proves nothing about a physical boot.
`radio-firmware-k5` is the `K5DRV-048` application, built with
`tool/build-k5.sh --release` and packaged with `tool/package-k5-image.sh`. It
configures the clock, drives UART1 on the programming connector, sends
`AFIK-K5-1.0 booted` in plain text once at power-on, and then answers the
read-only hello. It drives no display, keypad, memory or radio.

That makes the boot observable for the first time. After a write and a
power-cycle, either watch the port for the banner, or ask the running image who
it is:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-flasher -- \
  --device /dev/ttyUSB0 probe-normal
```

`firmware=AFIK-K5-1.0` is the witness `FLASH-012` has been waiting for: an
acknowledged page write says only that a bootloader accepted bytes, while this
says AFIK code is running on the part. `FLASH-012` remains incomplete until that
observation is followed by a successful stock recovery on the same unit.

The AFIK attempt additionally requires the exact same-unit recovery phrase and
the separately validated recovery file:

```sh
nix develop path:. -c cargo run --package radio-flasher-cli --bin afik-k5 -- \
  --device /dev/ttyUSB0 flash-afik afik-k5-v1.raw \
  --recovery recovery.raw --backup unit-eeprom.raw --version 2.01.23 \
  --confirm-target UV-K5-V1-DP32G030 \
  --confirm-image-crc32 00000000 \
  --confirm-recovery-rehearsed RECOVERY-REHEARSED-ON-THIS-UNIT
```

Again, the CRC is a placeholder, and `--version` is the negotiation value for
the exact unit's bootloader. Do not run this command merely because the host
checks pass: nothing on the host can prove that the image boots, which is the
whole reason the image now says so itself.

## Deliberate exclusions

This package does not use SWD to write or replace the bootloader, decrypt vendor
packages, flash bootloader v5/AES, support V2/V3 radios, infer MCU identity from
serial traffic, transmit RF, or import application/driver source from existing
UV-K5 firmware.
