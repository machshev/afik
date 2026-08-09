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

- **State:** open, first bracket taken
- **Impact:** a deterministic logical scan policy does not establish how long
  the fitted receiver needs to tune or settle, how often status can be sampled,
  whether squelch is reliable, or how scan behavior performs on physical RF.
- **Mitigation:** `SCAN-007` treats dwell/hold durations as explicit workflow
  configuration and timer expiries plus normalized signal samples as adapter
  inputs. Do not encode target timer rates, polling cadence, receiver settle
  time, RSSI thresholds, tone detection, or physical scan claims until the
  relevant chip/board behavior is sourced and measured safely.
- **Carrier-only squelch, 2026-08-09:** `EVID-K1-070` records a scan stopping on
  the wrong channel beside a close transmitter, because AFIK gates the squelch
  on carrier strength alone and its whole threshold range sat below that
  signal's adjacent-channel leakage. The range is now 8 dB a step to about
  -66 dBm, which is a workaround rather than a fix: noise-gated squelch is what
  distinguishes a signal on this channel from a strong one next to it, and its
  thresholds are per-unit calibration data this radio cannot yet read. Do not
  invent them. The remaining work is to read the unit's own squelch calibration
  from the vendor block, exactly as the battery calibration is read.
- **First measurement, 2026-08-09:** `EVID-K1-069` brackets the usable dwell on
  the exact unit between 60 and 100 milliseconds — the scan stops on a signal at
  100 and not at 60. This is one pass against one signal and measures the whole
  retune-settle-sample loop rather than any chip behaviour, so it is an upper
  bound on the floor and remains an operator-facing control rather than a
  constant. Nothing in the firmware encodes it.

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
- **Async runtime result:** the corrected Embassy image was power-cycled; the
  boot screen returned, normal hello passed, and all main-key labels were
  observed on the second display line. This closes the bounded keypad/display
  bring-up risk for the migrated path; RF/TX, side keys, persistence, and
  production calibration remain open.

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
- **USART1 update:** the evidenced PA9 TX / PA10 RX AF1 path, USART1 RCC and
  interrupt binding, 38,400-baud configuration, and bounded DMA types now pass
  strict target compilation through the real async HAL constructor. The
  constructor is not called by the image; clock preservation, interrupt/DMA
  delivery, serial error recovery, and responsiveness during rendering remain
  physical/runtime gates.
- **SPI1 update:** generated F071 metadata contains the evidenced SPI1 RCC and
  PA5 SCK / PA7 MOSI AF0 surfaces, but `py32-hal 0.4.1` has no SPI module. The
  HAL's own support table and TODO list confirm SPI is unimplemented. Do not
  infer an async display driver from PAC inventory; a bounded AFIK driver or
  separately reviewed HAL extension is required before migration.
- **SPI1 implementation update:** the local HAL now supplies only the bounded
  transmit-only display surface, with finite status waits and cooperative
  16-byte yields. Strict target compilation proves API/register/pin coherence,
  not executor progress or physical transfers. Keep startup unchanged until a
  deterministic scheduling proof and then separately guarded UART/display
  observations pass.
- **Cooperative-progress update:** a deterministic round-robin harness now
  proves that the exact 16-byte display schedule permits serial work between
  chunks for a complete 1,024-byte frame, and compile-time equality prevents
  schedule drift from the HAL driver. Runtime executor startup, interrupts,
  DMA, clock ownership, and physical UART/display coexistence remain open.
- **Runtime-composition update:** the executor, async USART1/DMA, and
  cooperative SPI1 now type-check as one explicitly owned optional bundle.
  This closes only Rust ownership/API composition. The bundle is not called;
  bootloader-clock handoff, interrupt and DMA execution, physical serial/SPI
  behavior, and recovery under peripheral faults remain open.
- **Clock-handoff update:** the pinned application records only the resulting
  48 MHz software value, not the bootloader's RCC fields. AFIK now has a
  read-only, fail-closed snapshot decoder and target compile proof, but it does
  not publish HAL clocks. Exact-unit observation remains required before HAL
  adoption or any runnable async migration.
- **Observation-surface update:** a bounded serial request can now return the
  four raw RCC registers and fail-closed result without modifying clock state.
  Static and Renode gates pass, and all 252 diagnostic-image pages were
  acknowledged without retry. This is not read-back or boot proof; do not
  publish HAL clocks until the post-power-cycle response is captured and reviewed.
