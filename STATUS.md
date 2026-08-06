# Project status

## Current work package

**Work Package 23 (`K1ASYNC-023`) is active: establish a pinned Embassy/PY32
runtime foundation before migrating the proven K1 keypad, UART, and display
paths.**

K1 has priority because an exact unit running Armel firmware is available for
inspection. `K1EVID-013` supplies the K1 evidence baseline and same-unit
recovery proof for this bounded host-tool task. Trusted existing firmware
remains evidence, not production source: AFIK will not port, link, or
incrementally translate its application or driver implementation.

`FLASH-012` is deferred with its software milestone intact and physical gates
incomplete. It can resume unchanged when the exact K5 V1 hardware is available.
The bounded AFIK witness image was flashed through the intended CH340 path and
returned the exact normal-mode hello after power-cycle. Full radio application
features remain outside this bounded slice.

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
  baseline complete; board/MCU and full-application follow-up remains open.
- Work Package 14 K1/K5 auto-detected recovery flasher: complete.
- Work Package 15 first AFIK K1 recovery-flasher hardware run: complete; this
  was a stock recovery-image exercise, not an AFIK application flash.
- Work Package 16 K1 reset-only application image and static/raw-image gates:
  complete; no physical K1 write or boot claim.
- Work Package 17 K1 physical boot witness: complete; the intended CH340/UART
  path, source-backed USART1 contract, serial witness image, and separately
  guarded K1 AFIK writer are implemented and locally verified. The witness
  image returned `AFIK-K1-0.1` after power-cycle.
- Work Package 18 next K1 application slice: complete; selected the bounded
  display-only witness tracked by `K1DISP-019`.
- Work Package 19 K1 display-only witness: complete; the fixed words were
  physically visible under bright external light and `AFIK-K1-0.2` responded.
- Work Package 20 constant K1 backlight: complete; bounded PF8 implementation
  and static/physical verification passed.
- Work Package 21 fixed K1 contrast: complete; exact one-byte command change,
  clearer physical text, backlight, and serial verification passed.
- Work Package 22 receive-only K1 keypad/UI witness: active; static gates and
  serial fallback pass, but the first physical keypad-label observation failed.
- Work Package 23 Embassy/PY32 runtime foundation: active; dependency, chip,
  MSRV, executor, time-driver, UART, and SPI feasibility must be proven before
  replacing the current boot path.
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
- Work Package 18 selected and bounded the next application slice: a fixed
  display-only AFIK boot witness with the serial responder retained.
- Work Package 22 keypad/UI witness definition: complete; the pinned matrix,
  electrical idle/scan levels, one-key decode, explicit-time debounce, and
  display-only result were bounded before implementation.
- Current smallest actionable task: define a separate guarded inherited-clock
  publication boundary using the now-validated 48 MHz tuple; do not yet start
  TIM15, interrupts, DMA, USART1, SPI1, keypad, or display tasks.

## Work Package 23 dependency and executor milestone

- Pinned `embassy-executor = 0.10.0` behind the K1-only `embassy-runtime`
  feature with only `platform-cortex-m` and `executor-thread` enabled.
- Added a HAL-independent constructor for the heap-free thread executor. It
  touches no clock, interrupt, timer, UART, SPI, GPIO, linker, or image state.
- Rust 1.86 target check and warning-denied Clippy with build-std/core passed for
  `thumbv6m-none-eabi`; focused firmware tests, Nix flake evaluation, workspace
  formatting, warning-denied Clippy, all workspace tests, and `git diff --check`
  passed.
- `py32-metapac 0.5.0` contains `py32f071c1b`, `py32f071k18`, `py32f071k1b`,
  and `py32f071r1b`. `py32-hal` 0.3.0, 0.4.0, and 0.4.1 expose no F071 feature;
  0.4.1 exposes only F002B, selected F030s, and F072C1B. Selecting F072 for
  this unit is prohibited.
- A local review extension exposed all four F071 package features without USB
  or any default feature. Each package selects the same 59-line generated
  metadata fragment, whose complete inventory is only GPIOA, WWDG,
  AES_LPUART1, and DMA1_CH1. It contains no RCC, USART1, SPI1, GPIOB, GPIOF, or
  timer metadata.
- A Rust 1.86 `thumbv6m-none-eabi` build-std/core check of that extension for
  `py32f071r1b` reached `py32-hal` generation and failed at its required RCC
  lookup (`build.rs:410-415`). This blocks a truthful HAL surface; copying F072
  metadata or hand-inventing the missing inventory is prohibited.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 156 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No target entry point or physical image changed and no flash was sent.

## Work Package 23 local PY32F071 inventory milestone

- Vendored a locally generated `py32-metapac 0.5.0` from pinned `py32-data`
  commit `eb33b9ab85aa4652006e3435d84e1f9f7e5eca50`. Its explicit PY32F071
  series uses the maintained DIE072 inventory with CAN disabled; AFIK does not
  select an F072 chip feature.
- Vendored the crates.io `py32-hal 0.4.1` source and added only bounded local
  compatibility changes: four concrete F071 features, regenerated PAC feature
  naming, safe generic-chip cfg parsing, and suppression of nonexistent DAC
  bindings. F071 ADC HAL bindings remain disabled because their constants have
  not been independently evidenced.
- Added the optional `py32f071-hal-inventory` compile contract. It names RCC,
  USART1, SPI1, TIM1/TIM3/TIM15, the observed USART/display/keypad pins, and
  PF8 without calling HAL initialization or changing the firmware entry point.
- All four concrete F071 package features compile separately for
  `thumbv6m-none-eabi` with default HAL features disabled. The R1B selection in
  AFIK's compile contract follows the available primary product-page package;
  it is not an observation of the exact fitted package suffix.
- `nix develop path:. -c tool/check-py32f071-hal.sh` — passed for C1B, K18,
  K1B, and R1B with offline dependency resolution and build-std/core.
- Warning-denied `thumbv6m-none-eabi` Clippy with build-std/core for
  `radio-firmware-k1 --features py32f071-hal-inventory --lib` — passed.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed after applying
  rustfmt to the new inventory contract.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 156 workspace
  unit, integration, and doc-test binaries passed.
- No physical image, linker contract, startup path, MMIO behavior, or flash
  operation changed.

## Work Package 23 TIM15 time-driver milestone

