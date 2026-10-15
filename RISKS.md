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
- **K5 configuration-memory narrowing, 2026-08-13:** `EVID-K5-028` adds a
  software-only compare/write/poll/read-back operation for one evidenced
  eight-byte block. No disposable address, power-loss behavior, or physical
  restore has been established, so the boot image has no call site and no
  physical EEPROM write is authorized by this implementation.

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
- **K5 diagnostic exception, 2026-08-12:** `EVID-DP32-010` and `EVID-K5-022`
  now support one fixed, receive-inert V1 display witness. It proves only the
  visible stage reached; the keypad and the full K5 operator interface remain
  unverified.
- **K5 keypad narrowing, 2026-08-12:** `EVID-K5-025` supports the V1 main-matrix
  adapter and its shared-pin restore state. Physical label mapping, settling,
  debounce, side keys, and PTT remain unverified until the stated experiment.
- **Shared semantic narrowing, 2026-08-13:** `APP-052` now shares only
  post-debounce logical key meaning and the receive semantic view. K1/K5
  physical scan timing, electrical behavior, display rendering, and PTT/TX
  behavior remain target-specific and are not newly claimed by host tests or
  the unflashed K5 package.
- **Shared activation narrowing, 2026-08-13:** the new
  `radio-platform::configuration::ActivatedConfiguration` shares only the
  common PMR446 identity, audio preference, and opaque generation value. K1
  persistence/object activation remains target-owned; K5 generation zero is a
  no-persistence fixture, not EEPROM evidence. K5 configuration retention,
  restore, power-loss behavior, and host configuration control remain open.
- **Shared change narrowing, 2026-08-13:**
  `radio-platform::configuration_change` proves only an in-memory application
  hand-off. Its application-local change identity prevents stale acknowledg-
  ments within one app instance, and matching rejection fails silent, but it
  is not a serial transaction, EEPROM journal, power-loss proof, or physical
  tune/read-back observation. K1 object commit/read-back and K5's lack of AFIK
  persistence remain separate target concerns; K5 generation zero remains a
  volatile capability.
- **Shared change atomicity correction, 2026-08-13:** pending candidates now
  block intervening start/navigation/audio mutations, receiver faults clear
  pending state, and a bounded application-instance token prevents queued
  acknowledgements crossing app replacement. K1/K5 acknowledgement results
  now require complete adapter-effect success and failure cleanup keeps the
  speaker muted. These are deterministic software guarantees only; they do
  not add serial, EEPROM, power-loss, or physical receiver evidence.
- **Shared serial-control narrowing, 2026-08-13:** `APP-052` now has a
  transport/storage-free request/result seam for semantic view, capabilities,
  logical keys, and acknowledged candidates. K1 and K5 capability sets are
  explicit, but the seam does not prove a common wire format, K5 persistence,
  K5 object configuration, or physical serial behavior. K1's existing wire
  adapters and K5's circular-DMA UART remain target-owned; target-only K1
  arbitrary/VFO/scan operations are not silently represented as K5 features.
- **Common payload narrowing, 2026-08-13:** `radio-platform::serial_control::wire`
  now fixes a small view/capability/logical-key payload and rejects candidates,
  malformed lengths, reserved state bits, and unknown key codes. K1 and K5
  target framing remains separate; host and simulator tests do not establish
  physical serial interoperability, K5 persistence, or K1 object-protocol
  compatibility for the new payload.
- **Target envelope mapping narrowing (historical key range superseded by
  ADR-085/ADR-086), 2026-08-13:** K1 now carries the common
  payload only as `RuntimeControl` command `0x18`, retaining its existing
  sequence/replay behavior. K5 now assigns `0x0600..=0x0614` and response
  `0x0620` inside the observed fixed normal-mode envelope, which has no
  sequence field and therefore receives no invented replay cache. Host and
  simulator tests prove framing recovery and shared dispatch, but this does
  not prove physical K1/K5 serial interoperability, K5 persistence, or
  candidate acknowledgement on a wire.
  The current unflashed `2.0P` K5 mapping is View `0x0600`, Capabilities
  `0x0601`, Side2 `0x0603`, Up `0x0606`, Down `0x0607`, AUDIO `0x0615`, and
  response `0x0620`; the latest STATUS final-gate record is authoritative for
  artifact evidence.

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
- **K5 adapter narrowing, 2026-08-12:** `EVID-K5-027` establishes the V1
  three-wire and speaker-gate bindings in source and implements them below the
  existing register contract. No operation has run on physical K5 silicon;
  read-back, initialization, receive, and audio remain required experiments.
- **K5 receive-image narrowing, 2026-08-13:** `EVID-K5-030` composes that bus
  with the existing source-backed BK4819 profile in a built, unflashed image.
  Both audio gates remain muted and the image has no TX path. Physical
  initialization, register read-back, and RSSI movement are still the required
  experiment; package success is not receiver evidence.
