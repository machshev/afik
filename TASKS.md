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

- **Status:** complete (2026-08-06)
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
- **Completion notes:** added programmer-owned snapshot validation, capacity
  reporting, and canonical backup encoding. Added the host-only
  `radio-programmer-cli` library and `afik-programmer` binary with strict
  backend/command/bank parsing; stable help, version, output, and exit codes;
  info, list, compile, transactional write, backup, and restore; bounded
  streaming image input; create-new output plus explicit force replacement;
  simulator and explicit `stty`/file serial transports; and exact post-write or
  restore read-back verification. Simulator, file-safety, parse/error, serial
  setup-failure, deterministic image, and process-level tests pass. All pinned
  workspace and Rust 1.86 gates are recorded in `STATUS.md`; physical serial
  interoperability remains open in `RISK-009`.

## GUI-009 — Local programmer web GUI

- **Status:** complete (2026-08-06)
- **Objective:** provide a real, readable local graphical front end for the
  complete supported programmer workflow without duplicating programming logic
  or broadening hardware claims.
- **Scope:** programmer-owned verified write/backup/restore workflows shared by
  front ends; a reusable host Linux serial transport; refactoring the CLI to
  those shared layers without behavior changes; a new host-only
  `radio-programmer-gui` library/binary; persistent simulator or explicit
  serial session; loopback-only bounded HTTP; embedded responsive HTML/CSS/JS;
  capability and object views; generated-bank editing; canonical image compile
  download; confirmed transactional write; backup download; validated restore
  upload; stable status/error presentation; per-session mutation token; and
  model, endpoint, asset, binary, CLI-regression, and workspace tests. Remote
  bind, multi-user service, authentication claims, server-side arbitrary paths,
  auto-discovery, firmware update, raw writes, unsupported project models, and
  physical interoperability claims are excluded.
- **Dependencies:** `CLI-008`, `radio-programmer`, `radio-sim`, canonical images,
  and the explicit host serial adapter contract.
- **Assumptions:** a single-user loopback web application is a practical
  dependency-free GUI on supported Linux hosts; browser assets can use standard
  fetch/file APIs; a session token and same-origin delivery reduce accidental
  mutation but are not user authentication; persistent simulator state is
  deterministic within one server process.
- **Likely files:** `Cargo.toml`, `crates/radio-programmer`, a new shared host
  serial crate, `crates/radio-programmer-cli`, a new
  `crates/radio-programmer-gui` with embedded assets, programmer/GUI/
  architecture documents, `DECISIONS.md`, `RISKS.md`, and handoff files.
- **Tests required:** shared verified write, backup, and restore return exact
  receipts/images and reject read-back mismatch; CLI behavior remains exact
  after shared-workflow/serial refactoring; GUI state persists in the simulator;
  generated-bank form parsing and duplicate/capacity failures are explicit;
  compile and backup downloads decode canonically; confirmed write/restore
  mutate and refresh generation; missing/wrong mutation token cannot mutate;
  non-loopback bind and oversized/malformed HTTP are rejected; HTML contains
  all required controls and responsive/accessibility landmarks; binary
  help/version smoke; serial setup failure remains distinct; deterministic
  endpoint scripts return identical results.
- **Acceptance criteria:** the GUI is a thin host leaf over programmer-owned
  compilation/image/transaction/verification workflows; the programmer library
  is UI-agnostic; one process retains one selected backend session; all network
  listening is loopback-only and request/header/body sizes are bounded;
  mutation requires the session token plus an explicit GUI confirmation path;
  backup/compile use downloads and restore uses bounded upload, never arbitrary
  server file paths; status is readable and deterministic; existing CLI output
  and exit contracts remain green; no target, remote-service, security, or
  physical-programming capability is claimed.
- **Completion notes:** moved verified write, backup, restore, and exact
  read-back mismatch handling into `radio-programmer`; extracted the explicit
  Linux `stty`/file adapter into `radio-programmer-serial`; and preserved the
  CLI's tested contracts over both shared layers. Added a persistent simulator
  or serial GUI session, strict bounded generated-bank parsing, canonical
  compile/backup downloads, confirmed verified write/restore, capability and
  object views, embedded responsive assets, and a single-threaded bounded HTTP
  server that accepts only loopback IP addresses. Mutation requires a random
  per-process token and explicit confirmation header. Model, endpoint, HTTP,
  asset, binary, CLI-regression, full workspace, and Rust 1.86 tests pass; local
  service and physical serial limits remain open in `RISK-010` and `RISK-009`.

## FREQ-010 — Frequency Copy research

- **Status:** complete (2026-08-06)
- **Objective:** determine what a safe AFIK Frequency Copy feature could claim,
  represent, and require before any BK4819 scan command or target integration is
  implemented.
