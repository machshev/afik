# TX policy

`radio-tx-policy` is the sole safe constructor of `TxAuthorisation`. The token
has no public constructor. Hardware transmit interfaces must require a borrowed
token before enabling modulation or PA. Each token carries the exact `TxClass`
approved by the policy, so downstream code can reject reuse for another class.

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

Serial configuration remains unable to write a permission object. Work Package
6 adds a post-initialization logical BK4819 driver whose only TX entry method
requires a borrowed token and checks its class against the active channel before
performing any register write. Unknown, faulted, invalid, or mismatched state
denies the TX-mode write. This is not a physical TX adapter, PA controller, or
persistence implementation.

Work Package 7 adds a higher workflow boundary: channel control refuses to ask
for policy authority while scanning. Only selected state can return an
`AuthorisedTransmission`, which pairs the exact active channel with the token
minted for its class. The RF driver still rechecks the class match; the bundle
does not create another token constructor or weaken the sole policy authority.
