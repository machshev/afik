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

### Bounded main-key matrix for K1KEY-022

The pinned source configures rows PB15..PB12 as pull-up inputs and columns
PB6..PB3 as push-pull outputs. All columns idle high; selecting one column
drives only it low, so one pressed key appears as one active-low row. AFIK's
independent matrix table is:

| Selected low column | PB15 | PB14 | PB13 | PB12 |
| --- | --- | --- | --- | --- |
| PB6 | MENU | 1 | 4 | 7 |
| PB5 | UP | 2 | 5 | 8 |
| PB4 | DOWN | 3 | 6 | 9 |
| PB3 | EXIT | STAR | 0 | F |

The table excludes the source's separate PTT PB10 input and special side-key
handling. Zero keys means release; multiple, changing, invalid, or failed
samples mean no action. Debounce receives elapsed time explicitly rather than
assuming the source's polling interval. The first physical witness may only
show a debounced main-key label on the already verified display.

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

## Exact-unit recovery rehearsal

After the two local backup and recovery copies were verified, the unchanged
candidate `archive/f4hwn.fusion.v5.5.0.bin` was written to the exact unit using
the independently implemented K1 bootloader path. The live beacon was
`7.03.01`; three `0x0530` version handshakes preceded 375 sequential `0x0519`
page requests. Every `0x051A` acknowledgement matched the per-run transaction
identifier and page index and returned zero. No page was retried and no reset
command was sent.

After a user power-cycle, the radio returned to normal Fusion `v5.5.0`. A new
complete `0x0514`/`0x051B` read produced 8,192 bytes and matched the pre-flash
backup byte-for-byte. This demonstrates same-unit recovery and preservation of
the logical calibration/configuration data; it is not an AFIK K1 application
boot proof.

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

This is a source- and vector-valid recovery candidate. The same-unit recovery
rehearsal is complete and the bounded K1 target/image contract now exists in
`K1BOOT-016`; the guarded AFIK writer requires this retained recovery image and
backup before any physical witness-image write. The two matching local copies
remain accepted for the current evidence package.

## K1BOOT-016 reset image

The first AFIK K1 target is an independently implemented reset-only image. It
is deliberately smaller than a radio application and does not copy or link
the pinned Armel implementation.

- Crate: `radio-firmware-k1`, standalone `no_std`, heap-free, and dependency-free.
- CPU target: Rust `thumbv6m-none-eabi`, compatible with the evidenced
  Cortex-M0+ Thumb instruction set.
- Application origin: `0x08002800`; exclusive application end:
  `0x08020000` (118 KiB).
- SRAM: `0x20000000..0x20004000`; initial stack pointer: `0x20004000`.
- Reset vector: placed in the application vector table at `0x08002800`.
- Reset behavior: stores development witness `0x4B31_B007` at RAM address
  `0x20000000`, then spins.
- Generated raw image: 616 bytes, SHA-256
  `877e2018ef4dd0e985dd16447d7120f61d60ff77259b149b3ad0ab6d37b95021`.

The linker, ELF verifier, raw-image verifier, and negative package checks are
implemented in `tool/build-k1.sh`, `tool/verify-k1-image.sh`,
`tool/verify-k1-raw-image.sh`, `tool/package-k1-image.sh`, and
`tool/test-k1-image.sh`. The RAM value is not a physical boot witness: no
clock, USB, display, keypad, GPIO, external flash, BK4819, audio, RF, TX, or
reset behavior is implemented. This reset-only milestone was superseded by the
serial witness image described below; the first physical AFIK image must use
the external CH340/UART path and have no RF side effect.

## K1WIT-017 physical witness status