- **K5 raw-metering result, 2026-08-13:** the flashed `1.6R` image booted and a
  second radio's selected-channel transmission produced a new sample higher
  than baseline noise. This closes the absence of any K5 AFIK register-write
  observation only as far as raw signal-correlated metering. Audio,
  intelligibility, sensitivity, calibrated squelch, selectivity, and exact
  meter values remain open.
- **K5 audio candidate, 2026-08-13:** built `1.7A` applies AFIK's carrier-only
  level three, samples the raw link on a 50 ms polled `SysTick`, and gates both
  chip AF routing and PC4. This is deliberately provisional until the unit's
  calibration is available. Software/package success does not establish quiet
  closure, demodulation, intelligibility, or clean retuning.
- **K5 audio result:** `1.7A` produced receive audio at `A1` and the operator
  confirmed squelch worked. This establishes the downstream path and recurring
  gate, but not timing, Menu control, retune transients, sensitivity,
  calibration, or selectivity. Later `0x0518` observations occurred after the
  operator deliberately re-entered DFU and are not application failures.
- **TXEVID-055 narrowing, 2026-10-13:** the separate pinned K1/BK4829 and
  K5 V1/BK4819 effect matrices establish source-declared PA/RX controls and
  bias-table locations, not physical polarity, safe unknown-state shutdown,
  conducted power, or an RF envelope. Source preparation already enables chip
  TX; source bias follows PA enable. Neither sequence qualifies AFIK's shared
  contract. `EVID-TX055-001..004` and `RISK-040` record the remaining gates;
  no target TX input was qualified and this risk remains open.

- **Re-scope, 2026-10-13 (`ADR-096`):** Armel's proven K5 V1 TX paths are
  accepted as register/board authority for a first lowest-power
  key-up (`TXK5-058`). This risk no longer blocks K5 TX wiring.

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
- **APP-052 software narrowing, 2026-08-13:** the K5 host control witness is
  one fixed-session request followed by one bounded response. It validates the
  existing CRC/footer/marker, exact `0x0620` response command and lengths, and
  bounded payload fields. It has no retry, EEPROM, persistence, TX, or flash
  path. Fake-transport and parser tests do not establish physical serial
  interoperability, target timing, or normal-mode boot.

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
- **2026-10-14 (`7.0.10`):** the size-optimised K1 build went blank with
  6,976 bytes of headroom. `-Z emit-stack-sizes` showed a 4,184-byte serial
  task poll frame calling a 2,728-byte `publish`: under `opt-level = "s"` the
  task's by-value service and image buffer, and the by-value arena in
  `publish`, sat in stack frames. `7.0.11` fills the arena in place and keeps
  the serial task's large state in statics (poll frame 1,768 bytes). The
  headroom floor alone did not catch this; per-frame sizes are the measure.
- **2026-10-14 (`7.0.14`):** the same pattern returns whenever a target task
  is an `async fn` that awaits a shared `radio-app` future: the wrapper builds
  the inner future in its poll frame (2,848 bytes) before moving it. Target
  tasks are plain `fn`s returning the shared future, which leaves the
  interface poll frame at 1,024 bytes and the serial one at 1,776.
- **Mitigation:** `crates/radio-firmware-k1/stack-headroom.x` asserts a 6,144-byte
  reserve at link time, so a build which eats the headroom fails to link rather
  than packaging; `tool/verify-k1-image.sh` checks the same bound and
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
- **Current facts:** the exact unit has frozen and has rebooted while a host was
  sending. The behaviour also occurs on `AFIK-K1-5.6`, so it predates
  `CTRL-044`. A host-provoked reboot on `AFIK-K1-6.0` reported the `RCC_CSR`
  software-reset flag; the panic handler was the image's only caller of
  `sys_reset`, so that reboot completed the panic handler. A cold boot reported
  the power-reset flag, and the flags were cleared after every read.
- **Reporter result:** `AFIK-K1-6.1` showed that `.uninit` reads as newly
  initialised after every application reset because the vendor bootloader runs
  first and uses that RAM. The file-and-line reporter added in `5.8` therefore
  cannot work on this unit. `DOC-045` removes that dead path while retaining the
  useful panic-to-reset recovery and reset-cause display.
- **Protocol evidence:** host tests pass two million arbitrary bytes and six
  hundred thousand well-formed frames through the decoder, every service and
  command, and both the ordinary and two-phase control paths without a panic.
  This is strong negative evidence for those pure paths, not proof about the
  target UART, executor, DMA, or task interaction.
- **Withdrawn observations:** `MEM` after a reboot did not show that the panic
  handler had been skipped; the bootloader had destroyed the report. A display
  row was also misread as nine received bytes when it actually said
  `RX000 TX000 D000 E000`. No observation currently establishes that an
  application-mode UART byte reached the firmware.
