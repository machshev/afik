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

GPIO, display, keypad, BK4819, audio, power, and scenario-YAML models belong to
later work packages; no behaviour for them is invented here.

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
