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
  Physical completion is recorded below.
- **Physical attempt:** one explicitly authorized write acknowledged all 190
  pages, but the screen was blank after power-cycle. The serial fallback still
  returned `AFIK-K1-0.2`, so no second write was authorized at that point. The
  next observation distinguished the separately controlled
  active-high PF8 backlight from LCD pixel generation.
- **Completion notes:** under bright external light, the user observed the fixed
  AFIK words on the panel. This establishes LCD initialization, page addressing,
  orientation, contrast sufficient for passive viewing, and fixed rendering on
  the exact unit. The separately controlled backlight remained off and is
  tracked under `K1BL-020`; it is not a display-controller failure.

## K1BL-020 — Constant K1 boot-witness backlight

- **Status:** complete (2026-08-06)
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
  and CRC-32 `a327eba0`. Physical completion is recorded below.
- **Completion notes:** the guarded 190-page write completed without retry.
  After power-cycle, the user observed both the backlight and the fixed words;
  the final read-only probe returned `AFIK-K1-0.2`. The words were faint, which
  is isolated as contrast calibration under `K1CON-021` rather than a PF8 or
  display-transport failure.

## K1CON-021 — Fixed K1 boot-witness contrast calibration

- **Status:** complete (2026-08-06)
- **Objective:** make the physically verified fixed words clearly readable by
  replacing AFIK's conservative electronic-volume value with the pinned board
  source's fixed startup value.
- **Scope:** change only the ST7565-compatible electronic-volume byte from 21
  (`0x15`) to 31 (`0x1f`); retain the exact display bytes, PF8 constant output,
  serial responder, controller power sequence, pins, SPI setup, and bounds.
- **Dependencies:** completed `K1DISP-019` and `K1BL-020`, the exact-unit faint
  text observation, retained recovery/backup artifacts, and the pinned fixed
  startup contrast value in `App/driver/st7565.c`.
- **Exclusions:** runtime contrast settings, keypad input, persistence, PWM,
  timers, DMA, automatic calibration, display inversion, RF/TX, audio, storage,
  USB, and a general UI.
- **Tests required:** update the exact init trace; prove no framebuffer or other
  command changes; run target/image/workspace gates; and require a separately
  confirmed physical write followed by readability and serial observations.
- **Acceptance criteria:** the only controller-stream change is electronic
  volume `0x15` to `0x1f`; existing display/backlight/serial observations remain
  intact; and no production brightness/contrast policy is claimed.
- **Implementation notes:** the one-byte command change and exact-trace test are
  implemented. All host, workspace, embedded Clippy, ELF, and raw-image gates
  pass. The 48,580-byte image has SHA-256
  `b2e6a38b965fcb0d419ec2ed7309aa3d6518285967c98d1646eddaa8718c8d32`
  and CRC-32 `4dfc4076`; physical writing remains separately confirmation-gated.
- **Completion notes:** after explicit authorization, K1 `7.03.01` acknowledged
  all 190 pages without retry. After power-cycle, the user confirmed the
  backlight remained on and the words were substantially clearer. The final
  read-only probe returned `AFIK-K1-0.2`.

## K1KEY-022 — Receive-only K1 keypad/UI witness

- **Status:** complete (2026-08-06)
- **Objective:** add the smallest physically observable, receive-only keypad
  slice: decode one main-key press and show its fixed label without including
  PTT or any RF behavior.
- **Scope:** the 4-by-4 main matrix uses pull-up inputs PB15, PB14, PB13, PB12
  as rows and push-pull outputs PB6, PB5, PB4, PB3 as columns. All columns are
  high while idle; a scan drives exactly one column low and restores all high.
  A pressed key is one low row on one selected column. The table, by column,
  is `MENU/1/4/7`, `UP/2/5/8`, `DOWN/3/6/9`, and `EXIT/*/0/F`.
- **Behavioral contract:** zero active cells is release; exactly one active
  cell is a candidate key; multiple active cells, changing samples, invalid
  row bits, or a scan/read failure produce no key. A hardware-independent
  debounce machine accepts monotonic elapsed-time samples, emits a press only
  after the same single candidate has remained stable for an AFIK-defined
  bounded interval, emits release only after stable absence, and resets to no
  key on ambiguity or time reversal. The target witness replaces the fixed key
  label only after a debounced press and otherwise retains the verified
  display, backlight, and serial behavior.
- **Dependencies:** completed `K1CON-021`, pinned board/keypad observations, the
  physically verified display/backlight/serial path, and a separately bounded
  no-transmit hardware experiment.
- **Exclusions:** PTT, side keys, multi-key/chord claims, RF/TX, EEPROM,
  interrupts, arbitrary timing assumptions, copied source, and general menus.
- **Tests required:** exhaustive 16-cell decode and release cases; rejection of
  every multi-cell row/column combination and invalid sample; explicit-time
  debounce press/release, bounce, ambiguity, and time-reversal traces; exact
  display labels; pure GPIO register-plan assertions before target binding;
  target/image/workspace gates; and one separately guarded physical write
  followed by all 16 key labels plus backlight and serial observations.
- **Acceptance criteria:** the evidence-backed table and fail-closed pure tests
  pass before GPIO implementation; target code touches only PB3..PB6 and
  PB12..PB15 in addition to the already verified display/backlight/USART paths;
  no scan state can reach PTT, side-key, RF, TX, persistence, or configuration
  behavior; and physical success is claimed only after all 16 main keys are
  observed correctly on the display and `probe-normal` still returns
  `AFIK-K1-0.2`.
- **Implementation notes:** the hardware-independent matrix decoder, 20 ms
  explicit-time debounce state machine, fixed labels, exact GPIOB register
  plan, cleanup-guaranteed scan trace, target MMIO binding, and display-only
  witness are implemented. Host/workspace/embedded Clippy, ELF, packaging, and
  raw-image gates pass. The 56,828-byte image has SHA-256
  `4ad5e4e205afd32e791409b371e111c0792110c48e1fc9c67a5c19d8628c06b0`
  and CRC-32 `a17da806`. The guarded write acknowledged all 222 pages without
  retry. After power-cycle, the user reported that key labels did not display;
  the final read-only serial probe still returned `AFIK-K1-0.2`. The failure is
  not yet localized, and no second write has been sent.
- **Correction note:** the first image placed labels on previously unobserved
  pages 6–7 at `y=50`. The bounded correction renders them on the physically
  verified `y=36` line in place of `K1 0.2`, with an exact page-range test;
  keypad GPIO and debounce behavior remain unchanged.
- **Correction artifact:** focused host, workspace Clippy, embedded Clippy,
  ELF, package, and raw-image gates pass. The 56,856-byte corrected image has
  SHA-256
  `417663dab22de56fbfe167049c3b1b5831e588c04db4eec9ac7ec16b5cf9130a`
  and CRC-32 `f4a9c1d6`.
- **Correction write:** K1 `7.03.01` acknowledged all 223 pages in transaction
  `fe6396d0` without retry. A normal power-cycle and physical label observation
  remain pending.
- **Diagnostic step:** add a simulation-only K1 Renode platform with bounded
  register storage, synthetic one-cell GPIO injection, and an ELF-symbol hook
  proving whether MENU reaches `render_key_witness`. This is execution evidence
  only and must not be reported as physical keypad/display validation.
- **Diagnostic result:** three repeated Renode runs prove initial display setup
  returns and synthetic PB6/PB15 MENU reaches the production render function.
  The remaining physical boundary requires a read-only serial raw-matrix probe;
  Renode does not establish actual PY32 GPIO or LCD behavior.
- **Raw-matrix diagnostic:** the normal-mode serial session now accepts one
  read-only request which runs the existing bounded scan and returns only four
  validated row masks plus scan validity. Host, workspace, embedded, image, and
  three-repeat Renode gates pass. The 57,860-byte image has SHA-256
  `c56f5a8d883cf240d4a70626a299ab0cc8a1cf2bba294cffb3e6308ec4426ba9`
  and CRC-32 `0a53af07`. K1 `7.03.01` acknowledged all 227 pages in transaction
  `0e4f6fc9` without retry; released and held-MENU observations remain pending.
- **Raw-matrix observation:** released returned a valid all-zero scan. Holding
  MENU caused two serial timeouts, including one with the prebuilt host tool;
  response recovered after release. The next diagnostic suppresses only the
  key-triggered synchronous SPI transfer while retaining scan, debounce, pure
  render execution, and serial raw reporting.
- **SPI-suppressed diagnostic artifact:** all focused, workspace, embedded,
  image, and three-repeat Renode gates pass. The 57,852-byte image has SHA-256
  `c50baea15ebcf11805e7fff670cc4e0734c5ad1d52e09512acdb58c68c6e7fb9`
  and CRC-32 `0b98c076`. K1 `7.03.01` acknowledged all 226 pages in transaction
  `1a79dec2` without retry; normal-boot raw observations remain pending.
- **SPI-suppressed observation:** released remained a valid all-zero scan, but
  MENU held still timed out. The synchronous display transfer is therefore not
  the cause. Pre-armed latch-until-release reporting is the next bounded step;
  Embassy remains outside this package until timer/interrupt evidence exists.
- **Latched diagnostic:** the latest nonzero raw scan is retained in bounded RAM
  until a released-key probe returns and clears it with `captured=true`. All
  host, workspace, embedded, image, and repeated-Renode gates pass. The
  58,380-byte image has SHA-256
  `eba38cc718a3de0e220bc28c4de657849960ea1d7098085df94c802cf903a328`
  and CRC-32 `823616ad`. K1 `7.03.01` acknowledged all 229 pages in transaction
  `20d50457` without retry; normal-boot capture observation remains pending.
- **Latched observation:** after MENU tap/release, the probe was valid but
  `captured=false` with all masks zero. No PB12..PB15 MENU input is established;
  raw GPIOB capture is the next bounded experiment.
- **Raw GPIOB artifact:** four exact 16-bit per-column IDR values are compared
  against a released baseline while ignoring only PB3..PB6. All required gates
  pass. The 61,128-byte image has SHA-256
  `25f900885cf0a4ca79c10ea16737c72878330e8d0e372eb74cde63c479b28f32`
  and CRC-32 `032e7309`. K1 `7.03.01` acknowledged all 239 pages in transaction
  `7422b31d` without retry; normal-boot observation remains pending.
- **Raw GPIOB result:** released PB6 was `f43c`; MENU tap latched PB6 `743c`
  while PB5/PB4/PB3 were unchanged. GPIOB bit 15 therefore goes low only for
  selected PB6, physically establishing MENU and the raw scan path. Remaining
  diagnosis is press-path render/SPI latency and visible update.

## K1ASYNC-023 — Embassy/PY32 runtime foundation

- **Status:** complete (2026-08-06)
- **Objective:** establish the smallest heap-free Embassy execution foundation
  for the K1 before migrating UART, keypad, display, or timing behavior.
- **Scope:** pin a Rust-1.86-compatible dependency set; prove PY32F071 chip
  generation; compile a thread-mode executor proof; then independently verify
  time, USART1, and SPI1 support against AFIK's already observed behavior.
- **Exclusions:** changing recovery or application origin; adopting unverified
  clock/interrupt/DMA behavior; RF/TX; migrating all drivers at once; and
  claiming cooperative async preempts CPU-bound code without an await.
- **Acceptance criteria:** versions and provenance are recorded; every feature
  builds in pinned Nix/Rust; tasks remain static and bounded; Renode proves
  scheduler progress; physical migrations are separately guarded; UART remains
  responsive during chunked rendering before visible-key acceptance.
- **First step:** resolve MSRV and exact PY32F071 feature coverage without
  changing the flashed image.
- **Dependency result:** pinned `embassy-executor 0.10.0` compiles with strict
  target Clippy on Rust 1.86. `py32-metapac 0.5.0` contains four F071 packages,
  but released `py32-hal` through 0.4.1 exposes none. The next step is a bounded
  reviewed HAL chip-surface extension; F072 substitution is not allowed.