- **Scope:** distinguish the UV-K5 Fast Copy/Frequency Meter workflow from Air
  Copy; record the FCC-filed user-visible behavior; confirm only Beken's public
  chip-level capabilities as high-confidence facts; assess the mirrored
  machine-translated BK4819(V3) scan fields and existing UV-K5-family firmware
  as separately labeled low-confidence evidence; inventory observable and
  non-observable channel properties; define a bounded receive-only result,
  explicit-input state-machine proposal, failure/timeout semantics, user review
  and storage handoff, TX-policy boundary, evidence gaps, and non-transmitting
  experiments; and issue an implementation-readiness verdict. Register-driver
  changes, target adapters, simulator register behavior, automatic channel
  writes, transmit defaults, Air Copy replication, RF emission, flashing, and
  physical-success claims are excluded.
- **Dependencies:** `RF-006`, `SCAN-007`, the logical UI/storage/TX-policy
  boundaries, official Beken product material, the FCC-filed Quansheng manual,
  and explicitly qualified secondary register/firmware evidence.
- **Assumptions:** the public product manual describes intended user behavior
  but not silicon behavior; Beken's product page confirms feature existence but
  not commands; the available V3 note and firmware descendants may guide
  experiments but cannot establish the fitted revision, board RF path, crystal,
  register preservation masks, timing, accuracy, or safe production sequence.
- **Likely files:** `docs/hardware-evidence.md`, a Frequency Copy feasibility
  document, `DECISIONS.md`, `RISKS.md`, `TASKS.md`, and `STATUS.md`.
- **Tests required:** no behavioral code is authorized. Review must trace every
  proposed field/transition to a source, label copied fact versus inference,
  reject unsupported channel properties and TX authority, and name bounded
  deterministic tests for a future implementation. Existing formatting,
  Clippy, workspace tests, and Rust 1.86 checks must remain green.
- **Acceptance criteria:** research records source identity/provenance and exact
  Fast Copy semantics; separates official capability, unverified register
  description, firmware observation, AFIK inference, and unknowns; defines a
  heap-free bounded candidate/state proposal with no transmit-capable output;
  identifies false-lock, timeout, tone/code, crystal, accuracy, cleanup, and
  board-path risks; specifies reproducible receive-only experiments and future
  simulator/fault tests; gives a clear implement/defer verdict; and makes no
  production or physical behavior claim.
- **Completion notes:** recorded the FCC exhibit identity, checksum, exact Fast
  Copy controls/results, separate known-frequency signalling scan, and distinct
  transmitting Air Copy workflow. Re-used Beken's advertised feature list only
  as capability evidence and labeled the mirrored V3 fields plus one exact
  descendant-firmware commit as low-confidence experiment-planning evidence;
  unexplained `REG_32` bits remain prohibited. Defined a bounded receive-only
  candidate, explicit-input/token state proposal, uncertainty/fault semantics,
  separate confirmed storage path constrained to `TxClass::Never`, required
  non-transmitting experiments, and a future deterministic test matrix. Verdict:
  design-ready but hardware-command blocked. No behavior, register driver,
  simulator register model, target adapter, or physical claim was added.

## APRS-011 — APRS receive feasibility and bounded repeater discovery

- **Status:** complete (2026-08-06)
- **Objective:** determine whether an AFIK target could responsibly receive
  APRS and implement the smallest standards-backed, hardware-independent path
  that discovers reviewable voice-repeater advertisements from already
  recovered packet frames.
- **Scope:** primary AX.25/APRS source provenance; a layer-by-layer feasibility
  analysis from RF input through FM/baseband access, symbol/clock recovery,
  NRZI/bit unstuffing, frame check, AX.25 UI framing, and APRS information;
  explicit board/chip/MCU evidence gaps and non-transmitting experiments; a new
  `no_std`, heap-free, allocation-free `radio-aprs` crate if supported by the
  primary specifications; bounded parsing of complete de-stuffed AX.25 frames
  with FCS, source/destination/path validation, UI/PID checks, and APRS
  object/item voice-repeater frequency fields; receive-only advertisement
  candidates; a fixed-capacity deterministic discovery table driven by
  explicit receive/expiry inputs; simulator composition; documentation and
  host/embedded tests. BK4819 modem/register commands, raw-audio DSP, ADC/DMA,
  NRZI/clock/HDLC bit recovery, interrupts/timers, target adapters, live RF,
  network directory services, automatic configuration writes, TX defaults,
  APRS transmission, igating/digipeating, flashing, and physical-success claims
  are excluded.
- **Dependencies:** `FREQ-010`, `SCAN-007`, `radio-domain`, `radio-tx-policy`,
  the existing deterministic simulator, primary AX.25/APRS specifications,
  official Beken material, and separately qualified secondary implementation
  evidence only where primary hardware documentation is unavailable.
