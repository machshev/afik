# Hardware evidence

Hardware facts are recorded before they are encoded in target or simulator
source. A simulator result confirms software behaviour against its declared
model; it does not increase confidence in the underlying silicon fact.

## Source selected by K1EVID-013

### Armel UV-K1/K5-V3 firmware

- **Repository:** `armel/uv-k1-k5v3-firmware-custom`.
- **Revision:** upstream default branch `main`, commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`, resolved 2026-08-06 with
  `git ls-remote --symref`; the upstream repository has no `master` branch.
- **Standing:** the user reports that the firmware currently runs on the exact
  available UV-K1 and that the project has direct manufacturer support and
  sponsorship. `K1EVID-013` accepts the pinned project as trusted,
  hardware-tested board evidence.
- **Permitted use:** cite exact source locations as evidence for experiments
  and an independent Rust implementation, corroborating MCU behavior against
  Puya documentation and board bindings against the exact unit.
- **Prohibited use:** copying, linking, porting, or incrementally translating
  its application or driver implementation into AFIK production source.
- **Observed identity:** the corrected fixed-session normal-mode read identified
  the exact unit as `F4HWN v5.5.0`; the user reports the displayed product line
  as Fusion `v5.5`. This does not prove that the unit runs the pinned `v5.8.0`
  checkout.
- **Remaining:** record physical markings and USB identities, and map each
  accepted source fact to an exact-unit observation and confidence.

The initial hashes, source-to-fact matrix, unit record, and safe experiment
order are maintained in `docs/k1-bring-up.md`.

### EVID-K1-016 — Exact-unit passive bootloader beacon

- **Observation:** on 2026-08-06 the available K1, reported to run Armel Fusion
  `v5.5`, was placed in bootloader mode and connected through a CH340/CH341
  `1a86:7523` serial adapter. At 38,400 baud it unsolicitedly emitted a valid
  `0x0518` device-info frame whose printable bootloader version was `7.03.01`.
- **Method:** AFIK passively captured for three seconds and decoded the frame
  using the pinned message-envelope evidence. No handshake or other byte was
  transmitted. The device UID field was present but is redacted and not stored
  in repository content.
- **Confidence:** high for this exact unit, adapter, beacon shape, and version;
  low for other K1 revisions and no confidence yet in write or recovery
  behavior.
- **Permitted use:** identify the exact observed bootloader family and reject
  K5 V1 bootloader-v2 assumptions. This observation does not authorize a write.
- **Required next experiment:** return to normal Fusion `v5.5`, create and
  validate the complete read-only configuration/calibration backup, then
  validate a known-good recovery image before any bootloader handshake.

### EVID-K1-017 — Initial timestamp-session normal-mode read path did not answer

- **Observation:** three 30-second dump-all attempts through the exact CH340
  adapter, with the unit visibly in normal Fusion `v5.5`, used the V2 tool's
  timestamp session word and received no response to the normal-mode hello.
  Reconnecting and power-cycling did not change that result; no backup file was
  created.
- **Safety boundary:** the selected workflow contains hello and reads only. No
  write, restore, reboot, bootloader handshake, firmware page, or reset was
  sent.
- **Resolution:** the user reported repeated successful CHIRP use with this
  exact cable. Pinned Armel CHIRP driver commit
  `a0e9314570cd4f5440aca8322ca1722163bad217` showed that CHIRP uses fixed
  session word `0x6457396A`; the unsuccessful V2 tool used a timestamp instead.
- **Confidence:** high that this was a request-shape mismatch rather than
  evidence of cable failure. AFIK's fixed-session read succeeded as recorded
  by `EVID-K1-018` and `EVID-K1-021`.

### EVID-K1-018 — Fixed-session complete backup succeeded

- **Observation:** AFIK's read-only fixed-session workflow received normal
  firmware identity `F4HWN v5.5.0` and all 8,192 configuration/calibration
  bytes through the same CH340 cable. Every response offset and length was
  checked before a mode-`0600` output was created outside the repository.
- **Safety boundary:** only `0x0514` hello and `0x051B` read requests were sent.
  The selected command exposes no EEPROM write, reset, bootloader, or firmware
  operation.
- **Privacy boundary:** calibration/configuration contents and their
  unit-specific hashes are not committed. The user receives the hashes for
  persistent-copy validation.
- **Confidence:** high for complete logical read-back on this exact unit and
  connection. This is not yet proof of restoration or firmware recovery.
- **Required next step:** identify and validate the exact known-good v5.5
  recovery image and procedure before any firmware write. Two verified local
  copies are accepted for the current evidence package; shared-filesystem
  durability remains a documented risk.

### EVID-K1-021 — Repeat fixed-session backup matches retained copy

- **Observation:** after a user power-cycle into normal Fusion mode, a raw
  `0x0515` response identified the exact unit as `F4HWN v5.5.0`. The bounded
  fixed-session read then received and validated all 8,192 configuration/
  calibration bytes through `/dev/ttyUSB0` and the same CH340/CH341 `1a86:7523`
  adapter.
- **Comparison:** the fresh mode-`0600` output matched the prior temporary read
  and `.private/k1/unit-backup.primary.raw` byte-for-byte. The fresh file was
  on filesystem device `43`; the repository and private copy were on device
  `56`. Its unit-specific hash is intentionally not recorded here.
- **Safety boundary:** only normal-mode hello `0x0514` and read `0x051B`
  requests were sent. No write, restore, reset, bootloader entry, firmware
  operation, or RF operation occurred.
- **Confidence:** high for repeatable normal-mode identity and complete logical
  read-back on this exact unit and connection; this remains neither recovery
  proof nor protection against shared-filesystem loss.
- **Required next step:** record the exact physical markings and USB identities,
  then validate the recovery procedure. Two verified local copies are accepted
  for the current evidence package.

### EVID-K1-019 — Pinned v5.5.0 recovery candidate is statically valid

- **Observation:** pinned Armel `main` contains raw image
  `archive/f4hwn.fusion.v5.5.0.bin`, SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`,
  length 95,836 bytes. Its initial SP is `0x20004000`, Reset is Thumb address
  `0x08002D49`, and its exclusive end from application origin `0x08002800` is
  `0x08019E5C`, within main flash.
- **Confidence:** high for source identity, raw shape, vectors, and range; high
  that its version name matches the exact unit's reported `F4HWN v5.5.0`.
- **Boundary:** static validation alone is not physical recovery proof. The
  unchanged-image recovery rehearsal is now recorded by `EVID-K1-022`; it does
  not authorize an AFIK K1 image, whose target contract does not exist.

### EVID-K1-020 — Pinned CPU, memory, and raw-image contract

- **Source:** Armel commit `fe9c4e9432694b50aea651084a043aae0b58673d`,
  `Core/startup_py32f071xx.s:39-42,68-115,140-146`,
  `Core/py32f071xb.ld:9-10,52-66`, and `Core/Src/main.c:46-72`.
- **Facts:** the selected source targets Cortex-M0+ Thumb code, links an
  application at `0x08002800` with 118 KiB of flash and 16 KiB of SRAM, uses a
  vector-table Reset entry, and assumes a bootloader-provided 48 MHz clock.
- **Image relationship:** the pinned raw v5.5.0 candidate has initial SP
  `0x20004000`, Thumb Reset `0x08002d49`, and exclusive end `0x08019e5c`, all
  within that source-declared application contract.
- **Confidence:** high for the pinned source and static candidate; medium for
  the physical unit until its MCU marking, reset handoff, and recovery behavior
  are observed. No K1 bootloader reservation or write protocol is inferred.
- **Permitted use:** define the evidence boundary for a later independent Rust
  reset-and-boot-witness package. It does not authorize an AFIK image write or
  TX.

### EVID-K1-022 — Same-unit recovery rehearsal and post-flash backup

- **Observation:** the unchanged pinned raw `F4HWN v5.5.0` image (95,836 bytes,
  published SHA-256 above) was written through the exact unit's `7.03.01`
  bootloader after the local backup and recovery copies were verified. All 375
  page acknowledgements matched the transaction and page index and returned
  zero.
- **Postcondition:** after a user power-cycle, normal mode identified
  `F4HWN v5.5.0`; a fresh complete 8,192-byte read matched the pre-flash backup
  byte-for-byte.
- **Confidence:** high for same-unit recovery to the known-good Armel firmware
  and preservation of logical calibration/configuration contents. This does not
  establish an AFIK K1 image contract or AFIK application boot.
- **Safety boundary:** no page was retried, no reset command was sent, and no
  EEPROM or RF operation was performed. The write path was an independent host
  experiment based on pinned protocol evidence, not imported firmware code.

### EVID-K1-023 — AFIK K1 recovery write and post-flash backup

- **Observation:** after the AFIK K1 device-trailer and bounded serial-read
  fixes were committed, the generic `afik-flasher` recovery command classified
  the exact live beacon as K1 `7.03.01` and acknowledged all 375 sequential
  256-byte pages of the unchanged pinned `F4HWN v5.5.0` image in transaction
  `074b2081`.
- **Postcondition:** after a user power-cycle, AFIK's read-only normal-mode
  workflow identified `F4HWN v5.5.0`, received all 8,192 bytes, and its
  mode-`0600` output matched both retained pre-flash configuration/calibration
  backups byte-for-byte.
- **Confidence:** high for the independently implemented AFIK host recovery
  protocol, the exact unit's observed beacon, sequential page acknowledgements,
  normal-mode identity, and logical backup preservation. This does not prove a
  K1 AFIK application image, AFIK reset execution, or RF behavior.
- **Safety boundary:** no page was retried, AFIK sent no reset or EEPROM write,
  and no RF operation occurred. The image was the known-good stock recovery
  candidate; its unit-specific backup bytes and hashes remain outside tracked
  repository content.

### EVID-K1-024 — K1 application serial path

- **Source:** pinned `armel/uv-k1-k5v3-firmware-custom` commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`.
- **Source locations:** `App/driver/uart.c` configures USART1 on PA9 TX and
  PA10 RX with alternate function 1 at 38,400 baud; `Core/Src/main.c` records
  that the bootloader provides a 48 MHz system clock; the pinned PY32F071
  device header defines the USART1, GPIOA, and RCC register map and the USART
  status/control bits used by that configuration.
- **Exact-unit observation:** the same unit answered the fixed-session normal
  mode hello and supplied a complete 8 KiB backup through the external CH340
  adapter, as recorded by `EVID-K1-018` and `EVID-K1-021`.
- **AFIK use:** AFIK independently implements a bounded polling USART1 hello
  witness over this serial path. It does not copy, link, or translate the
  source driver. The exact response is recorded in `EVID-K1-025`.
- **Confidence:** high for the existing unit's serial transport and normal
  protocol path; medium for the exact MCU/package register mapping until the
  physical marking is recorded. Native USB routing is not required by this
  witness.

### Puya PY32F071-E product documentation

- **Publisher:** Puya Semiconductor (Shanghai) Co., Ltd.
- **Product page:** `PY32F071R1BU7-E`, retrieved 2026-08-06 from
  <https://www.puyasemi.com/en/py32f071/3415.html>.
- **Document:** *PY32F071-E Datasheet*, version 1.4, published 2026-01-30 and
  linked from the official product page.
- **Facts used now:** Arm Cortex-M0+ up to 72 MHz, product variants with up to
  128 KiB main flash and 16 KiB SRAM, USB 2.0 full speed, SWD, GPIO, ADC/DAC,
  timers, serial peripherals, and boot modes.
- **Boundary:** the exact fitted part suffix is not recorded yet. Maximum
  family capabilities do not prove the exact unit's memory size, package, pin
  binding, oscillator, boot selection, or radio-board behavior.

### EVID-K1-025 — Physical AFIK K1 serial witness

- **Image:** independently built raw `radio-firmware-k1`, 44,008 bytes,
  SHA-256 `74be18d266e919c24faf1c7b022461c085990f33a6ad34475be3d9ae7424862f`.
- **Write observation:** the external CH340 path classified K1 bootloader
  `7.03.01`; the separately guarded AFIK writer acknowledged all 172 ascending
  pages in transaction `db2b80ec`. It sent no reset, EEPROM, RF, or TX command.
- **Postcondition:** after a user power-cycle, the same CH340 path accepted one
  fixed-session normal-mode hello and returned
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.1`.
- **Confidence:** high that the bounded AFIK Reset handler reached USART1 and
  answered on the exact unit through the external UART. This does not establish
  display, keypad, external flash, BK4819, RF, TX, EEPROM, or full application
  behavior.
- **Safety boundary:** the bootloader reported page acknowledgements only; no
  flash read-back or automatic reset was used. The retained stock recovery
  image and complete backup remain the rollback path.

### EVID-K1-026 — K1 display serial path selected for bounded experiment

- **Source:** pinned `armel/uv-k1-k5v3-firmware-custom` commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`.
- **Source locations:** `App/driver/st7565.h` declares 128 columns, 64 rows,
  and eight page buffers; `App/driver/st7565.c` configures SPI1 as master,
  eight-bit, mode 3, MSB-first, divide-by-64, with SCK PA5 AF0 and serial data
  PA7 AF0; `App/board.c` and the display source bind A0 to PA6 and active-low
  CS to PB2. The pinned PY32F071 device header supplies the corresponding RCC,
  GPIO, and SPI1 register definitions.
- **AFIK use:** `K1DISP-019` may independently generate the bounded controller
  init, clear, page-address, and fixed-screen byte stream and bind that stream
  to only the sourced K1 registers. The trusted source is evidence, not copied,
  linked, or incrementally translated production code.
- **Confidence:** high for the pinned source's intended board mapping and
  transfer configuration; unverified on the exact unit for controller marking,
  reset wiring, contrast, orientation, and AFIK-driven pixels.
- **Required experiment:** first pass exact command/framebuffer host tests and
  target/static gates. Then, only after explicit confirmation, write once,
  power-cycle, observe the fixed screen, and run the existing serial hello
  probe. Do not drive an unobserved reset pin, backlight, keypad/PTT, audio,
  storage, BK4819, RF, or TX.

### EVID-K1-027 — First AFIK display attempt was blank with serial fallback alive

- **Write observation:** the verified 48,436-byte image was written once after
  explicit authorization. K1 `7.03.01` acknowledged all 190 pages in
  transaction `d4f83080`; no retry or reset command was sent.
- **Physical observation:** after power-cycle, the user reported a blank screen.
  AFIK therefore claims no successful display initialization, pixels,
  orientation, contrast, or illumination.
- **Independent fallback:** the immediate read-only normal-mode hello returned
  `AFIK-K1-0.2`, proving Reset and USART1 still ran and the bounded display path
  did not trap the application.
- **New evidence boundary:** pinned `App/driver/st7565.c` states that hardware
  reset is unsupported on K1. Pinned `App/driver/gpio.h` and
  `App/driver/backlight.c` identify a separate active-high backlight on PF8;
  AFIK did not configure or drive it. Before another image, distinguish an
  unlit panel from absent pixels and separately bound any PF8 experiment.

### EVID-K1-028 — Display pixels work and PF8 illumination is isolated

- **Physical observation:** under bright external light, the user could see the
  fixed AFIK words produced by the first display image. The panel was not blank;
  its backlight was off.
- **Conclusion:** high confidence on the exact unit for LCD initialization,
  page/data transfer, visible orientation, and fixed rendering. This closes the
  display-controller witness without inferring keypad or other UI hardware.
- **Pinned backlight fact:** `App/driver/gpio.h` maps the backlight to PF8 and
  implements on as GPIO set/high; `App/board.c` configures PF8 as an output.
  The more complex existing brightness path uses TIM7 and DMA, which are not
  needed or authorized for the first AFIK illumination witness.
- **Permitted experiment:** `K1BL-020` may enable GPIOF, configure only PF8 as a
  push-pull output, and hold it high. PWM, timer, DMA, fades, settings, storage,
  RF, and TX remain excluded.

### EVID-K1-029 — PF8 illumination passed and fixed contrast is faint

- **Write observation:** K1 `7.03.01` acknowledged all 190 pages of the exact
  PF8 image in transaction `7e094920`, with no retry or reset.
- **Physical observation:** after power-cycle, the user saw the active backlight
  and fixed words. The words were faint. The immediate read-only serial probe
  returned `AFIK-K1-0.2`.
- **Conclusion:** high confidence for the constant active-high PF8 binding and
  retained display/USART paths. Faint pixels are now isolated to contrast
  calibration, not missing illumination or display transport.
- **Pinned candidate:** `App/driver/st7565.c` uses electronic-volume value 31
  during fixed startup, while AFIK used 21. `K1CON-021` may change only that
  command byte and must not infer a runtime setting or final panel policy.

### EVID-K1-030 — Fixed electronic-volume 31 improves readability

- **Write observation:** the exact one-byte contrast image was acknowledged for
  all 190 pages in transaction `3f6392fd`, without retry or reset.
- **Physical observation:** after power-cycle, the user confirmed the active
  backlight, visible fixed words, and substantially improved readability.
- **Independent fallback:** the immediate read-only serial probe returned
  `AFIK-K1-0.2`.
- **Conclusion:** electronic-volume 31 is physically useful for this fixed boot
  witness on the exact unit. It is not a general panel calibration, runtime
  setting, temperature/supply policy, or evidence for other units.

### EVID-K1-031 — K1 main keypad matrix and bounded scan contract

- **Source:** pinned `armel/uv-k1-k5v3-firmware-custom` commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`, specifically `App/board.c`,
  `App/driver/keyboard.c`, and the pinned PY32F071 GPIO definitions. This is
  trusted hardware evidence, not AFIK production source.
- **Board observation:** PB15, PB14, PB13, and PB12 are pull-up row inputs.
  PB6, PB5, PB4, and PB3 are push-pull column outputs. The source holds all
  columns high, selects one by driving it low, reads active-low rows, and
  restores all columns high.
- **Main-key table:** PB6 maps rows PB15..PB12 to `MENU, 1, 4, 7`; PB5 maps
  them to `UP, 2, 5, 8`; PB4 to `DOWN, 3, 6, 9`; and PB3 to
  `EXIT, STAR, 0, F`.
- **Excluded evidence:** the same source uses a special no-column condition for
  side keys and reads PTT separately on PB10. Neither behavior is part of the
  4-by-4 main matrix or authorized by `K1KEY-022`.
- **AFIK inference and confidence:** high confidence in the pinned source's
  intended pins, polarity, idle state, and table; unverified on the exact unit
  for bounce, settling, ghosting, multi-key behavior, stuck lines, and AFIK
  MMIO. AFIK will accept only one stable cell and will reject all ambiguity.
- **Required experiment:** first prove the table, explicit-time debounce,
  display labels, and exact GPIO register plan on the host, then pass target
  and raw-image gates. A separately guarded write may display the last
  debounced main key while retaining the fixed backlight and serial hello.
  Exercise all 16 keys individually; do not press PTT or side keys and do not
  add RF, TX, persistence, interrupts, or general menu behavior.