- **Metadata result:** exposing the four exact F071 feature names is
  insufficient. All select the same generated metadata stub containing only
  GPIOA, WWDG, AES_LPUART1, and DMA1_CH1; `py32-hal` fails its mandatory RCC
  lookup before library compilation. The next step is a source-backed upstream
  `py32-data`/`py32-metapac` inventory correction, not a guessed AFIK mapping.
- **Local inventory result:** AFIK now vendors a regenerated PAC from pinned
  `py32-data` commit `eb33b9ab85aa4652006e3435d84e1f9f7e5eca50` and a bounded
  local `py32-hal 0.4.1` compatibility patch. All four F071 package features
  compile on the pinned target, and an optional compile-only K1 contract names
  the evidenced RCC, USART1, SPI1, timer, and GPIO pin surfaces. No HAL init or
  physical-image path changed. The next independent boundary is time-driver
  evidence and compilation; USART1 and SPI1 behavior remain later milestones.
- **Time-driver result:** TIM15 is the bounded compile-only candidate: pinned
  metadata supplies `PCLK1_TIM`, RCC enable/reset, a dedicated interrupt, and
  the two compare channels required by the HAL's one-alarm Embassy driver.
  Optional `py32f071-time-driver` passes strict target Clippy without calling
  HAL initialization or changing the image. Runtime adoption remains blocked
  on explicit clock handoff and physical interrupt/timing proof. The next
  independent boundary is the evidenced USART1 path.
- **USART1 result:** the generated F071 inventory binds USART1 to `PCLK1`,
  `APBENR2.USART1EN`, `APBRSTR2.USART1RST`, interrupt 27, PA9 TX AF1, and PA10
  RX AF1. Optional `py32f071-usart1` compiles a real async constructor at the
  evidenced 38,400 baud with one bounded TX and RX DMA channel under strict
  target Clippy. It is not called by the firmware entry point, so clock
  ownership, interrupt delivery, DMA operation, and physical responsiveness
  remain unproven. The next independent boundary is the evidenced SPI1 display
  path.
- **SPI1 feasibility result:** the generated F071 PAC inventory contains the
  evidenced SPI1 RCC surface and PA5 SCK / PA7 MOSI AF0 mapping, but vendored
  `py32-hal 0.4.1` implements no SPI module or driver. Its support table marks
  SPI blank and its TODO list names SPI explicitly. There is therefore no
  Embassy-compatible SPI constructor to compile. The next step must define a
  bounded AFIK display-bus driver contract or a separately reviewed local HAL
  extension before changing startup; PAC inventory alone is not driver proof.
- **SPI1 implementation step:** add a bounded local `py32-hal` transmit-only
  SPI driver for the evidenced F071 display surface. It must own SPI1, PA5 SCK,
  and PA7 MOSI; configure mode 3, MSB first, and divide-by-64; expose an async
  write which yields between bounded chunks; report peripheral faults; and
  compile without entry-point adoption. MISO, hardware NSS, DMA, RX, other SPI
  instances/pins, and physical migration remain excluded.
- **SPI1 implementation result:** the local HAL now exposes generated SCK/MOSI
  pin traits plus a heap-free `SpiTx` which configures mode 3, MSB first,
  software NSS, one-line transmit, and divide-by-64. Async writes bound every
  status wait, report mode/overrun/CRC/timeout faults, and yield every 16 bytes
  or unsuccessful polls. The F071 SPI1 / PA5 / PA7 constructor passes strict
  target compilation and remains absent from the firmware entry point. The
  next boundary is deterministic executor-progress testing before migration.
- **Cooperative-progress result:** a deterministic no-hardware round-robin
  harness drives one complete 1,024-byte display frame through the exact
  16-byte chunk schedule and proves serial work runs between every adjacent
  display chunk. A compile-time equality check ties that schedule to the local
  HAL SPI driver. This proves the cooperative await boundary, not Cortex-M
  executor startup, physical SPI, or async USART behavior. The next step is a
  separately guarded runtime composition of the proven executor, USART1, and
  SPI1 surfaces without removing the polling recovery image.
- **Runtime-composition step:** add one optional compile-only K1 bundle which
  owns the thread executor, async USART1 with its two DMA channels, and
  cooperative SPI1 with PA5/PA7. It may construct only from caller-supplied HAL
  peripheral tokens and must not initialize the HAL, clocks, TIM15, A0/CS,
  keypad GPIO, static tasks, the firmware entry point, or a physical image.
  The polling recovery image remains the only runnable K1 application until a
  separate clock/interrupt/DMA adoption package is defined and guarded.
- **Runtime-composition result:** optional `py32f071-runtime-composition`
  type-checks one owned bundle containing the heap-free thread executor,
  USART1/PA9/PA10 with DMA1 channels 1/2, and SPI1/PA5/PA7. Construction takes
  explicit HAL tokens and remains absent from the firmware binary. No HAL init,
  clock, TIM15, task, A0/CS, keypad, interrupt execution, DMA transfer, or image
  behavior was adopted. The next boundary is an explicit bootloader-clock
  handoff contract before any runnable async entry point.
- **Clock-publication result:** optional `py32f071-clock-publication` accepts
  only the unforgeable result of the fail-closed exact-unit validator and then
  publishes the 48 MHz SYS/HCLK/PCLK/timer tuple to the HAL software table.
  The boundary performs no RCC write, remains absent from the entry point, and
  passes strict target compilation. The next larger step is a separately
  guarded runnable Embassy keypad/display/serial composition.
- **Inherited-runtime initialization result:** optional
  `py32f071-runtime-init` validates and publishes the live clock tree before
  taking singleton tokens and initializing GPIO, DMA, and the reserved TIM15
  time driver without running the HAL RCC configurator. Strict target Clippy
  passes. The boundary is not yet an entry point or flashable image.
- **Runnable migration step:** add a separate full-vector Cortex-M image which
  calls the guarded inherited initializer, runs static Embassy tasks, retains
  the exact normal-mode hello, scans only the evidenced 4-by-4 matrix with
  explicit Embassy time, and renders key labels through cooperative SPI1.
  Prove interrupt symbols, image bounds, deterministic scheduling, UART
  responsiveness during rendering, and all existing recovery gates before any
  physical write. Side keys, persistence, general UI, RF, and TX stay excluded.
- **Runnable migration software result:** `radio-firmware-k1-async` uses the
  full cortex-m-rt/F071 vectors, guarded inherited initialization, two static
  tasks, async USART1/DMA hello service, TIM15 timing, cooperative SPI1 display,
  PF8 backlight, and only the main matrix. The release package is 25,720 bytes
  with SHA-256 `874da6e7fe70d9564eb5b650581b3525a4aafa0077613c074a07e3fb4bc7bada`.
  Static positive/negative image gates pass. Physical write and observations
  remain the next guarded result.
- **Physical write result:** K1 bootloader `7.03.01` acknowledged all 101 pages
  under transaction `9bca3352` without retry. This is not read-back or boot
  proof; power-cycle, hello, display, and MENU observations remain pending.
- **First runtime/correction result:** blank display and two hello timeouts
  exposed the missing source-required VTOR relocation. The corrected entry sets
  VTOR to `0x08002800` before interrupt-enabled initialization; strict target,
  vector, package, negative-fixture, and focused test gates pass.
- **Corrected write result:** all 101 pages were acknowledged under transaction
  `5b0f91b5` without retry. Boot, UART, display, and keypad evidence still
  require the post-write power-cycle and observations.
- **Completion result:** after power-cycle the boot screen returned, normal
  hello returned `AFIK-K1-0.2`, and the user observed every main key correctly
  identified on the second display line. This closes the bounded async runtime
  foundation and keypad/display migration; RF, TX, side keys, persistence,
  and read-back remain outside scope.

## K1SIDE-024 — Receive-only side-key and PTT evidence boundary

- **Status:** complete (2026-08-06)
- **Objective:** establish the smallest evidence-backed contract for observing
  K1 side keys and PTT without interpreting them as UI actions or reaching RF.
- **Scope:** review the pinned source and exact-unit evidence; identify PTT and
  side-key GPIO surfaces only where independently supported; define bounded raw
  observation data, validity, provenance, and fail-closed handling; add focused
  host tests and documentation.
- **Dependencies:** completed `K1ASYNC-023`; pinned K1 source and existing
  recovery/serial witness remain evidence only.
- **Exclusions:** guessed side-key pins or polarity, copied firmware logic,
  semantic key events, display changes, persistence, menus, interrupts/DMA,
  audio, BK4819, RF, TX, and physical image writes.
- **Acceptance criteria:** PTT PB10 remains separately identified; side-key
  mapping is either source-backed with confidence and experiment requirements
  or explicitly recorded as unknown; raw observations are bounded and
  untrusted; malformed/ambiguous observations fail closed; no result can mint
  UI state, configuration, or transmit authority.
- **First step:** add the source/evidence decision and a hardware-independent
  raw-observation contract before any target binding.
- **Raw-observation result:** `radio-firmware-k1::aux_inputs` now accepts only
  stable, strictly newer, nonzero-sequence samples, retains GPIOB IDR and
  provenance, and exposes PB10 only as an uninterpreted raw bit. Focused and
  workspace host gates pass, and the embedded warning-denied
  `thumbv6m-none-eabi` target gate passed once filesystem space was available.
  Side-key mapping and any physical observation remain unverified.
- **Completion notes:** the source/evidence boundary and bounded host contract
  satisfy the package acceptance criteria. No side-key GPIO or polarity is
  inferred, no target binding or physical image changed, and the open mapping
  risk remains recorded in `RISKS.md` as `RISK-026`. Any future side-key work
  requires a new stable task ID and independently sourced mapping/experiment.
- **Clock-handoff step:** define a bounded local HAL entry which adopts only an
  explicitly validated inherited 48 MHz `SYSCLK`/`HCLK1`/`PCLK1`/`PCLK1_TIM`
  state. It must fail before publishing clocks when the observed RCC state does
  not match the pinned contract, must not switch or configure an oscillator,
  PLL, prescaler, flash latency, or clock mux, and must remain absent from the
  firmware entry point. Host tests must cover exact acceptance and every
  rejected field; target Clippy must compile the handoff with the owned runtime
  bundle. Physical clock, interrupt, DMA, TIM15, UART, SPI, keypad, display, and
  flash behavior remain separate gates.
- **Clock-handoff diagnostic result:** pinned-source review found only a 48 MHz
  software assignment and no inherited RCC field values, so adoption remains
  blocked rather than guessed. AFIK now has a pure fail-closed contract plus an
  optional read-only F071 PAC snapshot; exact acceptance and each rejected field
  are host-tested, and the snapshot compiles with the owned runtime bundle. No
  HAL clock is published and the firmware image is unchanged. The next step is
  one bounded, read-only exact-unit RCC observation before defining HAL adoption.
- **Exact-unit observation surface:** the existing polling image now supports
  one strict read-only request returning RCC CR, ICSCR, CFGR, PLLCFGR, and the
  fail-closed contract result. The host library and `probe-clock` CLI validate
  exact lengths, reserved bytes, and bounded fields. Static image/package and
  existing keypad Renode gates pass; no device is currently visible and no
  flash was sent during the static milestone. The guarded physical write later
  acknowledged all 252 pages without retry and remains unverified until the
  required power-cycle, normal hello, and `probe-clock` capture.
- **Exact-unit observation result:** after power-cycle, the normal hello passed,
  but two combined `probe-clock` attempts timed out; the hello still passed
  between attempts. No raw RCC field was observed. The next bounded step is an
  individually identified read-only register response which can distinguish CR,
  ICSCR, CFGR, PLLCFGR, validation, and response-transfer failure without clock
  writes, HAL adoption, keypad/display mutation, RF, or TX.
- **Register-isolation implementation:** CR, ICSCR, CFGR, and PLLCFGR now have
  separate fixed-session requests and individually identified exact responses.
  The host can issue one named read at a time. Static, host, package, negative,
  and Renode gates pass; the next step is a guarded write and ordered physical
  probes, with all clock adoption still prohibited.
- **Register-isolation write:** bootloader `7.03.01` acknowledged all 257 pages
  under transaction `7d527b6f` without retry. This is not read-back or boot
  proof; power-cycle, normal hello, and the four ordered reads remain pending.