- **Assumptions:** a complete de-stuffed frame including its FCS is a legitimate
  hardware-independent boundary; correct bytes supplied by a test or simulator
  do not prove physical demodulation; APRS advertisements are untrusted and may
  be malformed, stale, spoofed, or wrong; an advertised frequency/offset/tone
  does not convey regulatory authority or trusted plan membership.
- **Likely files:** `Cargo.toml`, a new `crates/radio-aprs`, `crates/radio-sim`,
  `docs/hardware-evidence.md`, APRS feasibility/protocol documents,
  `docs/architecture.md`, `docs/simulator.md`, `DECISIONS.md`, `RISKS.md`,
  `TASKS.md`, and `STATUS.md`.
- **Tests required:** exact sourced FCS vectors and rejection; minimum/maximum
  AX.25 address counts and malformed extension/reserved/callsign/SSID fields;
  non-UI or wrong-PID rejection; exact object/item and voice-frequency vectors;
  bounded text/numeric parsing; invalid range/offset/tone/ambiguity rejection or
  preservation as explicitly untrusted data; duplicate/newer/older/conflicting
  advertisement behavior; fixed-capacity and explicit expiry boundaries; stale
  inputs; identical input scripts yield identical tables/traces; no result can
  construct `ActiveChannel` or `TxAuthorisation`; full workspace, embedded
  `thumbv6m`, and Rust 1.86 gates remain green.
- **Acceptance criteria:** every implemented wire field and checksum rule has a
  primary citation and exact confidence boundary; the receive-chain report
  gives a clear implement/defer verdict for each layer and names equipment,
  recovery, corpus, performance, false-decode, and fault experiments; embedded
  logic is hardware-independent, `no_std`, heap-free, bounded, integer-only,
  and deterministic; lower-layer validated frames remain explicit inputs; a
  discovery candidate preserves packet provenance and uncertainty but contains
  no TX authority; table mutation is bounded and order/freshness rules are
  exact; invalid/stale/spoofable data fails closed and never mutates channel
  configuration; no physical decoder, target behavior, network data, or RF
  success is claimed.
- **Completion notes:** recorded primary AX.25/APRS/frequency-spec provenance,
  checksums, exact complete-frame and advertisement facts, conflicting path
  bounds, hardware unknowns, and a layer-by-layer verdict. Physical RF,
  baseband, BK4819 modem, MCU DSP, and bit recovery are deferred under
  `RISK-012` with a bounded receive-only experiment/corpus plan. Added the
  `no_std`, heap-free `radio-aprs` crate with strict addresses, zero-through-eight
  APRS paths, UI/PID, 1-through-256-byte information, CRC-X25/FCS residue,
  Object/Item, uncompressed position ambiguity, voice-repeater symbol, exact
  frequency, alternate input, tone/code, offset, and range parsing. Added a
  fixed-capacity kind/name/source table with explicit time, no eviction,
  same-origin kill freshness, stale/conflict handling, and explicit expiry.
  Host simulation yields identical parse/table/expiry traces and has no channel
  or RF-control connection. No modem/register command, raw-audio decoder,
  target integration, storage mutation, automatic tune, TX authority,
  transmission, or physical-success claim was added.

## FLASH-012 — Recovery-gated UV-K5 V1 firmware flashing

- **Status:** deferred (2026-08-06; software milestone complete, physical K5
  unavailable)
- **Objective:** add the smallest host and image path that can safely attempt a
  real firmware update on one explicitly identified UV-K5 V1/DP32G030 radio
  while preserving the stock serial bootloader and unit calibration.
- **Scope:** pin and record separately implemented observations of the UV-K5
  version-2
  serial bootloader; reserve `0xF000..=0xFFFF` from the target application;
  package and statically validate one complete raw application image; implement
  bounded packet framing, read-only normal-firmware EEPROM backup, bootloader
  probe/version negotiation, sequential 256-byte application-page writes, and
  exact acknowledgement checks; expose those workflows through one explicit
  Linux serial CLI at 38,400 baud; require board, backup, recovery-image, image
  identity, and destructive-action confirmations; and use a deterministic fake
  radio for malformed, timeout, ordering, rejection, and success tests.
- **Dependencies:** `DP32-003`, `CLI-008`, `RISK-001`, `RISK-002`, `RISK-005`,
  the DP32G030 v1.23 memory map, an exact physical test-radio inspection, and
  separately pinned reverse-engineered bootloader observations. Existing UV-K5
  firmware and programmer implementations are evidence only and must not be
  ported, linked, or translated into AFIK production source.