### EVID-K1-032 — K1 keypad witness image was page-acknowledged

- **Precondition:** read-only auto-identification found the external CH340 path
  and classified the unsolicited beacon as K1 bootloader `7.03.01`. The exact
  56,828-byte image had SHA-256
  `4ad5e4e205afd32e791409b371e111c0792110c48e1fc9c67a5c19d8628c06b0`
  and CRC-32 `a17da806`; the retained recovery and EEPROM backup gates passed.
- **Write observation:** the guarded K1 AFIK writer acknowledged all 222 pages
  in transaction `265b2c89` and reported `acknowledged_not_read_back`. It sent
  no retry, reset, EEPROM-write, RF, or TX command.
- **Evidence boundary:** page acknowledgement does not prove application boot,
  GPIO behavior, key mapping, debounce, or display output. Those claims remain
  pending a manual power-cycle, all 16 individual main-key observations, and
  the independent normal-mode serial probe.

### EVID-K1-033 — First K1 main-key display observation failed

- **Physical observation:** after power-cycle, the user reported that the key
  labels did not display. AFIK therefore claims no successful physical matrix
  scan, key mapping, debounce, or key-triggered display update.
- **Independent fallback:** the immediate read-only normal-mode probe returned
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.2` over the external
  CH340 path.
- **Conclusion:** the serial fallback proves the application loop remained
  reachable, but it does not distinguish GPIO configuration, selected-column
  drive, row sampling, debounce, or redraw failure. Inspect those boundaries
  with a focused host-visible witness before authorizing another image.
- **Bounded correction:** the first implementation rendered labels at `y=50`
  on pages 6–7, whereas the prior physical display witness directly observed
  the fixed line at `y=36`. The next image may replace that verified line with
  the key label and must leave pages 6–7 empty. This corrects an observation-
  coverage gap without changing GPIO scanning or inferring its success.

### EVID-K1-034 — Verified-line correction was page-acknowledged

- **Precondition:** the corrected 56,856-byte image had SHA-256
  `417663dab22de56fbfe167049c3b1b5831e588c04db4eec9ac7ec16b5cf9130a`
  and CRC-32 `f4a9c1d6`; fresh identification returned K1 bootloader `7.03.01`
  and all recovery/backup gates passed.
- **Write observation:** all 223 pages were acknowledged in transaction
  `fe6396d0`, without retry or reset. This remains page-acceptance evidence,
  not a label or keypad success claim.
- **Required observation:** after normal power-cycle, `MENU` should replace
  `K1 0.2` on the already verified second text line; only after that should the
  remaining 15 main keys and serial fallback be checked.

### EVID-K1-035 — K1 Renode execution reaches key rendering

- **Simulation construction:** Renode 1.16.1 executes the exact K1 ELF from its
  application Reset entry using Cortex-M0 instruction compatibility, evidenced
  flash/RAM bounds, and simulation-only register storage. A test-only control
  injects active-low PB15 only while the firmware has selected PB6 low.
- **Observation:** three repeated tests reached a CPU hook after initial display
  setup and then reached the ELF's `render_key_witness` symbol after synthetic
  MENU injection.
- **Confidence boundary:** high for this compiled control-flow path under the
  declared register responses. This does not model or prove PY32F071 peripheral
  semantics, instruction timing, physical GPIO levels, switch behavior, SPI
  electrical transfers, the LCD controller, or visible pixels.
- **Next experiment:** report the four raw row masks through the existing
  receive-only serial witness while one key is held. This separates physical
  scan input from display output without RF, TX, EEPROM, or side-key access.

### EVID-K1-036 — Raw keypad serial diagnostic contract

- **Construction:** one CRC-protected normal-mode request runs the existing
  PB6-to-PB3 main-matrix scan once and returns four raw, four-bit row masks plus
  scan validity. Host parsing rejects wrong commands, lengths, CRC, reserved
  bytes, out-of-range masks, and non-boolean status.
- **Expected MENU observation:** with no key held all masks should be zero; with
  MENU held, only `pb6_rows` should contain bit 0. These are predictions from
  the pinned mapping, not physical observations yet.
- **Confidence boundary:** the diagnostic is read-only and cannot interpret a
  key, update the LCD, access PTT/side keys, persist data, or reach RF/TX. A
  physical result establishes only the sampled matrix state for that request.
- **Write observation:** the guarded K1 `7.03.01` writer acknowledged all 227
  pages of the 57,860-byte diagnostic image in transaction `0e4f6fc9` and
  reported `acknowledged_not_read_back`. This proves page acknowledgments only;
  normal boot and raw matrix behavior still require separate observation.
- **First physical observation:** released returned a valid scan with all masks
  zero. Holding MENU caused two serial response timeouts, including one issued
  by the prebuilt host binary; response returned after release. This does not
  yet establish the raw held mask because the key-triggered synchronous SPI
  path executes before a later serial request can be serviced.
- **Isolated-image write:** K1 `7.03.01` acknowledged all 226 pages of the
  57,852-byte SPI-suppressed diagnostic in transaction `1a79dec2` and reported
  `acknowledged_not_read_back`. Normal boot and raw held-key behavior are not
  implied by these acknowledgments.
- **Isolated-image observation:** after normal boot, released again returned a
  valid all-zero scan, while MENU held still caused a serial timeout. Because
  this image performs no key-triggered SPI frame transfer, that transfer is
  excluded as the timeout cause. A held row mask remains unobserved.
- **Next construction:** retain the latest nonzero raw scan in volatile RAM and
  return it only after serial response is available, explicitly marked as a
  capture. This can distinguish temporary held-state disruption from reset;
  loss of the capture after release remains a valid negative result.
- **Write observation:** K1 `7.03.01` acknowledged all 229 pages of the
  58,380-byte latched diagnostic in transaction `20d50457` and reported
  `acknowledged_not_read_back`; this does not establish normal boot or capture.
- **Latch observation:** after MENU tap/release, the read-only response was
  valid but reported `captured=false` and four zero masks. The user saw no
  boot-screen restart; initial display appearance is approximately 15 seconds
  after power-on, while serial response returned promptly after release. This
  is evidence against the assumed PB6/PB15 observation on this exact unit, not
  evidence for any alternate pin mapping.
- **Raw GPIOB experiment:** the next image captures exact 16-bit GPIOB IDR
  snapshots for PB6 through PB3 selection, using the first released scan as a
  baseline and ignoring only PB3..PB6 output changes. Any captured difference
  identifies an observed GPIOB bit transition but does not establish its
  electrical role without follow-up repetition.
- **Write observation:** K1 `7.03.01` acknowledged all 239 pages of the
  61,128-byte raw GPIOB image in transaction `7422b31d` and reported
  `acknowledged_not_read_back`; no normal-boot or GPIO observation is implied.
- **Physical MENU observation:** released snapshots were PB6 `f43c`, PB5
  `f45c`, PB4 `f46c`, PB3 `f474`. After MENU tap, the one-shot capture returned
  PB6 `743c` with the other three unchanged. The `0x8000` difference physically
  establishes PB15 active-low while PB6 is selected on this exact unit.
- **Boundary:** the first immediate post-tap serial query timed out and a retry
  succeeded. This establishes scan input but not debounce timing, display-frame
  completion, visible labels, or the other 15 cells.

## Sources used by DP32-003

### DP32G030 reference manual v1.23

- **Document:** *DP32G030 Reference Manual — 32-bit Microcontroller*, version
  1.23, Chinese, revision dated 2022-02-21.
- **Publisher shown in document:** Action Dynamic Tech. (HK) Trading Co.; the
  document also names 深圳市动能世纪科技有限公司.
- **Retrieved:** 2026-08-05 from
  <https://alfaexploit.com/files/DP32G030.pdf>.
- **Mirror status:** this is a public third-party mirror; a chip-vendor download
  has not been located. The PDF is not vendored in this repository.
- **SHA-256:**
  `d1923c0a1830dada46706515ced53978f9a5086e04ce178deaf28d2928c62573`.

### Arm Cortex-M0 generic user guide

- **Document:** *Cortex-M0 Devices Generic User Guide*, ARM DUI 0497A,
  ID112109, 2009.
- **Retrieved:** 2026-08-05 from Arm's documentation service:
  <https://documentation-service.arm.com/static/5ea6ce5e9931941038def8c1>.
- **Scope:** generic Cortex-M0 architectural behaviour only; implementation
  choices remain governed by the DP32G030 manual.

## Accepted facts

### EVID-DP32-001 — Processor architecture and byte order

- **Fact:** the DP32G030 contains a 32-bit Arm Cortex-M0 processor. Its system
  interface supports little-endian data access.
- **Source:** DP32G030 manual sections 1.1 and 5.4.1–5.4.2, printed pages 20 and
  43–44. Section 5.1 also states that module data format is little-endian.
- **Method:** copied from the reference manual, not derived from existing
  firmware.
- **Confidence:** high for Work Package 3. The manual is internally consistent
  and agrees with Arm's Cortex-M0 vocabulary, but the mirrored document and
  target radio silicon have not been independently authenticated.
- **Permitted use:** select Rust's `thumbv6m-none-eabi` target and Renode's
  Cortex-M CPU model. No DP32G030 peripheral behaviour follows from this fact.
- **Required experiment:** inspect the emitted ELF architecture now; read the
  physical core identification through a non-destructive debug path only after
  recovery prerequisites are complete.

### EVID-DP32-002 — Program flash and data RAM ranges

- **Fact:** program flash occupies `0x0000_0000..=0x0000_FFFF` (64 KiB), and
  data RAM occupies `0x2000_0000..=0x2000_3FFF` (16 KiB).
- **Source:** DP32G030 manual table 5-1 in section 5.1, printed pages 39–40;
  section 5.2, printed page 41, independently states 64 KiB program flash and
  16 KiB data RAM.
- **Method:** copied. The inclusive end addresses and sizes agree exactly.
- **Confidence:** high for linker and minimal memory-model bounds; not yet
  confirmed on a physical UV-K5-family unit.
- **Permitted use:** define only flash and RAM regions in the linker script and
  Renode platform. Work Package 3 must not model the manual's peripheral ranges.
- **Required experiment:** statically check every allocated ELF section against
  these ranges; later compare non-destructive SWD memory reads with the map only
  after backup/recovery work is complete.

### EVID-ARM-003 — Cortex-M0 reset vectors

- **Fact:** the Cortex-M0 vector table is fixed at `0x0000_0000`. Its first word
  is the initial stack-pointer value, its second word is the Reset handler, and
  handler vectors have bit 0 set to identify Thumb code.
- **Source:** Arm DUI 0497A section 2.3.4 and figure 2-1, document pages 2-21 and
  2-22 (PDF pages 36–37).
- **Method:** copied from the Arm core guide, then combined with
  `EVID-DP32-002`, whose flash starts at the same address.
- **Confidence:** high for a Cortex-M0 model loaded without a boot remap.
- **Permitted use:** place a two-entry minimum vector table at the start of the
  image, set the initial stack pointer to the top of evidenced RAM, and let
  Renode begin through the ELF/vector-table reset path.
- **Required experiment:** inspect the first two ELF words and prove in Renode
  that execution reaches the Rust Reset handler without overriding the PC.

## Simulation-only observation contract

`DP32-003` may reserve one linker-controlled word inside the evidenced RAM and
write a fixed boot sentinel from the Reset handler. The Renode test may inspect
that word. The sentinel address and value are project test conventions, not
DP32G030 registers or claims about hardware behaviour.

## Unknowns deliberately excluded from DP32-003

- The UV-K5 bootloader's physical reset mapping, flash masking, image layout,
  and firmware packaging are not established.
- No clock, reset controller, flash controller, interrupt, UART, GPIO, display,
  keypad, BK4819, audio, or power register behaviour is accepted yet.
- The exact UV-K5-family board revision and fitted DP32G030 silicon have not
  been inspected.
- Hardware flashing remains blocked by `RISK-002`.

## Work Package 5 UI evidence boundary

`UI-005` introduces no hardware facts. `Menu`, `Back`, `Up`, `Down`, and
`Confirm` are product-level logical actions, and its display output is a
semantic enum rather than pixels or bus traffic. The `Menu+Back` boot gesture
is an AFIK workflow decision, not a claim about an existing radio key matrix.

The physical key matrix, side-key wiring, scan polarity and timing, display
controller, dimensions, bus, reset sequence, backlight, and board pins remain
unknown. A target UI adapter or peripheral simulator requires new sourced facts
and board experiments under `RISK-006` before implementation.

## Sources used by RF-006

### Beken BK4819 product page

- **Document:** Beken Corporation product page, *BK4819 — Half-duplex TDD FM
  Transceiver*.
- **Publisher:** Beken Corporation.
- **Retrieved:** 2026-08-06 from
  <https://www.bekencorp.com/en/goods/detail/cid/50.html>.
- **Scope:** current high-level product identity and capabilities only. The page
  contains no register map or board integration instructions.

### Beken BK4819 datasheet Rev.1.0

- **Document:** *BK4819 Analog Two Way Radio IC*, Rev.1.0, copyright 2018,
  22 pages.
- **Publisher shown in document:** Beken Corporation.
- **Retrieved:** 2026-08-06 from
  <https://alfaexploit.com/files/BK4819.pdf>.
- **Mirror status:** public third-party mirror; an equivalent download was not
  found on Beken's current product page. The PDF is not vendored.
- **SHA-256:**
  `a2b795a1f40f13e2708fc11720cc4df05fe00590eb0a8d82914699153321de02`.
- **Scope:** product architecture, 3-wire control framing/timing, pins, and
  electrical characteristics. The datasheet explicitly points to separate
  application notes and a register table and does not itself define registers.

### Mirrored BK4819(V3) application note

- **Document:** *BK4819(V3) Application Note*, page label 2020; the mirror title
  includes `20210428` and identifies the content as machine-translated English.
- **Publisher/authenticity:** the content presents itself as an application
  note but the accessible Scribd copy is user-uploaded and no original Beken
  download or untranslated copy has been located.
- **Retrieved:** 2026-08-06 from
  <https://www.scribd.com/document/716113950/BK4819-V3-Application-Note-20210428-machine-translated-English>.
- **Scope:** only the explicitly listed frequency, mode-control, RSSI, and
  squelch fields below. Function names, prose translations, defaults,
  initialization code, RF performance, and omitted register behavior are not
  accepted as facts.

## Accepted BK4819 facts and bounded inferences

### EVID-BK4819-004 — Product mode and 3-wire control envelope

- **Fact:** Beken identifies BK4819 as a half-duplex TDD FM transceiver with an
  MCU 3-wire interface. Datasheet section 1.3 and figures 5–7 encode one R/W bit,
  seven address bits `A6..A0`, and sixteen data bits `D15..D0`; input is latched
  on rising SCK and output changes on falling SCK. Table 6 limits SCK to 8 MHz.
- **Method:** copied from Beken's product page and Rev.1.0 datasheet pages 1,
  6, and 9 (PDF pages 1, 6, and 9).
- **Confidence:** high for an unbound logical register-bus envelope; low for
  applicability to any particular MCU pins or board timing implementation.
- **Permitted use:** define a bounded 7-bit register address, 16-bit value, and
  fallible read/write trait. `RF-006` does not bit-bang pins or model timing.
- **Required experiment:** identify the fitted chip/board revision and trace
  SCK, SCN, and bidirectional SDATA; only then verify edge timing with a logic
  analyzer using non-transmitting reads.

### EVID-BK4819-005 — Frequency word and receive-status fields

- **Fact:** the mirrored application note assigns a 32-bit frequency word with
  a 10 Hz unit across `REG_38` and `REG_39`. Its 409.75 MHz example produces
  `0x0271_3A98`; the adjacent table shows `REG_38 = 0x3A98` and
  `REG_39 = 0x0271`, establishing low and high words respectively. It labels
  `REG_67<8:0>` read-only RSSI with `RSSI dBm ~= raw/2 - 160`, and
  `REG_0C<1>` as a read-only squelch result where one is link/open and zero is
  loss/closed.
- **Method:** copied from the note's Squelch/RSSI and Frequency Setting sections,
  labeled pages 13–14.
- **Confidence:** medium-low for a BK4819(V3) simulation contract because the
  only accessible copy is machine-translated and its revision/authenticity are
  unconfirmed. The 10 Hz formula's example is internally consistent.
- **Permitted use:** pack only exactly 10-Hz-aligned integer frequencies into
  the two words and decode RSSI into signed half-dBm integer units plus one
  squelch boolean in a fake register-bus model.
- **Required experiment:** obtain the original application note/register table;
  confirm register order, write/latch ordering, and status fields by read-only
  observation on identified hardware before binding a target adapter.

### EVID-BK4819-006 — Mode-control fields and AFIK command-plan inference

- **Fact:** the mirrored application's Tx/Rx Mode Switch table labels
  `REG_30<15>` VCO calibration, `<13:10>` the receive link blocks
  LNA/mixer/PGA/ADC, `<9>` AF DAC, `<7:4>` PLL/VCO, `<3>` PA gain, `<2>` MIC
  ADC, `<1>` TX DSP, and `<0>` RX DSP. Zero disables every listed block.
- **Inference:** for deterministic simulation, AFIK composes `0xBEF1` as
  receive (calibration, all link blocks, AF DAC, all PLL/VCO blocks, RX DSP),
  `0x80FE` as transmit (calibration, all PLL/VCO blocks, PA gain, MIC ADC, TX
  DSP), and `0x0000` as neutral standby. AFIK writes standby, `REG_38` low,
  `REG_39` high, and the selected mode last so a partially completed command
  plan cannot intentionally reach transmit enable.
- **Method:** bit positions are copied from application-note labeled page 12;
  exact combined values and ordering are local fail-closed design inferences,
  not copied vendor sequences.
- **Confidence:** low for physical-chip operation and high only as a transparent
  deterministic command model. Initialization prerequisites, auto-clearing
  calibration behavior, high/low frequency write order, and required delays are
  unknown.
- **Permitted use:** encode exact command-order tests and a fake-bus simulator.
  The TX mode word may be emitted only behind a matching `TxAuthorisation`.
- **Required experiment:** compare an original register table/application note
  and capture known-safe vendor initialization/RX transitions first. TX
  verification additionally requires a recovery-proven unit, shielded dummy
  load, spectrum analyzer, current limiting, and independently controlled
  external PA/RF switching; it is outside `RF-006`.

### EVID-BK4819-007 — Published frequency ranges conflict

- **Fact:** the current Beken product page states 18–620 MHz and 840–1200 MHz,
  while the mirrored Rev.1.0 datasheet states 18–660 MHz and 840–1300 MHz.
- **Confidence:** high that the publications differ; unknown which revision or
  range applies to the fitted radio and its board filters/matching.
- **Permitted use:** none as a software acceptance limit. `RF-006` validates
  only 10 Hz representation; channel/regulatory bounds remain higher-layer
  responsibilities and board RF limits remain unknown.
- **Required experiment:** identify the exact fitted silicon revision and obtain
  its matching official datasheet plus board filter/switch schematic before
  defining supported receive or transmit bands.

## Unknowns deliberately excluded from RF-006

- BK4819 silicon revision and the BK4819(V3) application's applicability.
- Original register table, complete reset/initialization sequence, documented
  mode transition sequence, calibration completion/status, and required delays.
- MCU pins, electrical direction switching for SDATA, and the UV-K5-family
  board's RF switches, filters, audio route, external PA, and power controls.
- Crystal frequency/calibration values and all unit-specific calibration data.
- Physical RSSI accuracy, squelch behavior, receive sensitivity, emitted power,
  spectral purity, and regulatory compliance.
- Hardware access, flashing, physical receive claims, and all on-air TX remain
  blocked by `RISK-002`, `RISK-005`, and `RISK-007`.

## Work Package 7 scanning evidence boundary

`SCAN-007` introduces no hardware facts. Dwell and hold milliseconds are AFIK
workflow configuration, not BK4819 tune/settle requirements. Timer expiries and
normalized `SignalMeasurement` values are logical adapter inputs, not claims
about a DP32G030 timer, interrupt, RSSI threshold, polling interval, or physical
squelch accuracy.

The deterministic simulator proves controller scheduling, stale-token safety,
logical RF command ordering, and TX-policy composition against declared inputs.
Physical scan cadence, receiver settling, status sampling, tone detection, RF
performance, and target integration remain unknown under `RISK-008`.

### EVID-K1-069 — First bracket on the scan dwell this board needs

- **Source:** operator observation on the exact K1 unit running `AFIK-K1-5.2`,
  2026-08-09. Not a datasheet fact and not a chip fact.
- **Observed:** holding star scans, and the scan stops on a signal at a dwell of
  100 ms. At 60 ms it does not stop. The floor therefore lies between them.
- **What this measures:** the whole loop, not the synthesiser. A dwell has to
  cover the retune over the bit-banged three-wire bus, whatever the BK4819 needs
  to settle on the new frequency, and at least one squelch reading taken after
  that settling — the firmware discards the first reading after a retune
  deliberately, so a usable dwell must span two samples at the 5 ms scanning
  cadence as well. Any of those could be the binding term and this observation
  does not separate them.
- **Confidence:** moderate for the bracket, none for the mechanism. One pass, on
  one unit, against whatever signal was present at the time; a weak or
  intermittent signal would move the apparent floor upwards, so 100 ms is an
  upper bound on the floor rather than the floor.
- **What it does not establish:** any BK4819 settling time, any register-level
  timing requirement, or any figure that may be encoded as a constant. It
  narrows `RISK-008`; it does not close it.
- **Next:** `AFIK-K1-5.3` re-ranges the handset dwell list to 60, 70, 80, 90,
  100 and 150 ms to bisect this bracket in ten-millisecond steps.

### EVID-K1-070 — Carrier squelch alone stops a scan on the wrong channel

- **Source:** operator observation on the exact K1 unit running `AFIK-K1-5.3`,
  2026-08-09.
- **Observed:** with a handheld transmitting on PMR channel 4 close to the
  radio, the scan stopped two positions early. Frequency and channel numbering
  were checked and were correct, and when the scan did land on the transmitted
  channel the audio was the transmission, so the stop was a squelch decision
  and not an indexing fault.
- **Mechanism, and its confidence:** AFIK gates the squelch on carrier strength
  alone, which cannot distinguish a signal on this channel from a strong one a
  channel or two away. A transmitter at close range delivers far more into the
  adjacent channels than the highest threshold AFIK offered, so every level
  opened on it. This is consistent with the observation and with ordinary
  receiver behaviour; it is not a measurement of this board's selectivity, and
  no adjacent-channel rejection figure is claimed.
- **What the range was:** the nine levels spanned 3 dB each from about -130 dBm
  to about -106 dBm — all of it at the sensitive end, and all of it below a
  close transmitter's adjacent-channel leakage. AFIK's own note already recorded
  these as AFIK's values rather than the unit's.
- **What it is now:** 8 dB a step, about -130 dBm to about -66 dBm. Level one is
  unchanged for weak-signal work and the top of the range now reaches above a
  strong local signal. The steps are coarser, which is the trade: an operator
  who cannot shut the squelch at all has no useful resolution anywhere.
- **What this does not establish:** any per-unit calibration value, any noise or
  glitch threshold, and any figure for this board's selectivity or front-end
  overload behaviour. The proper fix is noise-gated squelch, which needs the
  per-unit calibration this radio cannot yet read; `RISK-008` carries that.

### EVID-K1-071 — Scan dwell and squelch level settled on the exact unit

- **Source:** operator observation on the exact K1 unit, 2026-08-09, bisecting
  the bracket `EVID-K1-069` opened.
- **Observed:** squelch level 3 and a 90 ms scan dwell are the settings at which
  this unit scans and stops on a transmission reliably. Level 3 is about
  -114 dBm under the widened range `EVID-K1-070` records.
- **Confidence:** good for this unit and this antenna, as a working setting
  found by trying them. It is not a measured floor: 90 ms is inside the 60-to-100
  bracket rather than at its edge, and no attempt was made to find the exact
  point between 80 and 90 at which it fails.
- **What it changed:** `RadioConfig::conservative()` now carries a 90 ms dwell
  rather than 150 ms, so a radio arrives configured the way this one wanted.
  `SquelchLevel::CONSERVATIVE` was already 3 and is unchanged.
- **What it does not establish:** anything about another unit. A radio with a
  different antenna, a different board, or a noisier environment may need
  longer, which is why the dwell stays host-programmable in whole milliseconds
  rather than becoming a constant.

## Sources used by FREQ-010

### FCC-filed Quansheng UV-K5 user manual

- **Document:** *Quansheng UV-K5 Two Way Radio User Manual*, FCC exhibit
  document ID `6401561`, 13 pages.
- **Applicant/publisher:** Fujian Quanzhou Quansheng Electronics Co., Ltd.;
  filed for FCC ID `XBPUV-K5`.
- **Created/filed metadata:** 2023-02-17; retrieved 2026-08-06 from
  <https://fccid.io/XBPUV-K5/User-Manual/User-manual-6401561.pdf> and the
  accompanying exhibit page
  <https://fccid.io/XBPUV-K5/User-Manual/User-manual-6401561>.
- **SHA-256:**
  `d6f30fea598abdde820ef47e5f9b0b77079dc229ec7039a97b3f87083b33b74b`.
- **Scope:** intended radio-level controls and displayed/saved results only.
  The manual contains no BK4819 register sequence, accuracy, timeout, or board
  implementation details.

### Public UV-K5-family firmware observation

- **Repository:** *egzumer/uv-k5-firmware-custom*, commit
  `7607f0a4bd6203d1f06b70556fc1ce0d7399d6b3`, retrieved 2026-08-06 from
  <https://github.com/egzumer/uv-k5-firmware-custom/tree/7607f0a4bd6203d1f06b70556fc1ce0d7399d6b3>.
- **Files inspected:** `driver/bk4819.c`, `app/scanner.c`, and `misc.c` at that
  exact commit.
- **Provenance limit:** the repository states that it combines earlier
  UV-K5-family firmware projects and derives from DualTachyon's work. It is one
  descendant implementation, not independent corroboration of the mirrored
  application note or original Quansheng production source.
- **Permitted use:** compare its state flow, polling, repeat-result filtering,
  failure handling, and unexplained constants when designing experiments. No
  source was copied, translated, linked, or treated as AFIK production code.
- **Additional repository checked:**
  `amnemonic/Quansheng_UV-K5_Firmware` at commit
  `94a36006b75ad2024d9b88f2b33b222c7efe53ba` contained binaries, documents,
  hardware PDFs, and patch scripts but no buildable firmware source from which
  an exact Frequency Copy flow could be established.

The Beken product page and mirrored BK4819(V3) application note already
identified under “Sources used by RF-006” were re-used for the narrowly bounded
observations below. Their confidence and provenance limitations are unchanged.

## Frequency Copy evidence and bounded inferences

### EVID-FCOPY-008 — Fast Copy measures one received transmission

- **Fact:** user-manual section 6.9 calls the feature “Fast Copy One Channel
  (ACT AS FREQUENCY METER).” It directs the user to place the radios close with
  both antennas installed and requires a sufficiently strong signal. `F+4`
  starts measurement; the display can show the carrier frequency and the
  transmitting channel's CTCSS/DCS. `*` starts another measurement, `MENU`
  saves the measured frequency and CTCSS/DCS to a chosen channel, and `EXIT` or
  PTT leaves the function.
- **Fact:** the same manual separately describes automatic CTCSS/DCS scanning
  at an already known receive frequency under `F+*`, with success/failure
  feedback and an explicit save action.
- **Method:** copied from manual sections 6.9 and 6.10. Wording here is
  paraphrased; key names and displayed data are retained because they define
  the user-visible workflow.
- **Confidence:** high for intended UV-K5 radio behavior; none for silicon
  sequence, physical accuracy, or AFIK target behavior.
- **Permitted use:** define a deliberate, receive-only AFIK measurement and
  review workflow. It does not establish any saved channel property other than
  an observed carrier frequency and an optional received tone/code indication.

### EVID-FCOPY-009 — Fast Copy is not Air Copy

- **Fact:** manual section 7.8 describes “Wireless Radio Replication” as a
  separate transfer between two radios on a shared data frequency, defaulting
  to 410.0125 MHz. It is entered with PTT plus side key 2 and transfers radio
  data with progress/error feedback. The feature table likewise lists
  Frequency Meter and Air Copy separately.
- **Confidence:** high for the product-level distinction.
- **Permitted use:** exclude Air Copy protocol discovery, configuration
  replication, and any transmission from `FREQ-010`. “Frequency Copy” in AFIK
  means local observation of a received signal, never radio-to-radio cloning.

### EVID-FCOPY-010 — Beken publishes a scan capability, not a command contract

- **Fact:** Beken's current BK4819 product page lists frequency scan, CTCSS
  receive/tail functions, 23/24-bit DCS, RSSI, and a 3-wire MCU interface.
- **Fact from low-confidence source:** the mirrored machine-translated
  BK4819(V3) note describes a frequency scan result spanning `REG_0D<10:0>` and
  `REG_0E<15:0>` in 10 Hz units, a busy bit in `REG_0D<15>`, and scan enable
  plus four nominal duration selections in `REG_32`. It describes CTCSS result
  fields in `REG_68` whose conversion depends on the crystal, and 23/24-bit DCS
  result fields in `REG_69` and `REG_6A`. The prose also gives an approximate
  strong-input condition.
- **Confidence:** high only that Beken advertises the capabilities; low for all
  named register fields, units, polarity, timing, threshold, latching, or
  applicability to the fitted radio because the accessible note is
  machine-translated, user-uploaded, and revision-unverified.
- **Permitted use:** form hypotheses and measurement vectors for a
  non-transmitting experiment. These fields are not accepted for a production
  driver or a simulator that claims physical register behavior.
- **Required experiment:** obtain original revision-matched documentation and
  perform the receive-only experiments in `frequency-copy-feasibility.md`.

### EVID-FCOPY-011 — One descendant firmware does not resolve the register gaps

- **Observation:** the inspected descendant polls the note's busy/result
  fields, seeks repeated nearby frequency results, then retunes and seeks a
  recognized CTCSS or DCS result. It bounds polling and requires an explicit
  save prompt. Its save path rounds a captured frequency to one of two channel
  step grids.
- **Observation:** its `REG_32` value includes bits which its own comment marks
  unknown, including an unexplained numeric field. Its code also supplies
  product choices such as thresholds, repeats, timeouts, code lookup, rounding,
  and copying receive configuration into transmit configuration; those choices
  are not BK4819 facts.
- **Confidence:** low as hardware evidence. Common project ancestry means the
  code and mirrored note are not independent corroboration.
- **Permitted use:** identify failure cases and experiments. AFIK must not copy
  its code, unexplained constants, automatic RX-to-TX configuration, or timing
  values.

## Unknowns deliberately retained by FREQ-010

- Exact fitted BK4819 revision, board revision, reference crystal/TCXO and
  calibration, RF switch/filter path, register reset state, and required
  preservation masks.
- Meaning of every `REG_32` bit used by existing firmware, start/stop/retrigger
  order, busy transitions, result latching/read order, stale-result behavior,
  cleanup sequence, and behavior after bus failure.
- Physical frequency accuracy, resolution versus displayed rounding, minimum
  usable level, acquisition time, repeatability, adjacent/multiple-signal
  behavior, and image/harmonic/strong-interferer false locks.
- CTCSS conversion for the fitted crystal, nearby-tone discrimination,
  no-tone and short-burst behavior, and the complete DCS bit-length, polarity,
  code-validation, and no-result semantics.
- Duplex/offset, paired repeater input, transmit permission/service, modulation,
  bandwidth, power, channel name, scan membership, scrambler, contacts, source
  identity, and any other channel metadata. None is observable from the
  documented one-transmission workflow.
- A trustworthy distinction between “no signalling present” and “signalling
  not detected before timeout” without separately validated carrier and decoder
  behavior.

Production scan commands, physical target integration, and automatic channel
creation remain prohibited by `RISK-011`.

## Sources used by APRS-011

### AX.25 Link Access Protocol Version 2.2

- **Document:** *AX.25 Link Access Protocol for Amateur Packet Radio*, version
  2.2, fourth edition, 1996.
- **Publishers shown in document:** American Radio Relay League and Tucson
  Amateur Packet Radio Corporation.
- **Retrieved:** 2026-08-06 from TAPR's file archive at
  <https://files.tapr.org/tech_docs/AX25/AX25.2.2.pdf>.
- **SHA-256:**
  `af2070954468ef6498143ababf9beaf5d72b683ef82491dd7eb8e3670b29475c`.
- **Scope:** HDLC flag/bit-stuff/FCS envelope, shifted AX.25 addresses,
  extension and repeated bits, UI control field, and PID placement. Physical
  APRS modulation is not specified here.

### APRS Protocol Reference Version 1.0.1

- **Document:** *APRS Protocol Reference — APRS Protocol Version 1.0*, document
  version 1.0.1, 2000-08-29.
- **Publisher:** APRS Working Group material hosted by APRS.org.
- **Retrieved:** 2026-08-06 from
  <https://www.aprs.org/doc/APRS101.PDF>.
- **SHA-256:**
  `78a72618c788b8b7f8369004884018f9b02f990069ff67915bb1e30738b1da01`.
- **Scope:** APRS use of AX.25 UI frames, APRS address/path limits, information
  type identifiers, time/position/ambiguity fields, and Object/Item lifecycle
  and syntax.

### APRS addendum and frequency specification

- **Documents:** APRS Specification Addendum 1.1, approved by the APRS Working
  Group in 2004; and *APRS Freq Spec — AFRS (Automatic Frequency Reporting
  System)*, revision 2019-09-30.
- **Retrieved:** 2026-08-06 from <https://www.aprs.org/aprs11.html> and
  <https://www.aprs.org/info/freqspec.txt>.
- **Frequency-spec SHA-256:**
  `d49be65c62a4c9907fdfb5c168813a5135b98cbfc7a2c6de144016307798972b`.
- **Scope:** the current voice-frequency comment/object conventions, exact
  frequency/tone/offset/range text fields, recommended local voice-repeater
  objects, permanent `111111z` timestamp behavior, and compatibility limits.
  The frequency specification is operational APRS Working Group guidance, not
  proof that an advertised value is current or authorized.

The Beken product page, mirrored Rev.1.0 datasheet, mirrored machine-translated
BK4819(V3) application note, and DP32G030 source limitations already recorded
above are re-used with unchanged provenance and confidence.

## APRS receive and discovery evidence

### EVID-APRS-012 — APRS is carried in bounded AX.25 UI frames

- **Fact:** APRS Protocol Reference chapter 3 specifies destination and source
  addresses, zero through eight digipeater addresses, control `0x03`, PID
  `0xF0`, an information field of 1 through 256 octets, and a 16-bit FCS. The
  first information octet is the APRS Data Type Identifier.
- **Fact:** AX.25 sections 3.1–3.12 define `0x7E` flags, removal of a zero after
  five consecutive one bits, a sixteen-bit ISO 3309 FCS, shifted six-character
  upper-case alphanumeric callsigns, four-bit SSIDs, seven-octet address
  subfields, and a final-address extension bit. UI frames contain PID and
  information without link flow control.
- **Conflict:** AX.25 version 2.2 limits its Layer-2 repeater chain to two,
  whereas APRS 1.0.1 explicitly permits zero through eight APRS digipeater
  addresses. An APRS receive parser follows the upper-layer APRS bound of eight
  while remaining receive-only; it does not implement AX.25 repeating.
- **Confidence:** high for a hardware-independent parser of a complete,
  already de-stuffed, octet-aligned frame including FCS.
- **Permitted use:** validate the complete frame, addresses, control/PID,
  information bounds, and FCS before exposing APRS information. Flags, NRZI,
  clock recovery, and bit unstuffing remain lower-layer inputs and are excluded.

### EVID-APRS-013 — Objects and Items carry explicit identity and lifecycle

- **Fact:** APRS Protocol Reference chapter 11 defines a case-sensitive
  nine-character printable Object name after `;`, followed by `*` for live or
  `_` for killed and a required seven-character timestamp. An Item begins with
  `)`, has a case-sensitive printable name of three through nine characters,
  and uses `!` for live or `_` for killed without a timestamp. Both can carry
  uncompressed or compressed position plus comment.
- **Fact:** a new report with the same Object/Item name replaces the earlier
  report, even if another station originated it; the sender callsign should be
  retained for display. The frequency specification gives permanent frequency
  Objects the pseudo timestamp `111111z` and says a different origin must not
  replace them.
- **Inference:** AFIK keys discovery entries by report kind, case-sensitive
  name, and originating AX.25 source. This deliberately preserves conflicting
  origins as separate untrusted observations and permits only the same source
  to update or kill its entry. Explicit local receive time resolves freshness;
  equal-time conflicting data is rejected rather than ordered implicitly.
- **Confidence:** high for syntax; high that the AFIK key is a conservative
  local safety choice, not APRS network ownership semantics.

### EVID-APRS-014 — Voice-repeater fields are advertisements, not authority

- **Fact:** the APRS frequency specification defines two main encodings: a
  leading ten-octet `FFF.FFFMHz` or `FFF.FF MHz` comment field, or a
  nine-character frequency Object name beginning `FFF.FFF` or `FFF.FF`. It
  defines optional `Tnnn`/`Cnnn` CTCSS, `Dnnn` DCS, signed three-digit offsets
  in 10 kHz units, and `Rxxm`/`Rxxk` nominal range. Lower-case tone prefixes
  advertise narrow bandwidth. The recommended voice-repeater Object examples
  use symbol code `r` and the permanent `111111z` timestamp.
- **Fact:** where both a frequency Object name and a comment frequency exist,
  the specification treats the Object name as the repeater transmit/output
  frequency and the comment value as a cross-band or non-standard receive/input
  frequency.
- **Inference:** AFIK labels the name frequency as the advertised repeater
  output—the frequency a listener would receive—and preserves offset, tone,
  range, alternate input, source, position, and age as untrusted fields. It does
  not calculate a transmit channel, map truncated tones into trusted
  `radio_domain::Tone`, infer regional standard offsets, or auto-tune/save.
- **Confidence:** high for field syntax; none for truth, freshness, origin
  authenticity, regulatory class, service availability, or physical reach.

### EVID-APRS-015 — BK4819 APRS demodulation is not established

- **Fact:** Beken's official product page and mirrored datasheet advertise an
  on-chip FSK data modem; the datasheet block diagram includes FSK modulation
  and demodulation.
- **Low-confidence description:** the machine-translated revision-unverified V3
  note describes receive modes called FSK 1.2/2.4K, FFSK 1200/1800, and FFSK
  1200/2400, with bounded preamble, two/four-byte sync, configured length,
  optional proprietary CRC, FIFO, and interrupts.
- **Comparison:** common 1200-baud APRS engineering practice uses Bell-202-style
  1200/2200 Hz AFSK followed by NRZI, HDLC flags, bit stuffing, and AX.25 FCS.
  None of the named V3 modem modes says 1200/2200, and its described fixed
  preamble/sync/length framing does not establish transparent AX.25 bit access.
- **Confidence:** high only that a generic FSK modem exists; low for the named
  register modes and no confidence that they can receive APRS correctly on the
  fitted silicon/board.
- **Permitted use:** support the feasibility experiment plan. No BK4819 APRS
  command, register fake, timing, interrupt, or FIFO behavior may be encoded.

## Unknowns deliberately retained by APRS-011

- Physical APRS frequency/band plan and permission for a specific jurisdiction;
  AFIK encodes no hard-coded national channel or transmission behavior.
- Fitted chip and board revision, BK4819 modem applicability, reference crystal,
  initialization state, modem tone/baud meaning, raw-bit availability, FIFO and
  interrupt semantics, and safe stop/cleanup.
- Whether suitable discriminator/unfiltered audio is exposed to the DP32G030,
  its voltage/bias/bandwidth/noise, board routing, ADC channel/sample rate,
  timer/interrupt/DMA wiring, CPU budget, buffer sizes, and power cost.
- NRZI polarity, clock recovery, carrier acquisition, de-emphasis effects,
  bit-stuff/flag/abort recovery, false-frame rate, FCS error behavior, and packet
  loss under weak, distorted, collided, over-deviated, or adjacent signals.
- Advertisement authenticity, freshness, correctness, coverage, regulatory
  classification, actual repeater availability, and whether optional offset or
  tone conventions are locally applicable.

`APRS-011` may implement only the complete-frame and APRS discovery boundary.
Physical demodulation and automatic channel mutation remain prohibited by
`RISK-012` and `RISK-013`.

## Sources used by FLASH-012

No manufacturer bootloader protocol or board-revision matrix has been located.
The following sources are pinned reverse-engineering evidence. Their code is
GPL-3.0 and is not copied, linked, translated, or used as AFIK production
source. Exact wire observations are recorded as facts; interpretations remain
explicitly bounded.

### sq5bpf/k5prog

- **Repository:** `sq5bpf/k5prog`, commit
  `241ab18b61f6d8933fecf60643fe94322fbf4198`, dated 2023-12-29.
- **Retrieved:** 2026-08-06 from <https://github.com/sq5bpf/k5prog>.
- **Files inspected:** `README` and `k5prog.c`; `k5prog.c` SHA-256
  `cc8a1f42208515c73bb6869233b9afa0f9cb151212d5ef9c78ef5f2d8ea5f6eb`.
- **Method stated upstream:** protocol reverse-engineering from traffic between
  the radio and original programming software, plus physical use by the author.
- **Scope:** packet envelope, normal-firmware EEPROM reads, bootloader-v2
  beacon/version/page messages, 38,400-baud observation, 256-byte pages, and
  the asserted bootloader reservation at `0xF000`.

### qrp73/K5TOOL

- **Repository:** `qrp73/K5TOOL`, commit
  `03cb33aef88fc17f9e6b71d9e6c4f0ac9b0dc436`, dated 2025-12-18.
- **Retrieved:** 2026-08-06 from <https://github.com/qrp73/K5TOOL>.
- **Files inspected:** `README.md`, `Packets/Envelope.cs`, and the V2 beacon,
  version, page-write, and acknowledgement packet types. Their respective
  SHA-256 values are
  `5d8ac90426ba0dc53f8612e1b908141330ecbe18f6225acd9b162a080c40f1cb`,
  `e7ea15effc47dbc8bf48771a2efa198de45fa1c182c17e3195d3c6ab0d097ec2`,
  `eada6c03a5162642e6e1fee00fe2de7e35c9a601aac5fa9cbe7b1d72b137a0b6`,
  `e37430167b9d435ecc87c2ac3979f60a60f0e5aa2efb75ecab6ac7e16c752ee7`,
  `168592c44f6c4a61ae7bd7aaf710600b35ab7b92d5a1d4839efc9bc1f279314d`,
  and `b62a843cff89640a9efdca868ddef18dda7825b788bfa1042dbd4d6afb04f026`.
- **Relationship:** K5TOOL is a separate implementation but cites k5prog for
  bootloader command semantics, so it is useful cross-check evidence rather
  than a fully independent observation.
- **Scope:** V1/V2/V3 incompatibility warning, V1/DP32G030 claim, `0xF000`
  maximum, bootloader-v2/v5 distinction, packet layout, page indexing, result
  handling, and a software bootloader simulator.

### Quansheng product and FCC exhibits

- **Product page:** Quansheng's current UV-K5 page, retrieved 2026-08-06 from
  <https://en.qsfj.com/products/3002>.
- **FCC filing:** FCC ID `XBPUV-K5`, including internal photographs, document
  `6401563`, filed 2023-03-09; the PDF SHA-256 reported by the exhibit service
  is `6e97aeeec6fb3870edf70895627fed2f0d6275e8fa9a49328ff9abd63fe17f15`.
- **Scope:** product/family and physical-inspection context only. These sources
  do not publish the bootloader wire protocol or establish that a particular
  user unit is the same MCU/board revision.

## Accepted FLASH-012 observations and bounded inferences

### EVID-K5-008 — Hardware revision must be established physically

- **Observation:** K5TOOL warns that its old V1 units use DP32G030, while later
  V2 and V3 markings can identify incompatible processor revisions. It provides
  distinct recovery images and workflows for those revisions.
- **Confidence:** medium-low. This is a maintained implementation report, not a
  Quansheng board matrix, and compatible-looking models/clones may differ.
- **Permitted use:** require the operator to record the exact model, under-
  battery revision marking, PCB revision, and readable MCU marking before a
  destructive command. A bootloader beacon alone cannot satisfy this gate.
- **Required experiment:** photograph the exact test unit and its fitted MCU;
  reject anything other than an explicitly confirmed V1/DP32G030 unit.

### EVID-K5-009 — The stock application boundary is below `0xF000`

- **Observation:** k5prog caps flash writes at `0xF000` because it reports a
  bootloader in `0xF000..=0xFFFF`. K5TOOL separately enforces a maximum end of
  `0xF000` and reports successful images approaching that boundary.
- **Inference:** AFIK will treat `0x0000..=0xEFFF` as the complete application
  region and the final 4 KiB as immutable stock-bootloader space. A packaged
  image is exactly 60 KiB, with unused application bytes set to `0xFF`, so no
  stale prior application content is intentionally retained.
- **Confidence:** medium for a qualified old V1 unit and low for the family in
  general. The placement has no located vendor specification and has not been
  observed on an AFIK-owned physical unit.
- **Permitted use:** reduce the linker FLASH length, statically reject ELF
  allocations reaching `0xF000`, and emit only an exact 60 KiB raw image.
  Nothing may address, erase, package, or write the final 4 KiB.
- **Required experiment:** after recovery is proven, compare the application
  boundary against a non-destructive read/debug observation on the exact unit.

### EVID-K5-010 — Legacy packet envelope and EEPROM read observations

- **Observation:** both implementations use 38,400 baud, 8 data bits, no
  parity, and one stop bit. A packet has `AB CD`, a little-endian payload
  length, payload plus little-endian CRC-16/XMODEM, and `DC BA`. Payload and CRC
  bytes are XORed with the repeating 16-byte sequence
  `16 6C 14 E6 2E 91 0D 40 21 35 D5 40 13 03 E9 80`. Observed radio responses
  decode to a `0xFFFF` CRC field rather than a calculated CRC.
- **Observation:** normal firmware accepts `0x0514` hello with the fixed
  `0x6457396A` session word and `0x051B` reads containing a 16-bit offset,
  at-most-128-byte length, zero padding, and that word. `0x0515` and `0x051C`
  responses carry a bounded firmware version and offset/length-tagged data.
- **Confidence:** medium for the exact old firmware samples represented by the
  two tools; low for other firmware, password/custom-key modes, or clones.
- **Permitted use:** a read-only, exact 8 KiB EEPROM backup workflow that
  rejects any response mismatch. No EEPROM write command is permitted.
- **Required experiment:** capture and compare every hello/read response from
  the exact stock test unit, then hash and retain its complete backup before
  entering bootloader mode.

### EVID-K5-011 — Version-2 bootloader page-write observations

- **Observation:** the version-2 beacon is command `0x0518`; the recorded
  36-byte sample contains printable version `2.00.06`. A `0x0530` request sends
  a zero-padded, at-most-16-byte ASCII firmware-family version. The wildcard
  accepted by third-party tools bypasses the observed version check.
- **Observation:** a `0x0519` request carries header-size `0x010C`, a 32-bit
  transaction word, little-endian 256-byte page index and total page count,
  actual length, zero padding, and one 256-byte data area. k5prog uses observed
  word `0x1D9F8D8A`; K5TOOL generates a nonzero word per run. `0x051A` returns
  that word, the page index, and a zero success result. Bootloader beacons may
  continue before the first page acknowledgement.
- **Inference:** AFIK will accept only a 36-byte `2.*` beacon, prohibit version
  wildcards, take an explicit nonzero per-run transaction word, write all 240
  full pages in ascending order, accept only exact matching zero-result
  acknowledgements, and stop on the first deviation. It will not retry an
  unacknowledged page because whether the prior write took effect is not
  observable.
- **Confidence:** medium for bootloader 2.00.06 on a qualified old V1 unit; low
  for other bootloaders. Page acknowledgement is not flash read-back.
- **Permitted use:** implement one fail-closed bootloader-v2 host workflow and
  a deterministic fake device. Command `0x057A`/bootloader v5 and all other
  beacons are rejected before the version request or any page write.
- **Required experiment:** probe the exact unit, retain the complete raw
  transcript, flash a known-good recovery image first, power-cycle, and prove
  stock boot. Only then may an AFIK application image be attempted.

### EVID-K5-012 — A `4.00.01` bootloader beacon on a V1-generation unit

- **Observation:** on 2026-08-11 a Quansheng UV-5R Plus supplied by the user was
  placed in bootloader mode and connected through a CH340/CH341 `1a86:7523`
  adapter on `/dev/ttyUSB0`. At 38,400 8-N-1 it unsolicitedly and repeatedly
  emitted one frame: `AB CD` header, declared frame length 36, payload command
  `0x0518` with inner length 32, `DC BA` footer. The printable bootloader
  version is `4.00.01`. Two passive captures several minutes apart produced
  byte-identical frames.
- **Method:** receive only. No byte was transmitted to the radio in either
  capture. The payload was deobfuscated with the recorded 16-byte key applied
  from the first payload byte.
- **Observation:** the 16-byte field between the command header and the version
  string is populated and differs in shape from a version. It is treated as
  per-unit identity, redacted, and not stored in repository content.
- **Observation:** the frame trailer is `FF FF` on the wire. Deobfuscated as the
  K5 path does, it reads `0x6ED1` rather than the `0xFFFF` response marker.
  XMODEM CRC over the payload at both 32 and 36 bytes, in plaintext and
  obfuscated form, matches none of these values.
- **Inference:** this bootloader emits the `0xFFFF` marker outside the XOR
  stream, so the trailer must be compared before deobfuscation. Confidence is
  high, and this is **not** specific to `4.00.*`: `EVID-K5-013` observes the same
  literal `FF FF` trailer from a `2.00.06` unit. An earlier revision of this
  entry attributed it to this bootloader and is corrected.
- **Observation:** the user reports that one `armel/uv-k5-firmware-custom`
  build, which targets DP32G030, runs correctly on this unit and on their UV-K5
  and UV-K6, and that all three are V1-generation radios.
- **Inference:** the fitted MCU is DP32G030, since that image would not run
  otherwise. Confidence medium-high, and this does **not** satisfy
  `EVID-K5-008`: no marking on this unit has been read or photographed.
- **Confidence:** high for this unit, adapter, frame shape, and version string,
  which are direct reads rather than inference. Nothing here establishes the
  flash geometry, the page-write protocol, or the meaning of the identity field.
- **Permitted use:** reject a `4.00.*` beacon from the qualified V1 path instead
  of assuming `EVID-K5-011` applies to it, and require a distinct classification
  and target-confirmation phrase. `UV-K5-V1-DP32G030` asserts a bootloader-v2
  unit and must not be reused for this radio.
- **Required experiment:** return the unit to normal mode and record the
  application identity from one read-only hello; photograph the model, revision
  and MCU markings per `EVID-K5-008`; and establish the page protocol read-only
  before any write is considered.

### EVID-K5-013 — The qualified path rejects a genuine `2.00.06` unit

- **Observation:** on 2026-08-11 a Quansheng UV-K6 supplied by the user, from the
  same set of three V1-generation radios as `EVID-K5-012` and running the same
  DP32G030 build, was placed in bootloader mode on the same adapter. It emitted
  the identical frame shape — `AB CD`, declared length 36, command `0x0518`,
  inner length 32, `DC BA` — with printable bootloader version `2.00.06`, which
  is the exact version `EVID-K5-011` is written against.
- **Observation:** its frame trailer is also `FF FF` on the wire.
- **Observation:** `afik-flasher identify` against this unit fails with
  `unexpected decoded radio CRC trailer: 0x6ed1`. The qualified V1 workflow
  therefore rejects its own qualified target, on the bootloader version it was
  designed for, before any operation is selected.
- **Cause:** the inbound trailer is deobfuscated with the payload
  (`crates/radio-flasher/src/codec.rs:101`) and the resulting value is required
  to equal `0xFFFF` (`workflow.rs:581`). Real devices send the marker literally,
  so `FF FF` decodes to `0x6ED1` at those key offsets and the check can never
  pass. The correct comparison is against the raw trailer.
- **Consequence for the K1 exemption:** `codec.rs` attributes the K1's skipped
  trailer check to a K1 peculiarity. That attribution is wrong. Both families
  send a literal `FF FF`; the K5 assumption was never true on hardware, and the
  K1 path works only because it happens to ignore the field. The general
  behaviour is the K1's, not the exemption.
- **Why this was not caught:** `EVID-K5-011` was derived from third-party tool
  reports rather than a physical unit, and the host encoder obfuscates its own
  outbound trailer, so the round-trip tests agree with themselves. This is the
  first AFIK observation of a K5-family bootloader on real hardware.
- **Independent corroboration:** `qrp73/K5TOOL`, `main` at
  `03cb33aef88fc17f9e6b71d9e6c4f0ac9b0dc436` (2025-12-18, GPL-3.0), computes the
  expected decoded trailer from a radio as
  `xorTable[len % 16] ^ 0xFF | (xorTable[(len + 1) % 16] ^ 0xFF) << 8`
  (`Packets/Envelope.cs`, `CheckCrc`). For `len = 36` and AFIK's recorded key
  that is `0x2E ^ 0xFF = 0xD1` and `0x91 ^ 0xFF = 0x6E`, giving exactly the
  `0x6ED1` measured here. An independent implementation therefore derives the
  same value from the same key, which also confirms the two projects' key tables
  agree.
- **Refinement this forces:** K5TOOL accepts *both* trailers. It treats a decoded
  `0xFFFF` as running mode and the deobfuscated-literal value as the other case,
  and only warns on mismatch. The rule is mode-dependent: a radio in normal mode
  sends a trailer that decodes to `0xFFFF`, and a radio in bootloader mode sends a
  literal `FF FF`. That is why AFIK's normal-mode hello succeeded on the K1 while
  bootloader classification fails here — the same code path is correct for one
  mode and wrong for the other.
- **Confidence:** high. Two units, two bootloader versions, one adapter, a
  reproduced failure from the shipped classifier, and an independent
  implementation that derives the measured value arithmetically.
- **Permitted use:** accept both trailer forms, keyed by mode rather than by
  family, instead of adding a per-family exemption. This says nothing about the
  **outbound** trailer: K5TOOL's `Envelope.Encode` obfuscates a real CRC-16 over
  the payload, which is what AFIK already sends, so the send path is corroborated
  and must not be changed.
- **Required experiment:** capture the third unit's beacon, then confirm the
  corrected classifier accepts `2.00.06` and `4.00.01` and still rejects a
  malformed frame.

### EVID-K5-014 — Per-unit identity field in the `0x0518` beacon

- **Observation:** the 16-byte field between the command header and the version
  string, from the two units above:

  | Unit | Bootloader | Field |
  | --- | --- | --- |
  | UV-K5 | `2.00.06` | `01 02` · `02 0b 0d` · block A · `ff 0e 2a 00 46 00` |
  | UV-5R Plus | `4.00.01` | `01 02` · `02 0b 0c` · block A · `ff 01 aa 00 31 00` |
  | UV-K6 | `2.00.06` | `01 02` · `03 02 0c` · block B · `ff 14 38 00 9f 00` |

  The five-byte block at offsets 9..14 is printable and is redacted. The leading
  `01 02`, the `ff` separator and the two `00` bytes are common to all three.
- **Correction:** an earlier revision of this entry called that block per-unit
  identity, on two samples. The third unit refutes it. The UV-K5 and the UV-5R
  Plus are two distinct radios — they report different bootloader versions in the
  same capture session — and they carry a **byte-identical** block. It is
  therefore a production property, not a serial number.
- **Observation:** the eight bytes following the version string are byte-identical
  on both units (`34 0A 00 00 00 00 00 20`), so they are not per-unit and are
  probably a separate field rather than version padding.
- **External sample and the split it proves:** `K5TOOL` commits two beacons in
  `Packets/V2/Packet2FlashBeaconAck.cs` and `Packets/V5/Packet5FlashBeaconAck.cs`
  at the revision pinned in `EVID-K5-013`. Both carry the same leading
  `01 02 02 06 1c` and the same five ASCII characters, but one reports bootloader
  `2.00.06` and the other `5.00.01`, and only offsets 11..15 differ between them.
  That is one unit observed under two bootloaders.
- **Inference:** offsets 0..9, including the ASCII characters, are unit or
  hardware properties that survive a bootloader change, and offsets 11..15 are
  bootloader-linked. The eight bytes after the version string are also
  bootloader-linked, not per-unit: `34 0A 00 00 00 00 00 20` appears on K5TOOL's
  `2.00.06` sample and on both units here, while its `5.00.01` sample reads
  `28 0C 00 00 00 00 00 20`. Confidence medium — one external unit supports the
  split and no unit has been observed across a bootloader change here.
- **Hypothesis, now better supported:** offsets 2..4 are `02 06 1c`, `02 0b 0c`,
  `02 0b 0d` and `03 02 0c` across the four known units. Every middle byte is a
  valid month and every last byte a valid day. The two units sharing a block
  differ only in the last byte, by one, which a date reads as consecutive days
  from one batch. Confidence medium: four samples fit, a same-batch pair fits
  particularly well, and no sample contradicts it, but other encodings would also
  fit and no marking or purchase record has been compared against it.
- **Not established:** whether offsets 2..4 encode a date, a model, or a hardware
  revision, and what the block identifies — batch, production line, or something
  else. No sample distinguishes a UV-K5 from a UV-5R Plus, which is expected if
  the field describes production rather than model.
- **Confidence:** high for the bytes; high for the block being production-linked
  rather than per-unit, since two radios share it; medium for the
  unit-versus-bootloader split and for the date reading; none for what any
  individual field names.
- **Permitted use:** none beyond recording. No workflow may branch on this field
  until its meaning is established.
- **Required experiment:** compare offsets 2..4 against physically read markings
  and, if available, a purchase or manufacture date; and capture one unit before
  and after a bootloader change to test the split directly.

### EVID-K5-017 — One production batch shipped two bootloader versions

- **Observation:** on 2026-08-11 the third unit, a UV-K5 with a broken display and
  a hand-fitted configuration memory, beaconed bootloader `2.00.06` on `0x0518`
  with a literal `FF FF` trailer. It carries the same production block as the
  UV-5R Plus of `EVID-K5-012`, whose bootloader is `4.00.01`, and a triple
  differing from it by one in the last byte.
- **Inference:** the bootloader version is not a property of the hardware. Two
  radios from what the beacon presents as one production batch, of two different
  model names, shipped `2.00.06` and `4.00.01`. Confidence high for the
  observation and medium-high for the inference, which depends on reading the
  shared block as a batch.
- **Consequence:** `4.00.01` is an ordinary vendor bootloader on this generation,
  not an anomaly and not a marker of different silicon. Any scheme that infers
  the processor, the geometry or the model from a bootloader version is refuted by
  these three radios directly, which is `ADR-070`'s premise and `RISK-037`'s open
  item stated from hardware rather than from reasoning.
- **Also observed:** all three units are `0x0518` page-write bootloaders, all
  three send the literal trailer, and the constant eight bytes after the version
  string are identical across all three despite two different versions.
- **Permitted use:** cite as the reason AFIK requires a declared target and reads
  a version only as a build description. This authorises no write to any of these
  units.
- **Required experiment:** read the physical markings of the pair sharing a block
  and confirm they are the two different models the operator reports.

### EVID-K5-018 — The bootloader cannot report the fitted memory

- **Observation:** the `EVID-K5-017` unit has a different same-family memory part
  fitted by hand, and its display is broken, so neither the operator nor a screen
  can report what is fitted. Its beacon carries no memory field, and the
  bootloader surface AFIK implements is beacon, version negotiation and page
  write, none of which reads the configuration memory.
- **Consequence:** the fitted capacity cannot be established in bootloader mode at
  all. The vendor configuration read is a normal-firmware command, so it requires
  the radio to be running its application, and `EVID-K5-016` already establishes
  that no processor or board query exists either.
- **Bearing on capacity probing:** the I²C 24Cxx family has no identification
  command, so capacity can only be inferred from address aliasing, and AFIK's
  backup path reads a fixed 8 KiB and cannot address beyond it. An
  explicit-offset read-only read is therefore a prerequisite for probing a larger
  fitted part.
- **Confidence:** high. This is a property of the implemented command surface.
- **Permitted use:** require normal firmware and an explicit-offset read before
  any capacity claim, and treat a hand-fitted part's capacity as declared and
  then verified rather than assumed.
- **Required experiment:** return the unit to normal mode, record its application
  identity by hello, then read the same distinct non-uniform region at offset 0
  and at each candidate capacity boundary and compare. Nothing is written.

### EVID-K5-015 — The beacon command identifies the protocol, the version does not

- **Observation:** at the revision pinned in `EVID-K5-013`, K5TOOL selects its
  bootloader protocol by beacon command: `Packet2FlashBeaconAck.ID = 0x0518` for
  the V2 protocol and `Packet5FlashBeaconAck.ID = 0x057A` for the V5 protocol,
  which its documentation describes as using AES internally. Its committed
  samples pair `0x0518` with version `2.00.06` and `0x057A` with `5.00.01`.
- **Observation:** the `4.00.01` unit in `EVID-K5-012` beacons on `0x0518`.
- **Inference:** the protocol family is signalled by the beacon command, and the
  printable version is metadata about the build rather than the protocol
  selector. A `4.00.01` bootloader announcing itself on `0x0518` is V2-shaped and
  is not the AES-bearing V5 path. Confidence medium-high: it follows from the
  external protocol split plus a direct observation, but no `4.00.01` page write
  has been attempted or observed anywhere.
- **Consequence for AFIK:** `parse_bootloader_family` gates on the version string
  prefix, `starts_with("2.")`, having already special-cased `0x057A`. That makes
  the version the discriminator and the command a special case, which is
  backwards, and it is why a protocol-compatible unit is refused.
- **Confidence:** high for what K5TOOL implements and for the observed command;
  medium-high for the inference.
- **Permitted use:** classify on the beacon command, treat `0x057A` as an
  unsupported AES path, and record the version string as evidence rather than
  using it as a gate. This does not authorise a write to a `4.00.*` unit: the
  page protocol still has to be established for it.
- **Required experiment:** establish the `4.00.01` page-write exchange read-only,
  and confirm no challenge or authentication step precedes it.

### EVID-K5-016 — No reviewed project can query the processor

- **Observation:** K5TOOL's documentation, at the revision pinned in
  `EVID-K5-013`, distinguishes generations by physical marking: V1 as processor
  DP32G030 with bootloader v2 or v5, V2 as PCB version V1.8 with processor
  PY32F030, and V3 as processor PY32F071. Its hello acknowledgement carries a
  version string, AES and password-lock flags, padding and a challenge, and no
  processor or board identifier.
- **Observation:** `armel/uvtools2`, the browser flasher for V3 and K1, documents
  which radios it supports and describes no processor detection. The `muzkr/ichi`
  bootloader updater likewise names the variants it targets without describing
  detection.
- **Inference:** across the projects reviewed, the processor is established by
  physical inspection or by whether a firmware runs, not by a query. AFIK should
  not expect a pre-flash MCU identification to exist.
- **Also observed:** none of these sources records a `4.00.*` bootloader. K5TOOL
  lists v2 or v5 for a V1 radio. The `EVID-K5-012` unit is therefore not covered
  by the external taxonomy, and the taxonomy should be treated as incomplete
  rather than as evidence that the unit is not V1.
- **Confidence:** medium-low for the taxonomy, which is a maintained
  implementation report rather than a Quansheng board matrix, consistent with
  `EVID-K5-008`. High for the absence of a documented processor query, which is a
  reviewable property of the sources.
- **Permitted use:** keep the physical-marking gate in `EVID-K5-008`, and place
  any processor identification in a running AFIK image rather than in a host
  pre-flash step. The Arm-defined `SCB CPUID` distinguishes Cortex-M0 from M0+
  and is the citable primary fact for that check.
- **Required experiment:** read `SCB CPUID` from a running image once one exists
  on this generation, and photograph the markings on all three units.

## FLASH-012 physical evidence boundary

The serial protocol can prove only that a qualified bootloader acknowledged
each requested page. It cannot identify the MCU, read back internal flash,
prove power-loss recovery, or prove that Reset reaches AFIK code. Until the
physical checklist in `docs/k5-flashing.md` is complete, all tests are host,
static-image, or simulation results and `RISK-002`/`RISK-005` remain open.
# Embassy/PY32 software evidence

### EVID-K1-037 — Embassy and community PY32 HAL feasibility

- **Embassy executor:** current official documentation describes a heap-free,
  statically allocated executor and a generic single-core Cortex-M thread
  platform using WFE/SEV. Tasks remain cooperative and require await points for
  interleaving: <https://docs.embassy.dev/embassy-executor/git/cortex-m/index.html>.
- **PY32 HAL:** Embassy's official project documentation lists the external
  `py32-hal` as Embassy-compatible. Published `py32-hal` documentation advertises
  PY32 family Embassy support but labels the project experimental:
  <https://github.com/embassy-rs/embassy> and
  <https://docs.rs/crate/py32-hal/latest>.
- **Confidence boundary:** these sources justify dependency/build feasibility
  work only. They do not prove the exact PY32F071 feature, AFIK's Rust 1.86
  compatibility, board clocks, interrupts, timers, USART1, SPI1, DMA, or
  physical behavior.
- **Local dependency observation:** in the locked Rust 1.86 shell,
  `embassy-executor 0.10.0` compiles for `thumbv6m-none-eabi` with its Cortex-M
  thread platform. `py32-metapac 0.5.0` contains four F071 chip features, while
  `py32-hal` releases 0.3.0, 0.4.0, and 0.4.1 expose no F071 feature. This is a
  software-support gap, not permission to use F072 metadata.
- **Generated-metadata observation:** the crates.io `py32-metapac 0.5.0`
  artifact (SHA-256
  `27f23b48cc298b69661d8b95bdfd09d91b2b86b4acc3b1b13a52caf0e3d91878`)
  maps `py32f071c1b`, `py32f071k18`, `py32f071k1b`, and `py32f071r1b` to the
  same 59-line metadata fragment. Its complete named inventory is GPIOA, WWDG,
  AES_LPUART1, and DMA1_CH1; it contains none of the RCC, USART1, SPI1, GPIOB,
  GPIOF, or timer surfaces required to assess this board.
- **Compile result:** adding only those four feature pass-throughs to an
  unmodified local review copy of `py32-hal 0.4.1`, with default features off,
  lets its generator select `py32f071r1b` but then fails the mandatory RCC
  lookup at `build.rs:410-415`. This is evidence that the released metadata is
  incomplete, not evidence that F071 shares F072 behavior.

### EVID-K1-038 — Local generated F071 inventory

- **Pinned source:** `py32-rs/py32-data` commit
  `eb33b9ab85aa4652006e3435d84e1f9f7e5eca50`, generated with its `./d gen`
  pipeline. Source SHA-256 values are `b17d1ab8392855b13eebd6bfdaf1bb29cca45ab7cd4d3d0c3c2e020f1651e471`
  for `svd/PY32F071xx.svd`,
  `07051b275aca1e98af6aa94b649ea374036fd0545e53a6ddf93b579cf203aae7`
  for `data/series/PY32F071.yaml`, and
  `52ec5a67c337b79835104b78858aeacb36e096c5a7305fbf924cf184db28cc0e`
  for `data/dies/DIE072/peripherals.yaml`.
- **Generated result:** all four concrete F071 packages select the same
  7,205-line metadata inventory (SHA-256
  `846f83baab53ee95b0faa7a5301fbac9914ead114f9df72830da6912ad7831bb`).
  It contains 31 peripherals, including RCC, USART1, SPI1, GPIOA/B/F, and the
  timer surfaces required for the next review. The source explicitly models
  F071 as a separate series on DIE072 with CAN disabled.
- **Local HAL result:** the bounded AFIK `py32-hal 0.4.1` patch exposes all four
  F071 features. Each compiles separately for `thumbv6m-none-eabi` on the
  pinned Rust toolchain with default features disabled. AFIK's compile-only K1
  contract additionally names RCC, USART1, SPI1, TIM1/TIM3/TIM15, and the
  observed USART/display/keypad/backlight pins.
- **Boundary:** generated inventory is software-source evidence, not proof of
  exact fitted package identity or peripheral behavior. No HAL init is called,
  and F071 ADC HAL bindings are deliberately disabled pending independent
  constants. Time, USART1, SPI1, DMA, interrupts, clocks, and physical behavior
  remain unproven.

### EVID-K1-039 — Compile-only TIM15 Embassy time boundary

- **Generated inventory:** TIM15 is at `0x40014000`, uses `PCLK1_TIM`, has
  `APBENR2.TIM15EN` and `APBRSTR2.TIM15RST`, exposes CH1 and CH2, and maps its
  break/update/trigger/commutation/capture-compare signals to the dedicated
  TIM15 interrupt.
- **Driver review:** vendored `py32-hal 0.4.1` selects TIM15 explicitly, uses
  CC1 plus overflow for extended timekeeping, and CC2 for one alarm. It obtains
  the timer frequency from the generated RCC binding and computes
  `frequency / TICK_HZ - 1`; Embassy time-driver 0.2.2 defaults to a 1 MHz tick
  when no tick-rate feature is selected. An evidenced 48 MHz timer clock would
  therefore produce prescaler 47.
- **Compile result:** `tool/check-py32f071-time-driver.sh` passes offline,
  warning-denied Clippy with build-std/core on Rust 1.86 for
  `thumbv6m-none-eabi`, including the F071R1B PAC, TIM15 RCC binding, runtime
  interrupt vector, and Embassy driver.
- **Boundary:** this is a static software result. The feature is not selected
  by the firmware entry point; HAL init is not called; and AFIK does not claim
  clock ownership, interrupt delivery, tick accuracy, or physical timing.
  Runtime migration requires an evidenced handoff from the bootloader-provided
  clock followed by target and physical verification.

### EVID-K1-040 — Compile-only USART1 Embassy boundary

- **Generated inventory:** USART1 is at `0x40013800`, uses `PCLK1`, has
  `APBENR2.USART1EN` and `APBRSTR2.USART1RST`, maps to dedicated interrupt 27,
  and exposes PA9 TX AF1 plus PA10 RX AF1. These match the already recorded and
  physically exercised K1 serial path in `EVID-K1-024` and `EVID-K1-025`.
- **Driver review:** the vendored async constructor requires the USART instance,
  RX/TX pins, its interrupt binding, and one TX plus RX DMA channel. The F071
  generated metadata supplies bounded DMA1 channel/request bindings for both
  directions. AFIK selects DMA1 channel 1 for TX and channel 2 for RX in the
  compile-only contract and fixes the configuration at 38,400 baud.
- **Compile result:** `tool/check-py32f071-usart1.sh` passes offline,
  warning-denied Clippy with build-std/core on Rust 1.86 for
  `thumbv6m-none-eabi`, including the F071R1B PAC, RCC and pin traits, USART1
  vector binding, DMA types, and real async `Uart` constructor.
- **Boundary:** the optional feature is absent from the firmware entry point and
  calls no HAL initialization. This is not evidence of clock preservation,
  interrupt or DMA delivery, serial error recovery, coexistence with display
  rendering, or physical async USART behavior.

### EVID-K1-041 — PY32 HAL has no SPI driver surface

- **Generated inventory:** SPI1 is at `0x40013000`, uses `PCLK1`, has
  `APBENR2.SPI1EN` and `APBRSTR2.SPI1RST`, and exposes PA5 SCK AF0 plus PA7
  MOSI AF0. Those surfaces agree with the already physically proven K1 display
  path recorded in `EVID-K1-026` through `EVID-K1-030`.
- **HAL review:** vendored `py32-hal 0.4.1` exports no `spi` module and contains
  no SPI driver source. Its README support table leaves SPI blank for every
  family, defining blank as not implemented, and its TODO list names SPI.
- **Result:** no honest blocking or async HAL SPI constructor can be compiled.
  Generated PAC registers and pin metadata prove inventory only, not a driver.
  A later step must bound an independent AFIK display-bus driver or a reviewed
  local HAL extension, retain chunked/yielding rendering, and separately prove
  clocks, transfers, scheduling, and physical UART responsiveness.

### EVID-K1-042 — Compile-only cooperative SPI1 transmit interface

- **Local HAL boundary:** AFIK adds generated SCK/MOSI pin traits and one
  transmit-only `SpiTx`. Its constructor owns SPI1, PA5, and PA7 and configures
  exactly mode 3, MSB-first, software NSS, bidirectional-data transmit mode, and
  `PCLK1 / 64`; it exposes no MISO, hardware NSS, RX, DMA, or other board path.
- **Async behavior:** each byte waits for TX-empty with a finite poll bound,
  writes the data register, and the operation waits for not-busy at completion.
  Mode fault, overrun, CRC error, and exhausted polls return explicit errors.
  The future yields after every 16 bytes and every 16 unsuccessful polls so it
  cannot monopolize the cooperative executor for a complete 128-byte page.
- **Compile result:** `tool/check-py32f071-spi1.sh` passes offline,
  warning-denied Clippy with build-std/core on Rust 1.86 for
  `thumbv6m-none-eabi`, including the F071R1B PAC, RCC, generated PA5/PA7 pin
  traits, local HAL driver, and K1 no-entry-point constructor.
- **Boundary:** no firmware entry point calls the constructor. Physical clocks,
  status flags, pin waveforms, display transfers, executor interleaving, and
  USART1 responsiveness remain unproven until later deterministic and guarded
  physical milestones.

### EVID-K1-043 — Deterministic cooperative display/serial progress

- **Schedule:** the visible display frame is 1,024 bytes and the local async SPI
  driver yields after each 16-byte chunk. A compile-time equality check binds
  the hardware-independent schedule to the driver constant.
- **Proof:** a no-hardware round-robin future harness completes exactly 64
  display chunks while servicing serial work between every adjacent pair.
- **Boundary:** this proves that the explicit await placement permits another
  cooperative task to run. It does not start the Cortex-M executor, initialize
  the HAL, deliver an interrupt, operate DMA, touch a peripheral, or establish
  physical UART responsiveness during display transfer.

### EVID-K1-044 — Compile-only K1 async ownership composition

- **Composition:** optional `py32f071-runtime-composition` owns one Cortex-M
  thread executor, USART1 on PA9/PA10 with DMA1 channels 1/2, and transmit-only
  SPI1 on PA5/PA7. Every peripheral arrives as an explicit HAL token.
- **Compile result:** `tool/check-py32f071-runtime-composition.sh` passes
  offline warning-denied target Clippy with Rust 1.86 and build-std/core for
  `thumbv6m-none-eabi`.
- **Boundary:** the feature is library-only and absent from the polling firmware
  image. It does not initialize the HAL or clocks, use TIM15, define static
  tasks, own display A0/CS or keypad pins, run interrupts/DMA, or prove physical
  USART/SPI behavior.

### EVID-K1-045 — Read-only inherited-clock contract

- **Pinned-source limit:** `Core/Src/main.c:46-72` at the recorded Armel commit
  calls only `LL_SetSystemCoreClock(48000000)` and says the bootloader configured
  the clock. It does not record the inherited RCC oscillator, PLL, or prescaler
  fields.
- **Fail-closed decoder:** AFIK accepts only ready 16 MHz HSI, ready x3 PLL
  sourced from HSI, requested and active PLL SYSCLK, and undivided AHB/APB.
  That exact state yields 48 MHz for SYSCLK, HCLK1, PCLK1, and PCLK1_TIM; every
  individually varied field is rejected by host tests.
- **Target surface:** optional `py32f071-clock-handoff` compiles a read-only PAC
  snapshot of RCC CR, ICSCR, CFGR, and PLLCFGR together with the existing owned
  async runtime bundle. It performs no RCC write and publishes no HAL clock.
- **Boundary:** compilation and source review do not establish the exact-unit
  register values. A read-only physical observation is required before AFIK can
  adopt the handoff or start TIM15, DMA, async USART1, or SPI1.

### EVID-K1-046 — Bounded serial RCC observation surface

- **Target behavior:** normal-mode command `0x7f12` reads RCC CR, ICSCR, CFGR,
  and PLLCFGR once and returns them under response `0x7f13` with the result of
  `EVID-K1-045` validation. No target register is written by this request.
- **Host behavior:** `afik-flasher probe-clock` accepts only the exact 24-byte
  payload, one-bit validity result, and zero reserved bytes, then prints all
  four raw registers without treating the software result as physical proof.
- **Static/simulation result:** protocol, raw-field, malformed-field, image,
  package, and existing keypad Renode gates pass. The 64,384-byte raw image has
  SHA-256 `c64ffa09da427060fadbc2527713826c3f6db4d70c3639b476fdcf64c64eebd3`
  and CRC-32 `0ed8ed53`.
- **Write observation:** exact K1 bootloader `7.03.01` acknowledged all 252
  diagnostic-image pages under transaction `736f8852` without retry. This is
  `acknowledged_not_read_back`, not application boot or RCC evidence.
- **Boundary:** the required power-cycle, normal hello, and clock response are
  pending, so exact-unit RCC values and the contract result remain unobserved.
- **Physical result:** after power-cycle the image answered the exact normal
  hello. The combined clock request then timed out twice, while an intervening
  hello still passed. This proves neither a register fault nor a framing fault;
  no raw value is recorded until an individually identified response isolates
  the failing boundary.

### EVID-K1-047 — Individually identified RCC register diagnostic

- **Isolation:** commands `0x7f14`, `0x7f16`, `0x7f18`, and `0x7f1a` read only
  CR, ICSCR, CFGR, and PLLCFGR respectively. Responses identify the register and
  carry one 32-bit value with strict zero reserved bytes.
- **Verified artifact:** focused host tests, ELF/raw package validation,
  negative fixtures, and keypad Renode pass. The 65,656-byte image has SHA-256
  `d319d961a93cad6d219a4d21b7a60a2d7337ea989ff4aff2b6e9e92c2f51c955`
  and CRC-32 `a895d521`.
- **Boundary:** this diagnostic only localizes the prior timeout. No register
  value, 48 MHz contract, HAL adoption, or async runtime behavior is claimed
  until the ordered physical probes complete.
- **Write observation:** exact K1 bootloader `7.03.01` acknowledged all 257
  isolation-image pages under transaction `7d527b6f` without retry. Status is
  `acknowledged_not_read_back`; normal boot and register observations remain
  pending.
- **Physical isolation result:** normal hello passed, the first isolated CR
  request timed out, and a subsequent hello passed. No other register was
  requested and no raw value was observed. A no-MMIO response through the same
  command path is required before assigning the fault to RCC access.

### EVID-K1-048 — Serial-only clock diagnostic image

- **Runtime boundary:** Reset stores the RAM witness, initializes only
  GPIOA/USART1, and polls for hello or clock-diagnostic commands. Display,
  keypad, backlight, debounce, matrix scanning, SPI1, GPIOB, and GPIOF are not
  initialized or serviced by this image.
- **No-MMIO control:** request `0x7f1c` returns response `0x7f1d` with fixed
  marker `0x4b31434c`. The host rejects the wrong command, length, reserved
  fields, or marker before reporting success.
- **Verified artifact:** Nix flake evaluation, formatting, warning-denied
  workspace Clippy, all 130 workspace tests, target clock-handoff Clippy, ELF
  and raw-image validation, and negative package fixtures passed. The
  51,340-byte raw image has SHA-256
  `ce97df6718d6ff2b9bee88ca8443ef15a63ea2484231b265501eef7739803585`
  and CRC-32 `b8731d25`.
- **Boundary:** the existing keypad Renode scenario is intentionally
  inapplicable because this entry point excludes keypad behavior. No
  application boot, no-MMIO response, or RCC value is claimed yet.
- **Write observation:** exact K1 bootloader `7.03.01` acknowledged all 201
  pages under transaction `8a6af71f` without retry. Status is
  `acknowledged_not_read_back`; power-cycle and application probes remain
  separate evidence gates.
- **Physical serial result:** after power-cycle, normal hello returned
  `AFIK-K1-0.2` and the no-MMIO request returned marker `0x4b31434c`. Isolated
  reads returned CR `0x03000500`, ICSCR `0x00e64d14`, CFGR `0x00000012`, and
  PLLCFGR `0x00000006`; the combined request returned the same values.
- **Contract result:** rejected. The decoder observes ready HSI and PLL, HSI as
  PLL source, requested and active PLL system clock, and undivided AHB/APB.
  `ICSCR.HSI_FS` decodes as `2`, while the provisional 24 MHz contract expects
  `4`. This records raw silicon state but does not yet assign a frequency to
  encoding `2` or authorize HAL clock publication.
- **Pinned inventory resolution:** the generated F071 PAC selects the maintained
  DIE072 RCC inventory. It defines `HSI_FS=2` as 16 MHz, two-bit `PLLSRC=2` as
  HSI, and `PLLMUL=1` as x3. Raw PLLCFGR `0x00000006` therefore encodes HSI x3,
  and the observed undivided tree is 48 MHz. The earlier decoder both assumed
  24 MHz x2 and masked PLLSRC to one bit; the corrected contract validates the
  exact-unit tuple and rejects every independently varied field.

### EVID-K1-049 — First runnable Embassy image write

- **Verified artifact:** the 25,720-byte receive-only async raw image has
  SHA-256 `874da6e7fe70d9564eb5b650581b3525a4aafa0077613c074a07e3fb4bc7bada`
  and CRC-32 `7bcffca6`. Its 192-byte vector table and exact DMA1, TIM15, and
  USART1 handler slots passed static verification and negative package tests.
- **Recovery gates:** the retained primary/secondary F4HWN v5.5.0 recovery
  images still matched each other at SHA-256
  `7b6b277c319e6924bd878f4e4208490875dc3f15beb205c366d20130c02a4463`;
  the two 8,192-byte EEPROM backups matched at SHA-256
  `81716a35daa7a7f05bd077e28713dad50be7e6c1cbb9791e330c0a423ccdeafa`.
- **Write observation:** exact K1 bootloader `7.03.01` acknowledged all 101
  pages under transaction `9bca3352`, without retry, and reported
  `acknowledged_not_read_back`.
- **Boundary:** this proves only bootloader page acknowledgements. A manual
  power-cycle plus normal hello, visible boot screen, and main-key label are
  required before claiming Reset, TIM15, DMA, async USART1, display, or keypad
  behavior.
- **First runtime result:** after power-cycle the display remained blank and two
  normal-mode hello probes timed out. The image had not relocated VTOR from the
  bootloader table before enabling TIM15/DMA/USART1 interrupts.
- **Pinned correction:** `Core/Src/system_py32f071.c:53-55,147-151` sets the
  application vector base to `FLASH_BASE | 0x2800`, and the PY32F071 device
  header declares VTOR present. The corrected AFIK entry writes only that exact
  source-backed address before inherited HAL initialization.
- **Corrected artifact:** the 25,784-byte raw image has SHA-256
  `3f39e7c2a9ffa282685da321e2da01006a880d747344cd23222e7480bc30adb2`.
  Static verification additionally requires the retained
  `k1_relocate_vectors` boundary before accepting the package.
- **Corrected write observation:** exact K1 bootloader `7.03.01` acknowledged
  all 101 pages of the CRC-32 `4401a861` image under transaction `5b0f91b5`,
  without retry. Status remains `acknowledged_not_read_back`; power-cycle and
  application observations are separate gates.

### EVID-K1-050 — Corrected Embassy image physical runtime result

- **Observation:** after power-cycle, the boot screen returned and faded in
  quickly. The user then observed all main keys correctly identified and
  rendered on the second display line.
- **Serial witness:** a post-boot normal-mode probe at 38,400 baud returned
  `AFIK-K1-0.2` from `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`.
- **Interpretation:** the source-backed VTOR relocation at `0x08002800`,
  inherited-clock initialization, TIM15 Embassy time path, async USART1/DMA,
  cooperative SPI1 display path, backlight, matrix scan, debounce, and key
  rendering all functioned together on this unit. This is bounded bring-up
  evidence only; no RF, TX, side-key, persistence, or flash read-back claim is
  made.

### EVID-K1-051 — Receive-only auxiliary observation contract

- **Source boundary:** the pinned K1 source identifies PTT on PB10 and refers
  to a separate special side-key path. It does not provide AFIK with an
  independently verified exact side-key GPIO mapping, polarity, settling, or
  electrical behavior.
- **Host contract:** `radio-firmware-k1::aux_inputs` retains a bounded GPIOB
  input-data-register sample with explicit target/test provenance and a
  nonzero, strictly newer adapter sequence. Unstable, stale, or invalid samples
  are rejected before becoming observations.
- **Interpretation boundary:** PB10 is exposed only as an uninterpreted raw
  bit. The contract creates no semantic key edge and cannot mutate display,
  persistence, channel state, RF, or TX authority. No target GPIOB binding or
  physical side-key observation was added.
- **Required experiment:** independently source the exact side-key mapping,
  then define a separately guarded receive-only observation that preserves the
  known-good recovery path. Do not infer side-key pins from the main matrix.

### EVID-K1-052 — Exact side-key mapping resolved as the unselected matrix column

- **Source:** `App/driver/keyboard.c` at the pinned Armel commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`. `keyboard[5][4]` at lines 149-183
  and `KEYBOARD_Poll` at lines 185-281.
