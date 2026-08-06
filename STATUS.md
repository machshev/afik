# Project status

## Current work package

**Work Package 15 (`K1HIL-015`) is active: first AFIK K1 recovery-flasher
hardware run.**

K1 has priority because an exact unit running Armel firmware is available for
inspection. `K1EVID-013` supplies the K1 evidence baseline and same-unit
recovery proof for this bounded host-tool task. Trusted existing firmware
remains evidence, not production source: AFIK will not port, link, or
incrementally translate its application or driver implementation.

`FLASH-012` is deferred with its software milestone intact and physical gates
incomplete. It can resume unchanged when the exact K5 V1 hardware is available.

## State

- Repository foundation and first architecture milestone: complete.
- Work Package 2 programmer and simulator protocol loop: complete.
- Work Package 3 minimal target boot proof: complete.
- Work Package 4 canonical image/compiler round trip: complete.
- Work Package 5 simulator-first boot UI and hidden TX permissions: complete.
- Work Package 6 BK4819 receive path and token-gated TX boundary: complete.
- Work Package 7 channel activation and deterministic scanning: complete.
- Work Package 8 programmer CLI: complete.
- Work Package 9 programmer GUI: complete.
- Work Package 10 Frequency Copy research: complete.
- Work Package 11 APRS receive feasibility and repeater discovery: complete.
- Work Package 12 recovery-gated UV-K5 V1 firmware flashing: deferred; software
  complete and physical hardware unavailable.
- Work Package 13 UV-K1/PY32F071 hardware evidence and target contract: evidence
  baseline complete; board/MCU and AFIK boot-witness follow-up remains open.
- Work Package 14 K1/K5 auto-detected recovery flasher: complete.
- Work Package 15 first AFIK K1 recovery-flasher hardware run: active; this is
  a stock recovery-image exercise, not an AFIK application flash.
- `UI-005` logical key edges, bounded semantic views, exact boot-only entry,
  release gate, draft editor, and checked persistence action: complete.
- `UI-005` separate persisted/active policy simulation, deterministic timed
  trace, corrupt-state denial, and reboot-only activation proof: complete.
- `STORE-004` allocation-free image codec, exact version/length/CRC contract,
  complete pre-iteration validation, and maximum-count bound: complete.
- `STORE-004` canonical compiler ordering, image round trip, capacity report,
  and negotiated-capability revalidation: complete.
- `DP32-003` CPU, byte-order, flash/RAM, and reset-vector evidence contract:
  complete.
- `DP32-003` target crate, minimum vector/Reset image, and static ELF bounds
  verification: complete.
- `DP32-003` minimal Renode platform and pre-start/post-start boot-sentinel
  test: complete.
- `DP32-003` Rust 1.86 target build and locked-Nix target/Renode CI gates:
  complete.
- `PROTO-002`: complete.
- Bounded, paged `LIST_OBJECTS`: complete.
- Out-of-order multi-object write/list/read-back: complete.
- Explicit abort isolation and subsequent transaction recovery: complete.
- Transaction state errors preserve active data: complete.
- Candidate validation and capacity errors preserve active data: complete.
- Unsupported service/command, malformed payload, and missing-object matrix:
  complete.
- Bounded duplicate-sequence replay and conflict rejection: complete.
- Fragmented and malformed stream recovery: complete.
- `RF-006` official product/datasheet provenance, mirrored-application-note
  boundary, interface/frequency/status/mode facts, low-confidence command-plan
  inference, published-band contradiction, and required experiments: complete.
- `RF-006` heap-free driver, exact command ordering/status decoding,
  class-bound capability token, fail-closed state recovery, deterministic RF
  simulation, and mismatch/failure trace proofs: complete.
- `SCAN-007` checked activation/navigation, explicit timer-token dwell/hold
  state, stale expiry safety, scan-time TX denial, selected-state policy bundle,
  and repeatable integrated control/RF traces: complete.
- `CLI-008` snapshot backup encoding, strict simulator/serial command front end,
  bounded safe files, stable output/status, transactional write/restore with
  read-back, and binary tests: complete.
- `GUI-009` shared verified workflows/serial transport, persistent local
  session, bounded loopback HTTP, responsive object workflow, canonical
  downloads/uploads, confirmed token-gated mutation, and binary tests:
  complete.
- `FREQ-010` FCC workflow provenance, Air Copy separation, bounded observation
  matrix, receive-only candidate/state proposal, storage/TX boundary,
  experiment plan, and hardware-command defer verdict: complete.
- `APRS-011` primary AX.25/APRS/frequency provenance, physical-layer defer
  verdict, complete-frame parser, Object/Item voice-repeater advertisements,
  fixed-capacity explicit-time table, and isolated deterministic simulator:
  complete.
- `FLASH-012` sourced bootloader-v2 evidence, reserved bootloader boundary,
  complete raw application package, read-only EEPROM backup, guarded flashing
  library, explicit Linux CLI, and deterministic protocol tests: complete.