- **Physical observation update:** normal hello remains responsive, but the
  combined RCC request timed out twice. Treat the source of the timeout as
  unknown; isolate individual reads/response progress rather than assuming a
  register value, PAC defect, or UART failure.
- **Isolation update:** one-register request/response pairs now distinguish CR,
  ICSCR, CFGR, and PLLCFGR progress. The isolation image was acknowledged in
  full, but has not yet passed normal boot or any register probe; its static and
  bootloader success does not resolve the physical timeout.
- **First-read result:** CR timed out while hello remained recoverable. This
  still does not distinguish MMIO fault from command/response behavior; require
  a no-MMIO control rather than inferring RCC accessibility.
- **Serial-only isolation update:** the runnable image now excludes display,
  keypad, backlight, debounce, matrix scans, SPI1, GPIOB, and GPIOF. A fixed
  no-MMIO response distinguishes command/framing progress from an RCC read.
  Static verification does not close the risk; require power-cycle, hello, then
  no-MMIO control before attempting the isolated RCC reads.
- **Exact-unit result:** serial-only hello, no-MMIO control, and every RCC read
  passed. The stable snapshot is CR `03000500`, ICSCR `00e64d14`, CFGR
  `00000012`, PLLCFGR `00000006`; the fail-closed decoder rejects only
  `ICSCR.HSI_FS=2` versus provisional expected encoding `4`. Do not change the
  decoder or publish clocks without primary PY32F071 field/frequency evidence.
- **Field-resolution update:** the pinned F071 DIE072 inventory resolves the
  tuple as 16 MHz HSI, HSI PLL source, and x3 multiplier. The decoder now checks
  the two-bit source and multiplier explicitly. This resolves register
  interpretation only; runnable HAL clock publication, interrupt/DMA behavior,
  and timer/peripheral coexistence remain open.
- **Publication-boundary update:** a feature-gated safe K1 wrapper now accepts
  only the private validated handoff proof before updating the HAL's software
  frequency table. It is target-compile proven but not called by the image;
  one-time ordering, TIM15, interrupt/DMA, and peripheral coexistence remain
  runtime and physical gates.
- **Inherited-init update:** the local HAL can now take singleton tokens and
  initialize GPIO, DMA, FLASH, and TIM15 after guarded publication without
  executing RCC clock configuration. Strict target compilation closes the API
  and ordering boundary only; no runnable image or interrupt/timing behavior
  has passed yet.
- **Runnable-image update:** the release ELF contains the complete 192-byte
  Cortex-M/F071 vector table and exact DMA1, TIM15, and USART1 handlers. Static
  packaging proves bounds and wiring, not physical interrupt delivery, timer
  rate, DMA completion, UART responsiveness, keypad input, or visible display.
  The first guarded write must retain the known-good recovery path and be
  followed by independent serial and visible keypad/display observations.
- **First runtime result:** blank display and two serial timeouts exposed a
  missing VTOR handoff, not a proven TIM15/DMA fault. The pinned K1 startup and
  PY32F071 header explicitly require/support VTOR at `0x08002800`; the next
  image adds that exact write before any interrupt-enabled initialization.
- **Correction result:** writing VTOR to `0x08002800` before inherited
  initialization restored the boot screen, async UART hello, display updates,
  and all observed main-key labels after power-cycle.

## RISK-026 — K1 side-key mapping and electrical behavior are unverified

- **State:** open; the mapping half is now resolved and the physical half is not
- **Impact:** guessing side-key pins from the main matrix could misread controls
  or interfere with the board.
- **Mapping resolved:** `EVID-K1-052` records the exact source mapping from the
  pinned `App/driver/keyboard.c`. The side keys have no dedicated GPIO: SIDE1 is
  PB15 and SIDE2 is PB14, active low, read during the unselected pass where all
  four columns PB6..PB3 remain high. PB13 and PB12 are explicitly invalid in
  that state. PTT PB10 remains a separate source fact.
- **Remaining exposure:** no side key has been physically observed on the exact
  unit, so this board's polarity, stability, and settling behavior are still
  unconfirmed against the source's intent.
- **Mitigation:** `K1SIDE-025` carries the bounded receive-only experiment. Raw
  samples stay bounded, provenance-tagged, and fail-closed, and cannot create
  semantic UI state or RF/TX authority. The undefined PB13/PB12 unselected case
  must never be decoded as a key.

## RISK-027 — No AFIK receive register write has been observed on hardware

