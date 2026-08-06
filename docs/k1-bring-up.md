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

The Armel locations below are relative to the read-only checkout at the pinned
commit. They are evidence references, not AFIK source dependencies.

| Surface | Puya primary fact | Pinned Armel location and observation | Confidence and next check |
| --- | --- | --- | --- |
| CPU | PY32F071-E is Arm Cortex-M0+, up to 72 MHz | `Core/startup_py32f071xx.s:39-42` selects Cortex-M0+ and Thumb | High for the selected source; read the exact package marking on the unit |
| Main flash | Up to 128 KiB | `Core/py32f071xb.ld:9-10,52-66` declares `0x08002800` and 118 KiB | High for this source; establish what owns the first 10 KiB before writing |
| SRAM | Up to 16 KiB | `Core/py32f071xb.ld:61-66` declares `0x20000000` and 16 KiB | High for this source; the exact MCU suffix must select the 16 KiB variant |
| Reset/startup | Cortex vector/reset support and boot modes exist | `Core/startup_py32f071xx.s:68-115,140-192` resets, initializes data/BSS, calls `main`, and declares vectors | High for the source; an independent AFIK vector contract is still required |
| Clock | The MCU supports the documented clock envelope | `Core/Src/main.c:46-72` assumes the bootloader left the clock at 48 MHz | Medium for board startup; do not encode it in AFIK until boot handoff is independently evidenced |
| USB | One USB 2.0 full-speed interface exists | `App/driver/vcp.c:28-43` enables USB CDC; `App/usb/usbd_cdc_if.c:6-24` declares VID/PID and CDC descriptors | Medium-high for source behavior; capture normal/DFU identities on the unit |
| LCD | MCU includes a segment-LCD controller | `App/board.c:93-112` and `App/driver/st7565.c:29-73` use an external ST7565 path, A0 PA6, CS PB2, SCK PA5, SDA PA7 | High for source mapping; controller marking and reset wiring remain to verify |
| Keypad/PTT | GPIO capability | `App/board.c:82-100` and `App/driver/gpio.h:29-35` identify rows PB12..PB15, columns PB3..PB6, and PTT PB10 | High for source mapping; verify levels and matrix without transmitting |
| BK4819 control | GPIO/SPI-class capability only | `App/board.c:108-112,126-129` identifies clock PB8, data PB9, and CS PF9 | High for source mapping; measure idle levels and transactions receive-only |
| Audio/backlight | GPIO/analog capability only | `App/driver/gpio.h:29-35` identifies audio PA8 and backlight PF8 | Medium-high; verify polarity, voltage, and safe sequencing |
| External storage | Not an MCU fact | `App/driver/py25q16.c:32-38,56-101,222-226` identifies PY25Q16 over SPI2 with CS PA3 | Medium-high; identify the fitted chip and preserve calibration/configuration first |
| Recovery | MCU documents SWD and system boot modes | `App/driver/vcp.c:28-43` documents the USB CDC path; the physical DFU path remains unobserved | Medium for source capability; record exact unit behavior before any write |
| TX controls | Not established by MCU documentation | Source board/RF control exists, but no exact-unit RF or safety observation is recorded | Insufficient for AFIK TX; PA, RF switch, filter, calibration, and fault experiments remain required |

The official Puya PY32F071-E product page and datasheet v1.4 are the primary
MCU sources. They establish the architectural envelope, not the K1 board
binding. The pinned Armel source supplies the trusted board observations, and
the exact unit supplies the final binding.

## CPU, memory, and image contract

This is the first bounded AFIK target contract. It is sufficient to define a
later reset-and-boot-witness package, but it does not authorize target code or
a physical write.

| Contract item | Pinned value | Evidence and boundary |
| --- | --- | --- |
| CPU ISA | Arm Cortex-M0+, Thumb | `Core/startup_py32f071xx.s:39-42`; Puya datasheet v1.4 remains the primary MCU source |
| Application origin | `0x08002800` | `Core/py32f071xb.ld:52-66`; ownership of `0x08000000..0x080027ff` is not inferred |
| Application capacity | `118 KiB` (`0x1d800`) | `Core/py32f071xb.ld:64-66`; exclusive declared end is `0x08020000` |
| SRAM | `0x20000000..0x20003fff` | `Core/py32f071xb.ld:61-66`; exact suffix and physical read-back remain pending |
| Reset entry | Vector word 0 is `_estack`; word 1 is `Reset_Handler` | `Core/startup_py32f071xx.s:140-146`; startup then assumes a 48 MHz bootloader handoff at `Core/Src/main.c:68-72` |
| Recovery image format | Raw application bytes loaded at `0x08002800` | Pinned `archive/f4hwn.fusion.v5.5.0.bin`; 95,836 bytes, SHA-256 `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463` |
| Recovery image vectors | Initial SP `0x20004000`; Thumb Reset `0x08002d49` | Static validation of the pinned raw image; this is not physical recovery proof |
| Recovery image end | `0x08019e5c` exclusive | `0x08002800 + 95,836`; below the declared `0x08020000` end |

The recovery image is source- and vector-valid, not a demonstrated recovery
image. AFIK must preserve the bootloader boundary and must not infer a K1
write protocol from K5 evidence. A later target package must independently
define its linker/vector contract in Rust and begin with a harmless witness.

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

## Exact-unit normal-mode backup attempt

With the unit visibly booted into normal Fusion `v5.5`, the pinned serial tool
was invoked in dump-all mode at 38,400 baud through the same CH340 adapter. Its
only transmitted operation before a response is the documented normal-mode
hello; the later path contains read requests only.

