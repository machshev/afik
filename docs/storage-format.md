# Storage object model

Device configuration is a bounded set of typed objects. Each object has a
stable kind, numeric ID, encoded length, and encoded bytes. The first supported
configuration object is a generated channel bank.

Transactions clone the active object set into a candidate, apply object writes
to that candidate, validate the complete candidate, and atomically replace the
active snapshot on commit. Abort or any validation error leaves the active
snapshot unchanged. This is logical atomicity; physical power-loss durability
is deferred and tracked as `RISK-004`.

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
