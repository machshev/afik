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

## Explicit channels, named banks, and radio configuration

Work Package 26 adds three object kinds beside the generated bank. Each is a
fixed-size version-1 payload, revalidated field by field on decode, and ordered
canonically by `(kind, id)`.

Object kinds are `1` generated bank, `2` channel, `3` channel bank, and `4`
radio configuration. The radio configuration is a singleton at ID `0`.

Channel payload version 1 is 42 bytes:

```text
format version : u8
channel ID     : u16 little endian
name length    : u8
name bytes     : fixed 12-byte field
receive Hz     : u32 little endian
transmit Hz    : u32 little endian
RX tone kind   : u8  (0 none, 1 CTCSS, 2 DCS, 3 DCS inverted)
RX tone value  : u16 little endian (CTCSS tenths of a hertz, or DCS octal code)
TX tone kind   : u8
TX tone value  : u16 little endian
modulation     : u8  (0 FM, 1 AM, 2 USB)
bandwidth      : u8  (0 narrow, 1 wide)
power          : u8  (0 low, 1 medium, 2 high)
step Hz        : u32 little endian
squelch level  : u8  (0 through 9)
flags          : u8  (bit 0 scan skip, 1 busy lockout, 2 reverse, 3 compander)
TX class       : u8
bank mask      : u16 little endian
```

Channel-bank payload version 1 is 22 bytes:

```text
format version : u8
bank ID        : u16 little endian (0 through 15)
name length    : u8
name bytes     : fixed 16-byte field
flags          : u8  (bit 0 scan enabled)
reserved       : u8  (must be zero)
```

Radio-configuration payload version 1 is 16 bytes:

```text
format version    : u8
squelch level     : u8  (0 through 9)
backlight seconds : u8  (0 off, 255 never times out)
scan resume       : u8  (0 after hold, 1 when carrier drops, 2 stop on signal)
scan dwell ms     : u32 little endian (non-zero)
scan hold ms      : u32 little endian (non-zero)
dual watch        : u8  (0 or 1)
battery save      : u8  (0 through 5)
flags             : u8  (bit 0 key beep, 1 busy lockout default, 2 AM fix,
                         3 tone tail elimination)
reserved          : u8  (must be zero)
```

Bank membership is a mask on the channel rather than a member list on the bank,
so every object stays fixed size and a channel may belong to several banks. See
`ADR-050`. The configuration compiler rejects any channel whose mask names a
bank the project does not define, before any device mutation begins.

Reserved bytes and reserved flag bits must be zero. Decoding revalidates every
constrained field, including tone envelopes, squelch levels, enumerations, and
the singleton configuration identity, so a malformed object can never become an
active configuration.
