# TX policy

`radio-tx-policy` is the sole safe constructor of `TxAuthorisation`. The token
has no public constructor. Hardware transmit interfaces must require a borrowed
token before enabling modulation or PA.

The default policy enables no class. `Never` is not authorisable under any
configuration. Persisted permissions contain a version, bitset, inverted
bitset, generation, and CRC. Version, inversion, reserved-bit, or CRC failure
constructs the default-deny policy.

Work Package 5 adds the hardware-independent hidden boot-time editor as the only
permission mutation workflow. The exact logical `Menu+Back` boot gesture must
be present at boot and released before editing; there is no runtime entry path.
The editor produces a new `StoredPermissions` record on deliberate save but
does not own or mutate the active `TxPolicy` and cannot construct
`TxAuthorisation`. A validated subsequent boot load is required before saved
permissions can authorize anything.

Serial configuration remains unable to write a permission object. There is
still no TX driver or physical persistence implementation.
