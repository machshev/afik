# Functional simulator

`radio-sim` is a deterministic protocol-level device model. `SimClock` advances
only when explicitly instructed, and every observable operation appends a
timestamped event. Two runs with the same input therefore produce identical
traces.

`SimTransport` implements the programmer's byte-oriented transport. Sending a
complete delimited frame synchronously drives the simulated device and queues
its encoded response. The device negotiates declared capabilities, stages
configuration objects transactionally, validates generated-bank objects, and
supports deterministic bounded listing and read-back from the active snapshot.
Object listings are sorted by stable `(kind, ID)` key rather than storage
insertion order and are tagged with the active generation.

Successful explicit aborts are recorded in the deterministic trace. Aborting
discards staged replacements without advancing the active generation, and the
device immediately accepts a new transaction.

The simulator caches the most recent request/response exchange. Exact duplicate
requests replay the prior response and conflicting reuse of the sequence is
rejected; both paths are explicit deterministic trace events.

Stream recovery tests interleave CRC failure, malformed COBS, fixed-buffer
overflow, and a valid request delivered one byte at a time. Only the valid
request produces a response, and discarded packet errors appear in exact input
order in the trace.

GPIO, physical display/keypad behavior, physical RF, audio, power, and
scenario-YAML models belong to later work packages; no behavior for them is
invented here.

## Boot UI and TX permissions

Work Package 5 adds a separate `UiSimulator` using the same explicit virtual
time principle. It records boot/load status, logical key edges, semantic views,
UI actions, and in-memory permission persistence in exact order. Identical
timed scripts must produce identical traces and bytes.

The simulator holds persisted permission bytes separately from the active
`TxPolicy`. A save action replaces only the simulated bytes. The currently
active policy remains unchanged until an explicit simulated reboot validates
the record; corrupt bytes default that reboot to deny-all. Cancel emits no
persistence event and preserves the original bytes.

This proves logical state-machine behavior, not non-volatile durability,
physical keypad scanning, display rendering, debounce timing, or security of a
particular key chord.

## BK4819 command simulation

Work Package 6 adds a separate `RfSimulator` around the hardware-independent
post-initialization driver. Its logical register bus records completed reads and
writes at explicit virtual timestamps and can fail exactly one operation after a
chosen number of successful operations. Completed receive, status, standby, TX,
and TX-stop commands are semantic trace events. A failed register operation is
recorded, faults the driver, and cannot produce a completed TX event.

Identical timed scripts must produce identical traces. A class-mismatched
`TxAuthorisation` produces neither a register operation nor a TX event; a bus
failure on the final TX-mode write records only that failed write. Recovery
requires an explicit successful neutral-mode write before another command.

The register values and command order are the bounded, low-confidence contract
in `docs/hardware-evidence.md`. The harness does not model physical 3-wire
timing, reset or initialization, calibration timing, board RF switches, filters,
audio, external PA control, propagation, signal strength, or emitted RF.

## Channel activation and scanning

Work Package 7 adds `ChannelSimulator` as a host composition of
`ChannelController` and `RfSimulator`. Initial construction validates every
generated-bank channel against the logical RF frequency representation before
any retained simulator state exists. Each controller activation then runs the
same receive command path and appends timestamped control and RF events.

Timer arms become absolute virtual deadlines. Advancing time performs no
implicit work; the caller must deliver the opaque token. The current token is
rejected before its deadline, while stale or cancelled tokens are delivered to
prove that the controller ignores them. Normalized signal samples are explicit
inputs. They are not synthesized from propagation, RSSI, or a physical receiver.

Scanning denies controller-level TX before RF operations. Selected-state TX
passes through `TxPolicy`, the class-bound capability bundle, and the BK4819
driver; TX stop resumes receive on the still-selected channel. Identical timed
scan/hold/stop/TX scripts produce identical control and RF traces.

## Minimal DP32G030 target model

Work Package 3 adds a separate Renode model for the target reset proof. It maps
only the evidenced 64 KiB flash and 16 KiB RAM ranges and instantiates Renode's
Cortex-M0 core with the NVIC plumbing required by that core model. There are no
DP32G030 clock, reset-controller, flash-controller, interrupt-source, or board
peripheral models.

The Robot Framework test loads the verified target ELF, confirms that the
simulation-only RAM sentinel is zero before execution, starts the machine
without changing PC or the vector-table offset, and requires the exact sentinel
written by the Rust Reset handler. The result confirms the declared model and
software reset path only; it is not physical-silicon evidence.
