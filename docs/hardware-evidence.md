# Hardware evidence

Hardware facts are recorded before they are encoded in target or simulator
source. A simulator result confirms software behaviour against its declared
model; it does not increase confidence in the underlying silicon fact.

## Sources used by DP32-003

### DP32G030 reference manual v1.23

- **Document:** *DP32G030 Reference Manual — 32-bit Microcontroller*, version
  1.23, Chinese, revision dated 2022-02-21.
- **Publisher shown in document:** Action Dynamic Tech. (HK) Trading Co.; the
  document also names 深圳市动能世纪科技有限公司.
- **Retrieved:** 2026-08-05 from
  <https://alfaexploit.com/files/DP32G030.pdf>.
- **Mirror status:** this is a public third-party mirror; a chip-vendor download
  has not been located. The PDF is not vendored in this repository.
- **SHA-256:**
  `d1923c0a1830dada46706515ced53978f9a5086e04ce178deaf28d2928c62573`.

### Arm Cortex-M0 generic user guide

- **Document:** *Cortex-M0 Devices Generic User Guide*, ARM DUI 0497A,
  ID112109, 2009.
- **Retrieved:** 2026-08-05 from Arm's documentation service:
  <https://documentation-service.arm.com/static/5ea6ce5e9931941038def8c1>.
- **Scope:** generic Cortex-M0 architectural behaviour only; implementation
  choices remain governed by the DP32G030 manual.

## Accepted facts

### EVID-DP32-001 — Processor architecture and byte order

- **Fact:** the DP32G030 contains a 32-bit Arm Cortex-M0 processor. Its system
  interface supports little-endian data access.
- **Source:** DP32G030 manual sections 1.1 and 5.4.1–5.4.2, printed pages 20 and
  43–44. Section 5.1 also states that module data format is little-endian.
- **Method:** copied from the reference manual, not derived from existing
  firmware.
- **Confidence:** high for Work Package 3. The manual is internally consistent
  and agrees with Arm's Cortex-M0 vocabulary, but the mirrored document and
  target radio silicon have not been independently authenticated.
- **Permitted use:** select Rust's `thumbv6m-none-eabi` target and Renode's
  Cortex-M CPU model. No DP32G030 peripheral behaviour follows from this fact.
- **Required experiment:** inspect the emitted ELF architecture now; read the
  physical core identification through a non-destructive debug path only after
  recovery prerequisites are complete.

### EVID-DP32-002 — Program flash and data RAM ranges

- **Fact:** program flash occupies `0x0000_0000..=0x0000_FFFF` (64 KiB), and
  data RAM occupies `0x2000_0000..=0x2000_3FFF` (16 KiB).
- **Source:** DP32G030 manual table 5-1 in section 5.1, printed pages 39–40;
  section 5.2, printed page 41, independently states 64 KiB program flash and
  16 KiB data RAM.
- **Method:** copied. The inclusive end addresses and sizes agree exactly.
- **Confidence:** high for linker and minimal memory-model bounds; not yet
  confirmed on a physical UV-K5-family unit.
- **Permitted use:** define only flash and RAM regions in the linker script and
  Renode platform. Work Package 3 must not model the manual's peripheral ranges.
- **Required experiment:** statically check every allocated ELF section against
  these ranges; later compare non-destructive SWD memory reads with the map only
  after backup/recovery work is complete.

### EVID-ARM-003 — Cortex-M0 reset vectors

- **Fact:** the Cortex-M0 vector table is fixed at `0x0000_0000`. Its first word
  is the initial stack-pointer value, its second word is the Reset handler, and
  handler vectors have bit 0 set to identify Thumb code.
- **Source:** Arm DUI 0497A section 2.3.4 and figure 2-1, document pages 2-21 and
  2-22 (PDF pages 36–37).
- **Method:** copied from the Arm core guide, then combined with
  `EVID-DP32-002`, whose flash starts at the same address.
- **Confidence:** high for a Cortex-M0 model loaded without a boot remap.
- **Permitted use:** place a two-entry minimum vector table at the start of the
  image, set the initial stack pointer to the top of evidenced RAM, and let
  Renode begin through the ELF/vector-table reset path.
- **Required experiment:** inspect the first two ELF words and prove in Renode
  that execution reaches the Rust Reset handler without overriding the PC.

## Simulation-only observation contract

`DP32-003` may reserve one linker-controlled word inside the evidenced RAM and
write a fixed boot sentinel from the Reset handler. The Renode test may inspect
that word. The sentinel address and value are project test conventions, not
DP32G030 registers or claims about hardware behaviour.

## Unknowns deliberately excluded from DP32-003

- The UV-K5 bootloader's physical reset mapping, flash masking, image layout,
  and firmware packaging are not established.
- No clock, reset controller, flash controller, interrupt, UART, GPIO, display,
  keypad, BK4819, audio, or power register behaviour is accepted yet.