- **Register-isolation result:** normal boot passed, but the first CR request
  timed out; the following hello still passed. No later register was requested.
  Add one same-path constant marker with no MMIO before attributing the timeout
  to RCC access, framing, reset, or UART behavior.
- **Serial-only isolation step:** remove display, keypad, backlight, debounce,
  and matrix scanning from the runnable diagnostic entry point. Retain only the
  RAM boot witness, polling USART hello, no-MMIO control, combined clock probe,
  and individual RCC reads. Existing pure modules/tests remain, but no board UI
  peripheral may initialize or run in this image.
- **Serial-only isolation implementation:** complete. The target entry point
  initializes only GPIOA/USART1 and services hello, a fixed no-MMIO marker,
  combined clock observation, and one-register clock observations. UI modules
  remain host-tested but are compiled out of the runnable image. The verified
  51,340-byte artifact is ready for the guarded exact-unit write; application
  boot and serial responses remain physical gates.
- **Serial-only isolation write:** bootloader `7.03.01` acknowledged all 201
  pages under transaction `8a6af71f` without retry. This is not read-back or
  application boot proof.
- **Serial-only physical result:** after power-cycle, hello and the no-MMIO
  marker passed. CR `03000500`, ICSCR `00e64d14`, CFGR `00000012`, and PLLCFGR
  `00000006` were identical in isolated and combined responses. Validation
  fails only because `ICSCR.HSI_FS` is encoding `2`, while the provisional
  contract requires `4`. Do not adopt clocks until that field is interpreted
  from primary PY32F071 evidence and the contract is re-reviewed.
- **Clock-field resolution:** the pinned F071 DIE072 register inventory defines
  `HSI_FS=2` as 16 MHz, two-bit `PLLSRC=2` as HSI, and `PLLMUL=1` as x3.
  PLLCFGR `00000006` therefore completes the observed 48 MHz contract. The pure
  decoder now checks all those fields and an exact-unit regression vector;
  publishing clocks and starting Embassy remain separate guarded steps.

## K1SIDE-025 — Receive-only unselected-column side-key observation

- **Status:** superseded (2026-08-06); its raw observation was folded into the
  completed semantic `K1SIDE-024` evidence
- **Objective:** physically observe SIDE1 and SIDE2 on the exact unit as raw
  active-low row bits read while no keypad column is selected, without creating
  any semantic side-key action.
- **Mapping evidence:** `EVID-K1-052` resolves the side keys from the pinned
  `App/driver/keyboard.c`. They are not separate pins: `KEY_SIDE1` is PB15 and
  `KEY_SIDE2` is PB14, read during the unselected pass where all four columns
  PB6..PB3 stay high. PB13 and PB12 are `KEY_INVALID` in that state.
- **Scope:** add one unselected-column row sample to the existing pure scan
  contract; extend the read-only `probe-keypad` response with that mask; report
  released, SIDE1-held, and SIDE2-held observations on the exact unit.
- **Dependencies:** completed `K1SIDE-024` and `K1ASYNC-023`; the existing
  guarded writer, recovery image, and retained EEPROM backup.
- **Exclusions:** semantic side-key events, display mutation, persistence,
  menus, PTT actuation, audio, BK4819, RF, TX, and any ported firmware logic.
  PB13/PB12 in the unselected state must stay an explicitly undefined
  observation rather than a decoded key.
- **Acceptance criteria:** the unselected sample is a bounded four-bit
  active-low mask with its own validity flag; an unstable or out-of-range
  sample fails closed; the mask cannot mint a `Key`, UI state, or transmit
  authority; released observation reads zero; and each held side key is
  reported as a distinct raw bit before any interpretation is proposed.
- **First step:** extend the pure scan/decode contract and its host tests for
  the unselected pass, keeping the existing four selected columns unchanged.
- **Implementation result:** `keypad::scan` reads the unselected pass first and
  returns `KeypadScan`; `Key` gained `Side1`, `Side2`, and `Ptt`; `MatrixBus`
  gained `read_ptt_pressed`. `decode` fails closed on the undefined PB13/PB12
  unselected rows, treats side-plus-main as ambiguous, and reports PTT only when
  no other key is active. The async image binds PB10 pull-up, samples the
  unselected pass with the same 10 us settling, and renders SIDE1/SIDE2/PTT on
  the verified `y=36` line. The hello identity moved to `AFIK-K1-0.3`.
- **Software status:** complete. 39 focused K1 tests, 41 workspace test
  binaries, target build/package/negative fixtures, embedded warning-denied
  target check, flake evaluation, formatting, Clippy, and `git diff --check`
  passed. Raw image 26,072 bytes, CRC-32 `85387ce8`.
- **Resolution:** the guarded physical observations and semantic decode were
  completed and recorded under `K1SIDE-024`; this narrower duplicate task has
  no remaining action.

## STORE-026 — Banked explicit-channel storage

- **Status:** complete (2026-08-07)
- **Objective:** store complete explicit channels, named banks, and one global
  radio configuration beside the existing generated banks, without breaking the
  canonical image contract.
- **Scope:** validated domain types for tones, squelch, modulation, bandwidth,
  power, and the global configuration; hardware-independent `ChannelRecord`,
  `ChannelFlags`, `BankMask`, and `ChannelBank`; exact version-1 encodings for
  three new object kinds; capacity accounting and referential integrity in the
  configuration compiler; front-end object naming.
- **Exclusions:** on-flash layout, power-loss durability, multi-object paging,
  member lists, and any device-side migration of existing images.
- **Acceptance criteria:** every encoded field is revalidated on decode;
  reserved bytes and flag bits must be zero; canonical images order every kind
  by `(kind, id)`; a channel referencing an undefined bank is rejected before
  any device mutation; existing generated-bank images still decode unchanged.
- **Result:** complete. `radio-storage` gained `Channel`, `ChannelBank`, and
  `RadioConfig` objects at 42, 22, and 16 bytes; `RadioProject` carries all
  three; `CapacityReport` counts explicit channels, banks, and the singleton
  configuration. The format is documented in `docs/storage-format.md` and the
  membership decision in `ADR-050`.

## RX-027 — Complete receive path and banked receive control

- **Status:** complete (2026-08-07)
- **Objective:** implement the full receive feature set from the pinned K1
  reference firmware's register values, plus the hardware-independent control
  layer which drives it.
- **Scope:** BK4819 receive configuration (power blocks, receive mode,
  AM/FM/USB demodulator, filter bandwidth, AGC tables, squelch thresholds,
  CTCSS and CDCSS sub-audio decoding, interrupt mask, RF filter path, audio
  routing); RSSI, glitch, noise, carrier squelch, and tone metering; banked
  memory and VFO control with monitor, tone-aware audio gating, scan-skip,
  three scan resume modes, and dual watch; the K1 three-wire register bus.
- **Exclusions:** transmit behaviour of any kind, calibration sources, audio
  amplifier and speaker control, FM broadcast receive, spectrum and frequency
  scanning, DTMF, VOX, compander programming, and any physical hardware claim.
- **Acceptance criteria:** every register value traces to `EVID-BK4819-053` or
  primary documentation; the receive path never writes the transmit mode word;
  any bus error latches `Faulted`; squelch thresholds remain caller-supplied
  and internally validated; control-layer inputs are deterministic and mint no
  transmit authority.
- **Result:** complete in software. `radio-bk4819` gained `configure_receive`,
  `set_af_output`, and `receive_metrics`; `radio-channel-control` gained the
  banked receive controller; `radio-firmware-k1::bk4819_bus` sequences the
  pinned three-wire pinout. Physical bring-up is deliberately out of scope and
  is tracked by `RISK-027`; the calibration gap is tracked by `RISK-028`.

## NGUI-028 — Native cross-platform editor and flashing front end

- **Status:** complete (2026-08-07)
- **Objective:** provide one native desktop application which edits channels,
  banks, and the radio configuration, programs a radio, and drives the guarded
  firmware and EEPROM operations.
- **Scope:** an `eframe`/`egui` application over a validated project model;
  canonical image load and save; simulator or explicit serial programmer
  sessions with verified writes and device read-back; a flash tab reusing the
  recovery-gated flasher workflows on a worker thread with progress reporting.
- **Exclusions:** remote or shared operation, authentication, automatic device
  selection for writes, any relaxation of the flasher's confirmation gates, and
  live receive control of a radio.
- **Acceptance criteria:** invalid input cannot reach an image, a device
  transaction, or a flashing workflow; every flashing guard is preserved
  unchanged; the model, session, flash-request validation, and option parsing
  are testable without a display.
- **Result:** complete. `radio-programmer-gui-native` provides `afik-studio`,
  documented in `docs/programmer-gui-native.md`, with its boundary recorded in
  `ADR-052` and its accepted local-tool exposure in `RISK-029`.

## RFK1-029 — K1 receive bring-up on the exact unit

- **Status:** complete (2026-08-07)
- **Objective:** drive the BK4819 from the K1 application over the pinned
  three-wire bus, establish a receive configuration, and observe raw receive
  metrics on the exact unit without any transmit capability.
- **Scope:** a PY32F071 pin adapter for CSN PF9, SCL PB8, and shared SDA PB9; a
  receive-only firmware task which reaches standby, configures the receiver at
  a fixed frequency, reads back a configured register, and samples metrics; a
  bounded read-only serial observation and its `probe-rf` host command; the
  guarded write and the post-power-cycle observation.
- **Dependencies:** completed `RX-027`, the pinned recovery image, and the
  retained EEPROM backup.
- **Exclusions:** transmit of any kind, audio amplifier and speaker control,
  squelch calibration from external flash, channel selection or UI, persistence,
  and any on-air activity.
- **Acceptance criteria:** the read-back register returns the exact non-trivial
  value the image configured, proving the bus carries real data in both
  directions; the sample counter advances; a bus or state failure is reported as
  a faulted stage rather than a plausible-looking sample; the transmit mode word
  is never written; the known-good recovery path stays available.
- **Write result:** the first image was written through bootloader `7.03.01`
  with every page acknowledged and no retry, but it did not receive. Three
  physical iterations found the causes, each recorded as evidence.
- **Correction 1 — serial starvation (`EVID-K1-058`):** a free-running receive
  task busy-waits for milliseconds while the serial responder reads one byte at
  a time, so the application answered nothing. The receiver now runs inside a
  serial request. `ADR-054` records the constraint.
- **Correction 2 — ordering and units (`EVID-BK4819-056`):** the receive mode
  word carries the VCO calibration request and must follow the frequency, and
  the source's filter-path split is 280 MHz because its frequencies are in
  10 Hz units.
- **Correction 3 — chip variant (`EVID-BK4829-055`):** the pinned K1 build
  compiles the BK4829 driver, whose power blocks, receive mode, audio bits,
  bandwidth, gain tables, and sub-audio values all differ. `ADR-053` records
  the explicit profile model.
- **Result:** complete. `AFIK-K1-0.8` (30,424 bytes, CRC-32 `be1f7f4a`) was
  written with all 119 pages acknowledged, booted, and answered `probe-rf`.
  The configured register `0x43` read back as `0x4048`, proving the bit-banged
  bus in both directions, and successive samples reported moving RSSI, glitch,
  and noise with the carrier squelch link opening. `EVID-K1-057` records the
  exact values. Audio, sensitivity, calibration, and tone decoding remain out
  of scope and are tracked by `RISK-030` and `RISK-031`.

## RFK1-030 — Audible receive and keypad-operated channel selection

- **Status:** superseded (2026-08-07); audible receive was confirmed here and
  later channel-selection work continued under `RFK1-031` and `K1VFO-033`
- **Objective:** make the K1 application usable as a receiver: audible
  demodulated audio, operator channel selection, and a display which reports
  what the receiver is doing.
- **Scope:** the `PA8` audio amplifier under keypad control; the operating
  screen with channel name, frequency, raw RSSI, squelch, and audio state; the
  shared banked receive controller and channel records driving tuning on the
  target; a published snapshot the serial responder reads without touching the
  bus.
- **Exclusions:** transmit of any kind, channel storage on the radio, squelch
  calibration from external flash, tone decoding, scanning, and menus.