- `FLASH-012` exact-unit inspection, physical backup, recovery rehearsal,
  page-acknowledged AFIK write, and independent application-boot observation:
  pending; no serial device is visible here.
- Current smallest actionable task: complete `K1HIL-015` with one AFIK-hosted
  stock recovery write and post-flash backup comparison, then return to the
  independently implemented AFIK reset/boot-witness package under
  `K1EVID-013`; K1 AFIK application flashing remains out of scope until that
  contract exists.

## Work Package 14 implementation milestone

- `radio-k5-flasher` was renamed to `radio-flasher`; the library now owns both
  the existing K5 V1 path and the independently implemented K1 recovery path.
  `afik-k5` remains as the explicit-device compatibility binary, while
  `afik-flasher` provides generic K1/K5 identification, backup, and recovery
  flashing.
- Auto mode prefers `/dev/serial/by-id/usb-*`, falls back to numeric
  `/dev/ttyUSB*` and `/dev/ttyACM*` candidates, rejects zero or multiple
  candidates, and never treats USB metadata as hardware identity. Protocol
  selection is fail-closed from validated `2.*` K5 or pinned `7.03.*` K1
  beacons.
- K1 recovery validates vectors and the bounded application range, performs the
  observed `0x0530` handshakes, sends 256-byte `0x0519` pages with final-page
  zero padding, and requires exact transaction/page/result acknowledgements
  without retry. The K1 device-side trailer convention is now a separate
  compatibility gate under `K1HIL-015`; K1 AFIK flashing remains unavailable.

## Work Package 15 hardware attempt

- The two private recovery-image copies remained mode `0600`, byte-identical,
  95,836 bytes, and matched the pinned SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`.
  The two private 8 KiB backup copies remained byte-identical. AFIK image
  CRC-32 confirmation was `fecee2ca`; vectors were `0x20004000` and Thumb
  `0x08002d49`.
- `nix develop path:. -c cargo run --quiet --package radio-flasher-cli
  --bin afik-flasher -- --device auto identify` — passed immediately before
  the write; `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`, K1,
  bootloader `7.03.01`.
- The guarded command was invoked once with the unchanged stock recovery image,
  backup, exact target phrase `UV-K1-F4HWN-7.03.01`, and CRC `fecee2ca`.
  It generated transaction `e8fb6b28` and stopped with `serial response
  timed out` before printing any page acknowledgement. Because the timeout
  does not prove whether a final page request reached the device, this run is
  recorded as ambiguous and will not be retried blindly.
- A subsequent read-only AFIK identify, a three-second passive capture, and a
  host-side DTR/RTS assertion plus passive read all received no beacon. No
  reset, EEPROM, or RF command was sent by AFIK. The next action is a user
  power-cycle into normal Fusion mode and a complete read-only backup/identity
  check before any further flash command.
- After the user power-cycled the unit, `afik-flasher identify` again timed out
  without a K1 beacon, and the read-only `afik-flasher backup-eeprom` workflow
  also timed out without a normal-mode hello. The serial adapter remains
  present, but the radio is currently reachable in neither observed mode.
  Recovery is paused until the exact physical bootloader-entry procedure is
  repeated and the `7.03.01` beacon returns; no further write is authorized.

Verification on 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed: 128 unit/integration
  tests and all doc tests.
- `git diff --check` — passed after the implementation and rename.

Work-package activation verification on 2026-08-06:

- `git ls-remote --symref https://github.com/armel/uv-k1-k5v3-firmware-custom.git HEAD`
  — passed; upstream `HEAD` is `refs/heads/main` at
  `fe9c4e9432694b50aea651084a043aae0b58673d`.
- `git ls-remote --heads https://github.com/armel/uv-k1-k5v3-firmware-custom.git`
  — passed and confirmed that the repository has no `master` branch.
- `nix develop path:. -c cargo fmt --all --check` — passed; the activation
  milestone changes documentation only.
- `git diff --check` — passed before the final status record.

## Work Package 13 first evidence milestone

- Pinned upstream `main` at
  `fe9c4e9432694b50aea651084a043aae0b58673d`, dated 2026-08-04, and recorded
  SHA-256 values for the linker, startup, main, and version evidence files.
- The pinned Fusion preset identifies version `v5.8.0`. The exact displayed
  version on the available unit remains to be supplied and must not be inferred
  from the source checkout.
- Puya's official PY32F071-E product page and datasheet v1.4 establish the
  Cortex-M0+, maximum 128 KiB flash/16 KiB SRAM, USB, SWD, and peripheral
  envelope. The exact fitted suffix remains pending physical inspection.
- Pinned Armel evidence places the application at `0x08002800` with 118 KiB,
  RAM at `0x20000000` with 16 KiB, and identifies initial LCD, keypad/PTT,
  BK4819, audio, backlight, and external-flash board mappings. These are
  evidence entries, not imported production code.
