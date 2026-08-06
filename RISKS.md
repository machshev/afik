# Risks and unknowns

## RISK-001 — DP32G030 peripheral evidence is incomplete

- **State:** open
- **Impact:** target startup beyond the architectural reset path, peripheral
  drivers, and a complete Renode model cannot be implemented responsibly.
- **Mitigation:** `DP32-003` records sourced Cortex-M0, byte-order, flash, RAM,
  and architectural reset-vector facts in `docs/hardware-evidence.md`. Those
  facts are sufficient only for a minimal CPU-and-memory Renode boot proof.
  Target startup details beyond the architectural reset path and every
  peripheral remain blocked until separately evidenced.

## RISK-002 — UV-K5 recovery and calibration backup are unverified

- **State:** open
- **Impact:** hardware flashing could brick a test radio or destroy calibration.
- **Mitigation:** `FLASH-012` may implement and simulate only the bounded
  recovery-gated path. Do not flash an AFIK image until the exact unit is
  identified, its complete read-only EEPROM backup is retained and validated,
  a known-good raw recovery image is recorded, and rewriting/rebooting that
  recovery image has succeeded on the same unit.

## RISK-003 — Protocol wire format may change during bring-up

- **State:** open
- **Impact:** early programmer artifacts may become incompatible.
- **Mitigation:** negotiate protocol/storage versions and treat the Work Package
  1 format as provisional until UART and bootloader constraints are known.

## RISK-004 — Transactional durability is only logically modelled

- **State:** open
- **Impact:** candidate/commit isolation does not yet prove recovery from power
  loss on physical non-volatile media.
- **Mitigation:** `STORE-004` adds checksum and complete validation only to an
  offline logical configuration image; it deliberately does not claim physical
  durability. After physical storage is identified, separately define and
  model dual-slot headers, generations, and power-loss fault injection before
  implementing a device persistence layout.

## RISK-005 — Physical reset mapping and firmware packaging are unverified

- **State:** open
- **Impact:** an ELF that boots from address zero in the minimal Renode model is
  not evidence that a packaged image will boot safely on a UV-K5-family radio.
- **Mitigation:** Work Package 3 does not package or flash images. `FLASH-012`
  treats the separately observed `0xF000` boundary as a qualified-V1 deployment
  assumption, emits no bytes above it, and retains static/Renode proof. A page
  acknowledgement is not a boot claim; physical completion still requires
  exact board identification, recovery proof, and an independent observation
  that Reset reached the AFIK application.

## RISK-006 — Physical display and keypad interfaces are unverified

- **State:** open
- **Impact:** logical UI behavior cannot yet drive or be validated against a
  UV-K5-family display, keypad matrix, side keys, timing, or electrical limits.
- **Mitigation:** `UI-005` uses only product-level logical keys and bounded
  semantic views. Do not add a target adapter, pin mapping, scan timing, display
  geometry, or peripheral model until board-specific evidence and required
  experiments are recorded in `docs/hardware-evidence.md`.

## RISK-007 — BK4819 register applicability and board RF control are unverified

- **State:** open
- **Impact:** a register command plan that behaves in simulation may be wrong
  for the fitted BK4819 revision, crystal, initialization state, RF switches,
  matching network, calibration, or external power amplifier. Incorrect target
  integration could emit unintended RF or damage hardware.
- **Mitigation:** `RF-006` records official high-level facts separately from a
  mirrored machine-translated BK4819(V3) application note and restricts its
  register fields to an unbound post-initialization command and simulator
  contract. The implemented fault latch and class-bound capability check reduce
  software authority risk but do not validate hardware behavior. Do not add a
  physical bus, board switching, external PA control, flashing, or on-air tests
  until chip/board identity, original register documentation, safe dummy-load
  test equipment, calibration backup, and recovery procedures are established.

## RISK-008 — Physical scan timing and signal inputs are unverified

- **State:** open
- **Impact:** a deterministic logical scan policy does not establish how long
  the fitted receiver needs to tune or settle, how often status can be sampled,
  whether squelch is reliable, or how scan behavior performs on physical RF.