- **Mapping:** the side keys are not separate GPIO pins. They occupy index `0`
  of the same PB15..PB12 row inputs used by the main matrix, read during the
  pass the source labels the "Zero col", where **no** column is driven low.
  `keyboard[0][0]` is `KEY_SIDE1` on PB15 and `keyboard[0][1]` is `KEY_SIDE2`
  on PB14. `keyboard[0][2]` and `keyboard[0][3]` are explicitly `KEY_INVALID`,
  so PB13 or PB12 low with all columns high is an undefined observation.
- **Electrical reading:** the scan loop runs `j = 0..4`. It drives all four
  columns PB6..PB3 high on every iteration and only calls
  `GPIO_ResetOutputPin(PIN_COL(j - 1))` when `j > 0`. With every column high no
  main-matrix button can pull a row low, so a row read low in that state must
  be a side key wired directly to the row. The source comment records this as
  the "special case of nothing pulled down".
- **Polarity and settling:** rows are active low and read through
  `LL_GPIO_ReadInputPort(GPIOB)` masked to PB15..PB12. Each column pass, the
  zero column included, uses up to eight reads separated by
  `SYSTICK_DelayUs(10)` and requires three consecutive identical samples
  (`match_count >= 2`); an unstable column is skipped without a key.
- **PTT remains separate:** `KEYBOARD_GetKey` consults `GPIO_IsPttPressed()`
  only when the matrix scan returned `KEY_INVALID`. `App/driver/gpio.h:31`
  fixes PTT at GPIOB pin 10, active low. `gpio.h` defines no side-key pin,
  which corroborates that side keys have no dedicated GPIO.