- Selected TIM15 as the compile-only Embassy time candidate. The pinned F071
  metadata gives it `PCLK1_TIM`, RCC enable/reset fields, a dedicated TIM15
  interrupt, and CH1/CH2 compare surfaces; the vendored HAL driver reserves
  CC1 for rollover accounting and uses CC2 for its single alarm.
- Added optional `py32f071-time-driver`, enabling only the existing F071
  inventory, HAL runtime interrupt vectors, and `time-driver-tim15`. It is not
  selected by the firmware feature or entry point.
- Embassy time defaults to 1 MHz when no tick-rate feature is selected. If a
  later evidenced HAL startup reports the observed bootloader-provided 48 MHz
  `PCLK1_TIM`, the driver computes prescaler 47 exactly. This milestone neither
  initializes nor adopts that clock.
- `nix develop path:. -c tool/check-py32f071-time-driver.sh` — passed; strict
  warning-denied target Clippy compiled the F071R1B TIM15 driver, interrupt
  binding, and build-std/core offline for `thumbv6m-none-eabi`.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 159 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No HAL initialization, target entry point, physical image, timing behavior,
  or flash operation changed.

## Work Package 23 USART1 async-driver milestone

- Reviewed the pinned F071 generated metadata against the physically evidenced
  K1 application serial path. USART1 uses `PCLK1`, RCC enable/reset fields, its
  dedicated interrupt 27, PA9 TX AF1, and PA10 RX AF1.
- Added optional `py32f071-usart1`, which compiles a real async `Uart`
  constructor at 38,400 baud using bounded DMA1 channels and the generated
  USART1 interrupt binding. The feature is not selected by the firmware image
  and the constructor is not called by its entry point.
- `nix develop path:. -c tool/check-py32f071-usart1.sh` — passed; strict
  warning-denied target Clippy compiled the F071R1B USART1 async constructor and
  build-std/core offline for `thumbv6m-none-eabi`.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 159 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No HAL initialization, target entry point, physical image, clock behavior,
  interrupt/DMA operation, or flash operation changed.

## Work Package 23 SPI1 feasibility milestone

- Reviewed the generated F071 inventory against the physically proven display
  path. SPI1 uses `PCLK1`, has `APBENR2.SPI1EN` and `APBRSTR2.SPI1RST`, and
  exposes PA5 SCK AF0 plus PA7 MOSI AF0.
- The vendored `py32-hal 0.4.1` contains no SPI module or driver. Its support
  table marks SPI unimplemented for every family and its TODO list explicitly
  includes SPI, so no Embassy-compatible constructor exists to compile.
- No code, HAL initialization, target entry point, physical image, clock/SPI
  behavior, or flash operation changed. The next driver step must be explicitly
  bounded rather than treating generated PAC metadata as HAL support.

## Work Package 23 async SPI1 interface milestone

- Added a bounded local `py32-hal` SPI surface: generated SCK/MOSI pin traits
  and a transmit-only `SpiTx` owning the peripheral and pins.
- The constructor configures only the evidenced display contract: SPI master,
  mode 3, MSB first, software NSS, one-line transmit, divide-by-64, PA5 SCK AF0,
  and PA7 MOSI AF0.
- Async writes use no heap or DMA. Hardware status polling has a finite limit,
  reports mode-fault, overrun, CRC, and timeout errors, and yields to Embassy
  every 16 transferred bytes or unsuccessful polls.
- Added optional `py32f071-spi1` and a no-entry-point K1 constructor plus
  `tool/check-py32f071-spi1.sh` for offline warning-denied target compilation.
- Updated the vendored dependency delta; no upstream or trusted-firmware driver
  implementation was copied.
- `nix develop path:. -c tool/check-py32f071-spi1.sh` — passed.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 159 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No HAL initialization, firmware entry point, physical image, physical SPI,
  scheduler, UART-coexistence, or flash operation changed.

## Work Package 23 cooperative-progress milestone

- Added a deterministic no-hardware round-robin future harness for one complete
  1,024-byte display frame and a concurrent serial-service future.
- The display future yields after each 16-byte chunk: exactly 64 chunks complete,
  with serial work observed between every adjacent pair.
- The hardware-independent chunk constant is compile-time checked against the
  local HAL SPI driver's constant, preventing the proof and driver from drifting.
- `nix develop path:. -c cargo test -p radio-firmware-k1` — passed; all 23 unit
  tests and doc-tests passed.
- `nix develop path:. -c tool/check-py32f071-spi1.sh` — passed with strict
  warning-denied target Clippy and build-std/core offline.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 160 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No executor was started on Cortex-M, no HAL initialization or entry point
  changed, and no physical image, interrupt, DMA, SPI, UART, keypad, or flash
  behavior changed.

## Work Package 23 runtime-composition milestone

- Added optional `py32f071-runtime-composition`, one compile-only owned bundle
  for the heap-free thread executor, async USART1/PA9/PA10 with DMA1 channels
  1/2, and cooperative SPI1/PA5/PA7.
- The constructor requires explicit caller-supplied HAL peripheral tokens. It
  does not call HAL initialization, select clocks, reserve TIM15, create tasks,
  or own display A0/CS or keypad GPIO.
- Added `tool/check-py32f071-runtime-composition.sh` for offline warning-denied
  Rust 1.86 `thumbv6m-none-eabi` compilation with build-std/core.
- `nix develop path:. -c tool/check-py32f071-runtime-composition.sh` — passed.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 160 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No firmware entry point, linker contract, physical image, interrupt delivery,
  DMA operation, peripheral behavior, keypad behavior, or flash path changed.

## Work Package 23 clock-handoff diagnostic milestone

- Re-read the pinned K1 application source. It records only the resulting
  48 MHz `SystemCoreClock` value and does not expose the bootloader's inherited
  RCC oscillator, PLL, or prescaler fields.
- Added a hardware-independent fail-closed clock snapshot contract. It accepts
  only ready 24 MHz HSI, ready fixed x2 HSI PLL, requested and active PLL
  SYSCLK, and undivided AHB/APB; every mismatch denies the handoff.
- Added optional `py32f071-clock-handoff`, which reads only CR, ICSCR, CFGR, and
  PLLCFGR through the generated PAC. It neither writes RCC nor publishes HAL
  clocks and remains absent from the firmware entry point.