- **Assumptions:** only a physically inspected V1 board fitted with DP32G030 is
  eligible; a version-2 beacon is necessary but not sufficient board identity;
  the stock bootloader occupies the final 4 KiB and accepts 256-byte indexed
  application writes below `0xF000`; firmware flashing does not intentionally
  write the external EEPROM; and page acknowledgements prove only bootloader
  acceptance, not flash read-back or application boot.
- **Exclusions:** bootloader v5/AES, UV-K5 V2 or V3 and compatible-looking
  radios, bootloader replacement, SWD writes, arbitrary address/partial writes,
  EEPROM writes, vendor packed-image decryption, automatic reset, display,
  keypad, UART target code, RF/audio integration, transmission, and any claim
  that an AFIK image runs on silicon before a separately observable boot proof.
- **Likely files:** `Cargo.toml`, `crates/radio-firmware-dp32g030`, a new host
  flashing library and thin CLI, `crates/radio-programmer-serial`, `tool/`,
  `.github/workflows/ci.yml`, `docs/hardware-evidence.md`, a deployment guide,
  `docs/architecture.md`, `DECISIONS.md`, `RISKS.md`, and `STATUS.md`.
- **Tests required:** exact framing/XOR/CRC vectors; partial, corrupt, oversized,
  and resynchronised input; exact v2 beacon and rejection of v5/unknown beacons;
  strict firmware-version and raw-image vector/range checks; complete 8 KiB
  read-only EEPROM backup with offset/length validation; page count, padding,
  ordering, final-length, sequence, and acknowledgement/error behavior; no
  write before all recovery gates and exact image confirmation pass; stop on
  first missing/mismatched/error acknowledgement; deterministic success and
  failure transcripts; full host, Rust 1.86, target-image, package, and Renode
  gates remain green.
- **Acceptance criteria:** every physical/protocol assumption has source,
  commit, confidence, and experiment boundaries; the packaged application is
  exactly bounded below `0xF000` and cannot overwrite the preserved bootloader;
  backup and flash files are bounded, validated, and never replaced without
  explicit force; the library owns protocol and flashing logic while the CLI
  remains thin; only the full application range can be flashed; bootloader v5
  and unqualified hardware fail closed; tests prove that malformed or stale
  responses cannot advance writes; and physical completion additionally
  requires the exact test unit to be identified, its EEPROM backup validated,
  a known-good raw recovery image recorded, recovery rehearsed, the AFIK image
  acknowledged page-by-page, and an independent application-boot observation.
- **Progress notes:** sourced protocol and hardware boundaries, the
  bootloader-preserving target/package path, guarded host library, explicit
  Linux CLI, deterministic success/failure tests, pinned host/minimum-target
  checks, package checks, and repeated Renode proof are complete in commits
  `7482299`, `825e1e9`, `33dbdf5`, `9f5247a`, and `24faeae`. Physical acceptance
  remains pending because no serial device or inspected test radio is available
  in the execution environment. The next action is exact board/MCU inspection
  followed by the read-only normal-firmware EEPROM backup; no bootloader write
  precedes those gates.

## K1EVID-013 — UV-K1/PY32F071 hardware evidence and target contract

- **Status:** complete (2026-08-06)
- **Objective:** establish the smallest reproducible, independently implemented
  AFIK target contract for one exact Quansheng UV-K1/PY32F071 unit currently
  available for inspection and recovery testing.
- **Scope:** pin Armel's latest default-branch UV-K1 firmware revision
  (`fe9c4e9432694b50aea651084a043aae0b58673d` from upstream `main`, resolved
  2026-08-06; the repository has no `master` branch) and relate it to the
  version currently demonstrated on the test unit; record its
  manufacturer-supported project provenance;
  identify the exact radio, PCB, MCU, flash/recovery path, and non-secret
  calibration/configuration boundaries; build a source-to-fact evidence matrix
  for CPU, memory, image format, boot path, clock, GPIO, display, keypad,
  BK4819 bus, audio, USB/serial, and RF control; corroborate MCU facts against
  Puya documentation; and specify the first harmless physical boot witness and
  recovery experiment for a later target implementation package.
- **Dependencies:** `FOUND-001`, the available UV-K1 test unit, the exact Armel
  firmware version and source revision, a retained known-good recovery image,
  a configuration/calibration backup, Puya PY32F071 documentation, and
  read-only inspection tools appropriate to the unit.
- **Assumptions:** the demonstrated Armel firmware is trusted, hardware-tested
  evidence for the supported UV-K1 configuration; manufacturer support raises
  its confidence but does not turn its implementation into AFIK production
  source; AFIK will independently implement evidenced behavior in Rust; model
  name alone does not identify every future revision; and a successful
  third-party firmware boot does not prove an AFIK image.