- `docs/k1-bring-up.md` records the first evidence matrix, exact-unit checklist,
  and safe backup/DFU/recovery order. No device was visible during that initial
  milestone, no hardware operation had then been performed, and TX remains
  prohibited.

Evidence-milestone verification on 2026-08-06:

- Detached checkout of commit
  `fe9c4e9432694b50aea651084a043aae0b58673d` — passed and reported commit date
  2026-08-04 17:45:07 +02:00.
- `sha256sum Core/py32f071xb.ld Core/startup_py32f071xx.s Core/Inc/main.h Core/Src/main.c App/version.h`
  in that checkout — passed and matched `docs/k1-bring-up.md`.
- `nix develop path:. -c cargo fmt --all --check` — passed. An initial sandboxed
  invocation could not access the Nix daemon; the permitted identical retry is
  the recorded verification result.
- `git diff --check` — passed before the final status record.

## Work Package 13 exact-unit passive beacon milestone

- The user identified the installed application as Armel Fusion `v5.5` and
  connected the exact K1 in bootloader mode through `/dev/ttyUSB0`.
- Read-only udev/sysfs inspection identified a QinHeng CH340/CH341 adapter,
  USB `1a86:7523`, Linux driver `ch341-uart`, vendor-specific interface
  `ff/01/02`, USB 1.10 at 12 Mbit/s.
- A three-second passive capture at 38,400 baud received 140 bytes. The pinned
  decoder found one complete `0x0518` device-info frame with printable
  bootloader version `7.03.01`. The UID field was present but redacted and is
  not recorded.
- The host transmitted no handshake, command, reset, or flash bytes. No backup
  or recovery proof exists yet, so no write is authorized.
- Current smallest actionable task: reboot the exact unit into normal Fusion
  `v5.5` and create a complete read-only 8 KiB configuration/calibration backup
  before returning to bootloader mode.

Passive-beacon verification on 2026-08-06:

- `udevadm info --query=all --name=/dev/ttyUSB0` — passed and reported the
  adapter and driver metadata above.
- `stty -F /dev/ttyUSB0 38400 raw -echo -crtscts` — passed; host adapter setup
  only.
- `timeout 3s dd if=/dev/ttyUSB0 of=/tmp/afik-k1-passive-beacon.bin bs=512 count=4 status=none`
  — passed and captured 140 unsolicited bytes without transmitting.
- Offline decode with pinned `tools/serialtool/msg.py` — passed with one
  `0x0518` frame, decoded length 36, data length 32, and version `7.03.01`;
  UID output was suppressed.

## Work Package 13 initial normal-mode backup experiment

- Three initial bounded dump attempts used the V2 timestamp-session tool through
  the CH340 adapter: initially, after a radio power cycle, and after both cable
  ends were reconnected. Each reached the normal-mode device-info hello and
  timed out after 30 seconds without a response.
- No backup file was created and no write, restore, reboot, bootloader
  handshake, firmware page, or reset command was sent.
- These failures were later explained by the pinned Armel CHIRP evidence: the
  V2 tool used a timestamp where this unit expects fixed session word
  `0x6457396A`. The corrected fixed-session result is recorded below.

## Work Package 13 corrected backup milestone

- The user reported that the exact CH340 cable has repeatedly worked with
  CHIRP on this radio, superseding cable failure as the leading explanation.
- Armel's K1-capable CHIRP driver was pinned at commit
  `a0e9314570cd4f5440aca8322ca1722163bad217`. Its normal hello/read requests
  use fixed session word `0x6457396A`; the failed V2 tool used a timestamp.
- AFIK's existing tested `backup-eeprom` command uses the fixed CHIRP session
  word and exposes reads only. It succeeded through `/dev/ttyUSB0`, identified
  `F4HWN v5.5.0`, validated all 8,192 bytes, and created a private mode-`0600`
  temporary backup outside the repository.
- The unit-specific CRC-32 and SHA-256 were reported to the user but are not
  committed. No EEPROM write, reset, bootloader handshake, firmware page, or
  RF operation occurred.
- The existing private primary and secondary copies match this read. One fresh
  mode-`0600` copy on `/tmp` was also byte-identical; its different filesystem
  device supplied a cross-filesystem check, but `/tmp` is not durable storage.
- The same-unit recovery rehearsal is now complete; current implementation
  work is tracked under `K1FLASH-014`.

## Work Package 13 repeat normal-mode verification

- After the user power-cycled the unit into normal Fusion mode, read-only
  inspection again found `/dev/ttyUSB0` as the CH340/CH341 `1a86:7523`
  interface.
- A raw hello response was `0x0515` and identified `F4HWN v5.5.0`. The
  corrected fixed-session reader then received all 8,192 bytes in 128-byte
  blocks and validated every response offset and length.
- The fresh mode-`0600` backup matched both `/tmp/afik-k1-unit-backup.raw` and
  `.private/k1/unit-backup.primary.raw` byte-for-byte. It was on filesystem
  device `43`; the repository and private copy are on device `56`.
