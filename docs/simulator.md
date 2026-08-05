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

GPIO, display, keypad, BK4819, audio, power, and scenario-YAML models are later
Work Package 1/2 extensions; no behaviour for them is invented here.
