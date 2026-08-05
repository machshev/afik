# Programmer library

`radio-programmer` is the authoritative host implementation. A
`ProtocolTransport` supplies ordered `send` and `receive` bytes; serial,
Renode, replay, CLI, and GUI adapters will implement that contract rather than
duplicating session logic.

Offline compilation takes a `RadioProject` plus target capabilities, validates
plan support and object/frame capacity, and produces deterministic bounded
storage objects with a capacity report. It performs no device mutation.

A connected `Programmer` performs `HELLO` and `GET_CAPABILITIES`, sends compiled
objects through begin/write/validate/commit, and reads active objects back.
Receive handling supports frames fragmented down to one byte per call and
discards malformed packets until the next COBS delimiter.

`list_objects` reconstructs bounded `LIST_OBJECTS` pages into one complete
generation-tagged listing. It rejects a changed generation or total, an
unexpected offset, an object count beyond negotiated capacity, an object
length beyond negotiated capacity, and keys that are not globally strictly
ordered.

The first milestone intentionally supports only generated banks. Project-file
metadata, explicit groups, regional plans, storage images, CSV, backups,
firmware updates, serial discovery, CLI, and GUI remain later tasks.
