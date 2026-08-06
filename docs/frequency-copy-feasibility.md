# Frequency Copy feasibility

This document is the research result for `FREQ-010`. It defines what a future
AFIK feature may represent and test without treating incomplete BK4819 evidence
as production behavior.

## Verdict

Frequency Copy is **design-ready but hardware-command blocked**.

AFIK can safely specify a bounded, receive-only candidate and an explicit-input
workflow now. It must defer the BK4819 scan command, target adapter, physical
timing, and register-level simulator until revision-matched original evidence or
the receive-only experiments below establish every written bit, transition,
unit, result rule, and cleanup path.

This verdict does not claim that AFIK can currently measure a physical signal.
It authorizes no behavioral code under `FREQ-010`.

## Product workflow and scope

The FCC-filed UV-K5 manual describes Fast Copy as a frequency-meter workflow:
a nearby strong transmission is measured, carrier frequency and possibly the
transmitting CTCSS/DCS are displayed, the user may remeasure, and saving to a
chosen channel is explicit. A related mode scans only CTCSS/DCS when the receive
frequency is already known.

That workflow is distinct from the manual's Wireless Radio Replication or Air
Copy feature. Air Copy transfers configuration data between radios on a shared
data frequency and necessarily transmits. It is out of scope for this proposal.

Source provenance, exact manual controls, chip-level evidence, and evidence
confidence are recorded in `hardware-evidence.md` under `EVID-FCOPY-008` through
`EVID-FCOPY-011`.

## What one received transmission can establish

| Property | Candidate treatment | Reason |
| --- | --- | --- |
| Carrier frequency | Observed receive-frequency candidate | The documented workflow displays it; physical units and accuracy still require verification. |
| CTCSS | Optional signalling observation | The workflow reports it, but crystal conversion, discrimination, and timeout behavior are unverified. |
| DCS | Optional raw validated observation including reported length | The public chip description names 23/24-bit DCS; polarity and exact result interpretation remain unverified. |
| Signal/stability evidence | Bounded measurement metadata | A future controller needs repeat count and spread to explain acceptance without claiming RF accuracy. |
| No signalling | `NotObserved`, not automatically `Tone::None` | No tone/code and failure to detect one are not yet distinguishable. |
| Duplex or offset | Not observable | One carrier does not identify the corresponding repeater input or simplex intent. |
| Transmit frequency/class/permission | Never inferred | Spectrum observation conveys no legal authority or trusted plan membership. |
| Modulation, bandwidth, power | Not established | They are not outputs of the documented measurement. |
| Name, scan lists, contacts, scrambler, source identity | Not established | These are configuration or identity data, not properties proved by one carrier measurement. |

A displayed or rounded frequency is a measurement result, not proof that the
source is correctly tuned, licensed, unique, or safe to transmit to.

## Proposed bounded domain boundary

The following is a design sketch, not an implemented or frozen Rust API:

```text
FrequencyCaptureCandidate {
    observed_receive: Frequency,
    signalling: SignallingObservation,
    quality: CaptureQuality,
}

SignallingObservation =
    NotMeasured
  | NotObserved
  | Ctcss { tenths_hz, confirmations }
  | DcsRaw { bits, bit_length, confirmations }

CaptureQuality {
    frequency_confirmations: u8,
    maximum_spread_hz: u32,
}
```

All integers must be checked and all counts must have compile-time bounds. No
heap, string, wall clock, driver handle, transmit frequency, `TxClass`, or
`TxAuthorisation` belongs in the candidate. `DcsRaw` is intentionally not the
existing trusted `Tone::Dcs` until original documentation and experiments
establish encoding, code validation, and polarity.

The candidate must not implement an implicit conversion to `ActiveChannel`.
That existing type includes a transmit frequency and trusted TX classification,
which observation cannot supply.

## Proposed explicit-input state machine

A future hardware-independent controller should own state but no clock, bus,
scheduler, or storage:

```text
Idle
  -> MeasuringFrequency { attempt, timer_token }
  -> FrequencyCandidate { receive, quality }
  -> MeasuringSignalling { receive, attempt, timer_token }
  -> ReviewCandidate { candidate }
  -> Saved | Cancelled | Failed
```

- Start, retry, skip-signalling, confirm-save, cancel, adapter result, adapter
  fault, and opaque timer expiry are explicit inputs.
- Each arm/retry receives a fresh bounded token. Results or expiries carrying a
  replaced, cancelled, or stale token cannot change current state.
- Acceptance requires bounded repeated results within an explicit configured
  tolerance. Neither repeat counts nor tolerance are copied from descendant
  firmware; they remain policy inputs until experiments justify defaults.
- A signalling timeout can retain a reviewed frequency candidate with
  `NotObserved`; it must not silently mean `Tone::None`.
- Cancel, timeout, rejection, completion, and every adapter error request a
  transition to a separately evidenced neutral or known receive state.
- If cleanup fails, the adapter remains faulted. No subsequent receive or
  transmit command is permitted until the existing fail-closed recovery
  boundary succeeds.

The hardware adapter should expose semantic start/poll/stop operations only
after its exact register plan is evidenced. The deterministic simulator should
model the controller's declared result/fault inputs, not invented BK4819 reset
values or timing.

## Failure and review semantics

A future API should distinguish at least:

- `NoSignal`: no candidate result under a separately validated carrier rule;
- `FrequencyUnstable`: bounded results disagree beyond configured tolerance;
- `FrequencyTimeout`: scan completion was not observed before the deadline;
- `SignallingNotObserved`: frequency retained, no validated tone/code result;
- `SignallingUnsupported`: bits or code cannot be represented safely;
- `SignallingTimeout`: decoder did not complete before its deadline;
- `BusFault`: one bus operation failed and physical state is unknown;
- `CleanupFault`: return to the evidenced neutral/receive state failed;
- `Cancelled`: deliberate exit completed cleanup.

Where physical evidence cannot distinguish two outcomes, the implementation
must return the less specific outcome. Errors retain no hidden partial channel
mutation. Review shows observed values, uncertainty, and the fact that transmit
settings are absent.

## Storage and TX-policy boundary

Measurement and storage are separate transactions:

1. Capture produces only the receive-only candidate.
2. The user reviews the frequency, signalling status, and quality.
3. A deliberate save request constructs a separately validated logical object.
4. Its receive frequency may come from the reviewed candidate. Its transmit
   frequency is not copied or derived, and its class is `TxClass::Never`.
5. Any later conversion to a regional/channel-plan object with a different TX
   class is an independent programming action subject to all existing plan and
   TX-policy validation.

The initial save representation may need a new receive-only storage object; the
current `ActiveChannel` and generated-bank expansion are deliberately unsuitable
because they contain transmit semantics. Until such an object is designed and
versioned, review may end without storage.

Frequency Copy never mints a `TxAuthorisation`, enables the hidden TX menu,
updates the active policy, or bypasses `radio-tx-policy`. Even an explicitly
saved `TxClass::Never` object remains untransmittable because policy denies that
class unconditionally.

## Required receive-only experiments

Before production driver work, write a test plan with equipment identifiers,
calibration dates, raw traces, board photos/revision, fitted chip marking, and
repeatable setup. The setup must physically prevent DUT transmission and use a
shielded/coupled signal path where practicable.

1. Identify the BK4819 silicon marking/revision, board revision, reference
   crystal or TCXO, calibration words, and RF switch/filter path. Obtain the
   original matching Beken register documentation.
2. Establish recovery first: preserve configuration/calibration, current-limit
   the DUT, prove reboot/recovery, disable PTT/TX controls, and monitor the RF
   output independently for unintended emission.
3. From a known receive baseline, capture all relevant registers before and
   after scan. Vary one proposed `REG_32` field at a time. Establish every bit,
   preservation mask, enable/disable action, busy transition, result read/latch
   order, retrigger behavior, stale-result behavior, and cleanup state.
4. Sweep calibrated generator frequencies and levels across each board receive
   path. Quantify raw units, bias, repeatability, acquisition distribution,
   display rounding, purported sensitivity threshold, band-edge behavior, and
   temperature/supply sensitivity where safe.
5. Test two signals, adjacent signals, harmonics, images, over-range input, and
   strong out-of-band interferers. Record false locks and whether repeat
   filtering can reject them without hiding instability.
6. Exercise standard and near-boundary CTCSS tones, no tone, short bursts, tone
   tails, changing tones, and the fitted crystal conversion. Exercise known DCS
   codes in every documented length/polarity combination, invalid/unrecognized
   patterns, no code, and interference.
7. Inject a bus failure at every read/write, delayed and never-clearing busy,
   cancellation at every state, stale results after retry, result changes
   between high/low reads, power interruption, and reboot. Prove bounded exit,
   no storage mutation, no TX-mode write, and safe cleanup or fault latch.
8. Observe an unmodified radio's product workflow non-destructively to compare
   displayed rounding, retry, timeout, tone/code, and save prompts. Treat this
   as behavior comparison, not proof of internal register semantics.

## Future deterministic test matrix

When evidence permits implementation, tests must cover:

- exact state/update traces for successful frequency-only, CTCSS, and DCS
  candidates;
- zero/maximum bounds, checked 10 Hz-to-Hz conversion, DCS length/code
  validation, repeat-count saturation, and frequency spread arithmetic;
- stable acceptance at the boundary and rejection immediately outside it;
- timeout, cancel, retry, no-result, unknown code, bus-fault, and cleanup-fault
  behavior from every phase;
- stale timer/result tokens after retry, phase change, cancellation, and token
  wrap/exhaustion;
- no project mutation before explicit confirmation and atomic failure when
  storage validation/capacity fails;
- exact persisted receive-only representation with `TxClass::Never`, followed
  by denial from `radio-tx-policy` and the BK4819 TX boundary;
- command-plan failure injection proving neutral-first operation, fault
  latching, and no TX-mode write;
- identical explicit-input scripts producing identical traces and bytes;
- a register fake derived only from newly accepted evidence, with physical test
  vectors kept distinct from controller simulation.

## Readiness gates

Implementation remains deferred until all of these are true:

- original documentation matches the identified silicon and resolves every
  command bit and result field;
- board path, crystal/calibration, preservation, cleanup, and recovery are
  known;
- receive-only experiments quantify accuracy, acquisition, false locks,
  signalling behavior, timeouts, cancellation, and faults;
- the evidence and raw artifacts are reviewable and update
  `hardware-evidence.md` plus `RISKS.md`;
- a new work package explicitly authorizes the bounded controller, storage
  object, adapter, and tests;
- TX remains outside the feature and all captured output is fail-closed.