- **Acceptance criteria:** audio is audible with the cable unplugged and
  toggled from the radio itself; channel selection retunes the receiver and the
  display agrees with the serial observation; no serial request touches the
  register bus; every built-in channel is `TxClass::Never`.
- **Audio result:** confirmed. `AFIK-K1-1.1` produced audible receiver noise on
  145.500 MHz when side key one was pressed with the cable unplugged; see
  `EVID-K1-060`. The shared-jack constraint is recorded as `EVID-K1-059` and
  `ADR-055`.
- **Resolution:** the built-in-channel image was superseded before that exact
  observation. Host-programmable selection continued under `RFK1-031`, and the
  later VFO and generated-plan images were confirmed on the unit under
  `K1VFO-033`, `PLAN-037`, and `ARENA-038`.

## RFK1-031 — Host-programmable channels, retained configuration, and the operator shell

- **Status:** software complete, physical confirmation pending (2026-08-07)
- **Objective:** make the receive-only K1 image a programmable radio: the studio
  editor writes channels to it, the radio keeps them across a power cycle, and
  the operator selects them from the keypad.
- **Scope:** the shared `radio-device` configuration service on USART1; the
  reserved retained-configuration flash sector; the bounded programmed
  configuration the receive path and interface consume; the pure operator shell
  with its operating, channel-list, and information screens; the incremental
  canonical-image encoder; and view positions on the banked receive controller.
- **Exclusions:** transmit of any kind, generated-plan expansion on the target,
  squelch calibration from external flash, tone decoding, scanning, and menus
  for the global configuration.
- **Acceptance criteria:** the host programmer writes and reads back a full
  configuration through the device's own byte stream; an over-large candidate is
  refused with `ValidationFailed` before activation; a retained image restores
  to the same state after a restart; every shell intent is receive-only; and the
  application image cannot reach the retained sector.
- **Host result:** complete. 279 workspace tests pass, including the end-to-end
  programming integration test against this exact device configuration, the
  retained round trip, and the refusal cases.
- **Image:** `AFIK-K1-2.0`, 72,480 bytes, SHA-256
  `80c1c6c0bbaf82bf9d4d44db82d13e14d9edb5d9c40173b1d66f615a562f5455`, CRC-32
  `50770197`, Reset `0x080028c1`, `text=71536 data=936 bss=9824`.
- **Remaining:** write the image, then on the exact unit confirm the boot
  information screen, `afik-programmer info`/`list` over serial, a configuration
  written from `afik-studio`, channel selection and the channel list on the
  keypad, and that the configuration survives a power cycle.

## NGUI-032 — Studio usability and named bank operation

- **Status:** software complete, physical confirmation pending (2026-08-07)
- **Objective:** make the editor usable without knowing device paths or the
  storage format, and make the radio's own bank filter readable.
- **Scope:** USB serial detection and selection in the studio with a manual
  override and a bounded baud list; `--device auto`; bank rows which carry their
  kind, so a compact generated plan can be entered with its base frequency,
  spacing, channel count, and transmit class; generated banks preserved through
  a canonical image load; collapsible channel rows, channel duplication, and
  named bank membership; a named bank list on the radio replacing the blind star
  cycle, with the programmed bank name on the operating screen.
- **Exclusions:** automatic connection without an operator action except for an
  explicit `--device auto`, automatic selection of a flashing target, generated
  plan expansion on the K1, and any relaxation of the flasher's gates.
- **Acceptance criteria:** detection never opens a port and never resolves an
  ambiguous choice; an unsupported baud cannot be entered; a generated plan
  survives an image round trip unchanged; the radio names the bank in force and
  clearing a filter is an explicit row rather than a side effect of cycling.
- **Result:** the selection logic, the bank drafts, and the shell are host
  tested; `NGUI-028`'s exclusion of automatic device selection is narrowed to
  automatic *writes*, which still require an operator action.
- **Image:** `AFIK-K1-2.3`, 74,952 bytes, SHA-256
  `b419aa485def159c92de8ba9ad4d2e17db3f705ec712d96445832e378f39f987`, CRC-32
  `e6893970`, Reset `0x080028c1`, `text=74012 data=936 bss=9824`.
- **Remaining:** write `AFIK-K1-2.3` and confirm on the exact unit that Star
  opens the bank list, that the names the studio wrote appear in it and on the
  operating screen, and that Menu applies the filter and "all channels" clears
  it.

## K1VFO-033 — VFO receive mode and studio default sets

- **Status:** complete (2026-08-07)
- **Objective:** give the radio something to tune when nothing is programmed, and
  give the editor a starting plan so a first configuration is not typed by hand.
- **Scope:** a VFO source in the operator shell with keypad frequency entry,
  step tuning, and a step list; one source list holding the VFO, every channel,
  and each named bank; removal of the built-in channel set and the separate
  unprogrammed screen; region default sets in the studio; the K1 flash path in
  the studio brought up to the flasher CLI's gates.
- **Exclusions:** transmit of any kind, scanning, dual watch, a supported-band
  claim of any sort, and firmware read-back, which no bootloader protocol here
  offers.
- **Acceptance criteria:** an unprogrammed radio is usable through the VFO with
  no special mode; a host write moves it onto its channels; the VFO reuses the
  shared banked controller rather than a second receive path; the VFO's bounds
  are representation limits and are documented as such against
  `EVID-BK4819-007`; a K1 write refuses a mismatched bootloader, a missing
  retained backup, or a wrong image CRC-32.
- **Result:** complete, and confirmed on the exact unit. Removing the built-in
  set while adding the VFO left the image smaller: `text` 74012 to 72984 and
  `bss` 9824 to 9328.
- **Image:** `AFIK-K1-2.4`, 73,920 bytes, SHA-256
  `a5cfa7cc11903f8ff393e54e5c92dcfcebe22ab7b3f1cc102ac59f30f1537682`, CRC-32
  `5d732fcd`, Reset `0x080028c1`, `text=72984 data=936 bss=9328`.
- **Remaining:** confirm the bank list contents, the switch onto memory after a
  host write, and VFO tuning from the keypad by observation on the unit.

## PLAN-034 — The channelised space-saving model in the UI and the radio

- **Status:** software complete (2026-08-07); physical confirmation pending
- **Objective:** make a generated plan the stored form of a bank of channels
  everywhere, rather than a storage-format claim no image honoured and an
  editor which presented banks as collections of channel rows.
- **Scope:** a per-channel template inside the generated-bank object and object
  format version 2; complete `ChannelRecord` expansion with derived names,
  plan-bank membership, and reserved identifiers; `ProgrammedMemory` composing
  stored channels with expanded plans behind the existing `ChannelSource`; the
  K1 image accepting, retaining, expanding, filtering, and scanning plans and
  advertising the encoding; the studio editing the template, expanding a plan in
  place, and reporting stored cost against saving.
- **Exclusions:** transmit of any kind; the remaining declared plan encodings,
  which stay model vocabulary; migration of version 1 objects, which are
  rejected; and any supported-band claim.
- **Acceptance criteria:** one plan object programmes a bank of channels a radio
  selects, filters, and scans exactly as it does stored channels; a stored
  channel cannot claim an expanded identifier; the retained-image budget still
  holds a full configuration; the editor shows the channels a plan becomes
  before it is written; host, workspace Clippy, format, and embedded gates stay
  green.
- **Result:** software complete. `radio-channel-plan` expands complete records,
  `radio-storage` carries the template at format version 2, `radio-channel-
  control` composes both channel kinds, the K1 activates four plans and 128
  expanded channels, and the studio edits the template and previews the
  expansion. 48 workspace test binaries pass.
- **Image:** `AFIK-K1-2.5`, 80,344 bytes, SHA-256
  `4e5b9cb6ac653359642a3cd31168caae69c28973ca7d25b11cdc475590932536`, CRC-32
  `6faaf8da`, Reset `0x080028c1`, `text=78576 data=1768 bss=11560`.
- **Physical write:** `AFIK-K1-2.5` was written to the exact unit on 2026-08-07,
  `314/314` pages acknowledged in transaction `6c497bdb`,
  `status=acknowledged_not_read_back`.
- **Remaining:** confirm on the unit that a plan programmed over serial appears
  as named channels, that its bank filters, and that the retained configuration
  survives a power cycle.

## EEPROM-035 — Channels and settings in external memory, flash for firmware only

- **Status:** complete (2026-08-08), confirmed on the exact unit
- **Objective:** move a radio's configuration out of the internal flash which
  holds its firmware and into the external memory it already carries, so
  programming a radio cannot compete with the space its own code needs and a
  configuration is not bounded by a spare sector.
- **Scope:** a bounded external serial-memory driver with a claimed-region
  boundary; the K1 board adapter on hardware `SPI2`; retention moved off
  internal flash with the reserved sector and its module removed; the region
  size declared in the capability profile and shown by the studio as space used
  and free; a read-only identification probe reported on the information screen.
- **Exclusions:** the radio's own firmware data, which AFIK never writes;
  wear levelling; power-loss atomicity of the erase-before-write boundary, which
  remains `RISK-004`; and any use of the remaining 2 MiB beyond the claimed
  region.
- **Acceptance criteria:** a configuration written over serial survives a power
  cycle with no internal-flash store present; an AFIK write cannot address the
  vendor's region; a memory which does not answer leaves a working receiver;
  host, workspace Clippy, format, and embedded gates stay green.
- **Result:** complete. `radio-eeprom` is the driver, `eeprom_bus` frames the
  transfers over the peripheral, and `py32f071_eeprom` claims one four-kilobyte
  region at one megabyte. `EVID-K1-061` identifies the fitted device as a 2 MiB
  Boya-family part answering `68 40 15`, correcting the pinned source's Puya
  part. `EVID-K1-062` records a PMR446 plan written as one 46-byte object,
  retained across a power cycle, and restored as sixteen channels.
- **Corrections made during this work:** `AFIK-K1-2.5` exhausted the stack and
  did not start; `2.6` to `2.9` drew the boot information screen and then
  ignored every key, because the interface task waited for the serial task's
  first publication. ADR-061 removes that coupling and adds the memory state and
  serial counters an operator can read without a host. The serial task also read
  the UART one byte per await, which lost bytes whenever the interface task held
  the core for a bit-banged BK4819 transfer; it now reads a frame at a time by
  DMA with idle-line delimiting.
- **Image:** `AFIK-K1-3.3`, 81,376 bytes, CRC-32 `2ede2fef`, Reset `0x080028c1`.
- **Physical confirmation, 2026-08-08:** fifteen objects and a 685-byte image
  spanning three pages were written, read back, power-cycled, and read back
  byte-identical. `list` and repeated `info` exchanges succeed with the radio
  holding a configuration, which the one-byte-per-await serial read had made
  impossible.
- **Remaining:** the erase-before-write boundary has no power-loss story. A
  power cut during a retain leaves the region erased and the previous
  configuration gone. Two alternating regions with a commit pointer would fix
  it, and the 2 MiB device has ample room; tracked as `RISK-004`.

## FIELD-036 — Operator fixes from the first day of real use

- **Status:** software complete (2026-08-08); physical confirmation pending
- **Objective:** fix what the first day of carrying the radio showed: the arrow
  keys moved lists the wrong way, audio was hidden behind a key, the squelch was
  never applied so the speaker carried noise, and the pack went flat without
  warning.
- **Scope:** arrow-key direction across every screen; receive audio enabled by
  the tuned channel rather than a key; a derived squelch threshold set applied
  through the existing driver path with the speaker following the squelch link;
  a handset settings menu whose choice is stored and retained; the battery sense
  path, its calibration, its discharge curve, and the operating-screen
  indicator; the studio's existing radio-wide squelch control labelled with what
  it governs.
- **Exclusions:** per-channel squelch as an override, which this image
  deliberately ignores in favour of the radio-wide level; battery type
  selection, which AFIK cannot read; charging-current detection, which the
  pinned firmware does not measure on this board either; and any further
  settings rows.
- **Dependencies:** `EEPROM-035`.
- **Tests required:** arrow direction on every list and both operating modes;
  the derived threshold set's monotonicity, hysteresis, and acceptance by the
  driver's own validator; the settings menu's navigation, digit entry, cancel
  path, and adoption of a programmed level; squelch storage preserving every
  other object and field, including on an unprogrammed radio; the battery
  scale, averaging, curve, and its refusal to report without a calibration.
