# Agent working agreement

This repository is a ground-up Rust radio firmware and programming platform.
Existing UV-K5 firmware is evidence, not production source: do not port, link,
or incrementally translate its application or driver code.

Before changing code, read `STATUS.md`, `TASKS.md`, `DECISIONS.md`, `RISKS.md`,
the relevant documents under `docs/`, the implementation, and its tests. Work
only on the current work package named in `STATUS.md`.

## Architecture constraints

- Keep hardware-independent radio crates independent of UV-K5 hardware.
- Embedded crates are `no_std`, heap-free, bounded, and use integer units.
- All TX paths go through `radio-tx-policy`; invalid state denies TX.
- Serial configuration uses validated object-level transactions, not raw writes.
- The programmer library owns programming logic; front ends remain thin.
- Simulation uses deterministic virtual time. Flashing hardware is not the
  primary debugging loop.
- Do not invent register behaviour. Record facts and confidence in
  `docs/hardware-evidence.md`; record unknowns and required experiments in
  `RISKS.md`.

## Task and handoff discipline

Use stable task IDs from `TASKS.md`. For each behavioural change, add tests.
After completing and verifying each step, commit it before starting the next
step. Keep each commit scoped to that step and do not defer commits until
handoff.

Before handoff:

1. Enter the pinned environment with `nix develop` (or allow `.envrc`).
2. Run `cargo fmt --all --check`.
3. Run `cargo clippy --workspace --all-targets -- -D warnings`.
4. Run relevant tests, normally `cargo test --workspace` for host work.
5. Record exact commands and results in `STATUS.md`.
6. Update task completion notes, decisions, and risks.
7. Name the next smallest actionable task.
8. Leave the workspace buildable and avoid unrelated changes.

## Proportionate process

Operator course correction, 2026-10-13 (`ADR-096`): documentation serves the
code, not the other way round.

- Every work package changes code and adds tests, and where hardware is
  involved ends with a hardware observation. Do not create evidence-only or
  plan-only work packages unless the operator asks for one.
- Proven reference firmware behaviour (`EVID-TX056-003`) is sufficient
  authority to implement and try. Do not re-derive it, and do not make
  hardening that no reference firmware provides a gate.
- Record findings in a few lines at their real scope. A `STATUS.md` entry is a
  short paragraph plus the exact gate commands and results.
- Once the operator authorizes a direction, do not re-request it for each step.