Three initial bounded attempts were made on 2026-08-06 with the V2
timestamp-session tool: initially, after a radio power cycle, and after
unplugging/replugging both ends of the cable. Each reached `Examining device
info` and received no response before the 30-second host timeout. No backup
file was created. No EEPROM write, restore, reboot, bootloader handshake,
firmware page, or reset command was sent.

The user reported repeated successful CHIRP use with the same cable. Inspection
of Armel's K1-capable CHIRP driver then established the protocol mismatch:
CHIRP sends fixed session word `0x6457396A`, while the unsuccessful V2 serial
tool substituted the current timestamp in both hello and later reads.

AFIK's existing tested `backup-eeprom` path uses CHIRP's fixed word, permits no
EEPROM write, validates every response offset/length, and writes the output
only after all 8 KiB arrive. That corrected read-only workflow succeeded:

- Normal firmware identified itself as `F4HWN v5.5.0`.
- Exactly 8,192 bytes were received and validated.
- The output was created mode `0600` outside the repository.
- Its CRC-32 and SHA-256 were reported to the user but are deliberately not
  committed because they identify a unit-specific calibration/configuration
  artifact.

The temporary backup is not a durable second copy. It must be copied to two
user-controlled persistent locations and re-hashed before any firmware write.
The repository reserves ignored `.private/` for local unit-specific artifacts;
none of its contents may be committed.

On 2026-08-06 the following local pairs were created mode `0600` under a
mode-`0700` `.private/k1/` directory and re-hashed successfully:

- `unit-backup.primary.raw` and `unit-backup.secondary.raw`: 8,192 bytes each,
  with identical expected SHA-256 (kept outside tracked documentation).
- `f4hwn-v5.5.0-recovery.primary.bin` and
  `f4hwn-v5.5.0-recovery.secondary.bin`: 95,836 bytes each, both matching the
  published recovery-candidate SHA-256 below.

Git reports the complete `.private/` tree ignored. Both copies in each pair are
on the same filesystem; they protect against accidental single-file loss but
not filesystem or disk failure. The user accepts this shared-filesystem risk
for the current evidence package; it is not an active gate.

The user additionally requested a private home recovery directory. It was
created mode `0700` outside the repository with one mode-`0600` backup and
recovery image, both re-hashed successfully. Its absolute path is deliberately
omitted from tracked content. The home directory and repository report the
same filesystem device; this is an extra out-of-Git copy but does not satisfy
the shared-filesystem durability risk against filesystem or disk failure.

The user then power-cycled the unit into normal mode. A repeat fixed-session
hello returned `0x0515` and `F4HWN v5.5.0`, followed by a complete 8 KiB
read-back in bounded blocks. The new mode-`0600` output matched the prior
temporary read and `.private/k1/unit-backup.primary.raw` byte-for-byte. The
new temporary file was on filesystem device `43`, while the repository and
private copies were on device `56`; this is useful cross-filesystem evidence
but is not durable user-controlled storage. No write, reset, bootloader entry,
or RF operation was performed.

## Candidate v5.5.0 recovery image

Pinned Armel commit `fe9c4e9432694b50aea651084a043aae0b58673d`
contains `archive/f4hwn.fusion.v5.5.0.bin`, matching the normal firmware
identity reported by the exact unit.

- Source bytes: 95,836
- SHA-256:
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`
- Format: raw Cortex-M application image; it already satisfies the pinned
  codec's raw-image test and must not be passed through packed-image decoding.
- Initial stack pointer: `0x20004000`, the top of the evidenced 16 KiB SRAM.
- Reset vector: `0x08002D49`, Thumb and inside the application range.
- File end when loaded at `0x08002800`: `0x08019E5C`, below the 128 KiB main
  flash end `0x08020000`.

This is a source- and vector-valid recovery candidate, not yet a physically
rehearsed recovery image. The two matching local copies are accepted for the
current evidence package. The first later write experiment must reflash this unchanged image,
prove normal `v5.5.0` boot, confirm the calibration/configuration backup, and
prove that bootloader `7.03.01` remains available.

## Pinned CHIRP protocol evidence

- Repository: `armel/uv-k5-chirp-driver`
- Upstream default branch: `main`
- Commit: `a0e9314570cd4f5440aca8322ca1722163bad217`
- Commit date: 2025-12-02 23:48:02 +01:00
- Relevant file: `uvk5_egzumer_f4hwn_ver_4_3_0.py`
- File SHA-256:
  `024ff9d263d7aeb8be03414754c99dd696ee20cf322e6e20c6a72f0287cf42a1`
- Fact used: normal-mode hello `0x0514` and read `0x051B` carry fixed session
  word `0x6457396A`; the download reads the complete 8 KiB in bounded blocks.

## Safe experiment order

1. Record the physical model, PCB, MCU, RF, display, and external-flash
   markings, plus normal and DFU USB identities.
2. The complete read-only configuration/calibration backup is already retained
   in two local copies and one additional home copy; the user accepts these
   copies for the current evidence package. Do not commit them.
3. The known-good Armel recovery candidate is statically validated and retained
   in matching local copies; validate the recovery procedure before writing.
4. Enter and leave DFU without writing; record descriptors and the procedure.
5. Reflash the same known-good Armel image only after those gates, confirm normal boot, and confirm
   that recovery remains available.
6. Only a later work package may build a minimal AFIK target. Its first physical
   image must provide a harmless visible or USB boot witness, contain no RF
   operation, and be followed immediately by proven Armel recovery.

The serial device was visible for the normal-mode verification above. Passive
bootloader observation and repeatable read-only backup are complete; physical
markings, normal/DFU USB identities, and physical recovery remain pending.