- **Acceptance criteria:** up moves towards the first row everywhere except VFO
  tuning; no key routes audio; the operator's squelch level reaches the chip and
  survives a power cycle; the charge is shown or honestly absent; workspace
  tests, Clippy, format, and the K1 image gates stay green.
- **Result:** software complete. `SquelchThresholds::for_level` derives the set,
  `store_squelch` rewrites the stored configuration through the validating path,
  and `battery` owns the voltage and curve arithmetic with no hardware in it.
  `EVID-K1-063` records the sense pin, converter configuration, calibration
  location, and curve from the pinned source, including that the F071
  precalibration delay matches the value the vendored HAL already carried.
- **Vendor change:** the vendored `py32-hal` had its ADC module disabled for the
  F071 because the generated metadata carries no analogue pin table and one
  constant was F072-only. Both are now supplied from the pinned Puya driver, and
  only the one evidenced channel is declared.
- **Image:** `AFIK-K1-3.6`, 85,752 bytes, CRC-32 `d752ab27`, Reset `0x080028c1`,
  SHA-256 `8c876273fce92282c61af4d64d13e4da8d9edc72702e51487e115d5dfa2dab3d`.
- **Correction made during this work:** `AFIK-K1-3.5` read `BAT ---%` because
  the `radio-eeprom` vendor bound sat at `0x010000`, below the calibration it
  was meant to allow reading and below data it was meant to stop AFIK erasing.
  `EVID-K1-064` establishes the real extent and the bound is now `0x020000`.
- **Remaining:** every physical claim. The squelch thresholds are AFIK's own and
  no level has been heard on air; the battery percentage has never been compared
  against a meter. Both experiments are named in `EVID-K1-063` and `RISK-034`.

## PLAN-037 — The shared bank model, unbounded, and held once

- **Status:** complete (2026-08-08); the operator observation it left open was
  made under `ARENA-038` on 2026-08-09
- **Objective:** answer whether the radio and the studio fully supported the
  channelised bank model they were meant to share. They did not. Remove the
  bound that was not a cost, name channels the way a band plan does, implement
  the second arithmetic encoding, correct what the editor claimed about bank
  membership, and stop the radio holding its configuration four times over.
- **Scope:** `radio-channel-plan` designator and first-number naming,
  `ChannelFlags::CALLING` and `GeneratedBank::calling_index`, the
  `LinearFixedOffset` encoding derived from the transmit offset,
  `generated_channel_parts` and `ChannelSource::member_at` so lookup and bank
  filtering resolve arithmetically; storage format version 3; `Programmed` as an
  index over one shared encoded snapshot; the K1 stack reserve; the studio's
  membership claim and plan editors; the CLI bank spec tail; every preset as
  plans; the leading frame delimiter in the serial transport.
- **Exclusions:** the byte arena and per-encoding plan tails, which `ARENA-038`
  carries; marine and business-radio presets, whose numbering is not arithmetic
  and which need a table encoding rather than a linear plan; the 8.33 kHz
  airband channel numbering for the same reason.
- **Dependencies:** `PLAN-034`, `EEPROM-035`, `FIELD-036`.
- **Tests required:** designator and calling-channel naming including the
  length bound refused at construction; identifier unpacking; a repeater plan's
  offset applied at both ends of its range; a band-sized plan accepted whole; a
  stored channel selected beside a plan's channels under one bank filter, proved
  by building the radio's own store from the editor's own validated objects;
  every preset validating, compiling, and expanding to the designators a band
  plan uses; the CLI spec tail including a preserved trailing space.
- **Acceptance criteria:** no image bound on expanded channels; host and device
  agree on bank membership by construction rather than by comment; the studio
  and the CLI can write the same plan; workspace tests, Clippy, format and the
  K1 image gates stay green.
- **Result:** software complete. Bounds are now the eleven bits the identifier
  packing has for an index, the `u16` selection space, and the storage each
  device advertises. `Programmed` holds roughly ninety bytes whether the radio
  holds four channels or four thousand; the objects live once, encoded, and a
  record is decoded on the lookup that needs it. Static RAM fell from 10,988
  bytes to 9,284 with 7,100 free.
- **Written to the exact unit:** three generated-bank objects, 177 stored bytes,
  forty channels, read back at generation 1. PMR446 as `PMR 1` to `PMR 16`, UK
  2 m simplex as `S8` to `S23` with `S20 CALL`, UK 70 cm simplex as `SU16` to
  `SU23` with `SU20 CALL`. The same set as explicit records is twelve channels
  for 570 bytes.
- **Image:** `AFIK-K1-4.3`, 84,120 bytes, CRC-32 `bfe9f80e`, Reset
  `0x080028c1`, SHA-256
  `10e7a83cf127e7fb0675151a76f16a73e7ff3176b8a9b1dc696c8244fd1e4717`.
- **Corrections made during this work.** `AFIK-K1-4.0` did not start. A slot
  rebalance added 512 bytes of statics and left 5,396 bytes of stack, and the
  scripted floor which should have caught it was set at 4,096, so it packaged
  and flashed without complaint. `RISK-033` had predicted exactly this and
  described the gate as missing when it was merely too low. The floor is now
  6,144 bytes and asserted at link time in `stack-headroom.x`, confirmed to fire
  by raising it above the current headroom rather than by assumption.
- **Second correction.** The application serial link had never worked. The radio
  heard every frame and refused every one, and two theories about why — a
  poisoned decoder and a wrong inherited clock — were both wrong; the inherited
  clock is 48 MHz in every cable state and divides to 38,400 exactly. A hello
  encodes to fourteen bytes and the radio received sixteen. Opening a USB serial
  port puts a byte or two on the line, frames were delimited only at the end, and
  the rubbish folded into the packet. The transport now sends a leading
  delimiter. What found it was counting rather than reasoning: the discarded
  counter added mid-investigation turned "bytes in, nothing out" into "a
  complete packet was refused", and comparing sent against received length did
  the rest.
- **Closed on 2026-08-09:** the radio was turned to `S20 CALL` and read
  145.500000, before and after a power cycle. The derived designator naming and
  the calling marker are observed. The image which was on the unit at the time
  was `AFIK-K1-5.0`, so what was confirmed is this model as `ARENA-038` left it.

## ARENA-038 — Storage-shaped bounds and minimal plan encodings

- **Status:** complete (2026-08-09), confirmed on the exact unit
- **Objective:** make stored bytes the only bound a radio declares, and charge
  each plan encoding what it actually costs. `MAX_CHANNELS`, `MAX_BANKS`,
  `MAX_GENERATED_BANKS` and `kind_limits` should stop existing.
- **Scope:** replace the fixed `MAX_OBJECT_DATA` object slot with a packed byte
  arena and a directory of `(key, offset, length)`; make `configuration_bytes`
  the binding capability rather than a displayed one; split `GeneratedBank` into
  a shared core and a per-encoding tail so a simplex plan stops carrying a
  repeater's four bytes of offset and the declared encoding is a property rather
  than an inference from a zero; a storage format bump carrying both.
- **Rationale:** a `StorageObject` was 70 bytes whatever it held — 42 for a
  channel, 22 for a named bank, 16 for the configuration, 59 for a plan — so the
  K1's active and candidate snapshots reserved 3,220 bytes to hold at most about
  880. The remaining declared encodings make this necessary rather than merely
  wasteful: `TableSimplex`, `TableMixedDuplex` and `SparseExceptions` are all
  variable length, and a slot store must size every slot for the worst case.
- **Dependencies:** `PLAN-037`.
- **Tests required:** arena compaction on replace and delete, including that a
  failed transaction leaves the active bytes untouched; per-encoding round trips
  at their own lengths; a project refused for bytes rather than for object
  count; the K1 accepting whatever fits its advertised region.
- **Acceptance criteria:** one declared number bounds a configuration; a simplex
  plan encodes shorter than a repeater plan; no per-kind object limit survives.
- **Result:** done, and the directory was not needed. Entries are packed end to
  end as `(kind, id, length, payload)` in strict key order, which is byte for
  byte what a canonical image carries after its header, so an arena and an image
  payload are the same bytes: retaining the active snapshot is a copy rather
  than a sorted rebuild, the shared snapshot the interface reads is that copy
  again, and a listing is a page of an order the store already holds. Decoders
  take a borrowed object, so nothing is copied into a worst-case buffer to be
  read.
- **What a device declares:** `configuration_bytes`, and everything else follows
  from it. `max_objects` is the count those bytes imply given the shortest
  object any kind encodes to — an upper bound rather than a second limit — and a
  host refuses a project for the bytes it needs, naming both numbers.
  `MAX_CHANNELS`, `MAX_BANKS`, `MAX_GENERATED_BANKS` and `kind_limits` are gone,
  as is `KindLimits` itself. `Programmed` holds no per-channel table at all: its
  arrays are the sixteen banks a membership mask addresses, which is structural.
- **Plan encodings:** a generated bank is a 56-byte shared core plus its own
  family's tail. `LinearSimplex` adds nothing; `LinearFixedOffset` adds four
  bytes of transmit offset. Both were 59 in version 3. The family is declared
  rather than inferred from a zero offset, so a repeater sub-band parked at zero
  survives a write and a read-back as what it is. The editor and the CLI ask for
  an offset and `linear_from_offset_with` is where that becomes a declaration.
  Storage format version 4.
- **What the K1 gained:** it declares 1,264 packed bytes, which is the 1,280 it
  retains less the image header. That is about twenty-six explicit channels
  where eight were allowed, or as many plans as fit, in any mixture. Static RAM
  fell from 9,284 bytes to 8,188 with 8,196 bytes of stack headroom.
- **Image:** `AFIK-K1-5.0`, 83,352 bytes, Reset `0x080028c1`, SHA-256
  `d085eef57ce72656d708cf714de9486cd801ca83159d7c50426bced553a1014b`.
- **Confirmed on the exact unit:** flashed over `/dev/ttyUSB0` at K1 bootloader
  `7.03.01`, `326/326` pages acknowledged. `info` reports storage 4,
  `configuration_bytes=1264`, `max_objects=60`, encodings `0x0003`. Three
  simplex plans — PMR446, UK 2 m, UK 70 cm — were written as 183 packed bytes
  expanding to forty channels and read back at generation 1 as three 56-byte
  objects, where version 3 stored 59 each.
- **Observed on the handset, at last:** the radio was turned to `S20 CALL` and
  read 145.500000, and did so again after a power cycle. The derived designator
  naming, the calling marker, the arithmetic frequencies and the retention of a
  version-4 image in external memory are now seen rather than asserted. This
  closes the physical claim `PLAN-037` left open as well as this one.
- **One thing that cost time and was not a fault:** after flashing, the host
  reported no complete response three times running while the radio's own
  counters showed frames received and answered. A raw sixteen-byte hello sent
  straight at the port returned the exact fifteen-byte reply, and the CLI then
  worked unchanged and has worked since. Nothing was found to fix, and nothing
  was changed on the strength of it; it is recorded because a reader of this
  file may hit it and should not go looking for a framing fault that the
  counters and the raw exchange had already ruled out.

## SCAN-039 — Scanning from the handset, and a remembered place

- **Status:** software complete (2026-08-09), flashed to the exact unit;
  the two handset observations are open
- **Objective:** let the operator scan the source they are already listening to,
  and let the radio come back to where they left it after a power cycle.
- **Scope:** a hold input in the shell and the two scan intents it produces; the
  scan clock in the interface task, which is the missing half of the controller
  the receive path already had; a `SCAN` marker on the operating screen; one
  sixteen-byte operator-state record in its own erase sector, written by the
  task which owns the memory bus and read once at start-up beside the
  configuration it refers to; and a bank walk which stops expanding the channels
  it steps over.
- **Rationale:** `SCAN-007` built the whole deterministic scan — dwell, hold,
  three resume modes, skip flags, stale-token safety — and `RX-027` wired its
  selection half into the radio. Nothing ever armed its timers, so no key could
  start one. Separately, a radio which forgets its channel every battery change
  is one the operator sets up again before every use, and squelch was already
  retained for exactly that reason.