- **Mitigation:** `SCAN-007` treats dwell/hold durations as explicit workflow
  configuration and timer expiries plus normalized signal samples as adapter
  inputs. Do not encode target timer rates, polling cadence, receiver settle
  time, RSSI thresholds, tone detection, or physical scan claims until the
  relevant chip/board behavior is sourced and measured safely.

## RISK-009 — Host serial interoperability is unverified

- **State:** open
- **Impact:** a host CLI transport that opens an explicitly configured serial
  path does not prove the target exposes this protocol, uses the selected baud,
  enters a safe programming mode, or preserves exchanges under real timing and
  disconnect conditions.
- **Mitigation:** `CLI-008` keeps serial path and baud explicit, uses the same
  bounded `ProtocolTransport` contract as simulation, exposes no raw writes, and
  makes no hardware-success claim. Establish target UART/boot behavior,
  recovery, timeout/retry requirements, and hardware-in-loop fixtures before
  documenting any device/baud default or physical programming workflow.

## RISK-010 — Local web GUI is not an authenticated service

- **State:** open
- **Impact:** another process or malicious browser context on the same host may
  attempt to reach a loopback programmer GUI. Treating it as a remote or
  multi-user service could expose configuration mutation without an appropriate
  authentication, authorization, origin, and deployment model.
- **Mitigation:** `GUI-009` binds only loopback, serves one local session, bounds
  headers and bodies, rejects ambiguous HTTP framing, exposes no arbitrary
  server file paths, and requires a per-process token plus an explicit
  replacement-confirmation header for mutation endpoints. The delivered UI
  sends that header only after deliberate confirmation. Do not add non-loopback
  bind, claim authentication, or deploy it as a shared service without a
  separate threat model and security package.

## RISK-011 — Frequency Copy silicon behavior is unverified

- **State:** open
- **Impact:** a frequency/tone result derived from unverified BK4819 scan
  fields, crystal assumptions, unexplained register constants, or an unknown
  board RF path may be wrong, stale, aliased, or leave the receiver in an
  unknown state. Automatically turning such a result into a transmit-capable
  channel could cause unintended transmission on an unverified frequency.
- **Mitigation:** `FREQ-010` is research-only. Treat the FCC-filed radio manual
  as user-workflow evidence, Beken's product page as feature-existence evidence,
  and the mirrored V3 note plus existing firmware only as experiment-planning
  evidence. A future feature must yield a receive-only reviewed candidate,
  never TX authority. A decoder timeout must not become a trusted no-tone value;
  a cleanup failure must leave the adapter faulted; and any later save must be a
  separate confirmed object with `TxClass::Never`. Production remains blocked
  until the fitted chip/crystal/board are identified and bounded
  non-transmitting signal-generator experiments verify scan start, completion,
  units, accuracy, false locks, tone/code results, timeout, retrigger, and safe
  cleanup as specified in `docs/frequency-copy-feasibility.md`.

## RISK-012 — The physical APRS receive chain is unverified

- **State:** open
- **Impact:** standards-correct AX.25/APRS parsing cannot prove that the fitted
  BK4819 and board expose suitable unfiltered FM/baseband data, that any modem
  mode matches 1200-baud packet, or that the DP32G030 can recover symbols and
  service buffers with acceptable loss and power. Invented register, audio,
  clock, DMA, or interrupt behavior would make target and simulator results
  misleading.
- **Mitigation:** `APRS-011` records a source- and layer-specific defer verdict
  plus a receive-only experiment plan. Implemented software accepts complete
  de-stuffed frames with FCS as explicit inputs and does not implement physical
  demodulation, bit recovery, or target integration. Keep those layers blocked
  until the fitted revisions and board path are identified and receive-only
  signal-generator/audio/logic-analyzer experiments establish bandwidth,
  levels, timing, buffering, error rates, cleanup, and recovery.

## RISK-013 — Repeater advertisements are untrusted and may be stale

