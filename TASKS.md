# Tasks

## FOUND-001 — First architecture milestone

- **Status:** complete (2026-08-05)
- **Objective:** establish the workspace and demonstrate one generated-bank
  configuration round trip through the programmer and simulated device.
- **Scope:** `radio-domain`, `radio-channel-plan`, `radio-storage`,
  `radio-tx-policy`, `radio-protocol`, `radio-programmer`, `radio-sim`, CI,
  Nix/direnv development environment, and foundation documents.
- **Dependencies:** none.
- **Assumptions:** host Rust is sufficient; no target register or packaging
  facts are required; simulated storage limits are declared capabilities.
- **Likely files:** root project files, `crates/radio-*`, `.github/workflows/ci.yml`,
  and architecture/protocol/storage/simulator/TX-policy documents.
- **Tests required:** checked domain arithmetic, plan bounds, storage transaction
  isolation, COBS/CRC framing and recovery, programmer/simulator negotiation and
  bank round trip, deterministic traces, default-deny/corrupt-policy tests.
- **Acceptance criteria:** workspace and host tests pass; embedded core crates
  are heap-free and `no_std`; framing recovers after malformed packets;
  capability negotiation and generated-bank transaction/read-back succeed;
  identical runs produce identical traces; invalid TX state fails closed.
- **Completion notes:** created the locked Nix/direnv workspace and seven
  crates; implemented checked domain types, linear generated banks, bounded
  object encoding, isolated candidate transactions, COBS/CRC framing,
  fail-closed TX permissions, offline compilation, capability negotiation,
  in-memory byte transport, simulated validation/commit/read-back, fragmented
  receive handling, and deterministic traces. All acceptance checks recorded
  in `STATUS.md` pass, including a separate Rust 1.86 test run.

## PROTO-002 — Expand object and transaction protocol coverage

- **Status:** complete (2026-08-05)
- **Objective:** add list, multiple-object read/write, abort, and explicit error
  response coverage without changing the transport contract.
- **Scope:** host protocol/programmer/simulator only.
- **Dependencies:** `FOUND-001`.
- **Assumptions:** first milestone wire-format decisions remain provisional.
- **Likely files:** protocol, programmer, simulator crates and protocol docs.
- **Tests required:** multi-object ordering, abort isolation, duplicate sequence,
  unsupported service/command, and malformed-stream recovery.
- **Acceptance criteria:** deterministic complete command/error matrix.
- **Completion notes:** bounded `LIST_OBJECTS` paging is implemented with
  generation and total consistency, strict stable-key ordering, negotiated
  host validation, deterministic simulator ordering independent of insertion,
  and protocol/simulator integration tests. `read_configuration` now reads all
  listed objects in stable-key order, checks descriptor lengths, and confirms
  the generation/listing afterward; an out-of-order three-object write is
  verified end to end. Explicit encoded abort discards a staged replacement,
  preserves active data/generation, and allows a subsequent transaction. The
  transaction-state matrix covers already-open, missing/wrong transaction, and
  commit-before-validation responses without active mutation. Candidate
  validation and capacity failures are abortable and preserve complete active
  snapshots. A table-driven encoded request matrix covers unsupported services,
  wrong-service commands, invalid flags, malformed payloads for every command
  family, out-of-range listing, and missing objects without active mutation.
  Immediate identical retries replay one cached response without repeating
  mutation, while conflicting sequence reuse is rejected explicitly. Combined
  stream tests recover deterministically from unknown wire values, invalid CRC,
  malformed COBS, and overflow before accepting a one-byte-fragmented frame.
  All acceptance checks recorded in `STATUS.md` pass on the pinned environment
  and Rust 1.86 minimum toolchain.

## DP32-003 — Minimal DP32G030 Rust image and Renode boot

- **Status:** complete (2026-08-05)
- **Objective:** establish the smallest evidence-backed DP32G030 target image
  and prove its reset path in a deterministic Renode machine.
- **Scope:** source-backed CPU and memory facts, one `no_std`/heap-free
  `thumbv6m-none-eabi` image, its linker/vector-table contract, a CPU-and-memory
  Renode platform, an automated boot-sentinel test, and pinned build/test
  integration. No radio peripheral, board I/O, flashing, or packaging logic.