- **Inconclusive bisection:** `AFIK-K1-6.2D` dropped bytes before parsing and did
  not reset, but its counters also showed no received bytes or receiver errors.
  If no byte arrived, neither variant exercised the code at the cut, so the
  result does not locate the panic.
- **Not established:** whether host traffic causes the panic rather than merely
  correlating with it, whether any byte arrives, where the panic occurs, or
  whether every observed freeze has the same mechanism as the proven reboot.
- **Required before more bisection:** fix and record the cable, seating, and
  power supply, confirm the handset remains responsive, then take one `RX` and
  `E` reading while the host sends. One evening produced a freeze, a reboot,
  and neither under nominally identical traffic; another firmware cut before a
  repeatable bench would measure the setup rather than the code.

## RISK-037 — "V1" names a generation, not a bootloader or a memory part

- **State:** open, not scheduled
- **Current facts:** `EVID-K5-012` records a V1-generation UV-5R Plus, running a
  DP32G030 build, whose bootloader beacon is `4.00.01`. `EVID-K5-013` records a
  UV-K6 from the same set, running the same build, whose beacon is `2.00.06`. Two
  bootloader versions, one generation, one working firmware. The qualified
  flasher path accepts only `2.*` and encodes one target as inlined constants:
  the confirmation phrase in `radio-flasher`, its duplicate in the CLI parser and
  help, and a hardcoded 240-page write loop.
- **Open defect:** per `EVID-K5-013`, that path currently rejects **both** units,
  including the `2.00.06` one it was written for, because it deobfuscates a
  trailer that a radio in bootloader mode sends literally. It fails closed, so
  nothing is at risk, but no V1 unit can be classified until it is fixed. The fix
  accepts both trailer forms keyed by mode, per the corroborated rule in that
  entry, and tests with real captured frames rather than host-encoded ones.
- **Wrong discriminator:** per `EVID-K5-015`, the family is signalled by the
  beacon command and AFIK gates on the version-string prefix instead. Fixing the
  trailer alone still leaves `4.00.01` refused.
- **Taxonomy is incomplete:** per `EVID-K5-016`, no reviewed external project
  records a `4.00.*` bootloader, and none can query the processor. The published
  V1/V2/V3 tables cannot be used to decide what this unit is, in either
  direction.
- **Settled by hardware:** `EVID-K5-017` closes the question of whether a version
  could stand in for a hardware fact. Two of these radios share a production
  block and ship `2.00.06` and `4.00.01`. The version is a build description, and
  no taxonomy built on it can be correct.
- **Memory capacity is unreachable where it is most needed:** per `EVID-K5-018`,
  the unit with a hand-fitted part has a broken display and cannot be asked in
  bootloader mode what is fitted. An explicit-offset read-only read against normal
  firmware is the prerequisite for probing it, and until then no build may assume
  a capacity for that unit.
- **Why it matters:** the existing constants describe one bootloader on one
  generation while reading as a claim about the family. A radio that is V1 by
  marking and DP32G030 by silicon can still refuse, or worse silently accept,
  a workflow qualified against `2.*`. Supporting a second bootloader means
  making the target a descriptor rather than adding a constant beside the
  existing ones.
- **Also unresolved:** the fitted configuration memory is not identifiable. The
  V1 store is I²C 24Cxx, which has no identification command, so capacity can
  only be inferred behaviourally from address aliasing and the addressing class
  from which device addresses acknowledge. `radio-eeprom` is a SPI NOR driver
  for the K1's PY25Q16 and does not apply; no I²C driver exists. The user
  reports a V1 unit with a different same-family part fitted by hand, so
  capacity is a real variable and not a constant.
- **Not established:** whether `4.00.*` shares the `EVID-K5-011` page protocol,
  the application origin and length on this unit, the fitted memory part on any
  V1 unit, and whether the beacon's per-unit identity field distinguishes
  generations.
- **Required before any V1 write path:** classify `4.00.*` read-only, establish
  the page protocol and geometry independently of the `2.*` inference, and give
  the target its own confirmation phrase. Any per-unit sizing must be verified
  by the running image against the hardware, because a profile claiming more
  memory than is fitted writes past the end of the calibration a resoldered
  unit cannot restore.