- The exact UV-K5-family board revision and fitted DP32G030 silicon have not
  been inspected.
- Hardware flashing remains blocked by `RISK-002`.

## Work Package 5 UI evidence boundary

`UI-005` introduces no hardware facts. `Menu`, `Back`, `Up`, `Down`, and
`Confirm` are product-level logical actions, and its display output is a
semantic enum rather than pixels or bus traffic. The `Menu+Back` boot gesture
is an AFIK workflow decision, not a claim about an existing radio key matrix.

The physical key matrix, side-key wiring, scan polarity and timing, display
controller, dimensions, bus, reset sequence, backlight, and board pins remain
unknown. A target UI adapter or peripheral simulator requires new sourced facts
and board experiments under `RISK-006` before implementation.

## Sources used by RF-006

### Beken BK4819 product page

- **Document:** Beken Corporation product page, *BK4819 — Half-duplex TDD FM
  Transceiver*.
- **Publisher:** Beken Corporation.
- **Retrieved:** 2026-08-06 from
  <https://www.bekencorp.com/en/goods/detail/cid/50.html>.
- **Scope:** current high-level product identity and capabilities only. The page
  contains no register map or board integration instructions.

### Beken BK4819 datasheet Rev.1.0

- **Document:** *BK4819 Analog Two Way Radio IC*, Rev.1.0, copyright 2018,
  22 pages.
- **Publisher shown in document:** Beken Corporation.
- **Retrieved:** 2026-08-06 from
  <https://alfaexploit.com/files/BK4819.pdf>.
- **Mirror status:** public third-party mirror; an equivalent download was not
  found on Beken's current product page. The PDF is not vendored.
- **SHA-256:**
  `a2b795a1f40f13e2708fc11720cc4df05fe00590eb0a8d82914699153321de02`.
- **Scope:** product architecture, 3-wire control framing/timing, pins, and
  electrical characteristics. The datasheet explicitly points to separate
  application notes and a register table and does not itself define registers.

### Mirrored BK4819(V3) application note

- **Document:** *BK4819(V3) Application Note*, page label 2020; the mirror title
  includes `20210428` and identifies the content as machine-translated English.
- **Publisher/authenticity:** the content presents itself as an application
  note but the accessible Scribd copy is user-uploaded and no original Beken
  download or untranslated copy has been located.
- **Retrieved:** 2026-08-06 from
  <https://www.scribd.com/document/716113950/BK4819-V3-Application-Note-20210428-machine-translated-English>.
- **Scope:** only the explicitly listed frequency, mode-control, RSSI, and
  squelch fields below. Function names, prose translations, defaults,
  initialization code, RF performance, and omitted register behavior are not
  accepted as facts.

## Accepted BK4819 facts and bounded inferences

### EVID-BK4819-004 — Product mode and 3-wire control envelope

- **Fact:** Beken identifies BK4819 as a half-duplex TDD FM transceiver with an
  MCU 3-wire interface. Datasheet section 1.3 and figures 5–7 encode one R/W bit,
  seven address bits `A6..A0`, and sixteen data bits `D15..D0`; input is latched
  on rising SCK and output changes on falling SCK. Table 6 limits SCK to 8 MHz.
- **Method:** copied from Beken's product page and Rev.1.0 datasheet pages 1,
  6, and 9 (PDF pages 1, 6, and 9).
- **Confidence:** high for an unbound logical register-bus envelope; low for
  applicability to any particular MCU pins or board timing implementation.
- **Permitted use:** define a bounded 7-bit register address, 16-bit value, and
  fallible read/write trait. `RF-006` does not bit-bang pins or model timing.
- **Required experiment:** identify the fitted chip/board revision and trace
  SCK, SCN, and bidirectional SDATA; only then verify edge timing with a logic
  analyzer using non-transmitting reads.

### EVID-BK4819-005 — Frequency word and receive-status fields

- **Fact:** the mirrored application note assigns a 32-bit frequency word with
  a 10 Hz unit across `REG_38` and `REG_39`. Its 409.75 MHz example produces
  `0x0271_3A98`; the adjacent table shows `REG_38 = 0x3A98` and
  `REG_39 = 0x0271`, establishing low and high words respectively. It labels
  `REG_67<8:0>` read-only RSSI with `RSSI dBm ~= raw/2 - 160`, and
  `REG_0C<1>` as a read-only squelch result where one is link/open and zero is
  loss/closed.
- **Method:** copied from the note's Squelch/RSSI and Frequency Setting sections,
  labeled pages 13–14.
- **Confidence:** medium-low for a BK4819(V3) simulation contract because the
  only accessible copy is machine-translated and its revision/authenticity are
  unconfirmed. The 10 Hz formula's example is internally consistent.
- **Permitted use:** pack only exactly 10-Hz-aligned integer frequencies into
  the two words and decode RSSI into signed half-dBm integer units plus one
  squelch boolean in a fake register-bus model.