- **Exclusions:** copying, linking, porting, or incrementally translating Armel
  or other existing application/driver code; destructive flashing before
  backup and recovery proof; K5 V3 family-wide claims; guessed registers or
  pins; production peripheral drivers; RF transmission; and any physical AFIK
  success claim in this evidence-only package.
- **Likely files:** `docs/hardware-evidence.md`, a new UV-K1 bring-up document,
  `docs/architecture.md`, `DECISIONS.md`, `RISKS.md`, `STATUS.md`, and this file.
- **Tests required:** source hashes and pinned revisions are reproducible;
  extracted facts cite exact source locations and confidence; contradictory or
  missing board facts remain explicit; backup/recovery artifacts are validated
  outside the repository without recording calibration bytes or device
  secrets; and documentation/build hygiene checks pass.
- **Acceptance criteria:** the exact test unit and running firmware revision are
  recorded without secrets; every proposed target/peripheral behavior has
  provenance, confidence, and a required physical observation; memory and
  image contracts are sufficient to define a later bounded PY32 target task;
  recovery is demonstrated before any AFIK image attempt; TX remains denied;
  and the next task is the smallest independently implemented target reset and
  boot-witness package rather than a general hardware port.
- **Progress notes:** the exact source CPU/memory/image contract is now recorded
  with relative file and line references in `docs/k1-bring-up.md` and
  `docs/hardware-evidence.md`. Repeated fixed-session normal-mode reads of the
  exact unit identified `F4HWN v5.5.0`, and the complete 8 KiB backup matched
  the retained private copies byte-for-byte. Physical model/MCU markings, USB
  identities remain open; the unchanged pinned recovery candidate has now been
  flashed and the unit returned to `F4HWN v5.5.0` with a byte-identical backup.
  No AFIK image write or TX operation is permitted.

## K1FLASH-014 — Auto-detected K1/K5 recovery flasher

- **Status:** complete (2026-08-06)
- **Objective:** extend the host flasher with an independently implemented,
  fail-closed K1 recovery path and automatic K1/K5 protocol classification.
- **Scope:** reuse the bounded legacy frame envelope; enumerate USB serial
  candidates when `--device auto` is selected; classify only validated
  bootloader beacons (`2.*` as the qualified K5 V1 protocol and the pinned
  `7.03.*` shape as the qualified K1 protocol); expose generic identify,
  read-only 8 KiB backup, and recovery-flash workflows; validate K1 raw image
  vectors/range and page acknowledgements; preserve the existing K5 V1 path;
  and test malformed, ambiguous, unsupported, and cross-family inputs.
- **Dependencies:** `K1EVID-013`, `FLASH-012` software protocol tests, the
  shared Linux serial transport, the pinned K1 `0x0518/0x0530/0x0519/0x051A`
  observations, and the K5 V1 beacon/page vectors.
- **Assumptions:** USB metadata narrows candidate paths but never proves a
  radio; a bootloader beacon identifies a protocol family, not a physical board
  or MCU; K1 recovery flashing is supported only for the validated raw-image
  contract; and K1 AFIK application flashing is not available.
- **Exclusions:** automatic hardware identity claims from USB or serial alone,
  K1 AFIK image generation or flashing, K1 bootloader replacement, EEPROM
  writes, RF operation, retries after ambiguous page results, and importing
  existing firmware or driver source.
- **Likely files:** `crates/radio-flasher`, a K1 flasher module/crate, the
  generic CLI, workspace manifests, programmer documentation, `DECISIONS.md`,
  `RISKS.md`, and `STATUS.md`.
- **Tests required:** exact K1/K5 beacon classification, USB candidate
  enumeration and ambiguity failure, K1 page encoding and zero-padding,
  transaction/page/error acknowledgement checks, complete K1 image validation,
  no-retry failure behavior, generic CLI routing, and all existing K5 tests.
- **Acceptance criteria:** auto mode enumerates candidates and fails closed on
  zero or multiple viable paths; K1 and K5 are selected only from validated
  beacon evidence; unsupported/unknown versions fail closed; K1 recovery writes
  are bounded and acknowledgement-checked; the existing K5 path remains green;
  and no K1 AFIK flashing capability is claimed.
- **Progress notes:** `radio-k5-flasher` and its CLI package are now named
  `radio-flasher` and `radio-flasher-cli`; `afik-k5` remains compatible and
  `afik-flasher` is the generic entry point. The shared serial crate discovers
  USB-by-id or numeric USB serial candidates, while protocol detection remains
  the source of K1/K5 family selection. K1/K5 beacon, image, page, ambiguity,
  acknowledgement, and CLI routing tests pass. The next task is the physical
  K1 board/MCU and AFIK reset/boot-witness work retained under `K1EVID-013`.

## K1HIL-015 — First AFIK K1 recovery-flasher hardware run