- **Shared operator UI narrowing, 2026-08-13:** the pure shell state machine and logical screen navigation now live in `radio-platform::operator_shell`. K5 uses the same operating/channel/source/Info/Settings/SquelchList navigation with explicit capability gates for its fixed PMR bank; VFO and scan remain unavailable rather than inferred. Handset squelch is a volatile acknowledged target effect, not persistence. K1/K5 share deterministic display pixels, glyphs, and layout; framebuffer storage, page writes, physical key scanning/debounce, and board timing remain target-owned. Host tests and an unflashed image do not establish physical K5 UI appearance, key ergonomics, persistence, or display parity.
- **Shared-pixel and monitor boundary, 2026-08-13:** the K5 adapter now emits
  the shared K1-layout frame and skips unchanged framebuffer writes, and the
  K5 matrix maps Side2 to the shared monitor effect. The same target now exposes
  Side1 Settings/SquelchList through the acknowledged shared application path;
  rejected or faulted choices leave both displayed and semantic settings
  unchanged. This removes the old target-specific PMR redraw approximation and
  proves only deterministic software behavior. Physical K5 pixel appearance,
  Side2 electrical decoding, audio audibility, and Menu/list ergonomics still
  require a separately authorized acceptance run; no K5 persistence is claimed.
- **APP-052 review corrections, 2026-08-13:** shared frames now distinguish a
  receiver fault from a healthy quiet channel, and K1 common PMR monitor state
  is synchronized through `SetMonitor`; arbitrary K1 channels remain outside
  that shared app boundary. Menu is shell-owned list confirmation and cannot
  silently toggle audio through serial; explicit audio command `0x0615` is
  software-only until physical normal-mode evidence. Physical fault-screen
  legibility, monitor audibility, and side-key electrical behavior remain
  unproven.
- **Common serial ownership narrowing, 2026-08-13:** K5 serial Up/Down/Side2
  parity is covered against the shared shell and ReceiveApp fixture. K1's
  banked controller and shell are owned by another task, so K1 now advertises
  view-only common serial access and refuses key/audio mutations; this avoids
  claiming that a serial request changed only the detached ReceiveApp. K1/K5
  physical serial interoperability and K1 controller forwarding remain open.
- **K5 display transport correction, 2026-08-13:** physical `2.0P` application
  boot and normal serial were observed with a left crop and random right-edge
  pixels. `2.1D` shifts only the target page transport to columns `4..=131`;
  the user then confirmed clean display edges and the requested normal screens
  and controls after the guarded `2.1D` write. This is acknowledged page-write
  and operator-observation evidence, not read-back or `2.2Q` evidence. The
  unflashed `2.2Q` package has only static/simulator proof; APP-052 remains
  open.

- **APP-052 Milestone 2 persistence model, 2026-08-14:**
  `radio-storage::dual_slot` now models two relative slots, commit-last
  tombstone-first publication, expected-byte protection, verified reads,
  corruption fallback, and deterministic power-loss faults. Its 1,280-byte canonical-image bound
  and relative block indices are software allocation choices only. The
  retained backup is not allocation evidence; no absolute address, disposable
  range, physical power-loss result, restore proof, K5 adapter, persistence
  capability, or hardware write is claimed. The model is 2,624 bytes total
  with caller-owned bounded scratch; it is ready for simulator object
  transport, while physical allocation and restore evidence remain open.

- **APP-052 Milestone 3 simulator boundary, 2026-08-14:** the K5 extended
  envelope, explicit object transport, canonical-image commit, deterministic
  reboot, and dual-slot fault tests are host/simulator evidence. They do not
  establish the K5 normal-mode envelope, an EEPROM address or disposable
  allocation, target timing, power-loss behaviour on hardware, persistence or
  restore capability, or a safe flash route. The target main and capability
  bits remain unchanged. Simulator durability failures now fail the protocol
  commit and restore the complete prior service snapshot; the exact durable
  error is replayed without another write, and typed reboot errors preserve
  service on backend read failure. Before any target adapter, obtain safe allocation
  evidence and complete bounded sizing/static checks. APP-052 remains open.

- **K5 guarded restore boundary, 2026-08-14:** `restore-eeprom` now models the
  source-backed stock normal-mode `0x051D` eight-byte write and `0x051E` offset
  response behind explicit target/current-image/recovery/restore gates. A
  complete live backup is compared and preserved before any write; the recovery
  source length and trusted out-of-band SHA-256 are checked before transport;
  each changed block is stable-checked with two immediate reads, then compared and read back,
  and the whole image is read back at the end. The vendor command has no atomic
  expected-byte compare, so exclusive serial ownership and a quiescent stock UI
  are required; the double read only narrows the race.
  Fake serial tests cover interruption and malformed responses. This remains
  host software evidence only: no hardware restore, exact current-unit restore
  rehearsal, power-loss result, or disposable address exists. The retained
  exact-unit K6 copies (8192 bytes; hashes retained privately) and
  separate 60,612-byte recovery artifact (hash retained privately) remain external and were not
  used by tests. No 2,624-byte relative model region is allocated; the next
  risk decision is deliberate vendor-memory repurposing.

## RISK-038 — Complete TX inputs still depend on authoritative adapters