- Only the normal hello and read requests were sent. No write, restore, reset,
  bootloader entry, firmware operation, or RF operation occurred.
- Exact physical markings/USB identity observations and physical recovery
  remain open. The two verified local copies are accepted for this package;
  their shared filesystem remains a documented durability risk, not a current
  blocker. The unchanged recovery image has since been flashed and verified
  by a byte-identical post-flash backup.

K1 evidence-policy and documentation verification on 2026-08-06:

- `udevadm info --query=property --name=/dev/ttyUSB0` — passed; the CH340/CH341
  `1a86:7523` interface was present.
- `python3 /tmp/afik-k1-readonly-backup.py /tmp/afik-k1-unit-backup-20260806-normal-final.raw`
  — passed; normal identity `F4HWN v5.5.0`, 8,192 bytes, every read offset and
  length validated.
- `cmp -s` against the prior temporary and `.private/k1/unit-backup.primary.raw`
  — passed; both comparisons were byte-identical. Unit-specific hashes remain
  outside tracked documentation.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all workspace unit,
  integration, and doc tests passed.
- `git diff --check` — passed.

## Work Package 13 same-unit recovery rehearsal milestone

- The unchanged pinned 95,836-byte `F4HWN v5.5.0` recovery candidate was
  flashed to the exact unit after the local backup and recovery copies were
  verified.
- The live bootloader beacon was exactly `7.03.01`. Three K1 handshakes
  preceded 375 sequential 256-byte page requests. Every acknowledgement
  matched the transaction identifier and page index and returned zero; no page
  was retried and no reset command was sent.
- After a user power-cycle, the unit identified as `F4HWN v5.5.0`. A complete
  8,192-byte read-only backup matched the pre-flash private backup byte-for-
  byte. This proves same-unit recovery to the known-good firmware, but not an
  AFIK K1 image boot.
- `python3 /tmp/afik-k1-flash-once.py --check` — passed; 95,836 bytes and 375
  pages matched the pinned candidate. The bounded writer and post-flash
  `python3 /tmp/afik-k1-readonly-backup.py` both passed. No RF operation was
  performed.

## Work Package 13 recovery-candidate milestone

- Pinned Armel `main` contains `archive/f4hwn.fusion.v5.5.0.bin`, matching the
  exact unit's normal firmware identity. It is a 95,836-byte raw image with
  SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`.
- Static validation passed: initial SP `0x20004000`, Thumb Reset vector
  `0x08002D49`, and exclusive end `0x08019E5C` inside the evidenced main flash.
  An initial attempt to apply the packed-image decoder correctly rejected the
  file because it already has valid raw-image vectors; no derived file was
  created.
- This is only a source- and vector-valid recovery candidate. Physical recovery
  is not proven and no firmware write is authorized until the exact unit and
  recovery procedure are validated. Two verified local copies are accepted for
  the current evidence package.

- The user selected a gitignored directory inside this repository for local
  artifacts. `.private/` is reserved for that purpose; its contents must remain
  untracked and must not appear in status, diffs, or commits.

## Work Package 13 private-artifact milestone

- `.private/k1/` was created mode `0700`. Primary and secondary copies of the
  8,192-byte unit backup and 95,836-byte v5.5.0 recovery candidate were created
  mode `0600`.
- Both backup copies matched the private SHA-256 reported to the user. Both
  recovery copies matched pinned SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`.
- `git status --short --ignored` reported only `!! .private/`; no private file
  is tracked or staged.
- These pairs share one filesystem and are not independent disaster-recovery
  copies. The user accepts that durability risk for the current evidence
  package; it remains recorded but is not an active gate.

## Work Package 13 home recovery-copy milestone

- At the user's request, a private home recovery directory was created mode
  `0700` outside the repository. It contains one mode-`0600` unit backup and
  one mode-`0600` v5.5.0 recovery candidate. Its absolute path is deliberately
  omitted from tracked content.