- **Dependencies:** `PROTO-002`.
- **Assumptions:** the DP32G030 v1.23 reference manual is sufficient evidence
  for the core and address ranges; Arm's Cortex-M reset contract is applicable;
  a RAM sentinel is a simulation-only boot observation and not a hardware
  register claim.
- **Likely files:** `Cargo.toml`, `flake.nix`, `.cargo/`, a new DP32G030 target
  crate, `renode/`, CI, and architecture/hardware/simulator documents.
- **Tests required:** host workspace checks remain green; the target image
  builds with the pinned Rust toolchain; ELF/vector/section bounds are checked;
  Renode starts from the vector table and observes the exact RAM sentinel; a
  negative or pre-start check demonstrates the assertion is behavioural.
- **Acceptance criteria:** every encoded hardware fact has a cited source and
  confidence; the target crate is embedded-only, heap-free, and independent of
  host crates; the minimal image fits the evidenced flash/RAM ranges; the
  Renode model contains no invented peripheral behaviour; the automated boot
  proof is deterministic and runs from the pinned environment; no hardware is
  flashed.
- **Completion notes:** recorded the DP32G030 v1.23 core, byte-order, memory-map,
  and Arm reset-vector evidence boundary; added a standalone feature-gated
  `no_std`/heap-free target crate with an exact two-entry vector table, Reset
  handler, bounded linker script, and static ELF contract verifier; added a
  CPU/flash/RAM-only Renode platform whose automated test proves the sentinel
  is zero before execution and written only after reset-from-vector startup;
  and gated the minimum Rust target build plus locked-Nix target verification
  and Renode proof in CI. Pinned host checks, Rust 1.86 host and target builds,
  static image verification, and three repeated Renode runs all pass as
  recorded in `STATUS.md`. No peripheral behaviour was modeled and no hardware
  was flashed.

## STORE-004 — Canonical configuration image and compiler round trip

- **Status:** complete (2026-08-06)
- **Objective:** define a canonical, versioned, checksummed configuration image
  and make the authoritative host compiler emit and consume it without device
  mutation.
- **Scope:** a caller-buffer image codec in `radio-storage`, stable object-key
  ordering, image integrity and structural validation, and negotiated-capability
  validation in `radio-programmer`. Documentation and host tests are included.
  Serial protocol changes, physical flash layout, power-loss durability,
  project-file parsing, and additional channel-plan encodings are excluded.
- **Dependencies:** `DP32-003`.
- **Assumptions:** the image is an offline interchange and backup container for
  a complete logical object set, not evidence for a DP32G030 non-volatile
  storage layout; existing object payload version 1 remains unchanged.
- **Likely files:** `crates/radio-storage`, `crates/radio-programmer`, storage
  and programmer documents, `DECISIONS.md`, `RISKS.md`, and handoff files.
- **Tests required:** exact image-format vectors; byte-identical output for the
  same logical objects supplied in different orders; compiler image round trip;
  empty and maximum-bounded inputs; rejection of bad magic/version/checksum,
  truncation/trailing bytes, unordered or duplicate keys, malformed objects,
  and target-capability violations.
- **Acceptance criteria:** the `no_std` storage codec allocates no heap and
  writes only to a caller-provided buffer; one canonical image has an explicit
  length and integrity contract; decoding validates the complete image before
  exposing objects; the compiler produces stable-key order and will not accept
  an image outside negotiated target bounds; existing protocol/simulator
  behaviour remains green; physical durability claims remain explicitly open.
- **Completion notes:** added a caller-buffer `AFIK` image codec with explicit
  container and object-format versions, exact lengths, canonical object
  envelopes, and CRC-32 integrity; decoding validates the complete image,
  strict keys, and all object payloads before iteration. The compiler now sorts
  by stable key, emits images, imports them back to the same objects and
  capacity report, and rechecks storage version, object count/size, write-frame
  size, and plan-encoding support. Exact vectors, order independence, empty and
  maximum-count images, corruption and structure rejection, capability
  rejection, simulator compatibility, pinned workspace gates, and Rust 1.86
  tests all pass as recorded in `STATUS.md`. Physical persistence remains open
  in `RISK-004`.

## UI-005 — Simulator-first boot UI and hidden TX permissions

