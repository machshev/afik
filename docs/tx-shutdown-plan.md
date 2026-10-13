# Non-transmitting shutdown acceptance plans

`TXPLAN-056` prepares the separate K1 and K5 V1 plans below. Here shutdown
means removing PA drive and stopping the chip TX path before returning to
receive; the handset remains powered. Armel's selected, hardware-tested
drivers and callers are the board/register authority. The
[source correlation](hardware-evidence.md#evid-tx056-003--hardware-conclusions-correlate-with-armels-proven-source)
records every hardware conclusion and the limits of its evidence.

No capture or electrical experiment has run for this package. The operator
reports the K5/K6 attached in firmware flash mode and has authorized required
flashing and real-hardware testing. That permission carries into the next
work package; it need not be requested again. Physical RF transmission remains
outside these plans. Both targets' TX hardware/calibration envelopes remain
unqualified.

## Immediate hardware slice

The next step is `TXOFF-057`: a K5 receive-only diagnostic image, using the
proven Armel zero-bias/PA-disable and chip-idle operations from established
initialization, followed by the existing full narrow-FM receive setup. Add
tests, build and verify the package, then flash the attached radio under the
standing authorization. Preserve EEPROM/calibration and the bootloader.

Capture boot identity, source-correlated register read-back, input-only PTT,
repeated off/receive cycles and normal serial responsiveness. A device response
and a register match are bus/software evidence; they do not measure RF silence,
active-TX decay, conducted power or independent fail-off. Report those limits
without blocking this receive-only slice on later RF qualification.

## Common experiment record and bounds

Use a separate record per exact unit, image and case. Record product/PCB/RF-chip
identification, source pins, image length/hash, actual bootloader/application
identity, supply/cable arrangement, case ID, stimulus, observations, result and
artifact hashes. Keep unit identifiers, calibration bytes and backup hashes
private. Revalidate the available recovery image and preserved calibration
coverage when needed; a historical logical 8 KiB backup does not by itself
identify which physical memory addresses it protects.

For instrumented qualification, add a reviewed pad/net photograph or schematic,
probe/ground arrangement, instrument range/loading, acquisition rate/timebase,
RF fixture/attenuation ratings and detection limits. The generic BK4829 circuit
does not identify a handset PCB. Select physical pads by fitted package and
net, not inherited `PIN29` strings. The source's off operation uses logical
GPIO1 index 1; the string is not a package address or a firmware defect.

Declare numeric limits before an instrumented pass: PA/bias off voltage bounds,
RF off threshold and coverage, maximum stimulus-to-off latency and acquisition
uncertainty. Missing limits or observation points produce **blocked** or
**partial evidence**, never an invented pass. A PA line or antenna measurement
alone cannot prove the chip TX path is off behind a disabled external PA.

Planning bounds: at most three deliberate trials per case, with one second
before and four seconds after each stimulus captured. These five-second windows
and counts are experiment bounds, not chip settling or shutdown guarantees.
An incomplete capture is invalid. Stop on any failure; successful repetition
is an intentional case, not automatic retry after a failure. Changes to limits,
image, fixture or acquisition require a new record.

Use a scope/logic analyzer for bus/PTT/control timing, a suitable voltage probe
for PA enable/bias and supply, and a rated terminated RF measurement path for
RF qualification. Current or audio alone is not an off measurement. Before
electrical fault injection, establish an external means of removing all DUT
energy, including the battery and cable back-power. Do not rely on the chip
bus as the emergency cutoff. Probe attachment and fault injection are separate
from the immediate boot/register/receive smoke test.

## K1 plan

Reference: `armel/uv-k1-k5v3-firmware-custom` at
`fe9c4e9432694b50aea651084a043aae0b58673d`, selected `App/driver/bk4829.c`.
The proven receive off strategy is zero bias and cleared logical GPIO1 PA
enable (`App/radio.c:782-784`); idle writes neutral chip mode
(`App/driver/bk4829.c:1159-1162`). Keep its known initialization/output
ownership prerequisites. Do not apply the unselected BK4819 driver or copy its
implementation. Source path lengths/normal operation do not establish fault
latency. K1 hardware is not reported attached for this run.

| Observation point | Armel source role | Physical mapping prerequisite |
| --- | --- | --- |
| Register select/clock/data | PF9 / PB8 / PB9, `App/board.c:108-112,126-129` | Match MCU nets to accessible pads and logic level. |
| PTT | PB10, `App/driver/gpio.h:29-35,77-80` | Input-only observation in a verified receive-only image. |
| PA enable / RX enable | Logical GPIO1 / GPIO0, `App/driver/bk4819-regs.h:137-143` | BK4829 document pins 1 / 2; match fitted chip and external nets. |
| UHF / VHF LNA | Logical GPIO3 / GPIO4, same source | BK4829 document pins 31 / 30; complete switch topology remains separate. |
| Bias / chip RF output | Bias helper `App/driver/bk4829.c:729-752`; published VRAMP / PAOUT | Document pins 18 / 17; use qualified access/probes, not a guessed PCB point. |

Run the case matrix with prefix `TXPLAN-056/K1`. Historical recovery and receive
observations remain useful evidence, not fresh captures. Preserve the distinct
SPI-NOR calibration map at source base `0x100D0`; do not substitute the K5 I2C
map. Complete source/unit/fixture identification before an instrumented run.

## K5 V1 / attached K5-K6 plan

Reference: `armel/uv-k5-firmware-custom` at
`fbcf26d8e9811b135b7e2d97bdefebaa4b3ed9e0`, selected `driver/bk4819.c`,
corroborating the existing machshev pin. Proven receive zero-bias/PA-clear
calls are `radio.c:721-723`; idle is `driver/bk4819.c:1077-1080`.
Use the known initialized output state; unknown shadow contents do not become
safe merely by clearing one bit. The attached radio is operator-reported in
flash mode; the visible CH340 path is `/dev/ttyUSB0`. A beacon supplements board
identity and does not describe the bootloader's RF state.

| Observation point | Armel source role | Physical mapping prerequisite |
| --- | --- | --- |
| Register select/clock/data | PC0 / PC1 / PC2, `driver/gpio.h:51-58` | Match MCU nets and accessible pads; preserve shared data direction. |
| PTT | PC5, same source | Input-only observation; never use a TX-capable stock image for held-PTT cases. |
| PA enable / RX enable | Logical GPIO1 / GPIO0, `driver/bk4819-regs.h:136-142` | BK4819 document pins 29 / 28; match fitted chip and board nets. |
| UHF / VHF LNA | Logical GPIO3 / GPIO4, same source | BK4819 document pins 31 / 32; they do not identify every external RF switch. |
| Bias / chip RF output | `driver/bk4819.c:661-683`; published VRAMP / PAOUT | Document pins 18 / 17; use qualified access/probes. |

Run the cases with prefix `TXPLAN-056/K5`. Preserve the source I2C calibration
map at base `0x1ED0` and the original bootloader. PC4 is the speaker gate;
neither speaker mute nor PC5 release establishes PA off. Correlate observed
register words with the selected Armel operations; record raw discrepancies
and stop restoration on an unverified shutdown result. Do not introduce a
target TX enable or synthetic power envelope to make a diagnostic pass.

## Case matrix for each target

All cases are initially **not run**. The receive-only diagnostic may establish
only the observations it implements; additional cases stay pending.

| Case suffix | Stimulus / starting condition | Required observation and limit |
| --- | --- | --- |
| `C01` cold start | Power on an identified receive-only image | Capture off/idle before RX, raw control words and boot identity. No energizing action. Startup electrical/RF state needs instruments. |
| `C02` warm reset / bootloader handoff | Separately identified reset or flash-to-application transition | Capture the full handoff, not just the first app response. MCU reset alone is not an independent PA inhibit. |
| `C03` held PTT | Hold the physical input across receive-only startup, then release | Off/receive behavior and fresh input observation persist. No chip/PA TX entry. Never run this on the TX-capable recovery image. |
| `C04` unknown software state | Fresh diagnostic/controller state over known non-transmitting hardware | Off attempt does not require known-TX software state. This does not test arbitrary physical register contents or prior active TX. |
| `C05` partial preparation | Stop after each approved non-energizing operation in a later instrumented witness | Off then idle; restore RX only after both verified completions. Never seed the source TX word to create this case. |
| `C06` target fault / failed disable | Inject a bounded software failure at each off-effect boundary | Attempt chip off even if PA off fails; no RX after uncertain disable. Electrical bus failure remains pending until independent cutoff is qualified. |
| `C07` repeated successful off | Three separately triggered off/receive cycles | Correlated words/order remain stable and normal identity/receive remain responsive. A failed trial terminates the case. |
| `C08` failed receive restore | Bounded failure during restoration in a later witness | Second PA-off/chip-off pair, distinct completion identities, latched fault, no automatic receive/TX retry. |
| `C09` changed receive selection | Change the authoritative selection before restoration | Restore the current full RX setup, tones/squelch/audio preference, not an obsolete request. Record the actually covered channel/mode subset. |

Stop on an unexpected energizing write, control mismatch, unexplained RF,
unstable supply, lost response, missing capture, or uncertain off completion.
Remove DUT energy with the established external cutoff if electrical/RF state
is uncertain; preserve the transcript and diagnose before another trial. Do
not automatically retry writes or invoke recovery/restore as an error handler.

## Evidence carried forward

Record **pass**, **fail**, **invalid capture**, **blocked** or **not run** per
case and per observation, with measured limits and exact source correlation.
Normal source operation stays trusted. Register read-back and application boot
can advance the immediate receive-only diagnostic without claiming a measured
physical shutdown. Instrumented off-state observations still do not establish
active-TX decay, calibrated power/deviation, spectral limits, deadline behavior
under active TX, or fail-off when the shared bus/executor is unavailable.
Those remain `RISK-040` items 2-5 and must precede any physical TX integration.