- Focused host tests passed: 25 tests, including exact acceptance and rejection
  coverage for every clock-contract field.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed; strict
  warning-denied target Clippy compiled the snapshot with the owned runtime
  bundle and build-std/core.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 162 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- No physical image, clock state, peripheral token, interrupt, DMA, TIM15,
  USART1, SPI1, keypad, display, or flash behavior changed.

## Work Package 23 exact-unit observation-surface milestone

- Added normal-mode request `0x7f12` and response `0x7f13`. The target performs
  exactly four volatile RCC reads and returns CR, ICSCR, CFGR, PLLCFGR, and the
  fail-closed contract result in a fixed CRC-protected frame.
- Added host `probe_clock_snapshot` and `afik-flasher probe-clock`; both reject
  malformed lengths, non-boolean validity, and nonzero reserved fields.
- Focused K1 firmware, flasher, and CLI tests passed. The regenerated image
  passed ELF, raw package, negative-fixture, and existing keypad Renode gates.
- The raw image is 64,384 bytes, SHA-256
  `c64ffa09da427060fadbc2527713826c3f6db4d70c3639b476fdcf64c64eebd3`,
  CRC-32 `0ed8ed53`, Reset `0x08002939`, and ELF end `0x08012380`.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 165 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- The exact K1 bootloader `7.03.01` was identified through the CH340 path. The
  guarded write acknowledged all 252 pages under transaction `736f8852` without
  retry and reported `acknowledged_not_read_back`.
- The manual power-cycle and normal-mode hello completed. A valid clock capture
  remains pending. No physical clock observation, clock adoption, TIM15,
  interrupt, DMA, async peripheral, RF, or TX operation occurred.
- **Post-power-cycle result:** normal hello returned `AFIK-K1-0.2`. Two
  `probe-clock` attempts timed out, and a normal hello between them still
  returned `AFIK-K1-0.2`. No RCC value was accepted or inferred. The next
  diagnostic must isolate individual reads and response progress before another
  guarded image write.
- **Isolation implementation:** four strict request/response pairs now read one
  named RCC register each. Host code can probe them independently, so a timeout
  cannot erase earlier register evidence. Focused protocol/flasher/CLI tests,
  ELF/raw package gates, negative fixtures, and keypad Renode passed.
- The isolation image is 65,656 bytes, SHA-256
  `d319d961a93cad6d219a4d21b7a60a2d7337ea989ff4aff2b6e9e92c2f51c955`,
  CRC-32 `a895d521`, Reset `0x08002939`, and ELF end `0x08012878`.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 167 workspace
  unit, integration, and doc-test binaries passed.
- `git diff --check` — passed.
- K1 bootloader `7.03.01` acknowledged all 257 isolation-image pages under
  transaction `7d527b6f` without retry and reported
  `acknowledged_not_read_back`. Normal boot and register reads remain pending.
- **Isolation physical result:** after power-cycle, normal hello passed. The
  first isolated CR request timed out before any later register was requested;
  a following normal hello passed again. No RCC value is observed. The next
  diagnostic must return a constant marker through the same path without MMIO.

## Work Package 23 serial-only diagnostic milestone

- Removed display, keypad, backlight, debounce, matrix scanning, SPI1, GPIOB,
  and GPIOF from the runnable entry point. Reset now initializes only the RAM
  witness and polling GPIOA/USART1 path.
- Added exact no-MMIO control request `0x7f1c`, response `0x7f1d`, and marker
  `0x4b31434c`, with strict firmware, host-library, and CLI tests.
- The raw image is 51,340 bytes, SHA-256
  `ce97df6718d6ff2b9bee88ca8443ef15a63ea2484231b265501eef7739803585`,
  CRC-32 `b8731d25`, Reset `0x08002905`, and ELF end `0x0800f08c`.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 169 unit and
  integration tests plus doc tests passed.
- `nix develop path:. -c tool/build-k1.sh` — passed.
- `nix develop path:. -c tool/package-k1-image.sh --force` — passed.
- `nix develop path:. -c tool/test-k1-image.sh` — passed, including negative
  truncated and oversized fixtures.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed.
- `git diff --check` — passed.
- The keypad Renode scenario is intentionally inapplicable to this image. No
  application response is claimed at this checkpoint.
- K1 bootloader `7.03.01` acknowledged all 201 serial-only-image pages under
  transaction `8a6af71f` without retry and reported
  `acknowledged_not_read_back`.
- After power-cycle, hello returned `AFIK-K1-0.2`, the no-MMIO marker returned
  `4b31434c`, and all isolated plus combined RCC reads returned CR `03000500`,
  ICSCR `00e64d14`, CFGR `00000012`, and PLLCFGR `00000006`.
- The provisional contract rejected the stable snapshot because it assumed
  24 MHz x2 and masked the two-bit PLL source to one bit. The pinned F071 DIE072
  inventory resolves the observed fields as 16 MHz HSI, HSI source, and x3.
  The corrected fail-closed contract accepts this exact 48 MHz tuple and now
  validates the multiplier explicitly. HAL clocks remain unpublished.

## Work Package 23 exact-unit clock-field resolution milestone

- The pinned F071 PAC uses the maintained DIE072 RCC inventory: `HSI_FS=2` is
  16 MHz, `PLLSRC=2` is HSI, and `PLLMUL=1` is x3. Observed PLLCFGR
  `0x00000006` therefore completes the inherited 48 MHz clock equation.
- Corrected the pure decoder to preserve both PLL-source bits and the multiplier
  field. Validation accepts only ready 16 MHz HSI, ready HSI x3 PLL,
  requested/active PLL SYSCLK, and undivided AHB/APB.
- Added direct regression coverage for the exact physical CR/ICSCR/CFGR/
  PLLCFGR tuple and independent rejection coverage for the multiplier.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 170 unit and
  integration tests plus doc tests passed.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed.
- `git diff --check` — passed.
- No target entry point, physical image, RCC state, HAL clock publication,
  interrupt, DMA, timer, serial, display, keypad, RF, TX, or flash behavior
  changed.

## Work Package 23 guarded clock-publication milestone

- Made the validated inherited-clock proof unforgeable outside its pure
  fail-closed validator and added frequency accessors.
