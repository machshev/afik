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
