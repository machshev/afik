# Serial protocol

The stream format is a COBS-encoded packet followed by `0x00`. The decoded
packet is:

```text
magic          : 2 bytes (`UR`)
version        : u8
service        : u8
flags          : u8
sequence       : u16 little endian
command        : u8
payload length : u16 little endian
payload        : bounded bytes
CRC            : CRC-16/CCITT-FALSE over header and payload
```

Work Package 2 services are Device Info and Configuration. It implements
HELLO, GET_CAPABILITIES, LIST_OBJECTS, READ_OBJECT, BEGIN_TRANSACTION,
WRITE_OBJECT, VALIDATE_TRANSACTION, COMMIT_TRANSACTION, and ABORT_TRANSACTION.

Responses retain the request sequence, set the response flag, and use either
the request command or the error command. Errors carry a stable one-byte code.
Unknown or corrupt packets are discarded; the delimiter provides stream
resynchronisation.

## Bounded object listing

`LIST_OBJECTS` requests contain a zero-based `u16` object offset. Successful
responses contain:

```text
active generation : u32 little endian
total objects     : u16 little endian
requested offset  : u16 little endian
returned objects  : u16 little endian
descriptors       : returned objects * 5 bytes
```

Each descriptor is an object-kind `u8`, kind-local ID `u16`, and encoded
payload length `u16`. Descriptors are strictly ordered by `(kind, ID)`, with no
duplicate keys. The fixed 128-byte protocol payload bounds a page to 23
descriptors. The host requests successive offsets and requires every page to
report the same generation and total, preventing pages from different active
snapshots from being combined. An offset beyond the total, an out-of-range
count, unordered keys, or trailing payload bytes is malformed.

## Transaction state errors

Beginning a transaction while one is open returns `TransactionAlreadyOpen`.
Writing, validating, committing, or aborting with no matching active transaction
returns `NoTransaction`. Committing a matching candidate before successful
validation returns `NotValidated`. These errors neither activate nor discard the
candidate and never alter the active generation or objects; the matching
transaction remains available for an explicit abort.

A structurally accepted candidate that fails whole-object validation returns
`ValidationFailed`. A write beyond the candidate object capacity returns
`CapacityExceeded`. Both failures leave the active snapshot untouched and the
matching candidate available for explicit abort.

Recognised but unimplemented services return `UnsupportedService`; recognised
commands sent to the wrong service return `UnsupportedCommand`. Nonzero request
flags and command payloads with missing, extra, invalid, or out-of-range fields
return `MalformedPayload`. A well-formed read for an absent key returns
`ObjectNotFound`. All error responses retain the request service and sequence,
use the error command, set response and error flags, and carry the rejected
command plus stable error code.

## Duplicate request sequences

The device retains exactly the most recent decoded request and response. An
immediately repeated byte-identical frame with the same sequence replays that
response without executing the command again. Reusing the cached sequence for
different service, flags, command, or payload returns `SequenceConflict` and
does not replace the cache or mutate device state. This bounded one-exchange
window supports synchronous response-loss retries without an unbounded replay
table.

Unknown service or command wire values, invalid CRCs, malformed COBS packets,
and packets exceeding the fixed stream buffer are discarded at their delimiter
without a response. The decoder then accepts the next valid frame, including
when it arrives one byte at a time.

Work Package 8 adds a host CLI adapter for an explicitly supplied Linux device
path and baud. It configures that path through `stty` as raw, no-echo, `min 0`,
`time 1`, then opens one ordered read/write file and implements the existing
`ProtocolTransport`. This introduces no new wire command, raw storage access,
target UART assumption, default device, or default baud. Physical
interoperability remains open in `RISK-009`.

## Common application payload mapping

`radio-platform::serial_control::wire` defines the small common payload which
target-owned serial adapters may carry inside an already validated frame. A
request is exactly two bytes: `01 00` queries the semantic receive view,
`02 00` queries the adapter's capability bits, `03 key` applies one bounded
canonical logical key, and `04 00` is an explicit audio toggle. Key codes are
stable (`Up=04`, `Down=05`, `Side2=01`); Up/Down follow the operator shell's
previous/next direction. Menu is owned by the operator shell and is not a
common serial receive command. K5 advertises and accepts the key/audio
mutation boundary. K1 advertises view-only common access because its banked
controller and shell live in another task; its target adapter refuses these
mutations rather than changing only `ReceiveApp`.

A view response is four bytes: `81 channel audio state`, or `83 channel audio
state` after a key/audio command. `audio` is `0` or `1`; `state` bit 0 is
sampled squelch, bit 1 is receiver health, and bit 2 is operator monitoring.
A View request may report a receiver fault diagnostically; a state-changing K5
response carrying that fault is rejected by the host as a typed operation
failure.
A capability response is three bytes:
`82` followed by a little-endian `u16` bitset. Unknown operations, reserved
bits, wrong lengths, and invalid key codes are rejected. The mapping is
transport/storage-free; K1 retains its COBS/object/runtime service and K5
retains its observed normal-mode envelope and circular-DMA adapter. Candidate
configuration changes are not encoded here because their exact
acknowledgement and persistence semantics remain target-owned.

## APP-052 target-owned envelope assignments

The common payload is carried only inside an already validated target
envelope. K1 retains the existing `RuntimeControl` service and assigns command
`0x18` (`CommonApplication`) to an exactly two-byte payload. Its response keeps
the request service, sequence, response flags, and command, so the existing
`DeviceService` replay and sequence-conflict rules continue to apply. K1's
`DeviceInfo`, `Configuration`, and existing runtime commands are unchanged.

K5's observed normal-mode envelope has no spare request payload bytes after
its fixed session word, so it assigns `0x0600` to common View, `0x0601` to
common Capabilities, `0x0603` for Side2, `0x0606` for Up, `0x0607` for Down,
and `0x0615` for explicit audio toggle. The K5 serial key domain omits
shell-owned Menu, Side1, PTT, Exit, Star, Function, and digits. K5 replies with
`0x0620` and places the bounded common
response at the start of the existing fixed response payload. Its inner length
is the common response data length (four bytes for View/key and three for
Capabilities), not the command/header size; the hello
command `0x0514`, bootloader values, and version command remain untouched.
Unknown assignments, bad CRC/session/trailer, malformed lengths, and reserved
command values are discarded without an application call. K5 has no replay
cache beyond its target envelope because no sequence field exists in the
observed normal-mode frame.

These assignments are target-owned adapters rather than a claim that K1 and K5
have interoperable physical serial links. Candidate configuration activation,
K5 persistence, and target-only K1 object/runtime operations remain excluded.

## K5 host control witness

The recovery flasher exposes one-at-a-time, normal-mode-only control requests:

```text
afik-k5 --device PATH control view
afik-k5 --device PATH control capabilities
afik-k5 --device PATH control key AUDIO|UP|DOWN
```

The host always sends the fixed session word and request CRC through the
existing K5 envelope encoder, then accepts only one bounded response. It
requires the exact `0x0620` response command, the expected inner length (four
bytes for a view/state-changing response or three for capabilities), zero reserved bytes,
valid response footer/marker, and valid view/capability payload bits. A bad
frame is an operation failure and cannot trigger another request. The command
does not write EEPROM, persist configuration, transmit, or flash. Its output is
line-oriented and identifies the request, response, channel/audio/squelch/
receiver/monitoring state, or capability labels. This is simulator/fake-transport and
static host evidence only; physical serial interoperability remains open.