- **State:** open
- **Impact:** APRS packets can be malformed, replayed, spoofed, stale, or simply
  incorrect. Automatically converting advertised frequency, offset, or tone
  into a transmit-capable channel could enable unintended or unauthorized RF.
- **Mitigation:** `radio-aprs` validates FCS and bounded syntax, retains source
  and explicit receive-time provenance, separates conflicting origins, rejects
  equal-time conflicts, prevents stale same-key resurrection, and never evicts
  a full table implicitly. Results remain receive-only advertisements. Never
  construct `ActiveChannel`, trusted plan membership, or `TxAuthorisation`, and
  never mutate configuration automatically. Any future reviewed save must be a
  separate transaction constrained to `TxClass::Never`.

## RISK-014 — K5 revision and bootloader identity can be misleading

- **State:** open
- **Impact:** visually similar UV-K5-family radios now use incompatible MCUs and
  bootloaders. Treating a beacon or model name as silicon identity could write a
  DP32G030 image to an incompatible device and leave it unable to boot.
- **Mitigation:** support only an opened/photographed V1 unit whose MCU marking
  is DP32G030, then additionally require an exact version-2 beacon. Reject v5,
  V2/V3 markings, unknown beacon lengths/versions, clones, and wildcard version
  negotiation. Preserve unit-specific inspection evidence with the physical
  experiment record; do not infer support for the product family.

## RISK-015 — The stock bootloader has no established read-back transaction

- **State:** open
- **Impact:** an acknowledgement may mean only that a page command was accepted.
  A cable fault, power loss, bootloader defect, or flash failure can leave a
  partial image, and blindly retrying an ambiguous page may have undocumented
  effects.
- **Mitigation:** validate every prerequisite before the first write; write only
  one complete 240-page image in ascending order; use a nonzero per-run
  transaction identifier; require an exact transaction/page/result
  acknowledgement; stop without retry on the first missing or mismatched
  acknowledgement; keep the bootloader region outside all artifacts; maintain
  power and cable stability; and prove stock recovery before AFIK. Report only
  acknowledged writes until separate read-back and boot evidence exists.

## RISK-016 — UV-K1 board identity and recovery contract are not yet pinned

- **State:** open
- **Impact:** a trusted firmware project may support multiple related boards,
  revisions, image layouts, or recovery methods. Applying the wrong variant or
  mistaking source behavior for the exact available unit could erase
  configuration/calibration, prevent boot, drive a peripheral incorrectly, or
  enable an unsafe RF path.
- **Mitigation:** `K1EVID-013` pins the exact running firmware and source
  revision, records the unit's model/PCB/MCU identity, validates backup and
  known-good recovery artifacts, separates Puya MCU facts from board mappings,
  and assigns every mapping a physical observation. No AFIK write or TX is
  permitted in this package; a later target package must begin with a harmless
  boot witness followed by demonstrated recovery on this unit.

## RISK-017 — K1/K5 bootloader classification is not board identity

- **State:** open
- **Impact:** related radios may share a serial envelope or version-shaped
  beacon; treating a protocol classification as a physical MCU/board identity
  could select an incompatible image or erase calibration.
- **Mitigation:** `K1FLASH-014` accepts only pinned K1 `7.03.*` and qualified K5
  V1 `2.*` beacon shapes, rejects unknown versions, reports protocol family
  separately from hardware identity, enumerates USB candidates without trusting
  the adapter alone, and routes K1 AFIK flashing through a separate exact-target
  command. Physical markings and image contracts remain separate evidence gates.

## RISK-018 — K1 device trailer does not provide a checked integrity marker

- **State:** open
- **Impact:** the observed K1 device-side trailer is not the K5 decoder's
  `0xffff` marker and does not match the recorded CRC calculation. Accepting
  the K1 frame without a trailer check could allow some serial corruption to
  pass unnoticed.
