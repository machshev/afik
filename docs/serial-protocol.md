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
