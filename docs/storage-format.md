# Storage object model

Device configuration is a bounded set of typed objects. Each object has a
stable kind, numeric ID, encoded length, and encoded bytes. The first supported
configuration object is a generated channel bank.

Transactions clone the active object set into a candidate, apply object writes
to that candidate, validate the complete candidate, and atomically replace the
active snapshot on commit. Abort or any validation error leaves the active
snapshot unchanged. This is logical atomicity; physical power-loss durability
is deferred and tracked as `RISK-004`.

## One bound, in bytes

A store is a packed byte arena and its size in bytes is the whole bound. There
is no object count, no per-kind count, and no fixed slot charged to every
object whatever it holds. Objects are held end to end as
`(kind, ID, length, payload)` entries in strict `(kind, ID)` order, which is
exactly what a canonical image carries after its header, so an arena's bytes
and an image payload are the same bytes. Writing an object which is already
present replaces it and compacts the entries around it; removing one closes the
gap it leaves.

A device advertises those bytes as `configuration_bytes`, and everything else
it reports follows from them: `max_objects` is the count the bytes imply given
the shortest object any kind encodes to, which is an upper bound rather than a
second limit. A host refuses a project for the bytes it needs and names both
numbers. A device which reports zero bytes declares nothing rather than a full
store, and is left to refuse what it cannot hold as the bytes arrive.

`MAX_OBJECT_DATA` remains, and bounds what one object may carry over the wire
and in an image. It is no longer what an object costs a device to keep.

## Where a radio keeps its configuration

A radio retains one canonical configuration image in the external serial memory
it already carries, not in the internal flash which holds its firmware. ADR-060
records why, and `EVID-K1-061` and `EVID-K1-062` record the device and the
observed retention.

`radio-eeprom` drives that memory and bounds every access twice, by the device
capacity and by the sector-aligned region the caller claimed. A region below a
fixed bound is refused outright, so an AFIK write cannot reach the channels,
settings, or calibration the radio's own firmware keeps in the bottom of the
same device. A write erases the whole claimed region before programming, so a
shorter configuration never leaves the tail of an older one to be read back
beside it.

The K1 image claims one four-kilobyte region at one megabyte. What it declares
in the capability profile is the smaller number that actually binds: the 1,264
packed bytes its store holds, which with the image header is the 1,280 bytes it
retains. A host can therefore report how much room a project leaves, and be
refused for the right reason.

The store keeps its objects in canonical `(kind, ID)` order, so a listing is a
page of that order rather than a sorted copy of every descriptor a device
holds, and a retained image needs no key index to write.

## Generated banks: a shared core and a per-encoding tail

A generated bank declares its encoding family and is stored at that family's
own length. The 56-byte core every family carries is:

```text
format version : u8  (`4`)
plan encoding  : u8  (`0` linear simplex, `1` linear fixed offset)
bank ID        : u16 little endian
name length    : u8
name bytes     : fixed 16-byte field
base Hz        : u32 little endian
spacing Hz     : u32 little endian
channel count  : u16 little endian
TX class       : u8
RX tone kind   : u8  (0 none, 1 CTCSS, 2 DCS, 3 DCS inverted)
RX tone value  : u16 little endian (CTCSS tenths of a hertz, or DCS octal code)
TX tone kind   : u8
TX tone value  : u16 little endian
modulation     : u8
bandwidth      : u8
power level    : u8
step Hz        : u32 little endian
squelch level  : u8
channel flags  : u8
designator len : u8
designator     : fixed 4-byte field
first number   : u16 little endian
calling index  : u16 little endian (`0xFFFF` marks no calling channel)
```

`LinearSimplex` adds nothing, so a simplex band is 56 bytes.
`LinearFixedOffset` adds four bytes of signed transmit offset, so a repeater
sub-band is 60. Both were 59 in version 3, where every plan paid for an offset
and the family was inferred from whether that offset was zero. The family is
now what the plan says it is, so a repeater sub-band parked at a zero offset
stays a repeater sub-band across a write and a read-back. Encodings which are
declared but not implemented are refused by name rather than given a length.

The fields from the RX tone to the channel flags are the `ChannelTemplate`
every channel of the plan shares. They are what makes the plan a complete
channel source rather than a list of frequencies: 56 bytes hold a whole bank,
against 42 bytes for each explicit channel record. Earlier versions are
rejected rather than reinterpreted.

## Canonical configuration image

Work Package 4 adds an offline container for one complete logical object set.
The container is not an on-flash layout and makes no power-loss or physical
storage claim. Encoding is allocation-free: callers provide both the canonical
object slice and the destination byte buffer.

The 16-byte image header is:

```text
magic                 : 4 bytes (`AFIK`)
image version         : u8 (`1`)
object format version : u8 (`4`)
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
validation. A device restores an image by staging its objects through the
ordinary transactional path, so an image which no longer fits the store leaves
the running configuration untouched.

## Explicit channels, named banks, and radio configuration

Work Package 26 adds three object kinds beside the generated bank. Each is a
fixed-size payload, revalidated field by field on decode, and ordered
canonically by `(kind, id)`.

Object kinds are `1` generated bank, `2` channel, `3` channel bank, and `4`
radio configuration. The radio configuration is a singleton at ID `0`.

Channel payload version 2 is 42 bytes:

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