- **Mitigation:** the K1 path retains bounded resynchronisation, exact footer,
  declared-length, command, version, transaction, page, and zero-result
  acknowledgement checks; it never retries an ambiguous page. Before the first
  AFIK hardware exercise the unchanged known-good stock recovery image,
  complete backup, distinct image, and exact confirmations must be present.
  After the write, normal-mode probing and the retained recovery route remain
  required; no RF operation is implemented.

## RISK-019 — K1 AFIK implementation is still witness-only

- **State:** open
- **Impact:** the serial witness proves only Reset and USART1 response. A future
  display, keypad, storage, or RF slice can still fail or have unsafe behavior.
- **Mitigation:** the first image's bounded USART1 hello witness was observed
  exactly as `AFIK-K1-0.1` after power-cycle. The guarded `flash-afik-k1` path
  still refuses missing rehearsal, invalid backup, bad CRC, mismatched
  version/target, or an image identical to recovery. Each future hardware
  surface requires its own evidence, test, and recovery observation.

## RISK-020 — Native K1 USB routing is unobserved but not required

- **State:** open
- **Impact:** the PY32F071 USB capability is not a usable AFIK application
  witness on the current unit. Treating the CH340 enumeration as native USB
  would produce a false USB claim.
- **Mitigation:** `K1WIT-017` uses the independently evidenced USART1/CH340
  path instead. AFIK adds no USB implementation or native-USB identity claim.

## RISK-021 — Exact K1 display binding is not yet physically observed by AFIK

- **State:** open
- **Impact:** the pinned board source can identify a likely ST7565-compatible
  128-by-64 path, pins, and SPI mode without proving the exact controller
  marking, reset wiring, panel orientation, contrast, or behavior of AFIK's
  independent implementation on this unit. A wrong sequence may leave the
  display blank or visually corrupted.
- **Mitigation:** `K1DISP-019` uses only the source-observed SPI1 SCK/data, A0,
  and active-low CS path; it does not drive an unobserved reset pin, backlight,
  keypad, audio, storage, BK4819, RF, or TX. Exact command traces and rendered
  bytes are host-tested before a separately confirmed physical write. The
  existing USART1 hello and retained stock recovery image remain independent
  observations and rollback paths.
- **Observed result:** the first guarded image write left the screen blank while
  the serial `AFIK-K1-0.2` fallback remained responsive. The current image does
  not drive the independently mapped active-high PF8 backlight. Do not assume
  this explains all missing pixels; distinguish illumination from controller
  traffic before revising the image or claiming a root cause.
- **Resolution:** bright external light revealed the expected words, so the LCD
  witness itself is complete. The remaining unlit state is isolated to the
  separately mapped PF8 backlight and is bounded under `K1BL-020`.

## RISK-022 — Constant K1 backlight has no brightness or power policy

- **State:** open
- **Impact:** holding PF8 high provides no brightness adjustment, timeout, fade,
  or battery-aware power management and is unsuitable as the final radio UI.
- **Mitigation:** `K1BL-020` is a boot witness only. It uses the pinned
  active-high PF8 mapping and deliberately excludes TIM7, DMA, settings, and
  persistence. A later UI/power package must define bounded brightness and
  shutdown behavior before treating the backlight as a production driver.

## RISK-023 — Fixed K1 contrast is not a production calibration policy

- **State:** open
- **Impact:** one fixed electronic-volume value may vary across panels,
  temperature, supply voltage, and viewing angle, and it provides no user
  adjustment or persistence.
- **Mitigation:** `K1CON-021` tests only the pinned startup value 31 after the
  exact unit showed faint pixels at 21. Treat success as a boot-witness
  readability result. A future UI/persistence package must bound adjustment,
  valid ranges, defaults, and recovery before exposing contrast as a setting.
- **Observed result:** value 31 produced substantially clearer fixed words on
  the exact unit while the backlight and serial fallback remained functional.
  The risk remains open for production/runtime calibration.

## RISK-024 — K1 keypad electrical behavior is not yet physically observed by AFIK