- Both files match their verified source hashes. The unit-specific backup hash
  remains outside tracked documentation; the recovery image matches pinned
  SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`.
- `stat` reports filesystem device `56` for both the repository and home-copy
  directory. The new location is outside Git and reduces accidental-deletion
  risk, but it is not independent storage against filesystem or disk failure.
- The user accepts the shared-filesystem risk for now; recovery remains gated
  by exact-unit identification and a validated non-destructive procedure.

## Work Package 13 CPU/memory/image contract milestone

- Added exact relative source locations for the Cortex-M0+ target, reset/vector
  startup, application flash origin and size, SRAM range, assumed 48 MHz clock,
  USB CDC, board GPIO, ST7565, BK4819, and PY25Q16 bindings.
- Recorded the first bounded target contract: application origin `0x08002800`,
  118 KiB capacity ending at `0x08020000`, 16 KiB SRAM at `0x20000000`, and a
  raw v5.5.0 recovery candidate whose vectors and exclusive end are inside that
  source-declared range.
- Kept the 48 MHz clock as a bootloader-handoff assumption, not a physical
  MCU fact, and kept all board bindings as source evidence pending exact-unit
  inspection. No production source was copied from Armel and no TX behavior was
  added.

Contract-milestone verification on 2026-08-06:

- `git -C /tmp/afik-armel-k1-evidence status --short --branch` — passed; the
  detached checkout is at pinned commit `fe9c4e9432694b50aea651084a043aae0b58673d`.
- `(cd /tmp/afik-armel-k1-evidence && sha256sum App/board.c App/driver/gpio.h
  App/driver/vcp.c App/usb/usbd_cdc_if.c CMakeLists.txt CMakePresets.json
  App/version.c archive/f4hwn.fusion.v5.5.0.bin)` — passed; all files were
  read from that pinned checkout.
- `find /dev -maxdepth 1 -type c \( -name 'ttyUSB*' -o -name 'ttyACM*' \)
  -print` — passed with no matches; no physical radio was available for
  markings, USB, DFU, or recovery observations.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed: 123 unit/integration
  tests and all doc tests, 0 failures.
- `git diff --check` — passed before the status record.

## Work Package 12 software milestone and verification

- Sources and confidence boundaries are recorded in
  `docs/hardware-evidence.md`; `docs/k5-flashing.md` is the physical runbook and
  experiment record. The implementation is intentionally limited to an
  inspected UV-K5 V1/DP32G030 unit with an exact version-2 bootloader beacon.
- The target linker owns only `0x0000..=0xEFFF`. Packaging verifies the ELF,
  emits exactly `0xF000` bytes padded with `0xFF`, and independently rejects
  truncation, corruption, or any overlap with the preserved
  `0xF000..=0xFFFF` stock bootloader.
- `radio-flasher` owns bounded legacy framing, CRC/XOR handling, strict
  version negotiation, complete read-only EEPROM backup, image validation,
  prerequisite checks before I/O, and exactly 240 sequential acknowledged
  256-byte writes without ambiguous retry. `afik-k5` keeps the serial front end
  explicit and thin.
- The generated AFIK package is 61,440 bytes, has SHA-256
  `89f93c262541985182599bebdcc808aa7a9af392f7c781a759c38e619481e14b`,
  application CRC-32 `78f0bfdc`, initial SP `0x20004000`, and Reset vector
  `0x00000101`. It is still only the minimal RAM-sentinel firmware, not a
  user-visible hardware build.
- No `/dev/ttyUSB*` or `/dev/ttyACM*` character device was visible. No radio was
  probed, backed up, written, or claimed to boot; physical completion remains
  gated exactly as specified by `FLASH-012`, ADR-020, RISK-014, and RISK-015.

Verification on 2026-08-06:

- `nix flake check path:. --no-build` — passed for the current x86_64-linux
  system; the flake reported its aarch64-linux output as incompatible and
  omitted it.
- `nix develop path:. -c rustc --version` and
  `nix develop path:. -c cargo --version` — reported Rust 1.97.1 and Cargo
  1.97.0 from the pinned shell.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed: 123 unit/integration
  tests and all doc tests.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-flash-012-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 123 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.
- `nix develop path:. -c tool/build-dp32g030.sh` and
  `nix develop path:. -c tool/verify-dp32g030-image.sh` — passed; flash image
  end `0x00000268`, declared application end `0x0000f000`, and vectors matched
  the values above.
- `nix develop path:. -c tool/package-k5-v1-image.sh --force` and
  `nix develop path:. -c tool/test-k5-v1-package.sh` — passed the package
  generation plus positive, truncated, and corrupt-image checks; the SHA-256
  matched the value above.
- `nix develop path:. -c tool/test-renode.sh --repeat 3` — passed all three
  Reset-to-Rust-sentinel iterations.
- `env RUSTC=/tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/rustc CARGO_HOME=/tmp/afik-cargo-home-1-86 CARGO_TARGET_DIR=/tmp/afik-flash-012-rust-1-86-thumb-target /tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/cargo build --package radio-firmware-dp32g030 --features firmware --bin radio-firmware-dp32g030 --target thumbv6m-none-eabi`
  — passed on Rust/Cargo 1.86.0. An initial attempt with the standalone Nix
  Rust 1.86 compiler failed before code generation because that output does not
  contain the `thumbv6m-none-eabi` core library; the target-complete pinned
  Rustup toolchain above is the applicable minimum-target gate.
- `nix develop path:. -c cargo run --quiet --package radio-flasher-cli --bin afik-k5 -- inspect target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030-k5-v1.raw`
  — passed and reported the package size, vectors, and CRC-32 above.
- `find /dev -maxdepth 1 -type c \\( -name 'ttyUSB*' -o -name 'ttyACM*' \\) -print`
  — passed with no matches, confirming that a physical serial exercise was not
  possible in this environment.

## Completed Work Package 11 exit criteria

- Primary AX.25, APRS 1.0.1/addendum, and APRS frequency-spec provenance,
  checksums, exact framing/field facts, conflicting path bounds, inferences,
  and hardware unknowns are recorded with confidence boundaries.
- `docs/aprs-feasibility.md` gives explicit implement/defer verdicts from RF
  through discovery and names receive-only equipment, recovery, corpus,
  performance, false-frame, overflow, cancellation, and cleanup experiments.
- `radio-aprs` is hardware-independent, `no_std`, heap-free, allocation-free,
  bounded, integer-only, and passes a `thumbv6m-none-eabi` warning-denied lint
  with `radio-domain`.
- Complete de-stuffed frames enforce zero through eight APRS path entries,
  shifted callsign/SSID/reserved/extension bits, UI `0x03`, PID `0xF0`, 1 through
  256 information octets, exact maximum length, and CRC-X25/FCS residue before
  exposing APRS information.
- Supported Object/Item reports validate names, lifecycle, timestamps,
  uncompressed coordinates and all ambiguity levels, voice-repeater symbol,
  both standard frequency widths, optional alternate input, CTCSS/DCS/off,
  signed 10 kHz offset, and nominal range. Values retain source/SSID and remain
  untrusted advertisements rather than trusted channel fields.
- The fixed-capacity kind/name/source table uses explicit monotonic receive
  time, rejects equal-time conflicts and stale input, never evicts, retains
  same-origin kill freshness against stale resurrection, and expires only on
  an explicit cutoff. Identical simulator scripts produce identical rejection,
  update, full-capacity, kill, and expiry traces.
- Discovery has no channel-control or RF-simulator connection and cannot
  construct `ActiveChannel`, trusted `Tone`, plan membership, `TxClass`, or
  `TxAuthorisation`. No modem/register command, audio DSP, NRZI/HDLC recovery,
  target adapter, persistence mutation, automatic tuning, transmission,
  flashing, or physical-success claim was added. `RISK-012` and `RISK-013`
  remain open.

## Work Package 11 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 104 unit/integration
  tests and all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-aprs --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-aprs` and `radio-domain`.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-aprs-011-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 104 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 10 exit criteria

