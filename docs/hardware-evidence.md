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
- **Fail-closed decoder:** AFIK accepts only ready 24 MHz HSI, ready fixed x2
  PLL sourced from HSI, requested and active PLL SYSCLK, and undivided AHB/APB.
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