- **Status:** complete (2026-08-06)
- **Objective:** exercise the independently implemented AFIK K1 recovery
  flasher against the exact attached unit using the unchanged, already
  recovered Armel `F4HWN v5.5.0` image, and verify the post-flash normal-mode
  identity and configuration/calibration backup.
- **Scope:** preserve strict envelope, command, length, transaction, page, and
  result checks; accommodate the observed K1 device-side trailer convention;
  add a captured-frame regression test; validate the selected image and two
  private backup/image copies; run one explicitly confirmed K1 recovery write;
  and record the exact command/result without storing unit secrets.
- **Dependencies:** `K1FLASH-014`, `K1EVID-013`, the two retained private
  backup and recovery-image copies, the exact `/dev/ttyUSB0` K1 in bootloader
  mode, and the prior same-unit recovery proof.
- **Assumptions:** the K1 bootloader envelope/footer and response payloads are
  bounded and structurally checked, but the decoded device trailer is not a
  reusable K5 response-integrity marker; the known-good stock image remains
  the only validated K1 image; and a successful recovery write is not an AFIK
  application boot.
- **Exclusions:** K1 AFIK image generation or flashing, target startup or
  peripheral implementation, reset commands, EEPROM writes, RF operation,
  retries after ambiguous results, and any board/MCU identity claim inferred
  only from the serial adapter or beacon.
- **Tests required:** the captured K1 beacon with trailer `0x6ed1` must decode
  and classify as `7.03.01`; malformed envelope/footer and page
  command/transaction/result checks must remain fail-closed; all workspace
  tests and required Nix checks must pass.
- **Acceptance criteria:** AFIK's generic command identifies the exact live
  `7.03.01` protocol family, the unchanged private image is written with all
  exact page acknowledgements, and after a user power-cycle a complete
  read-only normal-mode backup matches the pre-write backup byte-for-byte.
  The next task remains the independently implemented K1 reset/boot-witness
  package under `K1EVID-013`.
- **Completion notes:** after two bounded pre-fix timeouts and one bounded
  serial-read-window fix, AFIK's generic command acknowledged all 375 pages in
  transaction `074b2081`. The post-power-cycle normal-mode identity was
  `F4HWN v5.5.0`; the complete 8 KiB read matched both retained pre-flash
  backups byte-for-byte. No reset, EEPROM, or RF command was sent, and this
  remains a stock recovery write rather than a K1 AFIK application flash.

## K1BOOT-016 — Minimal K1 reset image and boot-witness boundary

- **Status:** complete (2026-08-06)
- **Objective:** create the smallest independently implemented AFIK K1
  application image that satisfies the pinned PY32F071 reset, memory, and raw
  application-origin contract.
- **Scope:** add a standalone `no_std`/heap-free K1 target leaf; link its
  vector table and Reset entry at `0x08002800`; bound code to the evidenced
  `0x08002800..0x08020000` application range and SRAM to
  `0x20000000..0x20004000`; write a RAM-only boot witness from Reset; emit and
  statically validate a raw image for the K1 recovery envelope; and cover the
  vector/range contract with deterministic host checks.
- **Dependencies:** `K1EVID-013`, `K1FLASH-014`, and `K1HIL-015`.
- **Assumptions:** the pinned source linker contract is sufficient for the
  application origin, capacity, Cortex-M0+ instruction set, and SRAM bounds;
  the bootloader transfers control through the application vector table; and
  the RAM witness is a software/simulation observation only until a physical
  witness is separately evidenced.
- **Exclusions:** copied or translated Armel source; guessed clock startup,
  USB, display, keypad, GPIO, external flash, BK4819, audio, RF, TX, reset
  commands, bootloader replacement, physical flashing, and any claim that the
  image has booted on the K1.
- **Likely files:** `Cargo.toml`, a new K1 firmware crate, `tool/`,
  `docs/k1-bring-up.md`, `docs/architecture.md`, `DECISIONS.md`, `RISKS.md`,
  `STATUS.md`, and CI.
- **Tests required:** pinned-environment format, clippy, and workspace tests;
  K1 target build; ELF architecture, vector, section, and range verification;
  raw-image extraction and vector validation; and negative checks for truncated,
  out-of-range, and non-Thumb images.
- **Acceptance criteria:** the target leaf has no host or hardware-driver
  dependencies, the raw image is bounded below the K1 application end and
  accepted by the existing K1 vector validator, the Reset handler has no
  peripheral side effects, and all verification commands are recorded before
  committing the milestone. Physical K1 flashing remains blocked pending a
  separate visible or USB boot-witness contract.