- **State:** open for `TXAUTH-053` / `TXCTRL-054` target integration;
  `TXEVID-055` evidence review completed without qualifying target inputs
- **Impact:** a caller can misrepresent selected state, reuse configuration
  identity, or supply fabricated hardware/calibration/operator facts. A pure
  software policy cannot measure or authenticate those facts.
- **Mitigation:** default-deny missing/invalid inputs, borrow immutable policy
  and request, compare the complete fresh request before logical TX entry,
  require non-reused instance/revision identities, and keep synthetic fixture
  envelopes out of firmware. Physical adapters require independently recorded
  evidence and a separately tested PTT/fault/timeout shutdown lifecycle.
- `RISK-007` remains the physical TX evidence gate. No PA, microphone, power
  calibration, safe target shutdown, or physical transmit claim follows from
  the authorization milestone.
- **Software result, 2026-10-13:** `TXAUTH-053` removes class-only authority,
  compares all current request facts, borrows policy/request/controller state,
  and rejects missing or mismatched unit-bound hardware/calibration limits.
  Behavioral and compile-fail tests pass. Configuration identity is still an
  adapter contract: supplying an old snapshot cannot reveal a real-world
  change. `TXCTRL-054` now supplies an explicit-time deadline, exact-request
  rechecks at each entry effect, and ordered logical PA/TX cleanup. The logical
  chip model still does not apply physical conducted power or own a target PA.
  Trusted facts, bounded physical execution, and independently evidenced target
  work remain required. Risk remains open.
- **Evidence result, 2026-10-13:** neither per-target source bias table proves
  an integer conducted-power mapping or a target/unit-bound calibration
  envelope. The SPI NOR K1 and I2C EEPROM K5 maps are distinct; backup bytes,
  low/mid/high labels, product frequency ranges, and receive profiles do not
  supply missing authority. Source pins are not an independently authenticated
  target identity. Hardware/calibration inputs remain absent and denied.

- **Re-scope, 2026-10-13 (`ADR-096`):** the K5 hardware/calibration envelope
  is declared in firmware from the unit's low calibration point and the
  operator's frequency, with a nominal milliwatt figure. That is accepted for
  bench bring-up; the adapter-authority concern remains for later widening.

## RISK-039 — PTT scheduling and shutdown acknowledgements require target proof

- **State:** open; host/simulator contract established by `TXCTRL-054`
- **Impact:** a blocked executor, missed PTT/deadline input, unbounded adapter
  operation, detached old effect, or fabricated success acknowledgement can
  leave physical TX active despite correct shared sequencing. A failed PA/TX
  disable cannot prove that either output is physically off.
- **Mitigation:** borrowed synchronous effects, one exact acknowledged operation
  at a time, explicit monotonic time, deadline from accepted press, fresh policy
  checks before each entry operation, startup shutdown, unconditional fault
  cleanup, and latched failures without automatic retry. Simulator tests cover
  rejected operations and uncertain partial completion at every effect boundary;
  receive restoration is blocked if either shutdown completion failed.
- **Required evidence before target wiring:** authoritative fresh snapshots and
  non-reused app instances; bounded execution and deadline delivery; safe PA/TX
  off from every relevant target state; independent fail-off/watchdog behavior
  where the scheduler cannot meet that guarantee. Record exact-unit observations
  and confidence under `docs/hardware-evidence.md`; no such fact follows from
  virtual-time or logical register tests. `RISK-007` remains the RF evidence gate.
- **TXEVID-055 result, 2026-10-13:** both current physical buses return
  `Infallible` software completion; emitting edges does not establish physical
  PA/chip shutdown. Source RX interrupt-clear loops are unbounded. K1 MCU
  panic-reset and K5 panic-spin/loop-count UI time do not establish monotonic
  TX deadlines or fail-off. No execution/acknowledgement bound was measured;
  the proposed checks are in `RISK-040`. Risk remains open.

- **Re-scope, 2026-10-13 (`ADR-096`):** the "required evidence before target
  wiring" list is no longer a gate for a first K5 bench key-up. The short
  timeout and the bench observation in `TXK5-058` suffice; measured latency and
  scheduler-independent fail-off move to `TXHARD-061`.

## RISK-040 — Source PA controls and bias do not qualify target TX effects

- **State:** open, re-scoped by `ADR-096` (2026-10-13). Items 1–3 below are
  covered for a first bench key-up by Armel's proven normal paths plus the
  `TXK5-058` bench observation; items 4–5 are follow-up hardening
  (`TXHARD-061`), not gates. See the re-scope note at the end of this entry.