- **Dependencies:** `SCAN-007`, `RX-027`, `EEPROM-035`, `ARENA-038`.
- **Tests required:** a source which counts the records a walk builds; the star
  tap and the star hold as distinct inputs; every key stopping a running scan;
  a record round trip, an erased slot, a foreign version, and every single-byte
  corruption refused.
- **Acceptance criteria:** a hold scans and any key stops it; the place survives
  a power cycle; a filtered walk expands only the channels it lands on.
- **Result — the walk:** selection and scanning asked the source for a record
  before asking whether the channel was even in the active bank, so stepping
  through a sixteen-channel bank inside a four-hundred-channel plan expanded
  every channel in between and discarded all of them. Membership is now asked
  first, which a plan answers from arithmetic, and a record is built only where
  a scan has to read the skip flag or where a channel is actually selected. The
  counting source proves it: one record for the channel selected, none for the
  channels walked over. Nothing is materialised either way — the record built is
  dropped again — so a scan of a band-sized plan costs no RAM beyond one record.
- **Result — the key:** star commits on release rather than on the way down, so
  a tap opens the source list and a hold scans it. Deciding on the press would
  either open a list the hold then had to close again or lose the tap. While a
  scan runs every key stops it and does nothing else, so no key both abandons
  the scan and acts on the channel it happened to be sitting on.
- **Result — the clock:** the controller expresses every deadline as a timer
  directive rather than a wait, and the interface task now arms, cancels, and
  reports expiry against it. Squelch observations already reached the
  controller and were discarded; their updates now reach the clock, which is
  what makes a busy channel hold. A stale token after a source change is
  answered as unchanged, which is what the token existed for.
- **Result — the place:** source, bank, channel index, the channel identifier
  beside it, VFO frequency and tuning step, sixteen bytes with a CRC-16. It is
  programmed into the next erased slot of its own erase sector at `0x101000`,
  two hundred and fifty-six slots to a sector, so the ordinary cost of
  remembering a channel change is one page program and an erase every two
  hundred and fifty-six saves. It is deliberately not a configuration object:
  the configuration is a canonical image erased and rewritten whole, and turning
  the channel knob must neither spend the channel list's erase cycles nor put it
  at risk in the window a place is being written. See `ADR-067`.
- **Result — what a restore checks:** a place is written only once the selection
  has held still for three seconds and never while scanning, so a walk across a
  bank is one record rather than thirty. On the way back in the channel
  identifier recorded beside the index has to still match what that index names,
  because a host may have reprogrammed the radio since; when it does not, the
  radio starts at the top of the view rather than on a channel nobody chose. A
  bank the current configuration no longer populates is dropped, and a VFO
  frequency or step outside range leaves the defaults in place.
- **Image:** `AFIK-K1-5.1`, 87,952 bytes, Reset `0x080028c1`, SHA-256
  `72f399dcf672efe7e68814d22c380302b7dcbf7244bd15c3231dfc4118151cfc`. Static RAM
  8,348 bytes, up 160 from `5.0`, with 8,036 bytes of stack headroom. The image
  grew 4,600 bytes because the controller's scan half was previously unreachable
  and stripped.
- **Flashed to the exact unit:** `/dev/ttyUSB0`, K1 bootloader `7.03.01`,
  `344/344` pages acknowledged. `info` answers protocol 1, storage 4,
  `configuration_bytes=1264`; `list` reads back the three generated banks at
  generation 1, so the configuration in external memory survived the reflash.
- **Result — what actually paced the scan:** not the dwell. Receive samples ran
  on a free-running sixty-millisecond grid which a retune did not reset, so the
  first reading of a scanned channel landed anywhere inside the dwell and how
  many readings a channel got at all was a matter of phase; a hundred and fifty
  millisecond dwell got two or one, and anything shorter would have got none. A
  retune now schedules its own first sample, and while scanning the samples run
  every five milliseconds and the interface loop every one, so a dwell is
  quantised to something far below itself rather than to the operating cadence.
- **Result — the first reading after a retune:** it updates the meter, which is
  honest about what was read, but it does not get to tell a scan that a channel
  is busy. How long this board needs to settle after a retune is unmeasured —
  `RISK-008` — and a false stop costs the whole hold, which is the one failure
  an operator would notice and could not explain. This is a workflow rule, not a
  chip fact, and it is stated as one.
- **Result — the dwell is the operator's:** a settings row beside squelch,
  twenty to three hundred milliseconds, stored through `store_setting` and
  retained like every other handset setting. The floor is deliberately not a
  number this firmware states: the list reaches well below the conservative
  default so the operator can find it on the unit, and well above it so a radio
  which needs longer can be given it.
- **Second image:** `AFIK-K1-5.2`, 90,064 bytes, SHA-256
  `cc349e3a062957e94ba68f919c2528d11d4961d8e259e486c8729dc262327776`. Static RAM
  8,356 bytes, 8,028 bytes of stack headroom. `352/352` pages acknowledged and
  the operator interface runs.
- **Confirmed on the exact unit:** holding star walks the bank and the scan
  stops on a signal. The first dwell measurement came with it: 100 ms stops, 60
  ms does not, so the floor is between them. `EVID-K1-069` records it and
  `RISK-008` is narrowed rather than closed — one pass against one signal,
  measuring the whole retune-settle-sample loop rather than any chip behaviour,
  so 100 ms bounds the floor from above rather than being it.
- **Third image:** `AFIK-K1-5.3`, 90,200 bytes, SHA-256
  `ad3dd476cd471c719d64b7bb28b4759a5674f1ec7988542af9e6c7d065b69454`, static RAM
  8,356 bytes with 8,028 bytes of stack headroom, `353/353` pages acknowledged.
  It re-ranges the handset dwell list to 60, 70, 80, 90, 100 and 150 ms. The
  list is an instrument for bisecting the bracket rather than a menu of
  supported speeds, and re-ranging it again as the bracket closes is expected
  rather than a change of design. A compile-time assertion ties the default row
  to `RadioConfig::conservative()`, so a future re-range cannot silently leave
  an unprogrammed radio scanning at a dwell its own menu cannot name.
- **Result — why it stopped on the wrong channel:** the squelch, not the scan.
  Beside a close transmitter the scan stopped two positions early; frequency and
  channel numbering were correct and the audio on the right channel was the
  transmission, which is what rules out an indexing fault. AFIK gates on carrier
  strength alone, and its nine levels spanned 3 dB each from about -130 dBm to
  about -106 dBm — the entire range below what a handheld a metre away delivers
  into the adjacent channels, so every level opened on it. `EVID-K1-070`.
- **Result — the range:** 8 dB a step, about -130 dBm to about -66 dBm. Level
  one is unchanged for weak-signal work and the top now reaches above a strong
  local signal. The steps are coarser and that is the trade: an operator who
  cannot shut the squelch at all has no useful resolution anywhere. Each squelch
  row now names the carrier strength it opens at, because the operator is
  choosing a threshold against a signal and a bare level number has no scale.
  These were already AFIK's own values rather than the unit's, which is what
  makes re-ranging them honest.
- **Not the fix, and recorded as such:** noise-gated squelch is what
  distinguishes a signal on this channel from a strong one beside it. Its
  thresholds are per-unit calibration data, the pinned firmware reads them from
  the vendor block, and AFIK must not invent them. The remaining work is to read
  this unit's own squelch calibration exactly as its battery calibration is
  read; `RISK-008` carries it.
- **Fourth image:** `AFIK-K1-5.4`, 90,304 bytes, SHA-256
  `126db4efddd3115fff7602dc502933ce2b1813d411e9d9f6bee2bcd03623f861`, `353/353`
  pages acknowledged.
- **Result — leaving the menu moved the operator:** a configuration republish
  was being treated as a boot. Only the boot publication carries a retained
  place, so a squelch or dwell change rebuilt the controller with nothing to
  keep and landed on the first eligible channel; the same path would have
  dragged a VFO operator into memory mode. Adopting the settings a
  configuration carries is now separate from restoring where the operator was,
  and the latter runs once. `BankedReceiveController::activate_at` keeps one
  selection across a rebuild, honouring it only if the index still exists,
  still passes the bank filter, and still carries the identifier recorded
  beside it — the same rule the retained place is restored under, now in one
  place and unit-tested rather than repeated in the firmware glue. A radio
  which had no channels and gains some still leaves the VFO for them; choosing
  a source still lands on that source's first channel, because that is a
  rebuild the operator asked for.
- **Fifth image:** `AFIK-K1-5.5`, 90,384 bytes, SHA-256
  `9e20abd35b31adf07732b60ace375f281302884414017f61cca0695e057b2bd5`, static RAM
  8,364 bytes with 8,020 bytes of stack headroom, `354/354` pages acknowledged.
- **Result — the settings the unit wanted:** squelch level 3 and a 90 ms dwell.
  `RadioConfig::conservative()` now carries 90 ms where it carried 150;
  `SquelchLevel::CONSERVATIVE` was already 3. `EVID-K1-071` records these as a
  working setting found by trying rather than a measured floor — 90 ms sits
  inside the 60-to-100 bracket rather than at its edge — which is why the dwell
  stays programmable instead of becoming a constant.
- **Result — the handset dwell menu is gone:** it existed to find that number on
  the unit and had done it. A compiled list can only offer the rows it was built
  with; the store already held the value at full resolution. `ADR-069` records
  the rule the menu failed: a settings row has to be something an operator
  changes in the field, with no host in reach.
- **Result — and the CLI can now write it:** the editor already exposed the
  whole `RadioConfig` at whole-millisecond resolution, but the CLI could not set
  it at all, so removing the handset control would have left the dwell reachable
  from one front end only. `--config SQUELCH:SCAN_DWELL_MS` closes that, either
  field `-` to keep its default. Nothing is rounded to a list and nothing is
  clamped: a unit which wants 84 ms is given 84, and a device reads back what
  was written to it. A zero dwell is refused by the domain's own validation
  rather than silently written as a scan which can never advance.
- **Sixth image:** `AFIK-K1-5.6`, 88,104 bytes, SHA-256
  `249b3704f4a963306e7b4595983f2a647cc1f6d08fb36a12169bc94e210129e3`, static RAM
  8,356 bytes with 8,028 bytes of stack headroom, `345/345` pages acknowledged.
  Removing the menu returned 2,280 bytes of image and 8 bytes of RAM.
- **One thing the workspace gate does not cover:** the K1 async binary builds
  only for the embedded target, so `cargo clippy --workspace --all-targets` does
  not compile it. A `match` arm deleted from its render function passed the
  whole host gate and failed only under `tool/build-k1-async.sh`. Run that
  before believing a green workspace.
- **Open, and only observable by hand:** that the channel the radio is left on
  is the channel it comes back to after a power cycle; where in 60 to 100 the
  dwell floor lies; and whether a raised squelch level now stops the scan on the
  transmitted channel. None can be established from the host.
- **Not confirmed over serial:** the `ARENA-038` serial dead end recurred
  immediately after the `5.2` write and did not clear, so `info` and `list` have
  not questioned this image the way they questioned `5.1`. The handset is its
  only witness so far.

## SWEEP-040 — What an off-tune probe can see

- **Status:** not started
- **Objective:** measure how a received signal's indicated strength falls off as
  the receiver is tuned away from it, on the exact unit, and decide from that
  measurement alone whether an off-tune probe can stand in for visiting a
  channel. This task produces evidence and a go/no-go. It authorises no scan
  behaviour and encodes no threshold.
- **Why now:** `EVID-K1-070` recorded a strong local transmitter opening the
  squelch on channels beside its own. That is a defect for the linear scan and a
  gradient for a faster one: a generated bank is a contiguous arithmetic range,
  so if off-tune strength falls off monotonically, the bank can be searched
  rather than walked. Whether that is true, and over what span, is unmeasured.
- **Scope:** bench measurement, `docs/hardware-evidence.md`, `RISKS.md`,
  `STATUS.md`. No crate changes, no firmware image, no new scan mode.
- **Dependencies:** `SCAN-039`. Requires a controllable transmitter, the exact
  unit, and a generated bank whose span is wider than the expected falloff.