- **Required experiment:** obtain the original application note/register table;
  confirm register order, write/latch ordering, and status fields by read-only
  observation on identified hardware before binding a target adapter.

### EVID-BK4819-006 — Mode-control fields and AFIK command-plan inference

- **Fact:** the mirrored application's Tx/Rx Mode Switch table labels
  `REG_30<15>` VCO calibration, `<13:10>` the receive link blocks
  LNA/mixer/PGA/ADC, `<9>` AF DAC, `<7:4>` PLL/VCO, `<3>` PA gain, `<2>` MIC
  ADC, `<1>` TX DSP, and `<0>` RX DSP. Zero disables every listed block.
- **Inference:** for deterministic simulation, AFIK composes `0xBEF1` as
  receive (calibration, all link blocks, AF DAC, all PLL/VCO blocks, RX DSP),
  `0x80FE` as transmit (calibration, all PLL/VCO blocks, PA gain, MIC ADC, TX
  DSP), and `0x0000` as neutral standby. AFIK writes standby, `REG_38` low,
  `REG_39` high, and the selected mode last so a partially completed command
  plan cannot intentionally reach transmit enable.
- **Method:** bit positions are copied from application-note labeled page 12;
  exact combined values and ordering are local fail-closed design inferences,
  not copied vendor sequences.
- **Confidence:** low for physical-chip operation and high only as a transparent
  deterministic command model. Initialization prerequisites, auto-clearing
  calibration behavior, high/low frequency write order, and required delays are
  unknown.
- **Permitted use:** encode exact command-order tests and a fake-bus simulator.
  The TX mode word may be emitted only behind a matching `TxAuthorisation`.
- **Required experiment:** compare an original register table/application note
  and capture known-safe vendor initialization/RX transitions first. TX
  verification additionally requires a recovery-proven unit, shielded dummy
  load, spectrum analyzer, current limiting, and independently controlled
  external PA/RF switching; it is outside `RF-006`.

### EVID-BK4819-007 — Published frequency ranges conflict

- **Fact:** the current Beken product page states 18–620 MHz and 840–1200 MHz,
  while the mirrored Rev.1.0 datasheet states 18–660 MHz and 840–1300 MHz.
- **Confidence:** high that the publications differ; unknown which revision or
  range applies to the fitted radio and its board filters/matching.
- **Permitted use:** none as a software acceptance limit. `RF-006` validates
  only 10 Hz representation; channel/regulatory bounds remain higher-layer
  responsibilities and board RF limits remain unknown.
- **Required experiment:** identify the exact fitted silicon revision and obtain
  its matching official datasheet plus board filter/switch schematic before
  defining supported receive or transmit bands.

## Unknowns deliberately excluded from RF-006

- BK4819 silicon revision and the BK4819(V3) application's applicability.
- Original register table, complete reset/initialization sequence, documented
  mode transition sequence, calibration completion/status, and required delays.
- MCU pins, electrical direction switching for SDATA, and the UV-K5-family
  board's RF switches, filters, audio route, external PA, and power controls.
- Crystal frequency/calibration values and all unit-specific calibration data.
- Physical RSSI accuracy, squelch behavior, receive sensitivity, emitted power,
  spectral purity, and regulatory compliance.
- Hardware access, flashing, physical receive claims, and all on-air TX remain
  blocked by `RISK-002`, `RISK-005`, and `RISK-007`.

## Work Package 7 scanning evidence boundary

`SCAN-007` introduces no hardware facts. Dwell and hold milliseconds are AFIK
workflow configuration, not BK4819 tune/settle requirements. Timer expiries and
normalized `SignalMeasurement` values are logical adapter inputs, not claims
about a DP32G030 timer, interrupt, RSSI threshold, polling interval, or physical
squelch accuracy.

The deterministic simulator proves controller scheduling, stale-token safety,
logical RF command ordering, and TX-policy composition against declared inputs.
Physical scan cadence, receiver settling, status sampling, tone detection, RF
performance, and target integration remain unknown under `RISK-008`.

## Sources used by FREQ-010

### FCC-filed Quansheng UV-K5 user manual

- **Document:** *Quansheng UV-K5 Two Way Radio User Manual*, FCC exhibit
  document ID `6401561`, 13 pages.
- **Applicant/publisher:** Fujian Quanzhou Quansheng Electronics Co., Ltd.;
  filed for FCC ID `XBPUV-K5`.
- **Created/filed metadata:** 2023-02-17; retrieved 2026-08-06 from
  <https://fccid.io/XBPUV-K5/User-Manual/User-manual-6401561.pdf> and the
  accompanying exhibit page
  <https://fccid.io/XBPUV-K5/User-Manual/User-manual-6401561>.
- **SHA-256:**
  `d6f30fea598abdde820ef47e5f9b0b77079dc229ec7039a97b3f87083b33b74b`.