- **State:** open
- **Impact:** the pinned board source gives a coherent 4-by-4 mapping and GPIO
  configuration, but it does not prove exact-unit contact bounce, settling
  time, ghosting, simultaneous-key behavior, stuck lines, or AFIK's independent
  scan implementation. A wrong drive sequence could misreport keys or contend
  with an unexpected board state.
- **Mitigation:** `K1KEY-022` holds every column high at idle, drives only one
  evidenced column low at a time, restores idle after every scan including
  failures, accepts only one stable matrix cell, and treats ambiguity and time
  reversal as no key. PTT, side keys, interrupts, RF/TX, persistence, and
  general menus remain unreachable. Host tests and static image gates precede
  one guarded receive-only hardware experiment with the serial recovery witness
  retained.
- **Observed result:** the first physical image booted and retained its serial
  fallback, but key labels did not display. No GPIO scan or key mapping is
  physically established; diagnose with a bounded observable witness before
  another write.
- **Current diagnostic:** a strictly read-only serial request reports one raw
  four-column scan with bounded row masks and scan validity. Comparing release
  with held MENU can localize the GPIO/display boundary without treating the
  response as proof of debounce, visible rendering, or the other 15 keys.
- **Observed diagnostic interference:** holding MENU prevented the serial probe
  from answering until release, consistent with entry into the synchronous
  key-triggered display path. A follow-up image suppresses that SPI transfer so
  raw GPIO observation does not depend on display completion.
- **Isolation result:** suppressing the key-triggered SPI write did not prevent
  the held-MENU timeout. Physical matrix closure may interfere with execution
  or UART response before a raw report can be returned; a pre-armed latch which
  transmits only after release is required to distinguish those cases.
- **Latch result:** MENU tap/release left no nonzero PB12..PB15 sample in RAM.
  The pinned mapping may not match this exact unit or the held circuit may halt
  sampling; do not change pin assignments without raw-register observation.
- **Raw observation update:** the wider latch captured PB15 low only during PB6
  selection, establishing MENU on this exact unit. Mapping uncertainty for that
  cell is closed; timing, other keys, debounce, and visible display update remain
  unverified.

## RISK-025 — Embassy/PY32 release and evidence compatibility

- **State:** open
- **Impact:** current Embassy releases guarantee latest-stable Rust rather than
  AFIK's pinned Rust 1.86, and community `py32-hal` must be checked for exact
  PY32F071 timer/USART1/SPI1 coverage. A compiling executor does not prove
  peripheral correctness.
- **Mitigation:** pin versions before migration; compile each feature in locked
  Nix; retain linker/image/recovery contracts; add Renode and physical tests one
  boundary at a time; do not enable interrupt/DMA behavior merely because a HAL
  exposes an API.
- **Observed:** Embassy executor 0.10.0 builds on pinned Rust 1.86. Exact F071
  metadata exists in `py32-metapac 0.5.0`, but `py32-hal 0.4.1` does not expose
  it, so HAL peripheral support is blocked pending a reviewed extension. All
  four F071 package features currently select the same incomplete generated
  inventory: GPIOA, WWDG, AES_LPUART1, and DMA1_CH1 only. A feature-only HAL
  extension fails because RCC is absent. Correct the source data upstream;
  never fill the gap with F072 metadata or inferred register compatibility.
- **Local mitigation update:** AFIK now carries a reproducible generated PAC
  from pinned `py32-data` source plus a small reviewed HAL compatibility delta.
  All concrete F071 features and the required K1 inventory compile locally.
  This closes the released-artifact inventory blocker but not the risk: exact
  package identity, time, USART1, SPI1, DMA, interrupt, clock, and physical HAL
  behavior still require separate evidence. F071 ADC HAL bindings remain off.
- **Time-driver update:** the interrupt-enabled TIM15 Embassy driver now passes
  strict F071 target compilation. Its metadata and channel capacity are
  sufficient for this static boundary, but runtime time remains unproven: HAL
  initialization would take ownership of RCC state, while the current image
  inherits a 48 MHz clock from the bootloader. Do not migrate startup until
  that handoff and physical TIM15 interrupt/tick behavior are separately proven.
