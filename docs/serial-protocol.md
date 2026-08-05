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

Work Package 1 services are Device Info and Configuration. It implements
HELLO, GET_CAPABILITIES, READ_OBJECT, BEGIN_TRANSACTION, WRITE_OBJECT,
VALIDATE_TRANSACTION, COMMIT_TRANSACTION, and ABORT_TRANSACTION.

Responses retain the request sequence, set the response flag, and use either
the request command or the error command. Errors carry a stable one-byte code.
Unknown or corrupt packets are discarded; the delimiter provides stream
resynchronisation.