- **Scope:** intended radio-level controls and displayed/saved results only.
  The manual contains no BK4819 register sequence, accuracy, timeout, or board
  implementation details.

### Public UV-K5-family firmware observation

- **Repository:** *egzumer/uv-k5-firmware-custom*, commit
  `7607f0a4bd6203d1f06b70556fc1ce0d7399d6b3`, retrieved 2026-08-06 from
  <https://github.com/egzumer/uv-k5-firmware-custom/tree/7607f0a4bd6203d1f06b70556fc1ce0d7399d6b3>.
- **Files inspected:** `driver/bk4819.c`, `app/scanner.c`, and `misc.c` at that
  exact commit.
- **Provenance limit:** the repository states that it combines earlier
  UV-K5-family firmware projects and derives from DualTachyon's work. It is one
  descendant implementation, not independent corroboration of the mirrored
  application note or original Quansheng production source.
- **Permitted use:** compare its state flow, polling, repeat-result filtering,
  failure handling, and unexplained constants when designing experiments. No
  source was copied, translated, linked, or treated as AFIK production code.
- **Additional repository checked:**
  `amnemonic/Quansheng_UV-K5_Firmware` at commit
  `94a36006b75ad2024d9b88f2b33b222c7efe53ba` contained binaries, documents,
  hardware PDFs, and patch scripts but no buildable firmware source from which
  an exact Frequency Copy flow could be established.

The Beken product page and mirrored BK4819(V3) application note already
identified under “Sources used by RF-006” were re-used for the narrowly bounded
observations below. Their confidence and provenance limitations are unchanged.

## Frequency Copy evidence and bounded inferences

### EVID-FCOPY-008 — Fast Copy measures one received transmission

- **Fact:** user-manual section 6.9 calls the feature “Fast Copy One Channel
  (ACT AS FREQUENCY METER).” It directs the user to place the radios close with
  both antennas installed and requires a sufficiently strong signal. `F+4`
  starts measurement; the display can show the carrier frequency and the
  transmitting channel's CTCSS/DCS. `*` starts another measurement, `MENU`
  saves the measured frequency and CTCSS/DCS to a chosen channel, and `EXIT` or
  PTT leaves the function.
- **Fact:** the same manual separately describes automatic CTCSS/DCS scanning
  at an already known receive frequency under `F+*`, with success/failure
  feedback and an explicit save action.
- **Method:** copied from manual sections 6.9 and 6.10. Wording here is
  paraphrased; key names and displayed data are retained because they define
  the user-visible workflow.
- **Confidence:** high for intended UV-K5 radio behavior; none for silicon
  sequence, physical accuracy, or AFIK target behavior.
- **Permitted use:** define a deliberate, receive-only AFIK measurement and
  review workflow. It does not establish any saved channel property other than
  an observed carrier frequency and an optional received tone/code indication.

### EVID-FCOPY-009 — Fast Copy is not Air Copy

- **Fact:** manual section 7.8 describes “Wireless Radio Replication” as a
  separate transfer between two radios on a shared data frequency, defaulting
  to 410.0125 MHz. It is entered with PTT plus side key 2 and transfers radio
  data with progress/error feedback. The feature table likewise lists
  Frequency Meter and Air Copy separately.
- **Confidence:** high for the product-level distinction.
- **Permitted use:** exclude Air Copy protocol discovery, configuration
  replication, and any transmission from `FREQ-010`. “Frequency Copy” in AFIK
  means local observation of a received signal, never radio-to-radio cloning.

### EVID-FCOPY-010 — Beken publishes a scan capability, not a command contract

- **Fact:** Beken's current BK4819 product page lists frequency scan, CTCSS
  receive/tail functions, 23/24-bit DCS, RSSI, and a 3-wire MCU interface.
- **Fact from low-confidence source:** the mirrored machine-translated
  BK4819(V3) note describes a frequency scan result spanning `REG_0D<10:0>` and
  `REG_0E<15:0>` in 10 Hz units, a busy bit in `REG_0D<15>`, and scan enable
  plus four nominal duration selections in `REG_32`. It describes CTCSS result
  fields in `REG_68` whose conversion depends on the crystal, and 23/24-bit DCS
  result fields in `REG_69` and `REG_6A`. The prose also gives an approximate
  strong-input condition.
- **Confidence:** high only that Beken advertises the capabilities; low for all
  named register fields, units, polarity, timing, threshold, latching, or
  applicability to the fitted radio because the accessible note is
  machine-translated, user-uploaded, and revision-unverified.
- **Permitted use:** form hypotheses and measurement vectors for a
  non-transmitting experiment. These fields are not accepted for a production
  driver or a simulator that claims physical register behavior.
- **Required experiment:** obtain original revision-matched documentation and
  perform the receive-only experiments in `frequency-copy-feasibility.md`.

### EVID-FCOPY-011 — One descendant firmware does not resolve the register gaps