- **State:** open
- **Impact:** the receive path reproduces the pinned firmware's register values
  exactly, but AFIK has never driven a BK4819 on the exact unit. An error in
  sequencing, timing, or the three-wire bus would produce silence, a deaf
  receiver, or unexpected chip state rather than an obvious failure.
- **Mitigation:** the driver only ever writes the documented receive mode block
  and faults closed on any bus error, so a failed transfer cannot leave the
  driver believing it is receiving. The transmit word stays behind the existing
  central-policy token.
- **Required experiment:** a separately guarded, receive-only bring-up on the
  exact unit which retains the known-good recovery image and the retained
  EEPROM backup, reads back a known register before writing any, and confirms
  RSSI movement against a known signal.

## RISK-028 — Squelch thresholds are calibration inputs AFIK does not yet read

- **State:** open
- **Impact:** the receive path validates threshold hysteresis but has no source
  of per-unit, per-band calibration values. Without them the squelch level in
  the channel record cannot yet be mapped to chip thresholds.
- **Mitigation:** thresholds are explicit inputs. The driver refuses
  inconsistent sets and never substitutes a default, so a missing calibration
  path is a compile-time gap rather than a silent wrong value.
- **Required experiment:** read the retained EEPROM backup's calibration
  region under its own evidence entry before defining the mapping from a
  squelch level to thresholds.

## RISK-029 — The native editor is a local tool with no authentication

- **State:** accepted
- **Impact:** anyone with access to the running desktop session can write a
  configuration or start a guarded flashing operation.
- **Mitigation:** the editor requires an explicit device path or simulator
  selection, every write is a validated transaction with read-back
  verification, and firmware writes keep the flasher library's exact
  confirmation phrases, recovery image, and EEPROM backup requirements. The
  editor opens no network socket.

## RISK-030 — Receive is proven only as raw metering on one unit

- **State:** open
- **Impact:** `EVID-K1-057` shows the bus, power-on table, tuning, and metering
  working on the exact unit, but nothing downstream. Demodulated audio, real
  sensitivity, tone decoding, and squelch behaviour are all unverified, so a
  reader could mistake "RSSI moves" for "the radio receives properly".
- **Mitigation:** the image reports raw fields only, keeps audio muted, and
  runs with the squelch-off threshold set rather than a calibrated one. No
  channel, UI, or persistence path consumes these samples.
- **Required experiments:** compare RSSI against a known-level source, enable
  the audio path and confirm demodulated audio, then read the unit's squelch
  calibration before claiming a working receiver.

## RISK-031 — The K1 squelch calibration lives in external SPI flash

- **State:** open
- **Impact:** the pinned source reads per-band, per-level squelch thresholds
  from the external PY25Q16 flash at `0x010000` and `0x010060`, not from the
  8 KiB EEPROM AFIK backs up. Until AFIK reads that device, no calibrated
  squelch is possible on this board.
- **Mitigation:** the driver still takes thresholds as validated inputs, and the
  K1 image now applies `SquelchThresholds::for_level`, which is AFIK's own
  table and is documented as such. It varies only the carrier-strength pair and
  leaves the noise and glitch pairs permissive, so the one thing it claims is
  the one thing the operator can judge by ear. `RISK-028` records the same gap
  at the driver boundary, and `RISK-034` tracks the consequence.
- **Required experiment:** map the pinned source's per-band calibration layout
  under its own evidence entry and use this unit's values instead of AFIK's.
  The read-only external-memory path this needs now exists: `Eeprom::read_vendor`
  already reads the battery calibration from the same device.

## RISK-032 — No AFIK flash write has been observed on this MCU

- **State:** open
- **Impact:** retaining a configuration is the first time AFIK programs the
  PY32F071's own flash from running code. The erase and program timing comes
  from the device's factory `CONFIGBYTES` parameter block through the vendored
  HAL, which is device-supplied rather than AFIK-observed, and no retained
  configuration has yet survived a power cycle on hardware.
- **Mitigation:** every access is bounded to the reserved sector, which holds no
  code and no calibration, so a failed write can lose only the retained
  configuration. A sector which does not decode is reported as "built-in set" on
  the information screen instead of being partly trusted, and the application
  region is kept out of the sector by the linker map and both image gates.
- **Required experiment:** program a configuration from the studio editor,
  power-cycle the unit, and confirm the information screen reports the same
  generation and channel count and that the channels are still selectable.

## RISK-033 — Target stack headroom is bounded by inspection, not by a gate