- **Status:** complete (2026-08-06)
- **Objective:** establish a bounded hardware-independent display/keypad state
  machine and prove that TX permissions can be inspected and deliberately
  changed only through a hidden boot-time physical-presence workflow.
- **Scope:** a new `no_std`, heap-free `radio-ui` crate; logical key sets and
  edge events; bounded display view models; exact boot-only hidden-menu entry;
  TX-class navigation, toggle, cancel, save, and generation handling; and a
  deterministic virtual-time simulator path. Documentation and host tests are
  included. Physical key scanning, display geometry/fonts, GPIO/registers,
  backlight, target integration, non-volatile writes, serial permission
  mutation, and a TX driver are excluded.
- **Dependencies:** `STORE-004`, `radio-domain`, and `radio-tx-policy`.
- **Assumptions:** logical keys are product-level actions whose physical mapping
  will be evidenced later; the hidden gesture prevents ordinary navigation and
  demonstrates physical presence but is not an authentication mechanism; an
  in-memory simulator persistence write proves logical behavior only. Saved
  permissions become active only after a subsequent validated boot load.
- **Likely files:** `Cargo.toml`, a new `crates/radio-ui`, `crates/radio-sim`, UI,
  simulator, architecture, and TX-policy documents, `DECISIONS.md`, `RISKS.md`,
  and handoff files.
- **Tests required:** ordinary boot and post-boot keys cannot reveal the menu;
  only the exact boot gesture enters and all held keys must be released before
  editing; extra or incomplete boot keys reject entry; selectable class order
  excludes `Never`; navigation/toggle views are bounded; cancel emits no
  record; save emits one valid next-generation record; generation exhaustion
  emits none; corrupt persisted bytes load denied; saved bytes authorize only
  selected classes after a validated simulated reboot; and identical timed
  input scripts produce identical traces and bytes.
- **Acceptance criteria:** the UI crate remains hardware-independent, `no_std`,
  heap-free, and allocation-free; display output is a bounded semantic view and
  keypad input is logical edge state; the hidden menu has no runtime entry
  path; no menu action constructs `TxAuthorisation` or mutates live policy;
  only a deliberate save yields a versioned redundant CRC-protected permission
  record; invalid state and persistence data deny TX; deterministic simulation
  covers save, cancel, corruption, reboot, and trace repeatability; no hardware
  behavior or serial permission object is invented.
- **Completion notes:** added `radio-ui` with bounded logical key sets/edges,
  semantic views, exact `Menu+Back` boot-only entry, held-key release gating,
  fixed authorisable-class navigation, draft toggle/cancel/save behavior, and
  checked generation advancement. Invalid records initialize deny-all; save
  emits the existing redundant CRC-protected record without touching live
  policy; a validated reboot is required to activate it. `UiSimulator` keeps
  persisted bytes and boot-loaded policy separate and records deterministic
  virtual-time boot, input, view, action, persistence, and reboot events.
  Exact entry rejection, edge de-duplication, cancel, corruption, generation
  exhaustion, selected-class authorization, reboot-only activation, and trace
  repeatability pass in tests. Pinned host, embedded `thumbv6m`, and Rust 1.86
  checks are recorded in `STATUS.md`; physical UI remains open in `RISK-006`.

## RF-006 — BK4819 receive path and token-gated TX boundary

- **Status:** complete (2026-08-06)
- **Objective:** implement the smallest evidence-backed, post-initialization
  BK4819 receive command path and prove that every modeled transition into
  transmit mode requires a matching central-policy authorization token.
- **Scope:** BK4819 source provenance and confidence; a new `no_std`, heap-free
  `radio-bk4819` crate; bounded 7-bit-register/16-bit-value bus operations;
  standby recovery, exact 10 Hz frequency-word packing, receive-mode entry,
  RSSI/squelch sampling, token- and class-gated transmit entry, transmit stop,
  fault latching, and deterministic virtual-time simulation. Documentation and
  host/embedded tests are included. Chip reset/initialization tables, physical
  SPI/GPIO, board RF switching, crystal/calibration choice, filters, audio,
  external PA control, power levels, interrupts, physical RF behavior,
  flashing, and on-air testing are excluded.