- **Observation:** the inspected descendant polls the note's busy/result
  fields, seeks repeated nearby frequency results, then retunes and seeks a
  recognized CTCSS or DCS result. It bounds polling and requires an explicit
  save prompt. Its save path rounds a captured frequency to one of two channel
  step grids.
- **Observation:** its `REG_32` value includes bits which its own comment marks
  unknown, including an unexplained numeric field. Its code also supplies
  product choices such as thresholds, repeats, timeouts, code lookup, rounding,
  and copying receive configuration into transmit configuration; those choices
  are not BK4819 facts.
- **Confidence:** low as hardware evidence. Common project ancestry means the
  code and mirrored note are not independent corroboration.
- **Permitted use:** identify failure cases and experiments. AFIK must not copy
  its code, unexplained constants, automatic RX-to-TX configuration, or timing
  values.

## Unknowns deliberately retained by FREQ-010

- Exact fitted BK4819 revision, board revision, reference crystal/TCXO and
  calibration, RF switch/filter path, register reset state, and required
  preservation masks.
- Meaning of every `REG_32` bit used by existing firmware, start/stop/retrigger
  order, busy transitions, result latching/read order, stale-result behavior,
  cleanup sequence, and behavior after bus failure.
- Physical frequency accuracy, resolution versus displayed rounding, minimum
  usable level, acquisition time, repeatability, adjacent/multiple-signal
  behavior, and image/harmonic/strong-interferer false locks.
- CTCSS conversion for the fitted crystal, nearby-tone discrimination,
  no-tone and short-burst behavior, and the complete DCS bit-length, polarity,
  code-validation, and no-result semantics.
- Duplex/offset, paired repeater input, transmit permission/service, modulation,
  bandwidth, power, channel name, scan membership, scrambler, contacts, source
  identity, and any other channel metadata. None is observable from the
  documented one-transmission workflow.
- A trustworthy distinction between “no signalling present” and “signalling
  not detected before timeout” without separately validated carrier and decoder
  behavior.

Production scan commands, physical target integration, and automatic channel
creation remain prohibited by `RISK-011`.

## Sources used by APRS-011

### AX.25 Link Access Protocol Version 2.2

- **Document:** *AX.25 Link Access Protocol for Amateur Packet Radio*, version
  2.2, fourth edition, 1996.
- **Publishers shown in document:** American Radio Relay League and Tucson
  Amateur Packet Radio Corporation.
- **Retrieved:** 2026-08-06 from TAPR's file archive at
  <https://files.tapr.org/tech_docs/AX25/AX25.2.2.pdf>.
- **SHA-256:**
  `af2070954468ef6498143ababf9beaf5d72b683ef82491dd7eb8e3670b29475c`.
- **Scope:** HDLC flag/bit-stuff/FCS envelope, shifted AX.25 addresses,
  extension and repeated bits, UI control field, and PID placement. Physical
  APRS modulation is not specified here.

### APRS Protocol Reference Version 1.0.1

- **Document:** *APRS Protocol Reference — APRS Protocol Version 1.0*, document
  version 1.0.1, 2000-08-29.
- **Publisher:** APRS Working Group material hosted by APRS.org.
- **Retrieved:** 2026-08-06 from
  <https://www.aprs.org/doc/APRS101.PDF>.
- **SHA-256:**
  `78a72618c788b8b7f8369004884018f9b02f990069ff67915bb1e30738b1da01`.
- **Scope:** APRS use of AX.25 UI frames, APRS address/path limits, information
  type identifiers, time/position/ambiguity fields, and Object/Item lifecycle
  and syntax.

### APRS addendum and frequency specification

- **Documents:** APRS Specification Addendum 1.1, approved by the APRS Working
  Group in 2004; and *APRS Freq Spec — AFRS (Automatic Frequency Reporting
  System)*, revision 2019-09-30.
- **Retrieved:** 2026-08-06 from <https://www.aprs.org/aprs11.html> and
  <https://www.aprs.org/info/freqspec.txt>.
- **Frequency-spec SHA-256:**
  `d49be65c62a4c9907fdfb5c168813a5135b98cbfc7a2c6de144016307798972b`.
- **Scope:** the current voice-frequency comment/object conventions, exact
  frequency/tone/offset/range text fields, recommended local voice-repeater
  objects, permanent `111111z` timestamp behavior, and compatibility limits.
  The frequency specification is operational APRS Working Group guidance, not
  proof that an advertised value is current or authorized.

The Beken product page, mirrored Rev.1.0 datasheet, mirrored machine-translated
BK4819(V3) application note, and DP32G030 source limitations already recorded
above are re-used with unchanged provenance and confidence.

## APRS receive and discovery evidence

### EVID-APRS-012 — APRS is carried in bounded AX.25 UI frames

