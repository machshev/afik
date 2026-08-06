# Storage object model

Device configuration is a bounded set of typed objects. Each object has a
stable kind, numeric ID, encoded length, and encoded bytes. The first supported
configuration object is a generated channel bank.

Transactions clone the active object set into a candidate, apply object writes
to that candidate, validate the complete candidate, and atomically replace the
active snapshot on commit. Abort or any validation error leaves the active
snapshot unchanged. This is logical atomicity; physical power-loss durability
is deferred and tracked as `RISK-004`.

The store exposes active objects without candidate data but does not define a
wire order. Protocol implementations sort listings by stable object kind and
ID before encoding them.

Generated-bank payload version 1 contains:

```text
format version : u8
bank ID        : u16 little endian
name length    : u8
name bytes     : fixed 16-byte field
base Hz        : u32 little endian
spacing Hz     : u32 little endian
channel count  : u16 little endian
TX class       : u8
```

## Canonical configuration image

Work Package 4 adds an offline container for one complete logical object set.
The container is not an on-flash layout and makes no power-loss or physical
storage claim. Encoding is allocation-free: callers provide both the canonical
object slice and the destination byte buffer.

The 16-byte image header is:

```text
magic                 : 4 bytes (`AFIK`)
image version         : u8 (`1`)
object format version : u8 (`1`)
object count          : u16 little endian
payload length        : u32 little endian
CRC-32                : u32 little endian
```

The payload contains exactly `object count` envelopes with no padding or
trailing bytes:

```text
object kind   : u8
object ID     : u16 little endian
object length : u16 little endian
object bytes  : object length bytes
```

Objects are strictly ordered by `(kind, ID)`, so duplicate and reordered keys
are invalid rather than alternative encodings. Every object payload must pass
its ordinary object decoder. CRC-32/ISO-HDLC uses reflected polynomial
`0xEDB88320`, initial and final XOR values `0xFFFFFFFF`, and covers header bytes
0 through 11 followed by the complete payload; the checksum field itself is
excluded.

Decoding checks the magic, both versions, exact total length, checksum, entry
bounds, strict key order, and every object before returning an iterable image.
The object count is bounded by `u16`, each object retains the existing
`MAX_OBJECT_DATA` bound, and no decoded object is exposed after partial
validation.