- **State:** open
- **Impact:** statics and task futures occupy about 8.0 KiB of the evidenced
  16 KiB of SRAM, leaving 8,196 bytes of stack. Nothing in the build tells us
  what peak stack use actually is, and an overflow would silently corrupt the
  top of `.bss`.
- **Mitigation:** `crates/radio-firmware-k1/stack-headroom.x` asserts a 6,144-byte
  reserve at link time, so a build which eats the headroom fails to link rather
  than packaging; `tool/verify-k1-async-image.sh` checks the same bound and
  records `llvm-size` with each image in `STATUS.md`. The reserve is a policy
  floor, not a measurement: it sits above the 5,396 bytes `AFIK-K1-4.0` had when
  it reached the operator and did not start. The configuration is held once,
  encoded, and `ARENA-038` packed it, so the store no longer reserves a
  worst-case slot per object and headroom rose from 7,100 bytes to 8,196.
- **Observed:** this risk has bitten twice. `AFIK-K1-2.5` exhausted the stack and
  never started. `AFIK-K1-4.0` did the same after a slot-budget change added 512
  bytes of statics, and cleared the then-4,096-byte scripted floor on its way to
  being flashed. The gate was not missing; it was set too low to catch either.
- **Required experiment:** a painted-stack high-water measurement on the exact
  unit. The linker assertion is now in place, so what remains open is knowing
  what peak use actually is rather than guessing a floor above what has failed.

## RISK-034 — The squelch levels and the battery percentage are unverified

- **State:** open
- **Impact:** two numbers the operator will act on are estimates. The squelch
  thresholds are AFIK's own, so a level may open on noise or shut on a workable
  signal, and the operator would reasonably read that as the radio being deaf.
  The battery percentage comes from a discharge curve the pinned source itself
  marks estimated, applied to a converter reading no AFIK build has compared
  against a meter, so it could show usable charge on a pack about to cut out,
  which is the failure the indicator exists to prevent.
- **Mitigation:** the squelch level is reachable in two key presses from the
  operating screen and level zero disables it outright, so an operator who does
  not trust it can turn it off without a host. The battery indicator reports
  nothing at all rather than a number when the calibration is absent or
  implausible, and its arithmetic is host-tested against the curve.
- **Required experiment:** on the exact unit, sweep the squelch levels against
  a known weak signal and record which levels open; and compare the reported
  voltage against a meter across the pack when charged and part discharged,
  then confirm the indicator falls monotonically over a discharge. Until both
  are done, neither number may be described as measured.

## RISK-035 — A packed store compacts inside a transaction

- **State:** open
- **Impact:** writing or removing an object moves every entry after it, up to
  about a kilobyte on the K1. This happens inside a candidate transaction and
  cannot corrupt the active bytes — the candidate is a separate copy and is
  discarded whole on failure — but it is new work on a 48 MHz core in the path
  of a host write, and it has been measured nowhere.
- **Mitigation:** the move is bounded by the declared store size and happens
  once per object written. Against an external-memory page program measured in
  milliseconds it should be noise, and the host write of three plans over serial
  showed no timeout on the exact unit. Compaction on replace, on growth, on
  shrink and on removal is covered by host tests which check both the resulting
  order and that the bytes before the change are untouched.
- **Required experiment:** time a full-store write on the exact unit — enough
  objects to fill the 1,264 bytes, written in an order which forces a move on
  every one — and record whether any serial exchange approaches its timeout.
  Until then the cost is argued rather than observed.

## RISK-036 — The radio stops dead and does not say why

- **State:** open, mechanism identified, cause unknown
- **Impact:** the exact unit has been seen to freeze with its last frame on the
  display, a completely unresponsive keypad, and a silent serial link. It has
  happened on the information screen more than once, and it reproduces on
  `AFIK-K1-5.6` as well as on `5.7`, so it predates the `CTRL-044` work rather
  than being caused by it.
- **Mechanism, established by reading the image:** `fail_closed` is an infinite
  spin loop and the panic handler called it, discarding `PanicInfo`. Any panic
  therefore produced exactly this signature, and the radio held the one piece
  of evidence that would explain it.
- **What is not established:** whether these freezes are panics at all. A hard
  fault, a stack overflow which faults, or a stalled bit-banged bus would look
  the same from outside. `RISK-033` already records that stack headroom is
  bounded by inspection rather than measurement, which makes an overflow a live
  candidate.