- **AFIK gap:** `radio-firmware-k1::keypad::scan` drives all columns high but
  never samples in that state; it immediately selects column 0. AFIK therefore
  cannot currently observe a side key at all. The bounded change is one
  additional unselected-state read, not a new pin binding.
- **Confidence:** high for the pinned source's intended mapping and scan order.
  No exact-unit physical observation of a side key has been made, so polarity
  and stability on this specific board remain unconfirmed.
- **Permitted use:** cite as the independently sourced mapping that `RISK-026`
  required. AFIK must implement its own receive-only scan; the pinned loop must
  not be ported or translated.
- **Required experiment:** extend the read-only `probe-keypad` surface with the
  unselected-column row mask, then observe released, SIDE1-held, and SIDE2-held
  states on the exact unit before any semantic side-key action exists.

### EVID-BK4819-053 — Receive register block from the pinned K1 firmware

- **Source:** `armel/uv-k1-k5v3-firmware-custom` at the pinned commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`: `App/driver/bk4819.c`,
  `App/driver/bk4819-regs.h`, `App/radio.c`, and `App/dcs.c`. The operator
  designated this source authoritative for register values and pinout where
  primary Beken documentation is silent.
- **Power and mode:** `BK4819_RX_TurnOn` writes `REG_37 = 0x1F0F`, clears
  `REG_30`, then writes the receive block `0xBEF1` (VCO calibration, RX link,
  AF DAC, discriminator, PLL/VCO, RX DSP, with PA gain, MIC ADC, and TX DSP
  disabled). AFIK writes exactly this block and never derives a transmit word
  from it.
- **Demodulator:** `RADIO_SetModulation` reads `REG_31` and sets bit zero for
  AM or clears it otherwise; AM additionally writes `REG_42 = 0x6F5C` and
  `REG_2A = 0x7434`, while FM and USB write `REG_42 = 0x6B5A`,
  `REG_2A = 0x7400`, `REG_2B = 0`, and `REG_2F = 0x9890`. Both paths write
  `REG_54 = 0x9009`, `REG_55 = 0x31A9`, AF DAC gain `REG_48 = 0xF`, and
  `REG_3D = 0x2AAB` except for USB which writes zero. AFC (`REG_73` bit 4) is
  disabled for every non-FM mode.
- **Audio path:** `BK4819_SetAF` writes `REG_47 = (6 << 12) | (AF << 8) | (1 << 6)`.
  `AF_MUTE` is 0, `AF_FM` is 1, and `AF_BASEBAND2` (USB) is 5. The pinned
  source drives AM through `AF_FM` plus the AM demodulator bit, which AFIK
  reproduces rather than selecting the documented `AF_AM` code.
- **Bandwidth:** `BK4819_SetFilterBandwidth` writes `REG_43 = 0x3628` for the
  wide filter and `0x3648` for the narrow filter in the weak-signal-equal
  variant used by `RADIO_SetupRegisters`.
- **Squelch:** `BK4819_SetupSquelch` writes `REG_4D = 0xA000 | close_glitch`,
  `REG_4E = (1 << 14) | (5 << 11) | (6 << 9) | open_glitch`,
  `REG_4F = (close_noise << 8) | open_noise`, and
  `REG_78 = (open_rssi << 8) | close_rssi`. RSSI thresholds are 0.5 dB per
  step and the noise fields are seven bits. The threshold values themselves are
  per-unit calibration data and are never invented by AFIK.
- **Sub-audio:** CTCSS uses `REG_51 = 0x904A` with
  `REG_07 = CTC1 | ((freq_tenths_hz * 206488 + 50000) / 100000)`. CDCSS uses
  `REG_51 = 0x8033`, `REG_07 = CTC1 | 2775`, then `REG_08` twice with the low
  twelve bits and `0x8000 |` the high twelve bits of the 23-bit code word.
  `App/dcs.c` builds that word as `golay(octal_code + 0x800)` with generator
  `0x08EA`, inverted by `^ 0x7FFFFF` for reverse polarity.
- **Metering:** RSSI is `REG_67 & 0x01FF` in 0.5 dB steps with
  `dBm = rssi / 2 - 160`; the glitch indicator is `REG_63 & 0x00FF` and the
  excess-noise indicator is `REG_65 & 0x007F`. Tone detection is latched in
  `REG_02`: CTCSS found/lost at bits 7 and 6, CDCSS found/lost at bits 9 and 8,
  and squelch found/lost at bits 3 and 2.
- **AGC:** `BK4819_InitAGC` writes gain tables `REG_13 = 0x03BE`,
  `REG_12 = 0x037B`, `REG_11 = 0x027B`, `REG_10 = 0x007A`, then
  `REG_14 = 0x0000` with `REG_49 = (50 << 7) | 32` for AM or `REG_14 = 0x0019`
  with `REG_49 = (84 << 7) | 56` otherwise, followed by `REG_7B = 0x8420`.
- **Filter path:** `BK4819_PickRXFilterPathBasedOnFrequency` selects the VHF
  low-noise amplifier below 28 MHz and the UHF path otherwise, driven through
  the `REG_33` GPIO output word (`0x40 >> pin`, VHF pin 4 and UHF pin 3).
- **Confidence:** high for reproducing the pinned firmware's behaviour, since
  every value is copied from a working implementation on the same chip.
  Medium for the underlying silicon semantics, which remain undocumented for
  the fields the source itself marks unknown. No AFIK receive register has yet
  been observed on hardware.

### EVID-K1-054 — K1 BK4819 three-wire pinout and transfer order

- **Source:** `App/driver/bk4819.c` lines 32-34 and the surrounding transfer
  helpers at the pinned Armel commit.
- **Pinout:** `PIN_CSN` is `GPIOF` pin 9, `PIN_SCL` is `GPIOB` pin 8, and
  `PIN_SDA` is `GPIOB` pin 9. `SDA` is bidirectional: reads switch the pin to
  an input and restore output afterwards. The K1 does not drive the BK4819
  through the SPI peripheral; the bus is bit-banged.
- **Transfer order:** every transfer releases chip select, drives the clock
  low, waits one microsecond, then asserts chip select. The address byte is
  shifted most significant bit first, data changing while the clock is low and
  latched on the rising edge, with one microsecond between edges. A write
  follows with the 16-bit value; a read sets bit 7 of the address and then
  shifts sixteen bits in. Every transfer ends by releasing chip select and
  leaving both clock and data high.
- **Confidence:** high for the pinout and ordering, which come directly from
  the working reference implementation. No AFIK transfer has been performed on
  hardware, so the electrical timing on this unit is unverified.

### EVID-BK4829-055 — The pinned K1 build compiles the BK4829 driver

- **Source:** `App/CMakeLists.txt:10` at the pinned Armel commit lists
  `driver/bk4829.c`, not `driver/bk4819.c`. Both drivers share the same
  three-wire bus, pinout, and register addresses, but write different values.
- **Differences that matter for reception:** initialisation power blocks
  `REG_37` `0x9D1F` (BK4819: `0x1D0F`); receive turn-on `REG_37` `0x9F1F`
  (`0x1F0F`) and mode `REG_30` `0xBFF1` (`0xBEF1`); audio output fixed bits
  `REG_47` `0x6042` (`0x6040`); filter bandwidth `REG_43` wide `0x3028` and
  narrow `0x4048` (`0x3628`/`0x3648`); sub-audio `REG_51` CTCSS `0x9040` and
  CDCSS `0xA033` (`0x904A`/`0x8033`); microphone gain `REG_7D` `0xE920`
  (`0xE940`); audio level `REG_48` `0x33A8` (`0xB3A8`); one fixed gain table
  `REG_10` `0x0318`, `REG_11` `0x033A`, `REG_12` `0x03DB`, `REG_13` `0x03DF`,
  `REG_14` `0x0210`, `REG_49` `0x2AB2`, `REG_7B` `0x73DC` with no
  modulation-dependent split and no automatic-mode switch; and a longer
  initialisation tail (`0x40`, `0x1C`, `0x1D`, `0x1E`, `0x1F`, `0x3E`, `0x73`,
  `0x77`, `0x19`, `0x28`, `0x29`, `0x2A`, `0x2C`, `0x2F`, `0x53`, `0x7E`,
  `0x46`, `0x4A`, `0x07`).
- **Physical confirmation:** with BK4819 values the exact unit reported RSSI
  raw `0`, glitch `255`, and noise `127` on every sample. With BK4829 values
  the same image reported moving RSSI, glitch, and noise. See `EVID-K1-057`.
- **Confidence:** high. AFIK models both variants explicitly and the K1 target
  selects the BK4829 profile.

### EVID-BK4819-056 — Two corrections to the recorded receive contract

- **Filter path split is 280 MHz, not 28 MHz.** The pinned source compares
  `Frequency < 28000000` where the frequency is held in 10 Hz units, the same
  units `REG_38`/`REG_39` take. AFIK holds hertz, so the boundary is
  `280_000_000`. The earlier value selected the UHF low-noise amplifier for
  every 2 m channel.
- **The receive mode word must be written after the frequency.** `REG_30`
  carries the VCO calibration request, and the pinned `RADIO_SetupRegisters`
  calls `BK4819_SetFrequency` before `BK4819_RX_TurnOn`. Writing the mode word
  first calibrates the synthesiser against the previous frequency.
- **Confidence:** high; both were found by comparing AFIK's behaviour on the
  exact unit against the pinned source and are now covered by ordering tests.

### EVID-K1-057 — Physical AFIK receive bring-up on the exact unit

- **Image:** `AFIK-K1-0.8`, 30,424 bytes, CRC-32 `be1f7f4a`, written through
  bootloader `7.03.01` with all 119 pages acknowledged and no retry.
- **Bus proof:** `probe-rf` returned the configured filter-bandwidth register
  `0x43` as `0x4048`, the exact non-trivial BK4829 narrow value the image
  wrote. Reads and writes therefore both work over the bit-banged three-wire
  bus with the shared data line.
- **Receive proof:** tuned to 145.500000 MHz, narrow FM, squelch-off
  thresholds. Successive samples reported RSSI raw 52, 56, 58, 57, 56, 56, 55
  (about -134 to -131 dBm), glitch 41, 29, 34, 39, 31, 35, and noise 83, 56,
  55, 56, 51, 56, with the carrier squelch link opening. The indicators move
  together and settle, which the earlier all-zero/all-ones readings did not.
- **Bounds:** this proves the bus, the power-on table, tuning, and metering on
  one unit. It does not prove demodulated audio, sensitivity, calibration, tone
  decoding, or any transmit behaviour, none of which the image implements.

### EVID-K1-058 — Bit-banging must not run beside an inbound serial frame

- **Observation:** an earlier image ran the receive bring-up in its own task.
  The display and keypad kept working, but the application answered no serial
  request. The bring-up busy-waits for several milliseconds while the serial
  task reads one byte at a time, so inbound bytes were lost and the framing
  window never completed.
- **Resolution:** the receiver is owned by the serial task and every transfer
  runs between a decoded request and its response, when the host is waiting and
  not transmitting. The serial task also yields after a read error so a
  persistent receiver fault cannot starve the display task.
- **Confidence:** high; the symptom reproduced on two units and disappeared
  once the work moved inside the request.

### EVID-K1-059 — The programming cable and the speaker share one jack

- **Observation:** driving the audio-path pin `PA8` in any state removes the
  serial link. Across three images on the same unit: `AFIK-K1-0.8` never
  touched `PA8` and serial worked; `AFIK-K1-0.9` drove it low (amplifier off)
  at boot and serial was dead while the display and keypad kept running;
  `AFIK-K1-1.0` left it untouched until requested, serial worked, and the link
  dropped the moment an audio request drove the pin.
- **Interpretation:** the K1 programming cable occupies the speaker and
  microphone jack, so the audio path and the host serial path contend for the
  same physical connection. With the cable inserted the internal speaker is
  disconnected, so receive audio cannot be heard over it either.
- **Consequence:** audio cannot be commanded or observed over the serial link.
  `AFIK-K1-1.1` moves the audio toggle to the keypad and shows the receive
  state on the display, so the operator unplugs the cable, listens, and reads
  the screen. See `ADR-055`.

### EVID-K1-060 — Demodulated receive audio confirmed on the exact unit

- **Image:** `AFIK-K1-1.1`, 31,672 bytes, CRC-32 `827b5f8e`, written through
  bootloader `7.03.01` with all 124 pages acknowledged and no retry.
- **Observation:** with the serial cable unplugged, pressing side key one
  switched the display to `AUDIO ON` and receiver noise was audible from the
  speaker on 145.500 MHz narrow FM.
- **What this establishes:** the complete receive chain works on this board:
  the bit-banged three-wire bus, the BK4829 power-on table, tuning, the
  demodulator and audio output register, and the `PA8` audio amplifier.
- **What it does not establish:** sensitivity, audio quality, calibrated
  squelch, tone decoding, or reception of a specific signal. The channel ran
  with the pinned squelch-off thresholds, so open-squelch noise is the expected
  sound. `RISK-030` and `RISK-031` remain open.

### EVID-K1-061 — External configuration memory identified on the exact unit

- **Source of the expectation:** pinned `armel/uv-k1-k5v3-firmware-custom` commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`. `App/driver/py25q16.c` drives a
  serial NOR memory on `SPI2` with `SCK` on `PA0`, `MOSI` on `PA1`, `MISO` on
  `PA2`, and chip select on `PA3`, using read `0x03`, page program `0x02`,
  sector erase `0x20`, write enable `0x06`, and status `0x05`, with a 256-byte
  page and a 4 KiB sector. `App/driver/eeprom_compat.c` maps the radio's logical
  EEPROM addresses onto that device, reaching approximately `0xD000`.