- Added optional `py32f071-clock-publication`: it re-reads and validates the
  live RCC tuple before publishing only the exact 48 MHz SYS/HCLK1/PCLK1/
  PCLK1_TIM values, 16 MHz HSI, and 48 MHz PLL to the HAL software clock table.
- The local HAL primitive is explicitly unsafe; the K1 wrapper is safe because
  it owns validation and documents the one-time startup ordering contract.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c tool/check-py32f071-clock-handoff.sh` — passed.
- `nix develop path:. -c cargo test -p radio-firmware-k1` — passed; all 30 unit
  tests and doc tests passed.
- `git diff --check` — passed.
- No entry point, physical image, RCC register, peripheral ownership, TIM15,
  interrupt, DMA, UART, SPI, display, keypad, RF, TX, or flash behavior changed.

## Work Package 23 inherited-runtime initialization milestone

- Added an unsafe local HAL initializer for an already published clock tree. It
  takes singleton tokens and initializes GPIO, DMA, FLASH, and the configured
  TIM15 time driver without calling the RCC clock configurator.
- Added a safe K1 wrapper which first validates and publishes the live clock
  snapshot, then calls that initializer exactly once and returns the owned
  peripheral tokens plus validated frequencies.
- Added `tool/check-py32f071-runtime-init.sh` for warning-denied Rust 1.86
  `thumbv6m-none-eabi` compilation with build-std/core.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c tool/check-py32f071-runtime-init.sh` — passed.
- `git diff --check` — passed.
- No entry point, physical image, executor task, RCC clock field, UART/SPI pin,
  keypad/display behavior, RF, TX, or flash path changed.

## Work Package 22 pure keypad milestone

- Added an allocation-free main-matrix decoder in the standalone K1 firmware
  crate. It maps exactly the 16 evidenced PB6..PB3 by PB15..PB12 cells, returns
  release for no active cell, and rejects multiple cells or row bits outside
  the evidenced mask.
- Added an explicit monotonic-millisecond debounce machine with a bounded 20 ms
  stability interval. Bounce restarts the candidate interval; ambiguity, scan
  failure, and time reversal reset immediately to no held key without emitting
  an application edge.
- Exact fixed labels are bounded to four ASCII bytes. PTT, side keys, GPIO
  registers, display mutation, target code, RF/TX, persistence, and flashing
  are absent from this milestone.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo test --package radio-firmware-k1` — passed; 14
  unit tests plus doc tests, including exhaustive 16-cell mapping and all 120
  two-cell ambiguity combinations.
- `nix develop path:. -c cargo clippy --package radio-firmware-k1 --all-targets
  -- -D warnings` — passed.
- `git diff --check` — passed.

## Work Package 22 pure GPIO scan-plan milestone

- Added an exact GPIOB configuration plan for only PB12..PB15 pull-up inputs
  and PB3..PB6 push-pull outputs, including the pinned high-speed/pull-up fields
  and PB6-to-PB3 selected-low order.
- Added a deterministic four-column scan boundary. It begins with all columns
  high, selects and reads each column in order, restores all columns high after
  every read, and attempts the same cleanup after every select/read failure.
  The bus owns settling, so the pure contract invents no target tick or delay.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo test --package radio-firmware-k1` — passed; 18
  unit tests plus doc tests, including exact register masks/scan trace and every
  select/read cleanup-failure position.
- `nix develop path:. -c cargo clippy --package radio-firmware-k1 --all-targets
  -- -D warnings` — passed.
- `git diff --check` — passed.

## Work Package 22 target implementation milestone

- The K1 target now binds only the verified GPIOB PB12..PB15 row inputs and
  PB3..PB6 column outputs. All columns are driven high before configuration and
  after every selected-low read. The active-low IDR bits are reordered into the
  pure PB15-to-PB12 contract.
- The pinned 10 us per-column settling observation is implemented as a bounded
  spin at the existing evidenced 48 MHz handoff. The application loop provides
  a conservative minimum 1 ms elapsed-time step, services a waiting serial
  hello without blocking keypad idle scans, and redraws only after a debounced
  press. Invalid/ambiguous scans cannot update the display.
- All 16 key labels render as distinct bounded frames beneath the unchanged
  `AFIK` / `K1 0.2` witness. PTT, side keys, interrupts, EEPROM, general menus,
  BK4819, RF, and TX remain absent.
- The raw image is 56,828 bytes, SHA-256
  `4ad5e4e205afd32e791409b371e111c0792110c48e1fc9c67a5c19d8628c06b0`,
  and CRC-32 `a17da806`.
- `nix flake check path:. --no-build` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all 156 workspace
  unit, integration, and doc-test binaries passed.
- Embedded warning-denied Clippy for `radio-firmware-k1` on
  `thumbv6m-none-eabi` with pinned build-std/linker flags — passed.
- `nix develop path:. -c tool/build-k1.sh` and
  `tool/verify-k1-image.sh` — passed.
- `nix develop path:. -c tool/package-k1-image.sh --force` and
  `tool/test-k1-image.sh` — passed, including positive and negative raw-image
  fixtures.
- `git diff --check` — passed. No keypad image write has yet been sent.

Physical keypad write on 2026-08-06:

- The immediate read-only identify reported
  `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`, K1 bootloader `7.03.01`.
- The guarded writer revalidated the 56,828-byte image, recovery image, retained
  EEPROM backup (`backup_crc32=99765400`), exact target/rehearsal phrases, and
  image CRC-32 `a17da806` before sending any page.
- K1 `7.03.01` acknowledged all `222/222` pages in transaction `265b2c89` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent.
- Physical completion remains pending a user power-cycle, individual
  observation of all 16 main-key labels, retained display/backlight behavior,
  and the read-only `AFIK-K1-0.2` serial probe.

Physical keypad observation after power-cycle on 2026-08-06:

- The user reported that the requested labels did not display. No main-key
  mapping, GPIO scan, physical debounce, or key-triggered redraw is therefore
  claimed from this image.