- The FCC-filed manual's exhibit identity, checksum, exact Fast Copy controls,
  displayed/saved outputs, known-frequency CTCSS/DCS scan, and distinct
  transmitting Air Copy workflow are recorded with scope and confidence.
- Beken's advertised scan/signalling capabilities are separated from the
  machine-translated revision-unverified register description and one
  non-independent descendant firmware observation. Unexplained constants are
  named and remain prohibited from production or physical simulation.
- The feasibility design inventories observable and non-observable properties,
  specifies a heap-free bounded receive-only candidate and explicit-input/token
  state flow, preserves signalling uncertainty, and treats cleanup failure as a
  fault latch.
- Capture cannot become `ActiveChannel` or mint TX authority. Any future save is
  separately confirmed, requires a new receive-only storage representation,
  and remains `TxClass::Never`; no RX-to-TX inference is permitted.
- Receive-only equipment/recovery, register, frequency/level, false-lock,
  CTCSS/DCS, cancellation, stale-result, bus-fault, and cleanup experiments plus
  future deterministic tests are specified. The explicit verdict is
  design-ready but hardware-command blocked under `RISK-011`.
- No behavioral code, register command, target adapter, register-level
  simulator, automatic storage mutation, transmission, flashing, or physical
  success claim was added.

## Work Package 10 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 82 unit tests and all
  doc tests, 0 failures.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-freq-010-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 82 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 9 exit criteria

- `radio-programmer` owns verified write, canonical backup, validated restore,
  and exact generation/object read-back mismatch handling. Both front ends use
  the shared `radio-programmer-serial` Linux path/baud adapter.
- `afik-programmer-gui` retains one explicitly selected simulator or serial
  session. Capability, generation, and object views refresh from that same
  session; simulator write/backup/restore behavior is repeatable.
- The dependency-free server accepts only loopback IP addresses, caps headers
  at 16 KiB and bodies at 8 MiB, rejects ambiguous/chunked framing, and survives
  client I/O failures without ending the selected device session.
- Generated-bank text is strict and capped at 64 KiB. Compile and backup return
  canonical downloads; restore accepts bounded uploaded bytes. No endpoint
  accepts a server filesystem path or exposes a raw write.
- Responsive embedded assets provide readable capabilities, object listing,
  project editing, status, downloads, and deliberate write/restore confirmation.
  Mutation also requires a random 256-bit per-process token and an explicit
  replacement header; CSP/no-store/no-sniff responses preserve the local
  same-origin boundary without claiming authentication.
- Model, endpoint, parser, asset, launcher, shared-workflow, mismatch, serial,
  and CLI-regression tests pass. `RISK-009` and `RISK-010` remain open; no target
  UART, physical programming, remote service, firmware flashing, or security
  capability is claimed.

## Work Package 9 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 82 unit/integration
  tests and all doc tests, 0 failures.