- **Impact:** both pinned targets control PA enable through the chip bus and a
  cached whole `REG_33` output word. A broken bus or stale shadow can defeat
  shutdown or alter another RF output. The source's named preparation already
  enters chip TX and applies bias after PA enable, so mapping functions by name
  violates AFIK's non-energizing preparation and power preparation before enable.
  Calibration bytes do not establish conducted milliwatts or safe RF limits.
- **Current facts:** `EVID-TX055-001..004` in `docs/hardware-evidence.md` record
  pinned source statements, exact-unit receive observations, and separate K1
  and K5 V1 effect matrices. Source-declared asserted/cleared PA bits are not
  observed PCB polarity. Literal `REG_30 = 0` is a candidate idle operation,
  not a measured unconditional physical TX-off guarantee. Zero-bias helper
  calls still set the source's PA-control enable field. Both current AFIK
  physical adapters return `Infallible`; they cannot observe an electrical
  write failure merely by returning `Ok`.
- **Mitigation:** no target TX wiring or hardware/calibration envelope is
  introduced. Keep unknown target inputs denied, qualify shutdown before
  energizing work, retain the shared fault latch and fresh authority checks,
  and keep source evidence separate from measurements. Do not transplant
  either source's function, shadow word, timing, power table, or TX mode word.
- **TXPLAN-056 source result, 2026-10-13:** `EVID-TX056-001` identifies a
  package-label distinction: the selected K1 BK4829 datasheet places GPIO1 on
  pin 1, while the inherited source symbol says `PIN29`. Armel's helper uses
  enum value 1 to select the GPIO output bit, not package pin 29; its proven
  implementation is not contradicted. GPIO0 and GPIO4 labels also differ from
  the BK4829 document. Match the package and board before selecting probe pads.
  `EVID-TX056-002` establishes no qualified exact-board schematic or independent
  off route from the inspected inventories and generic chip reference drawing.
  No electrical polarity, register behavior or physical measurement is added.
- **Operator correlation, 2026-10-13:** `EVID-TX056-003` maps each hardware
  conclusion to the selected K1 Armel driver/callers and a separately pinned
  Armel K5 V1 driver/callers. Accept their proven normal operating sequences
  as trusted hardware evidence. Zero bias plus cleared PA enable is a deliberate
  proven receive state; its control-enable bit is not a defect. Remaining
  unknown-state, bus-failure and latency guarantees are specific gaps beyond
  that normal-path evidence. No register behavior is invented to fill them.

Required evidence and proposed experiments, separately for the exact K1 and
the exact K5 V1 unit, in this order:

1. **Identify and bound the off path before any energizing experiment.** Record
   board/chip revision, applicable original register documentation or clearly
   bounded trusted source evidence, PA/RX/filter wiring, output direction,
   electrical asserted/deasserted levels, bias behavior, and reset/bootloader
   ownership. Prepare a non-transmitting acceptance plan and measurement
   points, with recovery and preserved applicable calibration data. A later
   separately authorized capture must establish PA/chip-off at cold start,
   warm reset, held PTT, unknown mode, partial preparation, target fault, and
   repeated shutdown. Establish a complete safe output-state strategy without
   reusing an unknown shadow or guessing a whole-register word. Record the
   off criterion, instrument detection limits and measured decay. Software
   zeros, speaker mute, status text, and register read-back alone do not prove
   RF silence. A non-transmitting plan cannot establish active-TX decay;
   that remains for a later contained, separately authorized test.
2. **Bound RF routing and non-energizing preparation.** From the identified
   board, establish PA-off while changing TX frequency, modulation/bandwidth,
   microphone, bias/gain and RF path; capture the proposed preparation without
   asserting chip TX or PA. Record required settling/calibration status and
   prove no energizing write or routing transient. The source's prepare function
   and its 10/5/10 ms delays cannot supply that proof. This needs its own
   authorized plan; no candidate register sequence is prescribed here.
3. **Qualify cleanup and current receive restoration.** Exercise each partial
   operation and shutdown completion boundary on the target, initially without
   TX. Verify PA-off before unconditional chip-off, no receive restoration when
   either is uncertain, and the second cleanup pair after failed restoration.
   Restore the current full configuration, including RX frequency, target chip
   profile, modulation/bandwidth, tones, squelch, chip audio and speaker state;
   a former TX request must not restore an obsolete selection. Later contained
   TX tests must cover the same paths from active TX and all supported
   modulation/tone states. Record physical outcomes independently of adapter
   return values, including how completion is established and faults detected.
4. **Measure an exact-unit power/modulation envelope only after off is proven.**
   Validate the relevant calibration map/table provenance without committing
   private bytes. With separately authorized contained RF equipment, a safe
   dummy load, spectrum/power measurement and current limiting, bound conducted
   milliwatts, bias/gain mapping, transient overshoot, deviation, occupied
   bandwidth, spurs and supported RF ranges across the qualified battery/load
   conditions. Use separate non-reused evidence/unit identities for hardware
   and calibration limits. A label, bias byte, regional ceiling or receive
   result cannot substitute for these measurements.