- `nix develop path:. -c cargo run --quiet --package radio-flasher-cli --bin
  afik-flasher -- --device auto probe-normal` — passed; device
  `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`, 38,400 baud,
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.2`.
- The passing serial probe proves the application remained alive; it does not
  localize the failure between GPIO configuration, scan sampling, debounce, or
  redraw. `K1KEY-022` remains active and no retry has been sent.
- Code review found that the first image placed dynamic labels at `y=50` on
  controller pages 6–7, outside the fixed text line physically verified by the
  prior witness. The next correction replaces `K1 0.2` at the already observed
  `y=36` line and adds a regression that pages 6–7 remain unused. GPIO and
  debounce behavior are unchanged; another write requires all gates first.

Verified-line correction milestone on 2026-08-06:

- Key labels now replace `K1 0.2` at the physically observed `y=36` line;
  `AFIK` remains at `y=20`, and controller pages 6–7 remain empty. The GPIO
  scan, decoder, debounce, backlight, and serial implementation are unchanged.
- Focused K1 tests passed: 20 unit tests plus doc tests, including distinct
  frames for all 16 labels and the verified-page regression. Workspace Clippy
  with `-D warnings` passed.
- Embedded warning-denied Clippy, target build, ELF verification, packaging,
  positive/negative raw-image tests, and `git diff --check` passed.
- The corrected raw image is 56,856 bytes, SHA-256
  `417663dab22de56fbfe167049c3b1b5831e588c04db4eec9ac7ec16b5cf9130a`,
  and CRC-32 `f4a9c1d6`. No corrected image write has yet been sent.

Corrected keypad write on 2026-08-06:

- A fresh read-only identify again reported K1 bootloader `7.03.01` on the
  external CH340 path.
- The guarded writer revalidated the corrected image, recovery image, EEPROM
  backup (`backup_crc32=99765400`), exact target/rehearsal phrases, and CRC-32
  `f4a9c1d6` before sending any page.
- K1 `7.03.01` acknowledged all `223/223` pages in transaction `fe6396d0` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent.
- Physical completion remains pending a normal power-cycle and key-label
  observation, beginning with `MENU` replacing `K1 0.2` on the verified line.

## Work Package 22 Renode diagnostic activation

- The existing Renode platform is DP32-only and cannot execute the K1 image or
  its peripheral loop.
- The bounded K1 diagnostic will model only Cortex-M0+-compatible execution,
  evidenced flash/RAM ranges, and test-only RCC/GPIO/SPI/USART register storage.
  A synthetic MENU cell will be visible only while PB6 is selected low, and a
  CPU hook on the ELF's `render_key_witness` symbol will prove whether the
  production scan/debounce path reaches redraw.
- This harness may measure simulated instruction progress and MMIO traces. It
  cannot prove PY32 timing, GPIO electrical behavior, LCD behavior, or physical
  key operation, and it will not be used as authority for RF/TX.

## Work Package 22 Renode diagnostic result

- Added a K1 simulation-only Cortex-M execution platform with evidenced
  flash/RAM bounds and bounded test register storage for only the RCC, GPIO,
  SPI, and USART addresses touched by this witness. GPIOB offset `0x100` is an
  explicit test convention which injects MENU only while PB6 is selected low;
  it is not represented as a PY32 register.
- The launcher resolves Reset, `keypad_init`, and `render_key_witness` directly
  from the built ELF, starts at the K1 application entry, and uses CPU hooks
  without changing production firmware.
- `nix develop path:. -c tool/test-k1-renode.sh --repeat 3` — passed all three
  iterations. Each run proved initial display setup returned, then synthetic
  PB6/PB15 MENU traversed the compiled scan/debounce path and reached
  `render_key_witness`.
- This narrows the unresolved physical failure to behavior the bounded model
  cannot validate: actual GPIO levels/timing or the subsequent physical display
  transfer. The next smallest diagnostic is a read-only serial report of raw
  per-column row masks while one main key is held.

## Work Package 22 raw-matrix diagnostic milestone

- Added a read-only `probe-keypad` request to the retained serial session. The
  target performs one existing four-column scan and returns only the four raw
  four-bit row masks plus an explicit scan-valid flag; it does not decode a
  key, mutate display state, access side keys/PTT, or reach RF/TX.
- The flasher library validates the exact response command, frame size, CRC,
  reserved bytes, row-mask bounds, and boolean status before exposing a bounded
  report. The thin CLI prints stable `pb6_rows` through `pb3_rows` fields.
- Focused firmware/flasher/CLI tests passed (21, 22, and 8 unit tests plus two
  CLI binary tests). Three repeated K1 Renode diagnostic runs passed.
- `nix flake check path:. --no-build`, workspace formatting, warning-denied
  workspace Clippy, all workspace tests, embedded warning-denied Clippy, target
  build, ELF verification, packaging, positive/negative raw-image tests, and
  `git diff --check` passed.
- The diagnostic raw image is 57,860 bytes, SHA-256
  `c56f5a8d883cf240d4a70626a299ab0cc8a1cf2bba294cffb3e6308ec4426ba9`,
  and CRC-32 `0a53af07`.

Diagnostic keypad-probe write on 2026-08-06:

- A fresh read-only identify reported K1 bootloader `7.03.01` on
  `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`.
- The guarded writer revalidated the diagnostic image, recovery image, retained
  EEPROM backup (`backup_crc32=99765400`), exact target/rehearsal phrases, and
  image CRC-32 `0a53af07` before sending any page.
- K1 `7.03.01` acknowledged all `227/227` pages in transaction `0e4f6fc9` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent.
- Physical completion remains pending a normal power-cycle followed by released
  and held-MENU `probe-keypad` observations.

First raw-matrix physical observation on 2026-08-06:

- With every key released, `probe-keypad` returned `scan_valid=true` and zero
  for all four row masks.
- With MENU held, two independently launched read-only probes timed out; the
  second used the already-built CLI and removed Nix/Cargo startup latency.
  Serial response returned after MENU was released, again with a valid all-zero
  scan.
- The press enters the key-triggered path before the serial request can be
  serviced. The next bounded image retains scan/debounce and pure frame render
  execution but suppresses the subsequent synchronous SPI frame transfer. This
  is diagnostic isolation, not a display fix or an async-runtime decision.
- The SPI-suppressed raw image is 57,852 bytes, SHA-256
  `c50baea15ebcf11805e7fff670cc4e0734c5ad1d52e09512acdb58c68c6e7fb9`,
  and CRC-32 `0b98c076`.
- Focused tests and warning-denied Clippy passed. The first embedded Clippy
  invocation omitted the required build-std environment and failed before
  source compilation because `core` was unavailable; the recorded pinned
  build-std/linker invocation then passed.
- Embedded build, ELF/package positive and negative checks, three repeated
  Renode runs, Nix flake evaluation, workspace formatting, warning-denied
  workspace Clippy, all workspace tests, and `git diff --check` passed. No
  source or artifact gate failed.

SPI-suppressed diagnostic write on 2026-08-06:

- A fresh read-only identify reported K1 bootloader `7.03.01` on the expected
  external CH340 path.
- The guarded writer revalidated the 57,852-byte image, recovery image, retained
  EEPROM backup (`backup_crc32=99765400`), exact guards, and CRC-32 `0b98c076`.
- K1 `7.03.01` acknowledged all `226/226` pages in transaction `1a79dec2` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent.
- Normal boot and released/held-MENU raw observations remain pending.

SPI-suppressed physical observation on 2026-08-06:

- After normal boot, the released probe again returned `scan_valid=true` with
  all four masks zero.
- Holding MENU still caused the prebuilt host probe to time out. Suppressing the
  key-triggered SPI frame transfer therefore did not remove the failure, so the
  LCD write is excluded as its cause.
- No held row mask is established. The next smallest diagnostic is to pre-arm
  serial capture while released, latch the first nonzero scan, wait for release,
  and only then transmit it; this tests whether the held circuit temporarily
  prevents execution or UART response without introducing async/interrupts.

Latched raw-matrix diagnostic milestone:

- The target now retains only the latest nonzero four-mask scan in bounded RAM.
  A later released-key probe returns it with `captured=true`, then clears it.
  This adds no waiting loop, timing assumption, interrupt, persistence, or
  display/RF/TX behavior.
- Firmware/flasher/CLI focused tests, workspace warning-denied Clippy and tests,
  embedded warning-denied Clippy/build, ELF/package checks, three repeated
  Renode runs, formatting, and `git diff --check` passed.
- The latched diagnostic image is 58,380 bytes, SHA-256
  `eba38cc718a3de0e220bc28c4de657849960ea1d7098085df94c802cf903a328`,
  and CRC-32 `823616ad`.
- K1 `7.03.01` acknowledged all `229/229` pages in transaction `20d50457` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent;
  normal boot and tap-then-probe observation remain pending.

Latched raw-matrix physical observation on 2026-08-06:

- After a normal boot and MENU tap/release, `probe-keypad` returned
  `scan_valid=true`, `captured=false`, and four zero masks. No nonzero PB12..PB15
  scan was retained.
- The user observed no boot-screen restart on the tap and reports that initial
  display appearance takes about 15 seconds after power-on. Because serial
  response resumed promptly after release rather than after that full startup
  interval, an application reset is less likely, though not independently
  excluded.
- No MENU mapping is physically established. The next bounded diagnostic must
  capture raw GPIOB observations/configuration rather than decoded row masks so
  an alternate exact-unit routing can be observed instead of invented.

Raw GPIOB snapshot diagnostic milestone:

- The target records the exact low 16 bits of GPIOB IDR for each PB6-to-PB3
  selection. The first observation is a released baseline; later changes
  outside the scanner-owned PB3..PB6 bits are latched once and reported after
  release with explicit validity/capture flags.
- Focused tests include baseline behavior, exclusion of column-output changes,
  one-shot capture, and strict 16-bit wire decoding. Workspace and embedded
  warning-denied Clippy, all workspace tests, target build, ELF/package checks,
  and three repeated Renode runs passed. The initial embedded gate found and
  corrected two target-only Clippy findings before packaging.
- The raw GPIOB image is 61,128 bytes, SHA-256
  `25f900885cf0a4ca79c10ea16737c72878330e8d0e372eb74cde63c479b28f32`,
  and CRC-32 `032e7309`.
- K1 `7.03.01` acknowledged all `239/239` pages in transaction `7422b31d` and
  reported `acknowledged_not_read_back`. No retry or reset command was sent;
  normal-boot baseline and tap capture remain pending.

Raw GPIOB MENU observation on 2026-08-06:

- Released baseline was PB6 `f43c`, PB5 `f45c`, PB4 `f46c`, PB3 `f474`.
- The first immediate post-tap probe timed out. One read-only retry then returned
  `captured=true`: PB6 `743c`, with PB5/PB4/PB3 unchanged.
- The exact difference is GPIOB bit 15 changing high-to-low only while PB6 is
  selected. This physically establishes the pinned PB6/PB15 MENU matrix cell
  and the AFIK raw scan path on this unit.
- The several-second response gap after a press localizes the remaining problem
  above GPIO sampling, in press-path execution/display work. The next smallest
  task is to measure and bound key frame rendering and SPI transfer separately.

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
- After the user re-entered bootloader mode, the recovery command was invoked
  once more with transaction `4cf88e71`; it again stopped with `serial response
  timed out` before any page acknowledgement. No retry was made. The bounded
  Linux serial read timer was then increased from 0.1 s to 0.2 s per read
  (4 s across the existing empty-read budget) in commit `72ba9f7`, matching the
  observed 3 s K1 beacon wait used by the recovery evidence procedure. A fresh
  bootloader session is required before testing that fix.
- In the next fresh bootloader session, the same guarded AFIK recovery command
  generated transaction `074b2081` and acknowledged all 375/375 sequential
  pages for the unchanged private `F4HWN v5.5.0` image. The reported result was
  `status=acknowledged_not_read_back`; no retry or reset command was sent. The
  next action is a user power-cycle into normal Fusion mode followed by a
  complete read-only identity and 8 KiB backup comparison.
- After that power-cycle, AFIK's read-only backup workflow identified
  `F4HWN v5.5.0`, received 8,192 bytes, and produced a mode-`0600` output that
  matched both retained pre-flash backups byte-for-byte. This closes
  `K1HIL-015` for the host recovery path. It does not establish a K1 AFIK
  application image or boot witness.

K1HIL-015 completion verification on 2026-08-06:

- `nix develop path:. -c cargo run --quiet --package radio-flasher-cli --bin
  afik-flasher -- --device auto backup-eeprom
  /tmp/afik-k1-post-afik-recovery.raw` — passed; normal identity `F4HWN
  v5.5.0`, 8,192 bytes.
- `cmp -s /tmp/afik-k1-post-afik-recovery.raw
  .private/k1/unit-backup.primary.raw` and the equivalent comparison with
  `unit-backup.secondary.raw` — passed; both were byte-identical.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all workspace unit,
  integration, and doc tests passed.
- `git diff --check` — passed.

## Work Package 16 K1 reset image milestone

- Added standalone crate `radio-firmware-k1` with no host or hardware-driver
  dependencies. Its linker contract is `0x08002800..0x08020000` application
  flash and `0x20000000..0x20004000` SRAM, with initial SP `0x20004000`.
- The two-word application vector table is at `0x08002800`; the verified Reset
  vector is `0x08002821`. Reset writes only the development RAM witness
  `0x4B31_B007` at `0x20000000` and then spins.
- The generated raw image is 616 bytes with SHA-256
  `877e2018ef4dd0e985dd16447d7120f61d60ff77259b149b3ad0ab6d37b95021`.
- No clock, USB, display, keypad, GPIO, external flash, BK4819, audio, RF, TX,
  reset, or bootloader behavior is implemented. This image has not been
  flashed and does not establish physical K1 application boot.

K1BOOT-016 verification on 2026-08-06:

- `nix develop path:. -c tool/build-k1.sh` — passed.
- `nix develop path:. -c tool/verify-k1-image.sh` — passed; origin
  `0x08002800`, initial SP `0x20004000`, Reset `0x08002821`, image end
  `0x08002a68`.
- `nix develop path:. -c tool/package-k1-image.sh --force` — passed; 616-byte
  raw image generated with the SHA-256 recorded above.
- `nix develop path:. -c tool/test-k1-image.sh` — passed; positive package
  comparison plus truncated, oversized, and non-Thumb negative fixtures.
- `nix develop path:. -c bash -c 'set -euo pipefail; export
  RUSTC_BOOTSTRAP=1; export __CARGO_TESTS_ONLY_SRC_ROOT="$RUST_SRC_PATH";
  export CARGO_TARGET_THUMBV6M_NONE_EABI_LINKER="$DP32_LLD"; export
  RUSTFLAGS="-C link-arg=-Tcrates/radio-firmware-k1/link.x -C
  link-arg=-z -C link-arg=max-page-size=4 -C panic=abort"; cargo clippy
  -Z build-std=core --package radio-firmware-k1 --features firmware --bin
  radio-firmware-k1 --target thumbv6m-none-eabi -- -D warnings'` — passed.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D
  warnings` — passed.
- `nix develop path:. -c cargo test --workspace` — passed; all workspace unit,
  integration, and doc tests passed.
- `git diff --check` — passed.

## Work Package 17 physical witness evidence

- The official PY32F071 evidence establishes a USB 2.0 full-speed MCU
  peripheral, but the exact K1 package, board routing, connector, and
  host-visible USB identity remain unobserved.
- Read-only host USB/sysfs inventory on 2026-08-06 found only the external
  QinHeng CH340 serial converter `1a86:7523`, exposed as
  `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0` and `/dev/ttyUSB0`.
  No native K1 USB device was present.
- `nix develop path:. -c cargo run --quiet --package radio-flasher-cli --bin
  afik-flasher -- --device auto identify` — passed; protocol family `K1`,
  bootloader `7.03.01`, hardware identity `not_proven_by_beacon`.
- The pinned board source records USART1 on PA9/PA10 AF1 at 38,400 baud and a
  bootloader-provided 48 MHz clock. This is recorded as `EVID-K1-024`; AFIK's
  implementation is independent and uses no copied driver source.
- The guarded K1 AFIK command sent the 44,008-byte witness image over the CH340
  path. It acknowledged all 172 pages, with transaction `db2b80ec`, and sent no
  reset, EEPROM, or RF command. The report was
  `acknowledged_not_read_back`.
- After a user power-cycle, the read-only normal-mode probe passed:
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.1`.
- This proves the bounded AFIK Reset/USART1 serial witness on the exact unit;
  it does not prove display, keypad, RF, TX, EEPROM, or a complete radio app.