- `nix develop --command cargo run -q -p radio-programmer-gui --bin afik-programmer-gui -- --help`
  — passed with the stable help document.
- `nix develop --command cargo run -q -p radio-programmer-gui --bin afik-programmer-gui -- --version`
  — passed with `afik-programmer-gui 0.1.0`.
- The process-level binary test also confirms exact help/version output and
  exit status 2 for a rejected `0.0.0.0:9000` listener.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-gui-009-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 82 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.
- The first minimum-toolchain audit exposed that its sandbox prohibited two
  test-created TCP sockets. HTTP parsing/serialization tests were made generic
  over deterministic byte streams, while production remains a loopback
  `TcpListener`; the final command above then passed. One intermediate retry
  pointed `RUSTDOC` at a nonexistent store path and stopped before doc tests;
  correcting that invocation required no code change.

## Completed Work Package 8 exit criteria

- `ConfigurationSnapshot` validates canonical order and supported objects,
  reports exact capacity, and emits the shared canonical image without front-end
  reimplementation.
- `afik-programmer` supports info, list, compile, write, backup, and restore over
  an explicitly selected fresh simulator or serial device path plus baud.
- CLI parsing validates backend exclusivity, supported baud, bounded bank
  fields, class names, command arity, and force semantics. Usage and operation
  errors have distinct stable exit codes.
- Input streaming is capped at 8 MiB. Compile/backup refuse existing outputs by
  default and replace only under explicit `--force`.
- Write and restore compile/decode in `radio-programmer`, use object-level
  transactions, then require exact generation and object read-back.
- The serial adapter adds no unsafe code, raw command, discovery/default, target
  UART assumption, or physical interoperability claim; `RISK-009` remains open.

## Work Package 8 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 68 unit/integration
  tests and all doc tests, 0 failures.
- `nix develop path:. -c cargo run --quiet --package radio-programmer-cli --bin afik-programmer -- --sim info`
  — passed with the exact six negotiated simulator capability fields.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-cli-008-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 68 unit/integration tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 7 exit criteria

- `radio-channel-control` is hardware-independent, `no_std`, heap-free,
  allocation-free, bounded, and passes a `thumbv6m-none-eabi` warning-denied
  lint with its embedded dependencies.
- Initial/manual indexes are checked before mutation; navigation and scan
  advancement wrap exactly; each update emits at most one activation.
- The controller owns no clock. Non-zero integer dwell/hold configuration,
  fresh bounded timer tokens, early-deadline enforcement in the host adapter,
  and stale/cancelled token tests make scheduling explicit and deterministic.
- Open squelch restarts/rearms hold without retuning; a closed hold expiry
  advances once and rearms dwell. Signal values remain logical inputs.
- Scanning cannot obtain TX authority. Selected state goes through `TxPolicy`,
  carries the exact class-bound token, and reaches logical TX only through the
  BK4819 driver; denial leaves the RF trace unchanged.
- Identical timed scan, hold, stop, and TX scripts produce identical control and
  RF traces. No physical timing, signal, target peripheral, or RF claim was
  added.

## Work Package 7 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 60 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-channel-control --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-channel-control` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-scan-007-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 60 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 6 exit criteria

- `radio-bk4819` is hardware-adapter-independent, `no_std`, heap-free, uses
  checked integer units, and passes a `thumbv6m-none-eabi` warning-denied lint.
- Register addresses, fields, formulas, combined-mode inferences, provenance,
  confidence, contradictory bands, and required physical experiments are
  recorded before and alongside the implementation.
- Exact frequency packing, standby-first receive/TX ordering, status decoding,
  state rejection, stop/recovery, and failure at every logical read/write step
  are tested.
- `TxAuthorisation` carries its approved class. The driver's only TX-mode path
  borrows a token, checks an exact channel-class match before any write, and
  cannot complete after a fault without explicit neutral-mode recovery.
- Identical virtual-time RF scripts produce identical traces. Mismatched
  authority emits no register operation or TX event, and a failed final
  TX-mode write emits no completed TX event.
- No physical bus, initialization sequence, board RF control, external PA,
  physical receive result, flashing, or on-air transmission was added.

## Work Package 6 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 50 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-bk4819 --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-bk4819` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-rf-006-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 50 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 5 exit criteria

- `radio-ui` is `no_std`, heap-free, allocation-free, hardware-independent, and
  passes a `thumbv6m-none-eabi` lint build.
- Only the exact initial logical `Menu+Back` set enters the hidden editor;
  incomplete, additional, and post-boot keys cannot enter, and all held keys
  must be released before editing.
- The fixed selectable order contains all six authorisable classes and excludes
  `Never`; bounded views expose selection, enabled/changed state, save errors,
  and saved generation without physical display assumptions.
- Cancel emits no record. Deliberate save emits one next-generation redundant
  CRC-protected record, while generation exhaustion emits none.
- The UI never owns live policy or constructs authorization. Simulator save
  changes only persisted bytes; only a validated reboot changes active policy.