- **Assumptions:** the existing `ReceiveMetrics` surface — `rssi_dbm_x2`,
  `glitch`, `noise` — is what a probe would have to decide on, so the
  measurement records those and nothing the firmware cannot read. Squelch levels
  are AFIK's own numbers rather than the unit's calibration, so a level is
  recorded as context and never as a measured quantity.
- **Likely files:** `docs/hardware-evidence.md` (new `EVID-K1-072` onward),
  `RISKS.md` under `RISK-008`, `STATUS.md`.
- **Safety:** the DUT does not transmit for any part of this task. The
  transmitter under test is a separate unit on a known frequency and power, and
  its identity, power setting and distance are recorded with every reading.

### Questions it has to answer

1. **Detection width.** How far off-tune can the receiver be and still separate
   a present signal from an absent one, as a function of the signal's
   on-channel indicated strength? This number sets probe spacing. If it is not
   comfortably larger than the bank's channel step, the whole idea is dead and
   the task ends there.
2. **Shape.** Is the falloff monotone either side of the signal, or are there
   shoulders, plateaus, images or spurious peaks? A hill-climbing search needs
   a single hill; anything else changes the search or rules it out.
3. **The blind band.** What on-channel strength is required before any off-tune
   probe registers at all? Everything between that and the squelch threshold is
   a signal the linear scan finds and a probing scan cannot. Record the size of
   that band in dB — it is the honest cost of the fast path.
4. **Which indicator.** Does `glitch` or `noise` separate on-channel from
   off-channel more sharply than `rssi_dbm_x2`? This also feeds the noise-gated
   squelch work `RISK-008` already carries.
5. **Steadiness.** How much does the indicated strength of an unchanged signal
   move between readings taken over the span of a whole search? A search
   compares readings taken at different times, so this sets whether that
   comparison means anything, and whether re-reading the starting point can
   detect a transmission that changed underneath it.
6. **Probe cost.** How long does a reading that only has to be compared need,
   against the 90 ms a dwell needs to make a squelch decision? A probe may be
   cheaper than a dwell, and if it is, that multiplies any structural saving.
   **This one cannot be answered by the host-driven sweep.** The radio holds
   its bit-banged bus idle for a quarter second after the last serial byte, so
   a host-driven reading measures that window rather than the receiver. Probe
   cost has to be measured by the radio's own scan, separately.
7. **Two signals.** With two transmitters in the same bank, what does the
   falloff look like between them? This is where a single-hill search converges
   on the wrong answer, and the failure needs to be seen rather than assumed.

### Method

- Record equipment identity, both radios' board and chip markings, the bank
  under test, ambient conditions, and the raw readings themselves. Sweep the
  receiver across the bank in single channel steps with the transmitter on a
  fixed channel, at several transmitter powers and distances, and repeat each
  sweep enough times to show spread rather than one pass.
- Take a no-signal sweep of the same bank first. Every claim about detecting a
  signal is a claim about a difference from that baseline.
- Vary one thing at a time and say which. A falloff curve that mixes changed
  power with changed distance measures neither.

### Acceptance criteria

- Every question above is answered with recorded readings, or explicitly
  recorded as unanswered and why.
- Results land in `docs/hardware-evidence.md` as numbered evidence carrying the
  confidence they have earned — one unit, one board, one afternoon — and
  `RISK-008` is updated with what is now known and what still is not.
- A stated go/no-go on probe spacing against the channel step, with the number
  behind it.
- No threshold, spacing, width or timing from this task is written into any
  crate. A later task may propose them; this one establishes them.

### Follow-on sequence, if the measurement says go

1. `SWEEP-041` — the search as a manually selected scan mode, chosen by the
   operator, with the linear scan untouched beside it. The point is to find out
   whether it is useful in the field at all before it earns any automatic
   behaviour or any more image space.
2. `SWEEP-042` — automatic fallback to the linear scan when no probe registers,
   so the operator stops having to know which mode to be in.
3. `SWEEP-043` — the interleaved sweep order, if the measurement supports it:
   passes strided by the probe spacing whose union is every channel, so full
   coverage costs what the linear scan already costs and strong signals are
   found in the first pass. This may make step 2 structural rather than a mode.

## CTRL-044 — Host control of the receiver over serial

- **Status:** software complete, unconfirmed on the unit (2026-08-09)
- **Objective:** implement the reserved `Service::RuntimeControl` as a second
  peer driving the operations the handset already drives — ask what the radio is
  doing, stop a scan, choose VFO or memory, tune, select a channel, read the
  metrics. Enough to drive `SWEEP-040` from a host, and the beginning of PC
  remote control rather than a measurement fixture.
- **Why this way:** `SWEEP-040` needs hundreds of readings across channels,
  powers and repeats, which is not a handset measurement. The host has to drive
  it, and full radio control over serial is wanted for its own sake, so the
  measurement is one caller of a general surface rather than its reason to
  exist. `Service::RuntimeControl = 2` has been reserved since `FOUND-001` with
  no commands defined; this fills it.
- **The key point:** this adds no radio behaviour. `BankedReceiveController`
  already exposes every operation needed — `enter_vfo`, `enter_memory`,
  `tune_to`, `tune_up`/`tune_down`, `select`, `select_next`/`select_previous`,
  `set_bank`, `set_monitor`, `start_scanning`, `stop_scanning` — and the
  interface task already calls them after decoding a key press. The host joins
  at that same seam and its results flow through the same update application, so
  retunes and scan timers are handled identically however the operation arrived.
  What this task adds is an encoding, a way across the task boundary, and a
  query.
- **Scope:** `radio-protocol` (commands in the runtime-control range),
  `radio-device` (service dispatch), `radio-firmware-k1` (serial-to-interface
  command path and the state query), `radio-programmer-cli` (control commands
  and a sweep), `radio-sim` (deterministic coverage), docs.
- **Dependencies:** `SCAN-039`, `ARENA-038`.
- **Assumptions:** the existing framing, CRC, sequence and capability
  negotiation are unchanged. Capability negotiation must advertise runtime
  control so a host can discover it rather than probe for it.

### The boundary that makes it safe

- **Receive only.** No runtime-control command mints, implies, or enables
  transmit authority. A command that would need it is not added to this service.
- **Two peers, one state machine.** The host does not take ownership and the
  interface is never suspended. Both drive the same controller, and the
  controller's state is the single answer to what the radio is doing. There is
  no host-control mode to be stuck in, nothing to time out, and no release path,
  because nothing was taken. The operator's keypad keeps working throughout.
- **The host can see what it is racing.** An operator can act in the middle of
  a host sequence, and the mode/selection query is how a host notices rather
  than something that protects it from happening. A measurement that wants an
  undisturbed radio re-reads the state and says so if it moved.
- **Control is not configuration.** A host-driven tune or selection is live
  state on exactly the same terms as the operator's: it never becomes a
  configuration write, and it reaches the retained place only by the rule that
  already governs the operator's own selection.
- **Cross-task, because the tasks own disjoint hardware.** The serial task owns
  USART1 and cannot touch the controller the interface task owns. A command
  crosses as a posted request and the resulting state comes back, mirroring the
  existing path in the other direction where the interface reports its place and
  the serial task writes it down.

### First slice

- **Query:** current mode — memory, VFO, or scanning — plus the selection,
  bank, and tuned frequency. This is what the display shows, in a form a host
  can act on.
- **Operations:** `StopScan` and `StartScan`, `EnterVfo` and `EnterMemory`,
  `TuneTo(hz)`, and `SelectChannel(index)`. Each maps to one existing
  controller method and returns the resulting state.
- **Metrics:** the raw `ReceiveMetrics` fields already defined —
  `rssi_dbm_x2`, `glitch`, `noise`, `squelch_open` — plus the frequency actually
  tuned and the sample counter. The counter is not decoration: `RISK-008` has
  the settle time unmeasured and the firmware already refuses to let the first
  sample after a retune declare a channel busy, so a host must be able to tell a
  settled reading from the one that arrived too early.
- **Host side:** a sweep that walks a range, waits for a fresh sample at each
  frequency, and emits one row per reading in a form that plots without further
  parsing.
- Nothing else. Squelch level, monitor, dual watch, audio routing and the
  settings surface are the obvious next commands and are deliberately not here.

### Tests required

- Command matrix over the new service: unsupported command, malformed payload,
  out-of-range frequency, and an operation refused by the controller in the
  state it was in.
- Equivalence: a host operation and the keypad intent that drives the same
  controller method leave identical state, proven against the controller rather
  than asserted.
- A host operation arriving while a scan runs behaves exactly as the keypad
  equivalent does, including the scan timer consequences.
- No configuration write from any runtime-control command, proven against the
  arena, and the retained place written only under its existing rule.
- Deterministic simulator coverage of the whole slice, and identical scripts
  producing identical traces.

### Acceptance criteria

- A host can ask what the radio is doing, stop a scan, tune, and read metrics,
  and the handset behaves throughout as if the host were not there.
- Capability negotiation advertises the service.
- The workspace gate passes and `tool/build-k1-async.sh` builds the image;
  image and RAM cost recorded in `STATUS.md`.
- `SWEEP-040` can be run from a host against the resulting image.

### Completion notes

- `Service::RuntimeControl` carries eight commands: state, metrics, stop and
  start scan, VFO and memory, tune, and select. Each maps to one existing
  controller method, and two tests assert that a host operation and the keypad
  intent driving the same method leave identical state.
- `DeviceService` gained a two-phase push. Replay is checked before the request
  is classified, so a resent frame replays its cached answer rather than
  performing the operation again. `push` is unchanged, so the simulator and
  every host-side driver still answer `UnsupportedService`.
- Samples are counted and kept so a host can distinguish a fresh reading from a
  repeated one. `rf-sweep` waits on that counter.
- Two `DeviceErrorCode` values were added rather than squashing refusals into
  `Internal`: `InvalidState` for an operation the radio cannot perform from
  where it is, and `OutOfRange` for a value it cannot reach.
- **Not established:** none of this has run on the unit. Normal-mode serial is
  unproven on images after `5.2` — the `ARENA-038` dead end — and this whole
  path rests on it.
- **Recorded, not fixed:** nothing in the control path knows the fitted
  receiver's band, so a host may tune outside it, be answered successfully, and
  have the tune fail later at the driver.

## DOC-045 — Reconcile the investigation ledger and remove the dead panic report

- **Status:** complete (2026-08-11)
- **Objective:** leave one coherent account of the `RISK-036` investigation,
  restore unique stable evidence identifiers and accurate task states, and
  remove the `.uninit` panic-report mechanism which the exact K1 proved cannot
  carry information across its bootloader.
- **Scope:** `STATUS.md`, `TASKS.md`, `RISKS.md`, evidence references in existing
  documentation, and the K1 panic-report/boot-counter display and firmware path.
  Retain panic recovery by software reset, reset-cause display, serial counters,
  and the host-side fuzz regressions.
- **Exclusions:** flashing hardware, taking new physical observations, changing
  runtime-control semantics, adding a replacement persistent panic reporter, or
  continuing the inconclusive serial bisection.
- **Tests required:** focused display tests for the retained reset-cause and
  serial-counter contract; the pinned formatting, warning-denied Clippy,
  workspace-test, K1 release-build, package, and image-verification gates.
- **Acceptance criteria:** exactly one current task is named in `STATUS.md`;
  superseded `RISK-036` conclusions are not presented as current facts; evidence
  headings are unique and their references unambiguous; stale task states are
  reconciled from their recorded completion evidence; no dead panic-report or
  boot-counter path remains; the K1 still resets on panic and reports its reset
  cause; all required gates pass and their exact results are recorded.
- **Next after completion:** define a separate bench-only task which fixes and
  records cable, seating, and power, then observes `RX` and `E` on a confirmed
  responsive radio while a host sends. No firmware bisection precedes that
  observation.