- **Completion notes:** added `radio-firmware-k1`, its absolute-origin linker,
  pinned build script, ELF/raw verifiers, raw package, CI gates, and negative
  package fixtures. The 616-byte image has initial SP `0x20004000`, Reset
  `0x08002821`, image end `0x08002a68`, and SHA-256
  `877e2018ef4dd0e985dd16447d7120f61d60ff77259b149b3ad0ab6d37b95021`.
  Verification passed in the pinned environment, including target Clippy and
  the full workspace suite. The RAM witness remains development-only; no
  physical image write or boot claim is permitted.
- **Next task:** define the independently evidenced K1 physical boot witness
  and only then add the guarded K1 AFIK application-flash workflow.

## K1WIT-017 — Evidence-backed physical K1 boot witness

- **Status:** complete (2026-08-06)
- **Objective:** establish one harmless, independently observable indication
  that an AFIK K1 image reached Reset on the exact unit before authorizing an
  AFIK application write.
- **Scope:** use the confirmed external CH340/UART programming path; collect
  the primary USART1, pin, clock, and register facts; independently implement
  the smallest bounded serial witness; and verify it on the exact unit while
  retaining the stock recovery image and backup. Native USB is outside this
  witness path.
- **Dependencies:** `K1EVID-013`, `K1HIL-015`, `K1BOOT-016`, the exact unit,
  and the retained CH340/UART path.
- **Assumptions:** the pinned board source and exact-unit normal-mode backup
  establish the intended CH340/UART path; source register facts are used only
  for an independent AFIK implementation; and a witness must not require RF,
  TX, EEPROM writes, or bootloader replacement.
- **Exclusions:** any physical write before the exact target, image, backup,
  version, CRC, and recovery-rehearsal confirmations are supplied, guessing
  peripheral registers, importing Armel driver code, native-USB identity
  claims, RF operation, and any general radio application implementation.
- **Tests required:** sourced register/pin facts, deterministic host-side
  witness framing or rendering checks, guarded writer and CLI parser tests,
  exact-unit observation, immediate stock recovery, and complete
  workspace/build hygiene.
- **Acceptance criteria:** the serial witness is independently observable on
  the exact K1 through the CH340 path, has no TX or calibration side effect,
  and is followed by a documented recovery check. The local writer must refuse
  a missing rehearsal, mismatched target, invalid backup, bad CRC, or identical
  recovery image before touching the serial transport.
- **Completion notes:** the 44,008-byte image was flashed over the external
  CH340/UART path after K1 `7.03.01` detection. All `172/172` pages were
  acknowledged; after power-cycle, `probe-normal` returned the exact
  `AFIK-K1-0.1` response. No reset, EEPROM, RF, or TX command was sent.
  The retained stock recovery image and complete backup remain available.

## K1APP-018 — Define the next evidence-backed K1 application slice

- **Status:** complete (2026-08-06)
- **Objective:** choose and bound the smallest useful K1 application feature
  beyond the proven serial witness without inventing board behavior.
- **Scope:** use the exact-unit serial witness, retained recovery route, and
  recorded MCU/source facts to select one next slice; record its inputs,
  outputs, hardware evidence, failure behavior, and rollback observation.
- **Dependencies:** `K1WIT-017`, `K1EVID-013`, `K1HIL-015`, and a separately
  bounded hardware experiment for any display, keypad, storage, or RF surface.
- **Exclusions:** a general radio application, native USB claims, RF/TX,
  EEPROM writes, copied Armel source, and physical changes without a new
  evidence and recovery gate.
- **Acceptance criteria:** one stable task contract and tests exist before
  implementation begins; the existing serial witness remains buildable and
  physically recoverable.
- **Completion notes:** selected a display-only boot witness as the next slice.
  `K1DISP-019` keeps the proven serial hello, adds an independently implemented
  heap-free ST7565 command/rendering layer, and renders one fixed AFIK identity
  screen. Key scanning, storage, RF, TX, audio, backlight control, and general
  application behavior remain excluded. Physical flashing is a separate,
  confirmation-gated experiment after static and host verification.

## K1DISP-019 — K1 display-only boot witness

- **Status:** complete (2026-08-06)
- **Objective:** extend the proven K1 serial-witness firmware with one bounded,
  independently implemented display witness while retaining serial recovery
  observability and every TX denial boundary.
- **Scope:** record the exact ST7565-compatible command, 128-by-64 page layout,
  SPI1 mode/rate, and PA5/PA6/PA7/PB2 board facts; add a hardware-independent,
  `no_std`, heap-free display command/rendering module; bind only the evidenced
  K1 GPIO/SPI1 registers in the target leaf; render a fixed `AFIK`/version boot
  screen; and continue answering the existing serial hello.
- **Dependencies:** `K1APP-018`, `K1WIT-017`, the retained stock recovery image
  and complete backup, the pinned Puya register definitions, and the pinned
  exact-board display observations.