The official [Puya PY32F071-E product page](https://www.puyasemi.com/en/py32f071/3415.html)
and [PY32F071-E datasheet v1.4](https://www.puyasemi.com/download_path/%E6%95%B0%E6%8D%AE%E6%89%8B%E5%86%8C/MCU/PY32F071-E_Datasheet_V1.4.pdf)
establish a PY32F071 USB 2.0 full-speed peripheral, but USB is not the K1
programming interface used here. The exact unit is connected through an
external CH340 serial adapter.

Read-only host observation on 2026-08-06 found:

- `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0` and `/dev/ttyUSB0`;
- USB device `1a86:7523`, QinHeng CH340 serial converter;
- no native K1 USB device or USB descriptor for the radio.

The AFIK generic identify probe classifies the external serial path as K1
bootloader `7.03.01`, with hardware identity explicitly unproven by the
beacon. The CH340 is the intended USB-to-UART transport; it is not a native
USB identity and is not treated as one.

## K1 serial application witness

The pinned exact-board evidence checkout is
`armel/uv-k1-k5v3-firmware-custom` commit
`fe9c4e9432694b50aea651084a043aae0b58673d`. Its UART source records USART1,
PA9 TX, PA10 RX, alternate function 1, and 38,400 baud in
`App/driver/uart.c`; its startup source records that the bootloader provides
the 48 MHz system clock in `Core/Src/main.c`. The device header supplies the
USART1, GPIOA, RCC register bases and status/control bit definitions. These
files are evidence only; AFIK does not copy or link their driver.

AFIK independently implements the first bounded K1 application slice:

- Reset stores the development RAM witness `0x4B31_B007`.
- USART1 is configured for 38,400 8-N-1 on the evidenced PA9/PA10 AF1 path.
- The application accepts only the fixed-session `0x0514` normal-mode hello,
  validates the existing CRC/XOR envelope, and returns a `0x0515` response
  identifying itself as `AFIK-K1-0.1`.
- No EEPROM, RF, TX, display, keypad, USB, external flash, or reset operation
  is implemented.

The host-side read-only witness command is:

```text
nix develop path:. -c cargo run --quiet --package radio-flasher-cli \
  --bin afik-flasher -- --device /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0 probe-normal
```

It reported `protocol=normal-firmware-hello` and
`firmware=AFIK-K1-0.1` after the user power-cycled the radio. The guarded write
acknowledged all 172 pages and did not issue a reset. This proves only the
bounded serial application slice, not a complete radio application.

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
5. The unchanged known-good Armel recovery rehearsal and the first AFIK-hosted
   recovery write are complete. The guarded `flash-afik-k1` command acknowledged
   the bounded serial witness image; after power-cycle, `probe-normal` returned
   the exact AFIK response.
6. `K1WIT-017` uses the external CH340/UART path, not native USB. The first
   physical image provided the exact `AFIK-K1-0.1` response and contained no RF
   operation. Any next application slice requires a new bounded evidence and
   recovery gate.

The serial device was visible for the normal-mode verification above. Passive
bootloader observation, repeatable read-only backup, same-unit recovery through
AFIK, and the bounded AFIK serial application boot witness are complete.
Physical markings, normal/DFU USB identities, and full radio-application
behavior remain pending.

## K1DISP-019 display-only witness boundary

The next AFIK slice is one fixed display identity screen while the proven
USART1 hello remains available. The pinned board observations identify a
128-by-64, eight-page ST7565-compatible serial display path using SPI1 clock on
PA5 AF0, data on PA7 AF0, A0 on PA6, and active-low chip select on PB2. They
also record SPI mode 3, MSB-first transfers, and a divide-by-64 rate from the
bootloader-provided 48 MHz clock. AFIK will independently implement this
bounded contract and will not copy the source driver.

The first implementation may initialise the controller, clear its eight pages,
and draw only fixed bounded `AFIK` and version glyphs. It will not use an
unobserved hardware reset pin or touch the keypad/PTT matrix, backlight, audio,
external flash, BK4819, RF, TX, USB, EEPROM, interrupts, or DMA. Exact command
and framebuffer traces must pass on the host before a physical image is
proposed. A physical write requires explicit confirmation and is successful
only after both the screen and the existing serial hello are observed; the
retained stock image remains the rollback route.

The static implementation is complete. It renders fixed `AFIK` and `K1 0.2`
text, retains a bounded `AFIK-K1-0.2` serial response, and de-selects the panel
after a bounded SPI timeout so display failure cannot trap the serial loop. The
48,436-byte raw image has SHA-256
`94ac835a473a8a910b740eb792c3a3567254ea297b1d23c31e2c7e52d0ec327b`.
This is not yet a physical display observation; the next write remains
explicitly confirmation-gated.

## Receive bring-up and the operating image

The K1 application now drives the radio chip directly. `EVID-K1-054` fixes the
three-wire pinout, `EVID-BK4829-055` fixes the chip variant, `EVID-K1-057`
records the first working reception, and `EVID-K1-060` records audible
demodulated audio.

Two board facts shape the image:

- **The chip is a BK4829.** The pinned K1 build compiles `driver/bk4829.c`, so
  the power blocks, receive mode word, audio output bits, filter bandwidth,
  gain tables, and sub-audio values all differ from the BK4819's. AFIK selects
  `BK4829_PROFILE` on this target.
- **The programming cable occupies the speaker jack.** Driving the audio path
  pin `PA8` removes the serial link, and the internal speaker is disconnected
  while the cable is inserted, so audio can be neither commanded nor heard over
  serial. `EVID-K1-059` records the evidence and `ADR-055` the consequence.

The operating image therefore puts the operator controls on the radio:

| Control | Action |
| --- | --- |
| Up / Down | Select the next or previous built-in channel |
| Side key 1 | Route or mute receive audio |

The display shows the channel name, the frequency in megahertz, the chip's raw
RSSI count, the squelch link, and the audio state. The serial link answers
`hello` and `probe-rf`, reading a published snapshot rather than touching the
bus, so a request can never bit-bang beside an inbound frame.

The image carries five receive-only built-in channels because AFIK does not yet
read channels from the radio. Every one is classified `TxClass::Never` and the
image constructs no transmit authority, so nothing in this path can key the
radio.

## The programmable receive image, `AFIK-K1-2.0`

The application is now programmed by the ordinary host tooling. USART1 carries
the AFIK configuration protocol, answered by the same `radio-device` service the
simulator uses, so `afik-studio`, `afik-programmer`, and the simulator all drive
one device implementation. Channels, named banks, and the global receive
configuration are written as a validated transaction and read back.

### Retained configuration

A committed configuration is written to the last 8 KiB erase sector of the
device, `0x0801E000` to `0x08020000`, as one canonical configuration image. The
application region stops at `0x0801E000`: the linker memory map, the raw-image
size gate, and the ELF LOAD gate all end there, so an application image can
never overwrite a retained configuration. A larger image is refused by
`tool/verify-k1-raw-image.sh` rather than truncated.

The image is retained *before* the commit response is sent. The host is waiting
for that response, so masking interrupts for the erase and page writes cannot
drop an inbound byte. An erased, foreign, truncated, or corrupt sector is
treated as "nothing retained": the built-in channels stay in charge and the
information screen says so.

Bounds are set by the evidenced 16 KiB of SRAM: twelve channels, sixteen named
banks, one radio configuration, and a 1,280-byte retained image. The device
advertises its object capacity and refuses a larger channel set with the stable
`ValidationFailed` code at validation time, before it could become active.

### Operator controls

| Control | Action |
| --- | --- |
| Up / Down | Previous or next channel, or move the list cursor |
| Menu | Open the channel list, or select the row under the cursor |
| Exit | Leave a screen, or clear a partly typed channel number |
| Digits | Type a channel number; two digits or a short pause selects it |
| Star | Open or close the bank list |
| Function | Show or hide the information screen |
| Side key 1 | Route or mute receive audio |
| Side key 2 | Hold the squelch open |

Typed numbers are the positions the operating screen shows, so what the operator
reads is what the operator can type. An out-of-range number selects nothing
rather than being clamped onto a channel nobody asked for.

The bank list offers every bank at least one programmed channel belongs to, by
the name the host gave it, plus an explicit "all channels" row. It opens on the
filter in force, Up and Down move the cursor, and Menu applies the row. Exit and
a second Star leave the filter alone. The operating screen shows the active
bank's name, falling back to `BANK nn` for a bank the host never named and `ALL`
when nothing is filtered.

The bit-banged radio bus only runs when the serial link has been quiet for
250 ms. Retuning is deferred, never dropped, so programming a radio while it is
tuned cannot corrupt either side.

### Serial witness

The earlier fixed-frame witness protocol is gone, so `afik-flasher probe-normal`
and `probe-rf` no longer apply to this image. The serial witness is now
`afik-programmer --device PATH --baud 38400 info`, which prints the negotiated
capabilities, and `list`, which prints the generation-tagged object listing. The
display witness is the information screen: image identity, active configuration
generation, channel count, and whether the configuration came from the radio's
own retained storage.