- **Fact:** APRS Protocol Reference chapter 3 specifies destination and source
  addresses, zero through eight digipeater addresses, control `0x03`, PID
  `0xF0`, an information field of 1 through 256 octets, and a 16-bit FCS. The
  first information octet is the APRS Data Type Identifier.
- **Fact:** AX.25 sections 3.1–3.12 define `0x7E` flags, removal of a zero after
  five consecutive one bits, a sixteen-bit ISO 3309 FCS, shifted six-character
  upper-case alphanumeric callsigns, four-bit SSIDs, seven-octet address
  subfields, and a final-address extension bit. UI frames contain PID and
  information without link flow control.
- **Conflict:** AX.25 version 2.2 limits its Layer-2 repeater chain to two,
  whereas APRS 1.0.1 explicitly permits zero through eight APRS digipeater
  addresses. An APRS receive parser follows the upper-layer APRS bound of eight
  while remaining receive-only; it does not implement AX.25 repeating.
- **Confidence:** high for a hardware-independent parser of a complete,
  already de-stuffed, octet-aligned frame including FCS.
- **Permitted use:** validate the complete frame, addresses, control/PID,
  information bounds, and FCS before exposing APRS information. Flags, NRZI,
  clock recovery, and bit unstuffing remain lower-layer inputs and are excluded.

### EVID-APRS-013 — Objects and Items carry explicit identity and lifecycle

- **Fact:** APRS Protocol Reference chapter 11 defines a case-sensitive
  nine-character printable Object name after `;`, followed by `*` for live or
  `_` for killed and a required seven-character timestamp. An Item begins with
  `)`, has a case-sensitive printable name of three through nine characters,
  and uses `!` for live or `_` for killed without a timestamp. Both can carry
  uncompressed or compressed position plus comment.
- **Fact:** a new report with the same Object/Item name replaces the earlier
  report, even if another station originated it; the sender callsign should be
  retained for display. The frequency specification gives permanent frequency
  Objects the pseudo timestamp `111111z` and says a different origin must not
  replace them.
- **Inference:** AFIK keys discovery entries by report kind, case-sensitive
  name, and originating AX.25 source. This deliberately preserves conflicting
  origins as separate untrusted observations and permits only the same source
  to update or kill its entry. Explicit local receive time resolves freshness;
  equal-time conflicting data is rejected rather than ordered implicitly.
- **Confidence:** high for syntax; high that the AFIK key is a conservative
  local safety choice, not APRS network ownership semantics.

### EVID-APRS-014 — Voice-repeater fields are advertisements, not authority

- **Fact:** the APRS frequency specification defines two main encodings: a
  leading ten-octet `FFF.FFFMHz` or `FFF.FF MHz` comment field, or a
  nine-character frequency Object name beginning `FFF.FFF` or `FFF.FF`. It
  defines optional `Tnnn`/`Cnnn` CTCSS, `Dnnn` DCS, signed three-digit offsets
  in 10 kHz units, and `Rxxm`/`Rxxk` nominal range. Lower-case tone prefixes
  advertise narrow bandwidth. The recommended voice-repeater Object examples
  use symbol code `r` and the permanent `111111z` timestamp.
- **Fact:** where both a frequency Object name and a comment frequency exist,
  the specification treats the Object name as the repeater transmit/output
  frequency and the comment value as a cross-band or non-standard receive/input
  frequency.
- **Inference:** AFIK labels the name frequency as the advertised repeater
  output—the frequency a listener would receive—and preserves offset, tone,
  range, alternate input, source, position, and age as untrusted fields. It does
  not calculate a transmit channel, map truncated tones into trusted
  `radio_domain::Tone`, infer regional standard offsets, or auto-tune/save.
- **Confidence:** high for field syntax; none for truth, freshness, origin
  authenticity, regulatory class, service availability, or physical reach.

### EVID-APRS-015 — BK4819 APRS demodulation is not established

- **Fact:** Beken's official product page and mirrored datasheet advertise an
  on-chip FSK data modem; the datasheet block diagram includes FSK modulation
  and demodulation.
- **Low-confidence description:** the machine-translated revision-unverified V3
  note describes receive modes called FSK 1.2/2.4K, FFSK 1200/1800, and FFSK
  1200/2400, with bounded preamble, two/four-byte sync, configured length,
  optional proprietary CRC, FIFO, and interrupts.
- **Comparison:** common 1200-baud APRS engineering practice uses Bell-202-style
  1200/2200 Hz AFSK followed by NRZI, HDLC flags, bit stuffing, and AX.25 FCS.
  None of the named V3 modem modes says 1200/2200, and its described fixed
  preamble/sync/length framing does not establish transparent AX.25 bit access.
- **Confidence:** high only that a generic FSK modem exists; low for the named
  register modes and no confidence that they can receive APRS correctly on the
  fitted silicon/board.
- **Permitted use:** support the feasibility experiment plan. No BK4819 APRS
  command, register fake, timing, interrupt, or FIFO behavior may be encoded.

