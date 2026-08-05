# TX policy

`radio-tx-policy` is the sole safe constructor of `TxAuthorisation`. The token
has no public constructor. Hardware transmit interfaces must require a borrowed
token before enabling modulation or PA.

The default policy enables no class. `Never` is not authorisable under any
configuration. Persisted permissions contain a version, bitset, inverted
bitset, generation, and CRC. Version, inversion, reserved-bit, or CRC failure
constructs the default-deny policy.

The first milestone has no menu or TX driver. Later serial configuration must
remain unable to write the permission object; the initial mutation path will be
the hidden boot-time radio menu only.
