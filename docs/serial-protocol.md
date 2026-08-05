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