- Corrupt persistence defaults both editor draft and active policy to deny-all;
  identical timed scripts produce identical traces and bytes.
- No physical key/display behavior, non-volatile write, serial permission
  object, hardware register access, or TX driver was added.

## Last verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 42 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; cargo clippy -Z build-std=core --package radio-ui --target thumbv6m-none-eabi -- -D warnings'`
  — passed for `radio-ui` and its embedded dependencies.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-ui-005-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 42 unit tests and all doc tests on Rust/Cargo 1.86.0.

## Completed Work Package 4 exit criteria

- The `no_std` storage codec has no heap dependency and encodes only into a
  caller-provided buffer.
- The `AFIK` image header, object envelopes, versions, lengths, canonical key
  order, and CRC-32 coverage are explicit and tested with an exact byte vector.
- Decoding checks the complete checksum, structure, order, and every object
  before returning an iterable image.
- Compiler output is byte-identical for equal logical projects regardless of
  insertion order; importing an image reconstructs the same objects and
  capacity report after enforcing every negotiated target bound.
- Empty and maximum-`u16`-count images pass; corrupt, truncated, trailing,
  reordered, duplicate, malformed, unsupported-version, and over-capacity
  images fail explicitly.
- Existing protocol and simulator behaviour remains green. The image remains
  offline and logical; physical layout and durability stay open in `RISK-004`.

## Work Package 4 verification

Verified 2026-08-06:

- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c cargo test --workspace` — passed: 35 unit tests and
  all doc tests, 0 failures.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc RUSTDOC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustdoc CARGO_TARGET_DIR=/tmp/afik-store-004-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 35 unit tests and all doc tests on Rust/Cargo 1.86.0.
- An initial minimum-toolchain invocation pinned `RUSTC` but not `RUSTDOC`;
  all 35 unit tests passed, then ambient Rustdoc 1.97 rejected the Rust 1.86
  artifacts as compiler-incompatible. Pinning both tools as above fixed the
  environment mismatch without a code change.

## Completed Work Package 3 exit criteria

- The target image uses only source-backed Cortex-M0 and memory-map facts.
- A pinned `thumbv6m-none-eabi` build emits a bounded, heap-free image with a
  valid initial stack pointer and reset vector.
- A minimal Renode Cortex-M0/flash/RAM platform boots that exact ELF and an
  automated test observes the expected RAM sentinel.
- Host workspace checks remain green and target/Renode commands are recorded.
- No peripheral behaviour is invented and no hardware is flashed.

## Work Package 3 verification

Verified 2026-08-05:

- `nix flake check path:. --no-build` — passed on `x86_64-linux`; incompatible
  `aarch64-linux` output was evaluation-skipped by Nix.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed for all host crates and targets.
- `nix develop path:. -c bash -c 'export RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH"; export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"; cargo clippy -Z build-std=core --package radio-firmware-dp32g030 --features firmware --bin radio-firmware-dp32g030 --target thumbv6m-none-eabi -- -D warnings'`
  — passed for the embedded target image.
- `nix develop path:. -c cargo test --workspace` — passed: 27 unit tests and
  all doc tests, 0 failures.
- `nix develop path:. -c tool/build-dp32g030.sh` — passed.
- `nix develop path:. -c tool/verify-dp32g030-image.sh` — passed: initial SP
  `0x20004000`, Reset vector `0x00000101`, boot sentinel `0x20000000`, and
  flash image end `0x00000268`.
- `nix develop path:. -c tool/test-renode.sh --repeat 3` — passed all three
  reset-from-vector boot-sentinel iterations.
- `env RUSTC=/nix/store/2mm3p5wcy1ifrcx5vp3bwsw7a76r77jc-rustc-1.86.0/bin/rustc CARGO_TARGET_DIR=/tmp/afik-host-rust-1-86-target /nix/store/npqlgsia03kfhv8m9mav6hfnbawpg0yg-cargo-1.86.0/bin/cargo test --workspace`
  — passed: 27 unit tests and all doc tests on Rust/Cargo 1.86.0.
- `env RUSTC=/tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/rustc CARGO_HOME=/tmp/afik-cargo-home-1-86 CARGO_TARGET_DIR=/tmp/afik-rust-1-86-target /tmp/afik-rustup-1-86/toolchains/1.86.0-x86_64-unknown-linux-gnu/bin/cargo build --package radio-firmware-dp32g030 --features firmware --bin radio-firmware-dp32g030 --target thumbv6m-none-eabi`
  — passed on Rust/Cargo 1.86.0.
- `nix develop path:. -c tool/verify-dp32g030-image.sh /tmp/afik-rust-1-86-target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030`
  — passed: initial SP `0x20004000`, Reset vector `0x000000cd`, boot sentinel
  `0x20000000`, and flash image end `0x00000220`.
- Hardware-in-loop tests — not run; flashing and physical-silicon claims were
  outside `DP32-003`, and recovery/package evidence remains open in `RISKS.md`.
