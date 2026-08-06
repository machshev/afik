# Programmer library

`radio-programmer` is the authoritative host implementation. A
`ProtocolTransport` supplies ordered `send` and `receive` bytes; serial,
Renode, replay, CLI, and GUI adapters will implement that contract rather than
duplicating session logic.

Offline compilation takes a `RadioProject` plus target capabilities, validates
plan support and object/frame capacity, and produces deterministic bounded
storage objects in canonical stable-key order with a capacity report. It
performs no device mutation.

A compiled configuration reports its exact canonical image length and encodes
that image into a caller-provided buffer. A target-bound compiler can import an
image only after the storage codec has validated the complete container. Import
then rechecks negotiated storage version, object count and size, write-frame
capacity, plan-encoding support, and capacity arithmetic. Compilation and image
import therefore produce the same `CompiledConfiguration` representation and
capacity report without contacting a radio.

A connected `Programmer` performs `HELLO` and `GET_CAPABILITIES`, sends compiled
objects through begin/write/validate/commit, and reads active objects back.
Receive handling supports frames fragmented down to one byte per call and
discards malformed packets until the next COBS delimiter.

`list_objects` reconstructs bounded `LIST_OBJECTS` pages into one complete
generation-tagged listing. It rejects a changed generation or total, an
unexpected offset, an object count beyond negotiated capacity, an object
length beyond negotiated capacity, and keys that are not globally strictly
ordered.

`read_configuration` uses that listing to read every active object in stable-key
order. It checks each returned payload length against its descriptor and
repeats the listing afterward, rejecting the result if the active generation or
object descriptors changed during the read.

The compiler still supports only generated banks. Project-file metadata,
explicit groups, regional plans, named backup management, CSV, firmware
updates, serial discovery, CLI, and GUI remain later tasks. The canonical image
is an offline logical-object container, not a physical flash layout.