Serial witness implementation on 2026-08-06:

- `nix develop path:. -c cargo test --package radio-firmware-k1 --package
  radio-flasher` — passed; pure K1 framing and host hello-probe tests passed.
- `nix develop path:. -c tool/build-k1.sh` — passed; target image rebuilt with
  independent USART1 MMIO and bounded hello responder.
- `nix develop path:. -c tool/verify-k1-image.sh` — passed; application origin,
  vector range, SRAM sentinel, and image end were checked.
- `nix develop path:. -c tool/package-k1-image.sh --force` and
  `nix develop path:. -c tool/test-k1-image.sh` — passed; the 44,008-byte raw
  image was packaged and negative fixtures passed. SHA-256:
  `74be18d266e919c24faf1c7b022461c085990f33a6ad34475be3d9ae7424862f`.
- `nix develop path:. -c cargo test --package radio-flasher --package
  radio-flasher-cli` — passed; K1 writer guards and CLI parser tests passed.
- Guarded physical write through `/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0`
  — passed; detected K1 `7.03.01`, acknowledged pages `172/172`, and reported
  `status=acknowledged_not_read_back`.
- After user power-cycle, read-only `probe-normal` through the same path —
  passed; `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.1`.

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
- `K1APP-018` completion: selected `K1DISP-019`, a display-only boot witness.
  It deliberately excludes keypad, storage, RF/TX, audio, backlight, USB, and a
  general application. The existing serial witness and stock recovery route
  remain required. Activation was verified with `cargo fmt --all --check` and
  `git diff --check` in the pinned environment before implementation.