## Unknowns deliberately retained by APRS-011

- Physical APRS frequency/band plan and permission for a specific jurisdiction;
  AFIK encodes no hard-coded national channel or transmission behavior.
- Fitted chip and board revision, BK4819 modem applicability, reference crystal,
  initialization state, modem tone/baud meaning, raw-bit availability, FIFO and
  interrupt semantics, and safe stop/cleanup.
- Whether suitable discriminator/unfiltered audio is exposed to the DP32G030,
  its voltage/bias/bandwidth/noise, board routing, ADC channel/sample rate,
  timer/interrupt/DMA wiring, CPU budget, buffer sizes, and power cost.
- NRZI polarity, clock recovery, carrier acquisition, de-emphasis effects,
  bit-stuff/flag/abort recovery, false-frame rate, FCS error behavior, and packet
  loss under weak, distorted, collided, over-deviated, or adjacent signals.
- Advertisement authenticity, freshness, correctness, coverage, regulatory
  classification, actual repeater availability, and whether optional offset or
  tone conventions are locally applicable.

`APRS-011` may implement only the complete-frame and APRS discovery boundary.
Physical demodulation and automatic channel mutation remain prohibited by
`RISK-012` and `RISK-013`.

## Sources used by FLASH-012

No manufacturer bootloader protocol or board-revision matrix has been located.
The following sources are pinned reverse-engineering evidence. Their code is
GPL-3.0 and is not copied, linked, translated, or used as AFIK production
source. Exact wire observations are recorded as facts; interpretations remain
explicitly bounded.

### sq5bpf/k5prog

- **Repository:** `sq5bpf/k5prog`, commit
  `241ab18b61f6d8933fecf60643fe94322fbf4198`, dated 2023-12-29.
- **Retrieved:** 2026-08-06 from <https://github.com/sq5bpf/k5prog>.
- **Files inspected:** `README` and `k5prog.c`; `k5prog.c` SHA-256
  `cc8a1f42208515c73bb6869233b9afa0f9cb151212d5ef9c78ef5f2d8ea5f6eb`.
- **Method stated upstream:** protocol reverse-engineering from traffic between
  the radio and original programming software, plus physical use by the author.
- **Scope:** packet envelope, normal-firmware EEPROM reads, bootloader-v2
  beacon/version/page messages, 38,400-baud observation, 256-byte pages, and
  the asserted bootloader reservation at `0xF000`.

### qrp73/K5TOOL

- **Repository:** `qrp73/K5TOOL`, commit
  `03cb33aef88fc17f9e6b71d9e6c4f0ac9b0dc436`, dated 2025-12-18.
- **Retrieved:** 2026-08-06 from <https://github.com/qrp73/K5TOOL>.
- **Files inspected:** `README.md`, `Packets/Envelope.cs`, and the V2 beacon,
  version, page-write, and acknowledgement packet types. Their respective
  SHA-256 values are
  `5d8ac90426ba0dc53f8612e1b908141330ecbe18f6225acd9b162a080c40f1cb`,
  `e7ea15effc47dbc8bf48771a2efa198de45fa1c182c17e3195d3c6ab0d097ec2`,
  `eada6c03a5162642e6e1fee00fe2de7e35c9a601aac5fa9cbe7b1d72b137a0b6`,
  `e37430167b9d435ecc87c2ac3979f60a60f0e5aa2efb75ecab6ac7e16c752ee7`,
  `168592c44f6c4a61ae7bd7aaf710600b35ab7b92d5a1d4839efc9bc1f279314d`,
  and `b62a843cff89640a9efdca868ddef18dda7825b788bfa1042dbd4d6afb04f026`.
- **Relationship:** K5TOOL is a separate implementation but cites k5prog for
  bootloader command semantics, so it is useful cross-check evidence rather
  than a fully independent observation.
- **Scope:** V1/V2/V3 incompatibility warning, V1/DP32G030 claim, `0xF000`
  maximum, bootloader-v2/v5 distinction, packet layout, page indexing, result
  handling, and a software bootloader simulator.

### Quansheng product and FCC exhibits

- **Product page:** Quansheng's current UV-K5 page, retrieved 2026-08-06 from
  <https://en.qsfj.com/products/3002>.
- **FCC filing:** FCC ID `XBPUV-K5`, including internal photographs, document
  `6401563`, filed 2023-03-09; the PDF SHA-256 reported by the exhibit service
  is `6e97aeeec6fb3870edf70895627fed2f0d6275e8fa9a49328ff9abd63fe17f15`.
- **Scope:** product/family and physical-inspection context only. These sources
  do not publish the bootloader wire protocol or establish that a particular
  user unit is the same MCU/board revision.

## Accepted FLASH-012 observations and bounded inferences

### EVID-K5-008 — Hardware revision must be established physically