5. **Measure deadline delivery and independent fail-off.** Define the allowed
   worst-case release/deadline-to-physical-off latency, including synchronous
   effect time; measure it under serial/display/storage load and injected
   stalls. Verify genuine monotonic time and held-PTT release interlocks.
   Establish what de-energizes the PA and chip when the shared bus or executor
   is unavailable, during panic/reset/bootloader handoff, and under watchdog
   or brownout. Qualify watchdog coverage of external RF controls, not just
   MCU restart. If an independent off route or the required bound cannot be
   established, target TX remains denied.

`TXPLAN-056` provides the separate target plans in `docs/tx-shutdown-plan.md`.
No case has run. The operator reported K5/K6 attached in flash mode and
authorized required flashing and prompt real-hardware testing on 2026-10-13.
Next is `TXOFF-057`, a source-backed receive-only K5 off/idle/receive witness;
boot/register observations do not require later RF power qualification first.
Instrumented fault/active-TX/power/fail-off measurements remain pending and
physical TX remains outside the authorized witness. `APP-052` remains paused
and `RISK-036` remains deferred by operator decision.

**Re-scope, 2026-10-13 (`ADR-096`, operator course correction).** The five
items above were written as gates before any energizing work. They ask for more
than the stock firmware or Armel ever established, and they were producing
documents rather than progress. They now apply as follows:

- **Items 1–3 (off path, preparation, cleanup/restore):** satisfied for a first
  lowest-power key-up (dummy load, or brief PMR446) by `EVID-TX056-003` plus
  host/simulator tests of `tx_control` order and failure cleanup, and an
  SDR/second-radio observation that the carrier stops on release and timeout (`TXK5-058`).
  AFIK's non-energizing preparation split is a design goal, not a precondition
  for following Armel's proven enter order.
- **Item 4 (power/modulation envelope):** first key-up uses only the lowest
  reference power level derived from the unit's own calibration, with a nominal, labelled milliwatt figure.
  Measuring the real envelope is part of widening power levels (`TXK5-059`).
- **Item 5 (deadline latency, independent fail-off, watchdog):** follow-up
  hardening in `TXHARD-061`. The first key-up relies on the short timeout and
  the operator holding the radio on the bench.

What still matters: the lowest power, a dummy load or a frequency the operator
may use (PMR446 for `TXK5-058`), a short timeout, and actually seeing the
carrier stop. If the carrier does not stop on release or timeout, stop and fix
that before anything else.

**Update 2026-10-14 (`TXRANGE-062`):** Mid and High are now offered, from the
unit's mid and high calibration rows as the reference reads them
(`radio.c:456-482`), with Armel's nominal 2 W and 5 W. They are unmeasured;
the hardware envelope rises to 5 W and each programmed range's ceiling still
applies, so PMR446 stays at 500 mW. Measure the real output on a dummy load
before relying on it (experiment 2). A level above the range's ceiling is
lowered to the highest level within it (operator request), by nominal figures.

## RISK-041 — Dual watch timing and the two-slot screen are unobserved

- **State:** open (2026-10-14, `DUALW-064`).
- The receiver retunes every scan dwell and judges a slot busy on settled
  readings only, as a scan does. On the K5 a 90 ms dwell heard nothing on the
  other slot and 200 ms works (`EVID-K5-036`, now the default); the floor
  between them, and whether 200 ms misses short transmissions, is unmeasured
  (`RISK-008`). PMR446 slots retune through the shared receive app each time.
- The two-slot layout and S meter were checked only as host-rendered pixels.
- Experiment: with dual watch on, key a second radio on the other slot and on
  the selected one; check the hold, the 3 s hang, the meter, and that PTT
  during dual watch transmits on the selected slot.

## RISK-043 — S meter accuracy is unmeasured

- **State:** open (2026-10-14, `SMETER-065`).
- The meter applies egzumer's per-band corrections to the chip's RSSI and
  reads it on the IARU S scale (`EVID-SMETER-065-001`). Whether these units
  read within a few dB of the true carrier is unknown, and AM fix gain is
  not compensated.
- Experiment: feed a signal generator (or a known-attenuated carrier) at
  -93 dBm and -73 dBm on 145 and 446 MHz; expect S9 and S9+20 within one S
  unit, and record the error per band.

## RISK-044 — TX outside 70 cm is unmeasured

- **State:** open (2026-10-14, operator request).
- The hardware envelope is now every calibrated Armel band, 18 MHz to
  1.3 GHz less the chip's 630–840 MHz gap, as the reference allows with its
  frequency lock off; the programmed ranges decide where TX goes. The bias
  comes from the unit's own rows for that band, so an unprogrammed band is
  refused. Output power, harmonics and spurious emissions are unmeasured on
  every band but 70 cm, and the K5's PA and filters are designed for 2 m and
  70 cm only: elsewhere expect little power and strong harmonics.
