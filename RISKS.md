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