## Work Package 19 display implementation milestone

- Added an allocation-free display command/rendering module in the standalone
  K1 firmware crate. It owns the exact bounded initialization/power sequence,
  eight visible page writes with the four-column panel offset, fixed 5-by-7
  `AFIK` and `K1 0.2` glyphs, and a fallible board-transport trait.
- Exact host traces cover initialization delays and commands, all page/data
  writes, deterministic bounded framebuffer output, and fail-stop behavior at
  an injected transfer error.
- The K1 target leaf independently binds only the pinned RCC, GPIOA/GPIOB, and
  SPI1 registers. It uses PA5/PA7 AF0 for clock/data, PA6 for A0, PB2 for
  active-low CS, SPI mode 3, MSB-first, and divide-by-64. All status polls are
  bounded; timeout de-selects the display and leaves the existing serial hello
  loop reachable.
- The serial identity is now `AFIK-K1-0.2`. No keypad/PTT, backlight, audio,
  storage, USB, BK4819, RF, TX, EEPROM, interrupt, or DMA behavior was added.
- The raw image is 48,436 bytes, SHA-256
  `94ac835a473a8a910b740eb792c3a3567254ea297b1d23c31e2c7e52d0ec327b`,
  with initial SP `0x20004000`, Reset `0x08002919`, and ELF image end
  `0x0800e534`, below the `0x08020000` application end.