- **Observation on the exact unit, 2026-08-08:** `AFIK-K1-3.1` read the
  identification over `SPI2` at start-up and reported `MEM ID 68 40 15` on the
  information screen. The capacity code `0x15` is 2 MiB, which matches the
  16 Mbit part the pinned source names.
- **Correction to the expectation:** manufacturer `0x68` is not Puya `0x85`, so
  the fitted device is a Boya-family `BY25Q16` rather than the `PY25Q16` the
  pinned source drives. Geometry, capacity, and the command set used here are
  the standard serial-NOR set both implement, and AFIK issues only those
  commands. AFIK claims no compatibility beyond what it exercises.
- **Confidence:** high for the wiring, the command set, and the fitted capacity
  on this unit; the pinned source remains the only evidence for the pinout, and
  the manufacturer differs from it.

### EVID-K1-062 — Configuration retained in external memory across a power cycle

- **Observation on the exact unit, 2026-08-08, running `AFIK-K1-3.2`:** a
  generated PMR446 plan was written over serial as one 46-byte object and the
  transaction verified by read-back, reporting `generation=1`. After a power
  cycle the information screen reported sixteen channels.
- **Why this is conclusive:** this image reserves no internal flash sector for
  configuration. `py32f071_retained` and its sector were removed when the store
  moved, and the packaging gates now allow the application the whole region
  through `0x08020000`. The external memory is therefore the only place the
  restored configuration could have come from.