- **Dependencies:** `UI-005`, `radio-domain`, and `radio-tx-policy`.
- **Assumptions:** the current Beken product page and mirrored Beken datasheet
  establish product/interface facts; a separately mirrored machine-translated
  BK4819(V3) application note is usable only for an explicitly bounded command
  model with recorded confidence. The fitted chip revision and application-note
  applicability are not confirmed. The command path starts from a separately
  initialized chip; the conflicting published frequency bands are not encoded
  as target limits.
- **Likely files:** `Cargo.toml`, a new `crates/radio-bk4819`,
  `crates/radio-tx-policy`, `crates/radio-sim`, RF, architecture, simulator,
  TX-policy, and hardware-evidence documents, `DECISIONS.md`, `RISKS.md`, and
  handoff files.
- **Tests required:** frequency-word exactness and non-10-Hz rejection;
  standby-first receive/write order and exact sourced mode fields; signed
  half-dB RSSI plus squelch decoding; unknown/faulted/invalid-state rejection;
  authorization class mismatch with zero bus writes; matching authorization as
  the only path to the final TX-enable write; standby transition on TX stop;
  injected bus failure at every write/read step latches fault and denies later
  TX until successful standby recovery; identical timed receive/TX/failure
  scripts produce identical traces; and no simulator TX event occurs without a
  matching token.
- **Acceptance criteria:** every register address, field, formula, inference,
  and uncertainty is recorded with source and confidence before code; the
  driver is hardware-adapter-independent, `no_std`, heap-free, bounded, and
  uses integer units; receive entry neutralizes mode before tuning and enabling
  only the documented receive blocks; receive status uses read-only sourced
  fields; the only TX entry API requires a borrowed `TxAuthorisation` whose
  class matches the channel; invalid state, mismatch, or any bus fault denies
  the TX-enable write and latches a recoverable fault; no target peripheral,
  board behavior, external PA operation, physical receive claim, or on-air
  transmission is added.
- **Completion notes:** recorded official product/datasheet provenance and a
  narrowly bounded mirrored-application-note contract, including contradictory
  published bands, exact field sources, local command-plan inferences,
  confidence, and required experiments. Added the `no_std`, heap-free
  `radio-bk4819` driver with exact 10-Hz packing, receive/status commands,
  class-bound token-gated TX, neutral stop/recovery, and fault latching at every
  logical bus failure. Added deterministic virtual-time logical-bus simulation
  with repeatable receive/failure/recovery/TX traces and proof that mismatched
  authority emits no write or TX event. Pinned host, `thumbv6m`, and Rust 1.86
  gates all pass as recorded in `STATUS.md`. No physical adapter or RF claim
  was added; `RISK-007` remains open.

## SCAN-007 — Channel activation and deterministic scanning

- **Status:** complete (2026-08-06)
- **Objective:** add the smallest bounded channel-control state machine that
  activates generated-bank channels, scans them deterministically, and denies
  controller-level TX authorization while scanning.
- **Scope:** a new `no_std`, heap-free `radio-channel-control` crate; one
  `GeneratedBank`; checked initial/manual channel selection; wraparound next and
  previous navigation; explicit start/stop scanning; configured integer dwell
  and hold durations; timer-token generation and stale-expiry rejection;
  squelch-driven hold/release state; last-signal observation; a policy-backed
  authorized-transmission bundle only from non-scanning state; and deterministic
  virtual-time integration with the logical BK4819 simulator. Documentation and
  host/embedded tests are included. Multiple banks, priority/dual-watch scan,
  scan lists and lockouts, tone detection, physical tune/settle or polling
  timing, physical signal/RF behavior, target integration, and on-air TX are
  excluded.
- **Dependencies:** `RF-006`, `radio-channel-plan`, `radio-domain`, and
  `radio-tx-policy`.
- **Assumptions:** dwell and hold durations are explicit AFIK workflow inputs,
  not measured hardware requirements; timer expiries and `SignalMeasurement`
  values are logical adapter events; a `GeneratedBank` expands every valid
  index without allocation; physical squelch and tuning remain unverified.
- **Likely files:** `Cargo.toml`, a new `crates/radio-channel-control`,
  `crates/radio-sim`, channel-plan, simulator, TX-policy, and architecture
  documents, `DECISIONS.md`, `RISKS.md`, and handoff files.