Static verification on 2026-08-06:

- `nix flake check path:. --no-build` — passed on `x86_64-linux`; incompatible
  `aarch64-linux` output was evaluation-skipped by Nix.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed: 132 unit/integration
  tests and all doc tests.
- Warning-denied `thumbv6m-none-eabi` Clippy with pinned build-std/core and LLD
  — passed for `radio-firmware-k1`.
- `nix develop path:. -c tool/build-k1.sh` and
  `nix develop path:. -c tool/verify-k1-image.sh` — passed.
- `nix develop path:. -c tool/package-k1-image.sh --force` — passed and emitted
  the bounded image/hash above.
- `nix develop path:. -c tool/test-k1-image.sh` — passed positive comparison
  plus truncated, oversized, and non-Thumb negative fixtures.
- `git diff --check` — passed. No physical write or display claim was made.

Physical display attempt on 2026-08-06:

- After explicit user authorization, the exact raw image and both retained
  recovery/backup pairs were revalidated. Read-only identify reported K1
  bootloader `7.03.01`; image CRC-32 was `3a2eb51b`.
- One guarded `flash-afik-k1` invocation acknowledged all `190/190` pages in
  transaction `d4f83080` and reported `acknowledged_not_read_back`. No retry or
  reset command was sent.
- After the user power-cycled the unit, the screen was blank. This is a failed
  display witness: no pixel, orientation, contrast, or illumination success is
  claimed.
- The immediate read-only normal-mode probe passed with
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.2`. Reset, USART1, and
  bounded return from display initialization therefore remain observed.
- Follow-up source inspection confirms the K1 display reset routine is empty
  and the illumination is a separate active-high PF8 backlight path. AFIK did
  not configure PF8, so the next non-writing observation must distinguish an
  unlit panel from missing LCD pixels before changing the target.
- Follow-up physical observation under bright external light showed the fixed
  words. This closes `K1DISP-019`: LCD controller setup, page/data transfer,
  orientation, and rendering worked. The isolated missing surface is the
  separately mapped PF8 backlight, now bounded under `K1BL-020`.

## Work Package 20 backlight implementation milestone

- Added a pure constant-backlight register plan which sets only the GPIOF clock
  and PF8 mode/type/speed/pull/output fields. Its exact-mask test passes.
- The K1 target applies the active-high PF8 output before display setup. It adds
  no timer, DMA, PWM, fade, brightness state, persistence, keypad, audio,
  storage, BK4819, RF/TX, USB, or interrupt behavior.
- The raw image is 48,580 bytes, SHA-256
  `249bccb1cf66ce3269cc64d80f8171fbafdb6835ab7f31a2df3fc152c9b93489`,
  CRC-32 `a327eba0`, with initial SP `0x20004000`, Reset `0x08002919`, and
  ELF image end `0x0800e5c4` below the `0x08020000` application end.

Static verification on 2026-08-06:

- `nix flake check path:. --no-build` — passed on `x86_64-linux`; incompatible
  `aarch64-linux` output was evaluation-skipped by Nix.
- `nix develop path:. -c cargo fmt --all --check` — passed.
- `nix develop path:. -c cargo clippy --workspace --all-targets -- -D warnings`
  — passed.
- `nix develop path:. -c cargo test --workspace` — passed: 133 unit/integration
  tests and all doc tests.
- Warning-denied `thumbv6m-none-eabi` Clippy with pinned build-std/core and LLD
  — passed for `radio-firmware-k1`.
- `tool/build-k1.sh`, `tool/verify-k1-image.sh`,
  `tool/package-k1-image.sh --force`, and `tool/test-k1-image.sh` through
  `nix develop path:. -c` — passed, including negative raw-image fixtures.
- `git diff --check` — passed. No revised image write was sent.

Physical backlight verification on 2026-08-06:

- After explicit authorization and revalidation of all guards, K1 `7.03.01`
  acknowledged all `190/190` pages of the 48,580-byte image in transaction
  `7e094920`; status was `acknowledged_not_read_back`. No retry or reset was
  sent.
- After power-cycle, the user observed the backlight and the fixed words. This
  completes the constant active-high PF8 witness.
- The words were faint. The current electronic-volume value is AFIK's initial
  conservative `0x15`; the pinned board source uses fixed startup value `0x1f`.
  A one-byte contrast calibration is activated separately as `K1CON-021`.
- The final read-only normal-mode probe passed with
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.2`.

## Work Package 21 contrast implementation milestone

- Changed only the ST7565-compatible electronic-volume data byte from `0x15`
  to the pinned fixed startup value `0x1f`; its exact initialization trace was
  updated. Framebuffer, power sequence, SPI/GPIO, PF8, and serial behavior are
  unchanged.
- The raw image remains 48,580 bytes, SHA-256
  `b2e6a38b965fcb0d419ec2ed7309aa3d6518285967c98d1646eddaa8718c8d32`,
  CRC-32 `4dfc4076`, with initial SP `0x20004000`, Reset `0x08002919`, and
  ELF image end `0x0800e5c4`.
- Focused host tests, host/target warning-denied Clippy, full workspace format,
  Clippy and 133 tests, target build/ELF verification, packaging, and positive/
  negative raw-image gates passed. `git diff --check` passed. No contrast image
  write has been sent.

Physical contrast verification on 2026-08-06:

- After explicit authorization and guard revalidation, K1 `7.03.01`
  acknowledged all `190/190` pages in transaction `3f6392fd`; status was
  `acknowledged_not_read_back`. No retry or reset was sent.
- After power-cycle, the user confirmed that the backlight and words were both
  present and that the words were substantially clearer.
- The final read-only normal-mode probe passed with
  `protocol=normal-firmware-hello`, `firmware=AFIK-K1-0.2`.
- `K1CON-021` is complete. This proves one fixed boot-witness contrast on the
  exact unit, not a runtime setting or production calibration policy.

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
