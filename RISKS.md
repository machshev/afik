# Risks and unknowns

## RISK-001 — DP32G030 evidence is not yet established

- **State:** open
- **Impact:** target startup, memory map, peripheral drivers, and Renode models
  cannot be implemented responsibly.
- **Mitigation:** Work Package 1 remained hardware-independent. A later evidence
  task must cite datasheets or measured behaviour in
  `docs/hardware-evidence.md` before target code is written.

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
- **Mitigation:** model dual-slot headers, generations, and fault injection in
  Work Package 4 after physical storage is identified.