- **Tests required:** reject zero timing and invalid initial/manual index without
  partial state; exact initial activation; next/previous and scan wraparound;
  start/stop timer lifecycle; dwell expiry retunes exactly once and rearms;
  squelch-open enters/restarts hold; open hold expiry rearms without retuning;
  closed hold expiry advances; stale/cancelled timer tokens do nothing; scan
  state denies TX authorization even when policy permits; selected state yields
  a matching class-bound token only when policy permits; identical timed scan,
  hold, stop, and denied/allowed TX scripts produce identical RF/control traces.
- **Acceptance criteria:** the controller is hardware-independent, `no_std`,
  heap-free, bounded, and uses integer units; it owns no clock and changes only
  on explicit inputs; each timer arm has a bounded opaque token and stale
  expiries cannot mutate state; every activation is checked and emits at most
  one exact channel retune; scanning never constructs or exposes TX authority;
  selected-state TX goes through `TxPolicy` and the class-bound BK4819 boundary;
  simulator time is explicit and repeatable; no hardware timing, register,
  physical signal, or RF behavior is invented.
- **Completion notes:** added `radio-channel-control` with checked one-bank
  activation, wraparound manual navigation, explicit selected/dwell/hold state,
  non-zero integer timing configuration, fresh bounded timer tokens, stale and
  cancelled expiry rejection, normalized signal observation, and non-mutating
  error paths. Scanning denies policy authority; selected state pairs the exact
  channel with a class-bound token minted only by `TxPolicy`. `ChannelSimulator`
  validates the complete bank against RF frequency representation, applies
  retunes through `RfSimulator`, enforces explicit deadlines, blocks control
  during TX, resumes receive after TX, and produces repeatable control/RF
  traces. Pinned host, `thumbv6m`, and Rust 1.86 gates pass as recorded in
  `STATUS.md`; physical timing and signal behavior remain open in `RISK-008`.

## CLI-008 — Complete programmer CLI for the supported protocol

- **Status:** active
- **Objective:** provide a complete thin command-line front end for every
  programmer operation currently implemented by `radio-programmer`, over both
  the deterministic simulator and an explicit host serial-device path.
- **Scope:** snapshot-to-canonical-image support in the programmer library; a
  new host `radio-programmer-cli` library/binary; strict manual argument and
  generated-bank-spec parsing; explicit `--sim` or `--device`/`--baud` backend;
  info, list, compile, transactional write, backup, and restore commands;
  bounded image input; canonical image output with no-overwrite default and an
  explicit force option; stable line-oriented output; documented exit codes;
  a safe `stty`-configured Linux file transport without unsafe code; and
  simulator-backed command and binary tests. Auto-discovery, project-file/CSV
  formats, firmware update, arbitrary raw reads/writes, GUI, target UART
  implementation, and claims of physical interoperability are excluded.
- **Dependencies:** `SCAN-007`, `radio-programmer`, `radio-sim`, the existing
  serial protocol and canonical image format.
- **Assumptions:** Linux `stty` plus an explicitly supplied device path and baud
  can provide a host byte stream without embedding platform-specific unsafe
  calls; actual target UART pins, baud, boot mode, and interoperability remain
  unverified; the CLI describes only the currently supported generated-bank
  project model and negotiated protocol capabilities.
- **Likely files:** `Cargo.toml`, `crates/radio-programmer`, a new
  `crates/radio-programmer-cli`, programmer/protocol/architecture documents,
  `DECISIONS.md`, `RISKS.md`, and handoff files.
- **Tests required:** stable snapshot image encoding and capacity accounting;
  exact help/version/parse errors and exit codes; reject missing/conflicting
  backends, unsupported baud, malformed/duplicate bank specs, unknown class,
  oversized input, and existing output without force; simulator info and empty
  list; deterministic compile output; write receipt and read-back; backup image
  validation; restore commit; transport and device errors stay distinct from
  usage errors; binary smoke tests; existing programmer/simulator behavior
  remains green.
- **Acceptance criteria:** the CLI owns only parsing, file handling,
  presentation, process status, and transport selection; all compile, image,
  protocol, transaction, listing, and snapshot logic remains in
  `radio-programmer`; no raw object or memory mutation command exists; inputs
  and files are bounded; output replacement requires explicit intent; write and
  restore use validated object-level transactions; simulator paths are
  deterministic; serial configuration uses an explicit path/baud and fails
  clearly; no target UART or successful hardware programming claim is added.
