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

`ConfigurationSnapshot` validates strict key order and every supported object,
reports exact object/payload/channel capacity, and encodes directly to the same
canonical image format as compiled configuration. Backup callers therefore do
not need to reconstruct compiler internals or duplicate image logic.

`write_configuration_verified`, `backup_configuration`, and
`restore_configuration_image` are the shared front-end workflows. A verified
write commits a compiled configuration and then requires the complete
generation-tagged read-back to match. Backup reads a stable snapshot and emits
its canonical image. Restore validates and imports the complete canonical image
against negotiated capabilities before mutation, commits it, and requires the
same exact read-back verification. A mismatch is an explicit programmer error.

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
explicit groups, regional plans, named backup catalogs, CSV, firmware updates,
and serial discovery remain later tasks. Work Packages 8 and 9 supply complete
CLI and local-GUI front ends for the operations the library currently supports;
the canonical image remains a logical-object container, not a physical flash
layout.