- Experiment: on a dummy load through an attenuator, key Low1, Mid and High
  on 145.500 MHz and record power and the second and third harmonics; then
  one frequency per other programmed band before any range there goes on air.

## RISK-045 — Broadcast receive and the K5 `opt-level=z` image are unobserved

- **State:** closed (2026-10-16): broadcast observed on both radios.
- The BK1080 sequences are the reference's, but AFIK has not yet driven the
  chip on either board: the K5's shared PA10/PA11 bus with the keypad, PB15's
  polarity, and the K1's PF5/PF6 timing are untested. A bus failure only
  silences broadcast; the BK4819 path is untouched.
- The meter reading assumes the BK1080 RSSI is in dBuV (`dBm = dBuV - 107`);
  the station test is the reference's, so a weak station may stay squelched
  unless the radio-wide squelch is open or the operator monitors.
- The K5 image is now built at `opt-level=z` (about 3 KiB smaller than `s`),
  which changes code for the whole image. Bit-banged delays are spin loops
  and timers come from SysTick, so timing should hold, but slower code paths
  (display, scan, dual watch) have not been watched at `z`.
- 2026-10-15, `7.0.34`: the K5 needed 1.7 KiB more for the F 4 catch. Now
  Embassy ticks at 1 kHz (timestamps are whole milliseconds), the static
  index is zero-initialised, names skip UTF-8 validation, and `memcpy`,
  `memmove` and `memset` are byte loops (`tool/test-k5-mem.sh` runs them
  under qemu-arm). 792 bytes are left; anything that copies a lot is now
  slower, and none of this has run on the radio yet.
- 2026-10-15, K5 `7.0.34` observed by the operator: the JO02eh FM bank
  plays (Classic FM 101.5 among others) with no beat, steps and scans, and
  ordinary receive including GB3EL is unchanged. The K5 half is retired; the
  K1 (PF5/PF6) has not run broadcast yet.
- 2026-10-16, K1 `7.0.38` observed by the operator: broadcast plays once the
  VHF LNA is selected (`98a8fec`; the K1's BK1080 hears the antenna through
  it), and PMR446 squelch shuts on noise. Closed.
- Experiment (done): flash the K1; tune an FM broadcast bank, listen, step and
  scan it, check the meter against a known strong station; on the K5 also
  check keypad, display, serial programming, scan and dual watch behave as on
  `7.0.27`.

## RISK-046 — CTCSS/DCS on air and the receive tone scan are unobserved

- **State:** open (2026-10-15, `TXK5-059` step 2).
- TX tones, tone squelch and repeater splits follow Armel's calls
  (`EVID-TX059-002`) but no tone has been decoded by another radio, and tone
  deviation is unmeasured. Showing a tone heard on an untoned channel runs
  Armel's scan-mode `REG_51` during ordinary receive; whether `REG_68`/`REG_69`
  report there, and whether that word changes receive audio, is unknown.
- Experiment: key a 70 cm repeater channel and confirm it opens and the SDR
  shows the input frequency and tone; listen on a toned channel with and
  without the tone; receive a toned and a DCS signal on an untoned channel
  and check the meter row shows them and audio is unchanged.
- 2026-10-15, `7.0.30`: the K5 opened GB3EL (heard on another radio), so
  TX tone and split work; its toned output then went silent on the untoned
  channel while the tone scan `REG_51` was in force. `7.0.31` drops the scan
  from ordinary receive; it was still silent. `7.0.32` sets RX enable on
  every receive setup, which no earlier AFIK image did, and the K5 now
  hears GB3EL's kerchunk and tail (`EVID-RX-067-001`). Still open: tone
  deviation, tone squelch and DCS on air.

## RISK-047 — Airport frequencies are volunteer data

- **State:** open (2026-10-15, `AIRPORT-068`).
- OurAirports is crowd-maintained; a listed frequency may be stale, or a
  navaid or company frequency filed under an airport. The studio says so
  when it adds a bank. Receive only, so a wrong entry costs a silent channel.
- Experiment: compare the bank for a familiar locator against the national
  AIP and listen to each channel.

## RISK-048 — Marine channels on OpenStreetMap are sparse volunteer data

- **State:** open (2026-10-16, `MARINE-070`).
- Most UK ports, VTS and harbour channels are not mapped, and a tag may be
  stale or name a channel for another use. The studio always adds the core
  and says the local channels are volunteer data. Receive only without a
  marine range, so a wrong entry costs a silent channel.
- Experiment: compare the bank for a familiar coastal locator against the
  harbour's published channels and listen to each.