- **Assumptions:** the exact unit follows the pinned board mapping for SPI1 SCK
  PA5 AF0, serial display data PA7 AF0, A0 PA6, active-low CS PB2, 128 columns,
  eight pages, SPI mode 3, MSB-first, and a 48 MHz bootloader clock. These are
  application-source observations until the display witness is seen on the
  exact unit; no unobserved reset pin is used.
- **Exclusions:** copied or translated Armel driver/application code; keypad or
  PTT scanning; backlight or audio control; external storage; BK4819 access;
  RF receive or transmit; EEPROM writes; interrupts, DMA, USB, fonts beyond the
  fixed bounded witness glyphs, menus, and a general radio application.
- **Likely files:** `crates/radio-firmware-k1`, `tool/`, `docs/k1-bring-up.md`,
  `docs/hardware-evidence.md`, `docs/architecture.md`, `DECISIONS.md`,
  `RISKS.md`, and `STATUS.md`.
- **Tests required:** exact display init/clear/page-write command traces;
  deterministic fixed-screen bytes and bounds; injected bus-failure behavior;
  serial framing regression; target build, ELF/raw-image gates, target Clippy,
  full workspace format/Clippy/tests, and diff hygiene. A physical write needs
  a separate explicit confirmation and must be followed by display observation
  plus the existing serial `probe-normal`; stock recovery remains available.
- **Acceptance criteria:** the display layer is allocation-free and independent
  of PY32F071 MMIO; the target binding touches only sourced RCC/GPIO/SPI1 and
  existing USART1 registers; a display failure cannot enable any other device
  or TX path; serial hello remains responsive; and static verification passes
  before any physical write is proposed. Physical success is claimed only if
  the fixed AFIK screen and serial identity are independently observed.
- **Implementation notes:** the pure command/rendering module, exact trace and
  failure tests, bounded PY32F071 SPI1/GPIO binding, fixed screen, and
  `AFIK-K1-0.2` serial regression are implemented and pass all static gates.
  The generated raw image is 48,436 bytes with SHA-256
  `94ac835a473a8a910b740eb792c3a3567254ea297b1d23c31e2c7e52d0ec327b`.
  The task remains active until a separately confirmed physical write is
  followed by visible-screen and serial observations.
- **Physical attempt:** one explicitly authorized write acknowledged all 190
  pages, but the screen was blank after power-cycle. The serial fallback still
  returned `AFIK-K1-0.2`, so the task remains active and no second write is
  authorized. The next observation must distinguish the separately controlled
  active-high PF8 backlight from LCD pixel generation.
- **Completion notes:** under bright external light, the user observed the fixed
  AFIK words on the panel. This establishes LCD initialization, page addressing,
  orientation, contrast sufficient for passive viewing, and fixed rendering on
  the exact unit. The separately controlled backlight remained off and is
  tracked under `K1BL-020`; it is not a display-controller failure.

## K1BL-020 — Constant K1 boot-witness backlight

- **Status:** active (2026-08-06)
- **Objective:** make the physically verified fixed K1 display witness readable
  without external illumination using the smallest evidenced backlight action.
- **Scope:** record PF8 as the pinned active-high backlight output; configure
  GPIOF clock, PF8 push-pull output mode, and set PF8 high before the display
  witness; retain the `AFIK-K1-0.2` serial responder and exact LCD image.
- **Dependencies:** completed `K1DISP-019`, `EVID-K1-027`, retained recovery and
  backup artifacts, and the pinned PF8 active-high board observation.
- **Exclusions:** PWM, TIM7, DMA, fading, brightness levels, EEPROM/settings,
  keypad/PTT, audio, storage, BK4819, RF/TX, USB, interrupts, or a general UI.
- **Tests required:** pure register-plan assertions for only GPIOF clock/PF8
  output/high operations; target Clippy/build/image gates; full workspace
  checks; and one separately confirmed physical write followed by visible
  illumination, fixed words, and serial identity observation.
- **Acceptance criteria:** the target touches only the sourced GPIOF/PF8 surface
  in addition to the already verified display/USART paths; no timer or DMA is
  enabled; failure cannot grant RF/TX behavior; and success is claimed only if
  the exact unit shows the already verified words with the backlight illuminated
  while `probe-normal` still returns `AFIK-K1-0.2`.
- **Implementation notes:** the exact GPIOF/PF8-only register plan and target
  binding are implemented and pass host, workspace, target Clippy, ELF, and raw
  image gates. The 48,580-byte image has SHA-256
  `249bccb1cf66ce3269cc64d80f8171fbafdb6835ab7f31a2df3fc152c9b93489`
  and CRC-32 `a327eba0`. The task remains active pending one separately
  confirmed physical write and illumination/serial observations.
