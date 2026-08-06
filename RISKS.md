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
- **Mitigation:** do not flash hardware until a tested backup and recovery
  procedure exists.

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
- **Mitigation:** Work Package 3 does not package or flash images. Establish the
  bootloader mask/remap, application region, image format, board revision, and
  non-destructive recovery procedure before any hardware deployment task.

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