- **Completion notes:** reconciled the duplicate `EVID-K1-060` by assigning the
  external-memory observation `EVID-K1-061` and updating all references;
  resolved the stale `STORE-004`, `K1SIDE-025`, and `RFK1-030` states from their
  own completion or supersession evidence; and rewrote `RISK-036` so withdrawn
  claims are not current conclusions. Removed the disproven `.uninit` panic
  report and boot counter while retaining panic-to-reset recovery, reset-cause
  reporting, serial counters, and fuzz regressions. `AFIK-K1-6.3` is 92,072
  bytes with 8,456 bytes static RAM and 7,928 bytes stack headroom. All pinned
  workspace and K1 package gates pass as recorded in `STATUS.md`; no hardware
  was flashed or observed.

## BENCH-046 — Establish a repeatable K1 serial bench before more bisection

- **Status:** ready; not active
- **Objective:** determine whether any application-mode UART byte reaches a
  responsive exact K1 under one recorded, repeatable physical setup.
- **Scope:** record cable identity and seating, radio power source, host serial
  path and baud, flashed image identity, the handset responsiveness check, the
  exact host operation, and the `RX` and `E` values before and after it. Update
  `docs/hardware-evidence.md`, `RISKS.md`, and `STATUS.md` with that one result.
- **Exclusions:** firmware changes, a new diagnostic image, flashing, protocol
  conclusions beyond the observed counters, RF transmission, or continuing
  the earlier bisection.
- **Dependencies:** `DOC-045`; access to the exact unit and its programming
  cable.
- **Acceptance criteria:** the setup is specific enough to repeat; the radio is
  responsive immediately before and after the host operation or the loss of
  responsiveness is recorded; exact before/after `RX` and `E` values are
  captured without interpreting a reset-cleared row as a pre-reset count; and
  the result chooses only the next diagnostic branch, not its answer.

## V1FAM-047 — What three V1 radios said, and what the operator declares

- **Status:** partially complete; the observation half is done and committed
- **Objective:** stop classifying Quansheng hardware from its bootloader beacon,
  per `ADR-070`, and make the beacon evidence that may contradict an operator's
  declaration but never substitute for it.
- **Completed:** `EVID-K5-012` to `EVID-K5-016` record two units, two bootloader
  versions, the identity field and its unit-versus-bootloader split, the beacon
  command as the protocol discriminator, and the absence of any processor query
  in the reviewed projects. `ADR-070` records the decision. The literal-marker
  defect is fixed and verified against the UV-K6 over serial, which is the first
  AFIK classification of a physical Quansheng radio. `observe_bootloader` reports
  a beacon without judging it, and `identify` uses it.
- **Remaining, and deliberately not started:** the declared-target comparison on
  the write paths. `EVID-K5-015` establishes that a `4.00.01` unit speaks the
  page-write protocol; it does not establish that unit's page exchange or
  geometry. Wiring a declaration into `flash_application` without that evidence
  would let an operator's phrase authorise a write nothing has qualified, which
  is the opposite of what `ADR-070` decided. The declaration and the per-target
  descriptor land together, after the experiment below.
- **Scope of the remainder:** a declared target carrying protocol shape,
  application origin and length, page size and count, and memory addressing
  class and capacity; a comparison that fails only on positive contradiction; a
  confirmation phrase derived from the declared target rather than compiled in;
  and the boot-time verification `ADR-070` requires of the image.
- **Exclusions:** widening any write path on the strength of a beacon; branching
  on the identity field while `EVID-K5-014` records its meaning as unknown; a
  `4.00.*` write of any kind.
- **Dependencies:** a read-only establishment of the `4.00.01` page exchange, and
  a physical marking record per `EVID-K5-008` for all three units.
- **Acceptance criteria:** an unfamiliar bootloader version never blocks an
  observation and never authorises a write; the qualified paths behave exactly as
  before for the targets they already covered; and every per-unit number a build
  depends on is verified by the running image against the hardware.

## K5DRV-048 — DP32G030 drivers and a flashable K5 application

- **Status:** complete
- **Objective:** give the V1/DP32G030 target the drivers it has never had —
  clock, peripheral gating, pin function selection, general-purpose IO and a
  polled UART — and use them to build a K5 application image which the qualified
  bootloader path can write and which the host can then observe running.
- **Scope:** a `radio-dp32g030` driver crate holding every DP32G030 register
  address and field this image needs, with the unsafe memory-mapped access
  contained in one module; a `radio-firmware-k5` image which starts up its own
  `.data` and `.bss`, selects 48 MHz RCHF, binds UART1 to PA7/PA8 and answers the
  read-only normal-mode hello with its own printable identity; the build,
  packaging and verification tools for that image; and the host command which
  writes it to a qualified V1 bootloader.
- **Exclusions:** display, keypad, BK4819, configuration memory, battery, audio,
  interrupts, DMA, any transmit path, and any write to a `4.00.*` bootloader.
  The image serves one read-only request and holds no configuration.
- **Dependencies:** `EVID-DP32-004` to `EVID-DP32-009` and `EVID-K5-019`; the
  corrected bootloader classifier from `V1FAM-047`; an operator-supplied
  recovery application and EEPROM backup for the exact unit.
- **Tests required:** host tests for the baud divider, the corrected-frequency
  arithmetic, the register field encodings, and the hello framing; the pinned
  formatting, warning-denied Clippy and workspace-test gates; and static
  verification of the emitted image's vectors, section placement and length.
- **Acceptance criteria:** the driver crate builds for `thumbv6m-none-eabi` and
  its pure logic is tested on the host; the packaged image is exactly `0xF000`
  bytes with a Thumb reset vector inside the application region; no AFIK write
  reaches the stock bootloader region; and the image, once written to the exact
  unit, answers `afik-flasher probe-normal` with its own identity.
- **Next after completion:** the drivers the operator interface needs — the
  ST7565 display bus, the keypad matrix, and a timer — each with its own board
  evidence, followed by the BK4819 three-wire bus.

## K5RX-049 — Receive a complete frame on a V1 radio

- **Status:** complete
- **Objective:** make a V1 image receive fourteen back-to-back bytes intact, so
  the host exchange `K5DRV-048` built can complete.
- **Current facts:** `EVID-K5-021`. Bytes with 20 ms gaps arrive perfectly.
  Bytes back-to-back arrive with a correct prefix — ten bytes, repeatedly — and
  a corrupt remainder, with `STOPE` latched, into a receiver that is otherwise
  silent. `AFIK-K5-1.2` added acknowledgement of the latched error bits and has
  not been shown to boot, let alone to fix it.
- **First step, before any theory:** confirm whether `AFIK-K5-1.2` boots, by
  capturing the wire across a power-cycle with nothing else holding the port.
  A silent `1.2` is a regression in the driver change and is a different
  question from the receive defect.
- **Scope:** first add a fixed on-screen boot/stage witness through the evidenced
  V1 ST7565-compatible display wiring, because three acknowledged images have
  produced no repeatable serial output and serial alone cannot distinguish Reset
  from UART failure. Keep the application-facing display contract independent of
  DP32G030 registers. Then run a burst-length sweep from the host, which costs no
  flash cycle and bounds where reception breaks; if the break is at the FIFO
  boundary, use the DMA receive path the pinned V1 firmware uses, with its receive
  timeout, as the candidate fix. Any SPI or DMA registers used must be recorded
  from the reference manual first.
- **Exclusions:** guessing at register values, changing the qualified flashing
  path, keypad, storage, audio, RF, or TX. The display is a fixed diagnostic,
  not yet the operator interface.
- **Acceptance criteria:** a host `probe-normal` reports the image's identity
  from the exact unit, repeatably, and the burst-length sweep is recorded
  whatever the outcome.
- **Result:** the display-only sweep established that the host request is sixteen
  wire bytes. The silent diagnostic received all sixteen through DMA after the
  channel source selector was corrected from `HSREQ_MS0` to UART1's
  `HSREQ_MS1`, with no parity, stop, or FIFO-overflow error. `AFIK-K5-1.4`
  then used that same bounded DMA adapter behind the unchanged `RequestReader`;
  three consecutive physical `probe-normal` exchanges returned its identity.

## PLAT-050 — Shared K1/K5 application platform boundary

- **Status:** complete
- **Objective:** make application behavior compile once against bounded serial
  and display contracts while K1 and K5 supply target-specific adapters.
- **First step:** inventory the K1 and K5 application loops and extract the
  smallest shared serial request/response service without moving register,
  interrupt, DMA, or pin behavior out of the hardware crates.
- **Scope:** application-facing serial receive/transmit and display contracts,
  shared protocol dispatch, target adapters, and host tests proving both targets
  run the same service logic.
- **Exclusions:** keypad, EEPROM, RF/audio, BK4819, TX, publishing dependencies,
  or claiming the unproven K5 SPI0 adapter works. K5 may retain its physically
  proven synchronous-GPIO display adapter behind the shared display contract.
- **Acceptance criteria:** one hardware-independent service test suite covers
  both target adapters; both embedded images build warning-free; and each target
  keeps all MCU-specific types and registers below the application boundary.
- **Result:** `radio-platform` now owns the common normal-mode hello stream
  service and boot-display contract. K1 delegates common command validation and
  response encoding while retaining its additional diagnostics; K5 runs the
  same service over its proven circular-DMA receive adapter. K1 and K5 display
  adapters implement the same application contract, with the K5 synchronous
  GPIO implementation retained. Focused host tests cover both adapters and both
  embedded images build; no radio was flashed.

## K5APP-051 — K5 receive-only operator hardware bring-up

- **Status:** software adapter milestone complete; physical read-back and
  receive/audio experiments pending
- **Objective:** add the K5 V1 keypad, read-only configuration-memory access,
  BK4819 receive control, and demodulated-audio enablement as separately tested
  and committed steps.
- **Scope order:** main keypad matrix; EEPROM identification/read only; BK4819
  three-wire read-back then receive initialization/metering; speaker audio only
  after receive is established.
- **Exclusions:** EEPROM writes, PTT semantics, TX register values, PA/RF switch
  control, RF emission, calibration claims, side keys, and voice-chip behavior.
- **Safety boundary:** each physical bus begins with an identity or known-register
  read; ambiguous results stop the package. Shared keypad/I2C/voice pins are
  restored after every keypad scan. No physical image write is implied.
- **First result:** the source-pinned PA3..PA6 by PA10..PA13 main matrix is a
  bounded adapter with all sixteen mappings, stable-sample rejection, and an
  explicit shared-pin restore contract covered by host tests.
- **Second result:** the PA10/PA11 EEPROM adapter exposes only bounded random
  reads, checks every address acknowledgement, uses released-high lines, and
  has exact transaction and capacity tests. It is not yet physically confirmed.
- **Third result:** the PC0/PC1/PC2 target adapter implements the existing
  `radio-bk4819::RegisterBus` contract, including MSB-first write/read framing
  and bidirectional data. PC4 is a separately initialized-muted speaker gate.
  Host framing tests and the embedded build pass; neither is called by the K5
  boot image until known-register read-back is physically approved and observed.
- **Fourth result:** the K5 EEPROM adapter now supports one guarded, aligned
  eight-byte replacement with compare-before-write, bounded ready polling, and
  mandatory read-back. Stale state, bad acknowledgements, busy timeout, and
  verification mismatch are errors. It remains unreachable from the boot image
  and no physical EEPROM write was performed.
- **Fifth step:** `AFIK-K5-1.5V` composes the proven serial/display paths with
  read-only EEPROM offset-zero sampling, read-only BK4819 `REG_00` sampling,
  and continuous main-keypad decoding. The display reports a bounded EEPROM
  byte sum, raw BK register value, and decoded key label. It performs no EEPROM
  write, BK4819 write, audio enable, RF switch, PA, PTT, or TX operation.
- **Physical acceptance:** after guarded flashing and a normal power-cycle, the
  screen must show `1.5V`, an EEPROM value other than failure dashes, a stable
  BK value rather than an assumed interpretation, and all sixteen key labels;
  three normal probes must return `AFIK-K5-1.5V`. Any ambiguous result stops.
- **First observation:** `1.5V` booted, returned three normal hellos, displayed
  raw BK `4819`, and decoded the implemented main keys. EEPROM did not
  acknowledge because AFIK's high phase used input release instead of the
  source-backed push-pull sequence. `1.5E` corrects that and adds read-only
  side-key/PTT labels; it is built but not yet flashed.