- **Multi-page observation, same unit, running `AFIK-K1-3.4`:** a complete
  configuration of twelve explicit channels, two named banks, and one generated
  plan was written over serial and verified: fifteen objects, 594 stored bytes,
  a 685-byte canonical image spanning three 256-byte pages. Read back before and
  after a power cycle, the image was byte-identical, SHA-256
  `b72c662a7a93d7bbe86652d143fd0d15...`. Page splitting, the whole-region erase,
  and the yielding retain therefore hold for a configuration larger than one
  page.
- **Observed detail:** a restored radio reports `generation=1` rather than the
  generation it was written with. The generation counts commits in the running
  session and is not carried in the image; the objects themselves are identical.
- **Confidence:** high for retention and exact restoration of a multi-page
  configuration on this unit. Wear behaviour and the erase-before-write boundary
  under power loss remain unobserved; the latter is `RISK-004`.

### EVID-K1-063 — K1 battery sense path, calibration, and discharge curve

- **Source:** `armel/uv-k1-k5v3-firmware-custom` at the pinned commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`: `App/board.c`,
  `App/helper/battery.c`, `App/settings.c`, `App/CMakeLists.txt`, and
  `Drivers/PY32F071_HAL_Driver`. The operator designated this source
  authoritative for register values and pinout where primary documentation is
  silent.
- **Sense input:** `BOARD_ADC_Init` enables `GPIOB`, sets `PB0` and `PB1` to
  analogue mode, enables `ADC1`, selects the ADC clock as `PCLK/4`, twelve-bit
  resolution, right-aligned data, scan disabled, software trigger, single
  conversion, no DMA, and sets regular sequencer rank one to
  `LL_ADC_CHANNEL_8` with a 41.5-cycle sampling time. It then calibrates and
  enables the converter. `PB1` is set to analogue mode but no channel is
  assigned to it, so AFIK claims nothing about it.
- **Reading:** `BOARD_ADC_GetBatteryInfo` starts one software conversion, waits
  for end-of-sequence, and returns the twelve-bit result as the voltage. It
  returns a constant zero for current, so the pinned firmware measures no
  charging current on this board and neither does AFIK.
- **Converter model:** the Puya `PY32F071` LL header uses `CR2` with `CAL`,
  `SQR3` regular ranks, and `SMPR3` sampling times, which is the register model
  the vendored `py32-metapac` selects as `adc_v2` for this part, and its
  `CYCLES41_5` encoding matches `LL_ADC_SAMPLINGTIME_41CYCLES_5` exactly.
  `ADC_PRECALIBRATION_DELAY_ADCCLOCKCYCLES` is `2` in
  `py32f071_hal_adc_ex.c:73`, the same value the vendored HAL already carries
  for the F072, so enabling that driver for the F071 introduces no new constant.
- **Calibration:** `SETTINGS_InitEEPROM` reads six half-words from the external
  memory at `0x010000 + 0x140` into `gBatteryCalibration`. Entry three is the
  count the sense input reads at 7.60 V: `BATTERY_GetReadings` computes
  `average * 760 / gBatteryCalibration[3]` in hundredths of a volt from a
  rolling four-conversion average. This is per-unit data, is below the region
  AFIK claims at `0x100000`, and AFIK reads it without a region so it can never
  be written.
- **Discharge curve:** `Voltage2PercentageTable` holds one piecewise-linear
  curve per battery type. The 1500 mAh K1 entry is `{828,100}, {813,97},
  {758,25}, {726,6}, {630,0}` in hundredths of a volt and percent, and the
  source comments it as an estimated curve to be improved. `BATTERY_VoltsToPercent`
  interpolates linearly between adjacent points and clamps to 0..100. Below
  630 the pinned firmware declares the pack critical and reduces service.
- **AFIK boundary:** AFIK selects the 1500 mAh curve because it cannot read
  which pack is fitted, and reports no charge at all when the calibration is
  absent, erased, or outside the converter's range. The percentage is therefore
  an estimate from an estimated curve; it is a warning that the pack is going,
  not a measurement of energy remaining.
- **Confidence:** high for the sense pin, channel, converter configuration,
  calibration location, and scale, all of which come directly from a working
  implementation on this board. Medium for the curve, which the pinned source
  itself marks estimated. No AFIK conversion has yet been observed on hardware.
- **Required experiment:** on the exact unit, compare the reported voltage
  against a meter across the pack at a charged and a part-discharged state, and
  confirm the indicator falls monotonically over a discharge. Until that is
  done the percentage is unverified on this radio.

### EVID-K1-064 — Extent of the vendor's external-memory map on the K1

- **Source:** `armel/uv-k1-k5v3-firmware-custom` at the pinned commit
  `fe9c4e9432694b50aea651084a043aae0b58673d`, every `PY25Q16_ReadBuffer` and
  `PY25Q16_WriteBuffer` call site: `App/settings.c`, `App/radio.c`, and
  `App/ui/welcome.c`.
- **Addresses used:** settings and the boot-logo line data from `0x00A008`
  through `0x00A160`; a calibration block based at `0x010000` holding the RSSI
  calibration at `+0xC0` and `+0xC8`, the per-band transmit-power table at
  `0x0100D0`, the battery calibration at `+0x140`, the VOX thresholds at
  `+0x150` and `+0x168`, and miscellaneous retained state at `+0x188`; and a
  whole boot-logo sector at `LOGO_FLASH_ADDR` `0x011000`, which the welcome
  screen reads a status line and a full framebuffer out of.
- **Highest address touched:** the end of the boot-logo sector, `0x012000`.
- **Correction this forced:** `radio-eeprom` guarded only below `0x010000`,
  which is the first address of the calibration block rather than the last
  address of the vendor's map. A region claimed at exactly `0x010000` was
  therefore accepted, and its erase-before-write would have destroyed this
  unit's battery calibration, RSSI calibration, transmit-power table, VOX
  thresholds, and boot logo. No AFIK build ever claimed such a region: the K1
  image claims `0x100000`, so nothing was written and no unit was damaged. The
  bound is now `0x020000`.
- **Second correction:** the same wrong bound made the read-only vendor path
  refuse the battery calibration at `0x010140`, which is why `AFIK-K1-3.5`
  reported `BAT ---%` on the exact unit rather than a charge.
- **Confidence:** high for the addresses, which are literal constants at their
  call sites in a working implementation for this board. The map is not
  necessarily complete: it covers what this build touches, not everything the
  factory tooling may have written, which is why the bound is rounded up to the
  next whole 64 KiB rather than set to `0x012000`.
- **Permitted use:** cite as the reason AFIK claims no external-memory region
  below `0x020000` and as the source of the battery calibration address. AFIK
  reads the battery calibration and writes nothing in this range.

## Sources used by K5DRV-048

### DP32G030 reference manual v1.23, retrieved copy

- **Document:** the manual already recorded under *Sources used by DP32-003*.
- **Retrieved again:** 2026-08-11 from the same mirror. The downloaded file's
  SHA-256 is `d1923c0a1830dada46706515ced53978f9a5086e04ce178deaf28d2928c62573`,
  which matches the hash recorded for `DP32-003` exactly, so the two work
  packages read the same bytes.
- **Method:** text extracted with `pdftotext -layout`. Every register fact below
  cites the printed page it was copied from.

### `machshev/uv-k5-firmware-custom` working checkout

- **Repository:** local clone at `~/base/uv-k5-firmware-custom`, remote
  `machshev/uv-k5-firmware-custom` with upstream `egzumer/uv-k5-firmware-custom`,
  commit `7f959e8d09b435845753182b329e3a88490ebe32`, resolved 2026-08-11.
- **Standing:** this is the build `EVID-K5-012` records the operator running
  correctly on all three V1-generation units, including the exact UV-K6 attached
  for this work package. It is therefore hardware-tested board evidence for the
  V1 board, in the same standing as the pinned K1 project.
- **Permitted use:** cite exact source locations as evidence for board bindings
  which the reference manual cannot supply, such as which pin a peripheral is
  wired to on this board.
- **Prohibited use:** copying, linking, porting, or incrementally translating its
  application or driver implementation into AFIK source. Every register fact
  below is taken from the reference manual; this project is cited only for board
  wiring and for corroboration.

## Accepted K5DRV-048 facts

### EVID-DP32-004 — Peripheral base addresses

- **Fact:** SYSCON is based at `0x4000_0000`, PMU at `0x4000_0800`, GPIOA at
  `0x4006_0000`, GPIOB at `0x4006_0800`, GPIOC at `0x4006_1000`, UART0 at
  `0x4006_B000`, UART1 at `0x4006_B800`, UART2 at `0x4006_C000`, and PORTCON at
  `0x400B_0000`.
- **Source:** DP32G030 manual section 5.1 address map, and the base line printed
  in each module's own register map: PMU printed page 62, SYSCON page 80,
  PORTCON page 94, UART page 263.
- **Method:** copied. The address map and each module's own register map agree.
- **Confidence:** high; not yet confirmed by a read on a physical unit.
- **Permitted use:** address these modules from a DP32G030 driver.
- **Required experiment:** observe the UART1 base working on the exact unit by
  receiving a frame the image sent.

### EVID-DP32-005 — Clock sources and their reset state

- **Fact:** the internal high-frequency RC oscillator RCHF is nominally 48 MHz.
  `PMU_SRC_CFG` at offset `0x10` resets to `0x03`; its bit 0 `RCHF_EN` enables
  RCHF and its bit 1 `RCHF_FSEL` selects 48 MHz when clear and 24 MHz when set.
  After reset the system clock is RCHF. `SYSCON_CLK_SEL` at offset `0x00` resets
  to `0x02`; its bit 0 `SYS_CLK_SEL` selects RCHF when clear and the divided
  clock when set, and bits 3:1 `DIV_CLK_SEL` divide the source clock.
  `SYSCON_RC_FREQ_DELTA` at offset `0x78` reports the measured deviation of the
  real RCHF frequency from 48 MHz: bit 31 `RCHF_SIG` is the sign, positive when
  set, and bits 30:11 `RCHF_DELTA` the magnitude in hertz.
- **Source:** DP32G030 manual printed pages 62 and 66 for `SRC_CFG`, 74 and 81
  for the clock network and `CLK_SEL`, 84 for `RC_FREQ_DELTA`, and section 5.6.4
  page 73 for the statement that the system clock starts on RCHF.
- **Method:** copied.
- **Confidence:** high for the register fields. The reset default is therefore
  RCHF at 24 MHz, which an image must not assume is still the case: a bootloader
  runs before it.
- **Permitted use:** select 48 MHz RCHF explicitly, select RCHF as the system
  clock explicitly, and correct the nominal frequency by `RC_FREQ_DELTA` before
  deriving any timing from it.
- **Required experiment:** confirm the derived frequency indirectly by whether a
  baud rate computed from it is received correctly by the host.

### EVID-DP32-006 — Peripheral clock gating

- **Fact:** `SYSCON_DEV_CLK_GATE` at offset `0x08` resets to `0x00` and gates
  each peripheral's clock with one bit, set to enable: bit 0 GPIOA, bit 1 GPIOB,
  bit 2 GPIOC, bit 4 IIC0, bit 5 IIC1, bit 6 UART0, bit 7 UART1, bit 8 UART2,
  bit 10 SPI0, bit 11 SPI1, bits 12 to 14 TIMER_BASE0 to 2, bits 15 and 16
  TIMER_PLUS0 and 1, bits 17 and 18 PWM_BASE0 and 1, bits 20 and 21 PWM_PLUS0
  and 1, bit 22 RTC, bit 23 IWDT, bit 24 WWDT, bit 25 SARADC, bit 27 CRC, and
  bit 28 AES.
- **Source:** DP32G030 manual `DEV_CLK_GATE` description, printed pages 82 to 83.
- **Method:** copied.
- **Confidence:** high.
- **Permitted use:** enable exactly the peripherals a driver uses, and no others.
- **Required experiment:** none beyond observing that a gated peripheral works.

### EVID-DP32-007 — Pin function selection

- **Fact:** PORTCON holds `PORTA_SEL0` at `0x00` and `PORTA_SEL1` at `0x04`,
  `PORTB_SEL0` at `0x08`, `PORTB_SEL1` at `0x0C`, `PORTC_SEL0` at `0x10`, the
  input-enable registers `PORTA_IE`, `PORTB_IE` and `PORTC_IE` at `0x100`,
  `0x104` and `0x108`, pull-up at `0x200`/`0x204`/`0x208`, pull-down at
  `0x300`/`0x304`/`0x308`, and open-drain at `0x400`/`0x404`/`0x408`. Each
  `SELn` register holds one four-bit field per pin, pin `n` of the low eight in
  `SEL0` at bits `4n+3:4n` and pin `n+8` in `SEL1` at the same position. Field
  value `0` selects the digital GPIO function on every pin. On PA7 value `1`
  selects `UART1_TX`, and on PA8 value `1` selects `UART1_RX`.
- **Source:** DP32G030 manual PORTCON register map printed page 94, `PORTA_SEL0`
  pages 95 to 96, `PORTA_SEL1` pages 97 to 98, and the pin-function table in
  section 3, printed pages 20 to 22, which lists `UART1_TX` on PA7 and
  `UART1_RX` on PA8.
- **Method:** copied.
- **Confidence:** high.
- **Permitted use:** select a pin's peripheral function, and enable the input
  buffer for a pin a peripheral or a scan must read.
- **Required experiment:** none beyond observing the selected function working.

### EVID-DP32-008 — General-purpose IO

- **Fact:** each GPIO port exposes `GPIODATA` at offset `0x00` and `GPIODIR` at
  offset `0x04`, one bit per pin, with `GPIODIR` set for output and clear for
  input, and both reset to zero. The GPIO module's clock is enabled through
  `SYSCON_DEV_CLK_GATE`.
- **Source:** DP32G030 manual GPIO register descriptions, printed page 119, and
  section 5.8.3, printed page 110.
- **Method:** copied.
- **Confidence:** high.
- **Permitted use:** drive and read board pins whose function is GPIO.
- **Required experiment:** none for this work package, which drives no GPIO.

### EVID-DP32-009 — UART controller

- **Fact:** each UART exposes `UART_CTRL` at `0x00`, `UART_BAUD` at `0x04`,
  `UART_TDR` at `0x08`, `UART_RDR` at `0x0C`, `UART_IE` at `0x10`, `UART_IF` at
  `0x14`, `UART_FIFO` at `0x18`, `UART_FC` at `0x1C` and `UART_RXTO` at `0x20`.
  In `UART_CTRL`, bit 0 `UARTEN` enables the module, bit 1 `RXEN` receive, bit 2
  `TXEN` transmit, bit 3 `RXDMAEN` and bit 4 `TXDMAEN` select DMA rather than CPU
  access to the data registers, bit 5 `NINEBIT` selects nine-bit data, and bit 6
  `PAREN` enables parity; all reset to zero, so an eight-bit, no-parity,
  CPU-driven configuration is the register's own default once the enable,
  receive and transmit bits are set. `UART_BAUD` bits 15:0 hold the divider. In
  `UART_IF`, bit 10 `RXFIFO_EMPTY` and bit 13 `TXFIFO_EMPTY` report the two FIFO
  empty states and reset set, bit 14 `TXFIFO_FULL` reports the transmit FIFO
  full, and bit 16 `TXBUSY` reports the transmitter busy. `UART_FIFO` bit 6
  `RF_CLR` and bit 7 `TF_CLR` clear the receive and transmit FIFOs and
  self-clear. Both FIFOs are eight bytes deep.
- **Source:** DP32G030 manual UART register map printed page 263, `UART_CTRL`
  pages 264 to 265, `UART_BAUD` and the data registers pages 265 to 266,
  `UART_IF` pages 267 to 268, `UART_FIFO` page 269, and the module description
  page 255 for the FIFO depth.
- **Fact:** the divider is the module clock frequency divided by the wanted baud
  rate, rounded. The manual gives no formula in text; it prints one as an image
  and then works the example `48 MHz / 115200 = 416.6`, choosing 417.
- **Source:** DP32G030 manual section 5.16.4, printed page 257.
- **Method:** copied for the register fields; the divider rule is read off the
  manual's own worked example rather than from the formula image.
- **Confidence:** high for the fields. Medium-high for the divider rule, which
  rests on one worked example; it is corroborated by the pinned V1 firmware,
  which divides a `RC_FREQ_DELTA`-corrected frequency by a constant of the same
  order (`driver/uart.c`, `UART1->BAUD = Frequency / 39053U`).
- **Permitted use:** drive UART1 by polling `UART_IF`, at a divider computed from
  the corrected RCHF frequency.
- **Required experiment:** send a frame the host can decode, which settles both
  the divider rule and the corrected frequency at once.

### EVID-K5-019 — V1 board bindings the manual cannot supply

- **Observation:** in the pinned `uv-k5-firmware-custom` checkout, `board.c`
  selects `PORTCON_PORTA_SEL0_A7_BITS_UART1_TX` and
  `PORTCON_PORTA_SEL1_A8_BITS_UART1_RX`, and its comments record that both pins
  are already left in that state by the stock bootloader. `main.c` enables
  GPIOA, GPIOB, GPIOC, UART1, SPI0, SARADC, CRC, AES and PWM_PLUS0 in
  `SYSCON_DEV_CLK_GATE`, and `driver/system.c` selects 48 MHz RCHF and leaves the
  system clock on RCHF.
- **Inference:** on the V1 board the programming connector is UART1 on PA7/PA8.
  Confidence high: this build runs on all three units per `EVID-K5-012`, the
  stock bootloader talks to a host over that same connector at 38,400 baud, and
  no other UART is configured for it.
- **Observation:** `start.S` sets the initial stack pointer to `0x2000_3FF0`,
  sixteen bytes below the top of the evidenced RAM, and its vector table carries
  the full Cortex-M0 and DP32G030 interrupt list rather than two entries.
- **Not established:** why the top sixteen bytes are avoided. It is not
  documented in that project, and nothing here shows the bootloader uses them.
  AFIK follows it because the cost is sixteen bytes and the alternative is an
  assumption about what a bootloader left behind.
- **Permitted use:** bind UART1 to PA7/PA8 in the K5 image, gate only the clocks
  the image uses, and start the stack at `0x2000_3FF0`.
- **Required experiment:** confirm the binding by receiving, on the host, a frame
  the AFIK image sent through UART1 on the exact unit.

### EVID-K5-020 — First AFIK code observed running on a V1 radio

- **Unit:** the `EVID-K5-013` UV-K6, bootloader `2.00.06`, on the `1a86:7523`
  adapter at `/dev/ttyUSB0`, 2026-08-12. Its EEPROM was backed up first
  (`crc32=bd475dd8`, 8,192 bytes, firmware reported `F4HWN v4.3`) and retained
  outside the repository in two copies.
- **Observation:** `AFIK-K5-1.0` was written through the qualified V1 path, all
  240 pages acknowledged, and after a power-cycle it was completely silent: no
  plain-text banner in a 60-second capture across power-on, and three
  `probe-normal` attempts timed out.
- **Observation:** a diagnostic image which changed only three things — it did
  not touch the clock, gated GPIOA, GPIOB and GPIOC on beside UART1, and set PA7
  to output and PA8 to input explicitly — transmitted legibly at the first
  attempt, at a divider computed for 48 MHz.
- **Inference:** the application had been running all along and could not drive
  its pin. The GPIO port clock and the pad direction are required for a pin
  whose function PORTCON has assigned to the UART; selecting the function is not
  sufficient. Confidence high: one change, one image, one legible line.
- **Observation:** `AFIK-K5-1.1`, which folds those into the application,
  banners `AFIK-K5-1.1 booted clk=47796863 div=1245` on power-on. This is the
  first AFIK image observed running on UV-K5 V1 hardware.
- **Established by that line:** UART1 on PA7 is the programming connector's
  transmit pin on this board (`EVID-K5-019`); the bootloader leaves the part on
  RCHF and the image's own 48 MHz selection works; `RC_FREQ_DELTA` on this part
  reads 47,796,863 Hz, 203,137 Hz below nominal, a deviation of 0.42%; and the
  divider rule of `EVID-DP32-009` is correct, since 1245 produces a rate the
  host decodes without error.
- **Confidence:** high. The text was read directly off the wire and its content
  is generated by the image from registers it read.

### EVID-K5-021 — The V1 receive path breaks on back-to-back bytes

- **Observation:** `AFIK-K5-1.1` transmits but never answers a hello. A
  diagnostic image reports a received-byte counter, a sticky `UART_IF` word, and
  the first bytes received verbatim.
- **Observation:** with the host sending complete frames as single bursts, the
  radio received `ab cd 08 00 02 69 10 e6 44 a8 6e f7`. The first ten bytes are
  exactly what the host sent; the eleventh and twelfth are not — `5a 24` was
  sent — and the frame's remaining CRC and `dc ba` footer never arrived. Two
  frames, 28 bytes, produced 23 counted bytes.
- **Observation:** the same sixteen bytes sent one at a time with 20 ms gaps
  were received complete and uncorrupted: the counter advanced by exactly
  sixteen. Sent again as one burst with the host port held open afterwards, it
  advanced by fourteen.
- **Observation:** the sticky status latches `STOPE`, the stop-bit error. The
  manual states that a byte with a stop-bit or parity error is not written to
  the receive FIFO, which is how bytes can be lost without the overflow flag
  ever setting; no `RXFIFO_OVF` was observed.
- **Established:** the pin, the pad, the clock and the sampling rate are all
  correct, because bytes with gaps arrive perfectly. The fault appears only
  under back-to-back bytes, and it corrupts the tail of a burst rather than its
  head.
- **Not established:** the cause. The open candidates are the image's own
  transmission colliding with reception on a cable which may couple them, and a
  receive path which cannot sustain back-to-back bytes under this polled
  driver. The pinned V1 firmware receives by DMA with a receive timeout rather
  than by polling, which is consistent with either.
- **Permitted use:** none beyond recording. No AFIK V1 image may claim a working
  host exchange until a complete frame is observed arriving intact.
- **Required experiment:** an image which stays silent while it listens. A
  frame which arrives intact into a quiet receiver attributes the corruption to
  the image's own transmission; one which is still corrupted moves the question
  to the receive path and its DMA alternative.
- **Follow-up, 2026-08-12:** `AFIK-K5-1.2` was written again, with 240/240 pages
  acknowledged. A passive capture held across a normal power-cycle received no
  boot banner, and a read-only normal probe timed out. This establishes that
  `1.2` does not reach its observable banner on this attempt. It does not
  attribute the failure to its `UART_IF` acknowledgement, because `read_byte`
  is called only after the banner has been sent and flushed.

### EVID-DP32-010 — SPI0 subset for the V1 display witness

- **Fact:** SPI0 is based at `0x400B8000`. Its control register is at offset
  `0x00`, write-data at `0x04`, interrupt enable at `0x10`, and FIFO status at
  `0x18`. Control bit 3 enables SPI, bit 4 selects the second sampling edge,
  bit 5 selects idle-high clock, bit 6 selects master mode, bit 7 selects
  least-significant-bit first when set, bit 12 drives master SSN high when set,
  and bits 2:0 select pclk divisors from 4 through 512. FIFO-status bit 4 says
  transmit full and bit 3 says transmit empty. Both FIFOs are eight bytes deep.
- **Source:** DP32G030 reference manual section 5.17, printed pages 272 to 291;
  register map and fields on printed pages 286 to 290. The PDF is the same
  `d1923c0a...c62573` artifact recorded for `K5DRV-048`.
- **Method:** copied from the manual. The driver uses CPU writes only, disables
  every interrupt, and selects pclk/16, CPOL=1, CPHA=1, MSB first.
- **Confidence:** high for the MCU fields; physical display output remains the
  required experiment.
- **Permitted use:** a bounded SPI0 transmit-only display witness.

### EVID-K5-022 — V1 ST7565-compatible display bindings

- **Observation:** the pinned V1 firmware of `EVID-K5-019` binds SPI0 SSN to
  PB7, clock to PB8, MOSI to PB10, display A0 to GPIO PB9, and display reset to
  GPIO PB11. It uses a 128-by-64, eight-page ST7565-compatible command path.
- **Source:** pinned `machshev/uv-k5-firmware-custom` commit
  `7f959e8d09b435845753182b329e3a88490ebe32`, `board.c`, `driver/gpio.h`, and
  `driver/st7565.c`.
- **Method:** board wiring and controller command evidence only. No application
  or register-driver code is copied; DP32G030 register behavior comes solely
  from `EVID-DP32-010`.
- **Confidence:** high for this exact V1-generation board family because the
  pinned build runs on the three observed V1 units. Visible AFIK pixels on the
  exact UV-K6 remain the required physical confirmation.
- **Permitted use:** fixed receive-inert boot diagnostics. No keypad, RF, audio,
  storage, or transmit authority follows from display output.
- **First physical result:** the first display image was acknowledged 240/240
  pages and left DFU, but the operator observed no backlight and no obvious
  writing. Since that image deliberately did not configure the independently
  wired backlight, darkness is expected and does not by itself settle whether
  pixels were present.
- **GPIO-isolation result:** replacing only AFIK's SPI0 adapter with synchronous
  GPIO on the same evidenced pins produced an illuminated screen reading
  `AFIK`, `K5`, and `SERIAL` after a 240/240-page acknowledged write and normal
  power-cycle. This proves Reset, PB6 backlight, PB7/PB8/PB9/PB10/PB11 display
  bindings, controller reset/commands, rendering, UART configuration, and the
  application-facing `BootDisplay` path on the exact unit. It isolates the
  earlier blank panel to AFIK's SPI0 adapter.
- **Serial result beside the visible witness:** `probe-normal` still timed out
  while `SERIAL` remained visible. The application and its receive loop are
  running; the remaining defect is receive/frame completion, not startup.

### EVID-DP32-011 — PWM_PLUS0 subset for the V1 backlight

- **Fact:** PWM_PLUS0 is based at `0x400B4000`, with configuration at `0x00`,
  generation at `0x04`, clock source at `0x08`, period at `0x1C`, and channel
  zero compare at `0x20`. Configuration bit 0 enables counting and bit 2 repeats
  it. Generation bit 24 enables channel-zero output and bit 16 inverts it. The
  clock-source high halfword is a pclk predivider minus one; period and compare
  are sixteen-bit values.
- **Source:** DP32G030 reference manual section 5.14, printed pages 220 to 229,
  from the same verified PDF as `EVID-DP32-010`.
- **Observation:** the pinned V1 board source maps the backlight to PB6 and
  selects PWM_PLUS0 channel zero there. Its known-running build uses an inverted
  1 kHz waveform with period 1023.
- **Method:** MCU fields copied from the manual; only the PB6 binding and the
  board-proven fixed waveform are taken from the pinned V1 source.
- **Permitted use:** constant diagnostic illumination while the fixed boot
  witness is displayed. Brightness policy and power management remain deferred.
- **Physical result:** after a 240/240-page acknowledged write and normal
  power-cycle, the backlight illuminated but no text was visible. This proves
  Reset reached the board adapter through `enable_diagnostic_backlight`; it
  does not prove the subsequent SPI0/display path completed.

### EVID-DP32-012 — DMA channel zero for UART1 receive

- **Fact:** DMA is based at `0x40001000`, with global enable at `0x00` and
  channel zero at `0x100`. A channel control word holds enable at bit 0, the
  transfer count minus one in bits 12:1, circular operation at bit 13, and
  priority at bits 15:14. The mode word selects an incrementing SRAM destination
  with bit 8 and channel-zero UART1_RX source request 1 with source-select value
  `001` in bits 5:3. Source and destination addresses are at channel offsets
  `0x08` and `0x0C`; current transfer count is the low twelve bits of `0x10`.
- **Source:** DP32G030 reference manual section 5.24, printed pages 390 to 401,
  from the same verified PDF as the other DP32G030 driver evidence.
- **Observation:** the pinned V1 firmware uses this exact request, fixed UART1
  receive-data source, incrementing 256-byte SRAM destination, and circular
  channel-zero shape. It is corroboration of the manual mapping, not driver
  source for AFIK.
- **Permitted use:** interrupt-free circular UART1 receive into one statically
  owned 256-byte buffer. No other DMA channel or peripheral is authorised.