- **Mitigation, `AFIK-K1-5.8`:** the panic handler records the panicking file
  and line into the section startup does not clear and resets. The next boot
  draws `PANIC <file>:<line>` on its first frame and on the information screen.
  A freeze which now still freezes is evidence in itself: it means the fault is
  reached without the panic handler running, and a `HardFault` handler doing the
  same thing is the next instrument.
- **Consequence for `ARENA-038`:** the serial dead end has always been witnessed
  through the information screen, which is the screen the radio has been seen to
  freeze on. Counter readings taken from a frozen display describe when the
  radio stopped, not what the link did. `RX0000 TX0000 D0000` observed on
  2026-08-09 was read this way and cannot be used as evidence that no bytes
  arrived.
- **Observed 2026-08-09, `AFIK-K1-5.9`:** with the radio confirmed responsive
  and the operator watching the information screen, ten host frames produced a
  screen which repeatedly blanked and returned to the operating screen on a
  channel with a live meter — a reboot each time. The bottom row read `MEM`, not
  `PANIC`, so the panic handler did not run and these resets are not panics.
- **What that explains:** every counter reading ever taken from this screen. The
  counters are zeroed by the reboot, so `RX000 TX000 D000 E000` describes a
  radio which has just restarted rather than a link with nothing on it. The
  `ARENA-038` dead end is therefore not known to be a protocol, framing, baud or
  clock fault: the radio resets when the host sends to it, and so can never
  answer.
- **Ruled out, same session:** the host's modem control lines. Opening and
  closing the port twice with no data sent produced no visible effect, so DTR
  and RTS toggling — two open/close cycles per CLI invocation, `hupcl` never
  disabled — is not the trigger. Data on the wire is.
- **Required next:** the reset cause, read from `RCC_CSR` at boot and shown on
  the information screen. It distinguishes a pin reset, a watchdog, a brown-out
  and a software reset from each other, and it is the one measurement which says
  what kind of fault this is rather than narrowing what it is not. A `HardFault`
  handler recording as the panic handler now does would separate a fault from an
  external or supply-side reset.
- **Established 2026-08-09, `AFIK-K1-6.0`:** `RCC_CSR` reads `SFT` after a
  reboot provoked by host frames. The panic handler is the only caller of
  `sys_reset` in the image and `sys_reset` is its last statement, so the handler
  ran to completion. **These resets are panics**, not brown-outs, not watchdogs,
  and not the reset pin. A cold boot reads `PWR` as expected, and the flags are
  cleared each boot, so neither reading is inherited.
- **Established 2026-08-09, `AFIK-K1-6.1`:** a boot counter in the same
  `.uninit` section reads one after every software reset. **That memory does not
  survive a reset on this radio** — the vendor bootloader runs before the
  application and uses it. The panic reporter added in `5.8` therefore cannot
  work here: the handler writes the file and line correctly and the next boot
  can never read them. `MEM` on the information screen has never meant "no panic
  occurred"; it means the report was gone before it could be read.
- **What still works from that change:** the radio recovers instead of freezing,
  and the reset cause is readable. Those are why `5.8` through `6.1` are worth
  keeping despite the report itself being dead on this hardware.
- **Bisection attempt, `AFIK-K1-6.2D`, inconclusive and partly contradictory:**
  an image which counts each received byte and drops it without parsing did not
  reset under the same host traffic which resets a parsing image. But its
  counters afterwards read `RX000 TX000 D000 E000` — no bytes received and no
  receiver errors either. If nothing arrived, the parsing code never ran in
  either build, and the difference between them cannot be what stopped the
  panic. One of those two observations is wrong, or the link is intermittent
  between runs.
- **Withdrawn:** an earlier note in this file's history claimed bytes were shown
  arriving. That came from misreading a screen report and is not supported.
  Whether any byte has ever reached this radio's UART in application mode
  remains unestablished.
- **Not established:** that the reset is caused by the data rather than
  correlated with it, whether any byte arrives at all, and where the panic is.
  The `.uninit` route to the panic location is closed; the remaining routes are
  a RAM region the bootloader does not touch, a blocking display write from the
  handler, or further bisection.
- **Required before more bisection:** a repeatable physical setup. Results
  across one evening included a freeze, a reboot, and neither, under nominally
  identical traffic, which is the signature of an intermittent connection rather
  than of firmware. The cable, its seating, and whether the radio is on battery
  or otherwise powered should be fixed and recorded before another cut is taken,
  or each cut will measure the setup instead of the code.