- **Observation:** K5TOOL warns that its old V1 units use DP32G030, while later
  V2 and V3 markings can identify incompatible processor revisions. It provides
  distinct recovery images and workflows for those revisions.
- **Confidence:** medium-low. This is a maintained implementation report, not a
  Quansheng board matrix, and compatible-looking models/clones may differ.
- **Permitted use:** require the operator to record the exact model, under-
  battery revision marking, PCB revision, and readable MCU marking before a
  destructive command. A bootloader beacon alone cannot satisfy this gate.
- **Required experiment:** photograph the exact test unit and its fitted MCU;
  reject anything other than an explicitly confirmed V1/DP32G030 unit.

### EVID-K5-009 — The stock application boundary is below `0xF000`

- **Observation:** k5prog caps flash writes at `0xF000` because it reports a
  bootloader in `0xF000..=0xFFFF`. K5TOOL separately enforces a maximum end of
  `0xF000` and reports successful images approaching that boundary.
- **Inference:** AFIK will treat `0x0000..=0xEFFF` as the complete application
  region and the final 4 KiB as immutable stock-bootloader space. A packaged
  image is exactly 60 KiB, with unused application bytes set to `0xFF`, so no
  stale prior application content is intentionally retained.
- **Confidence:** medium for a qualified old V1 unit and low for the family in
  general. The placement has no located vendor specification and has not been
  observed on an AFIK-owned physical unit.
- **Permitted use:** reduce the linker FLASH length, statically reject ELF
  allocations reaching `0xF000`, and emit only an exact 60 KiB raw image.
  Nothing may address, erase, package, or write the final 4 KiB.
- **Required experiment:** after recovery is proven, compare the application
  boundary against a non-destructive read/debug observation on the exact unit.

### EVID-K5-010 — Legacy packet envelope and EEPROM read observations

- **Observation:** both implementations use 38,400 baud, 8 data bits, no
  parity, and one stop bit. A packet has `AB CD`, a little-endian payload
  length, payload plus little-endian CRC-16/XMODEM, and `DC BA`. Payload and CRC
  bytes are XORed with the repeating 16-byte sequence
  `16 6C 14 E6 2E 91 0D 40 21 35 D5 40 13 03 E9 80`. Observed radio responses
  decode to a `0xFFFF` CRC field rather than a calculated CRC.
- **Observation:** normal firmware accepts `0x0514` hello with the fixed
  `0x6457396A` session word and `0x051B` reads containing a 16-bit offset,
  at-most-128-byte length, zero padding, and that word. `0x0515` and `0x051C`
  responses carry a bounded firmware version and offset/length-tagged data.
- **Confidence:** medium for the exact old firmware samples represented by the
  two tools; low for other firmware, password/custom-key modes, or clones.
- **Permitted use:** a read-only, exact 8 KiB EEPROM backup workflow that
  rejects any response mismatch. No EEPROM write command is permitted.
- **Required experiment:** capture and compare every hello/read response from
  the exact stock test unit, then hash and retain its complete backup before
  entering bootloader mode.

### EVID-K5-011 — Version-2 bootloader page-write observations

- **Observation:** the version-2 beacon is command `0x0518`; the recorded
  36-byte sample contains printable version `2.00.06`. A `0x0530` request sends
  a zero-padded, at-most-16-byte ASCII firmware-family version. The wildcard
  accepted by third-party tools bypasses the observed version check.
- **Observation:** a `0x0519` request carries header-size `0x010C`, transaction
  word `0x1D9F8D8A`, little-endian 256-byte page index and total page count,
  actual length, zero padding, and one 256-byte data area. `0x051A` returns the
  transaction word, page index, and a zero success result. Bootloader beacons
  may continue before the first page acknowledgement.
- **Inference:** AFIK will accept only a 36-byte `2.*` beacon, prohibit version
  wildcards, write all 240 full pages in ascending order, accept only exact
  matching zero-result acknowledgements, and stop on the first deviation. It
  will not retry an unacknowledged page because whether the prior write took
  effect is not observable.
- **Confidence:** medium for bootloader 2.00.06 on a qualified old V1 unit; low
  for other bootloaders. Page acknowledgement is not flash read-back.
- **Permitted use:** implement one fail-closed bootloader-v2 host workflow and
  a deterministic fake device. Command `0x057A`/bootloader v5 and all other
  beacons are rejected before the version request or any page write.
- **Required experiment:** probe the exact unit, retain the complete raw
  transcript, flash a known-good recovery image first, power-cycle, and prove
  stock boot. Only then may an AFIK application image be attempted.

## FLASH-012 physical evidence boundary

The serial protocol can prove only that a qualified bootloader acknowledged
each requested page. It cannot identify the MCU, read back internal flash,
prove power-loss recovery, or prove that Reset reaches AFIK code. Until the
physical checklist in `docs/k5-flashing.md` is complete, all tests are host,
static-image, or simulation results and `RISK-002`/`RISK-005` remain open.
