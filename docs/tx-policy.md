# TX policy

`radio-tx-policy` is the sole safe constructor of `TxAuthorisation`. The token
has no public constructor. Hardware transmit interfaces must require a borrowed
token before enabling modulation or PA. `TXAUTH-053` replaces the historical
class-only token with authority for one complete immutable `TxRequest`.

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
6 added a post-initialization logical BK4819 driver whose only TX entry method
now requires a borrowed token and compares the complete current request before
performing any register write. Unknown, faulted, invalid, or mismatched state
denies the TX-mode write. This is not a physical TX adapter, PA controller, or
persistence implementation.

Work Package 7 adds a higher workflow boundary: channel control refuses to ask
for policy authority while scanning. Only selected state can return an
`AuthorisedTransmission`, which pairs the exact active channel with the token
minted for the complete request. The RF driver still rechecks that request;
the bundle does not create another token constructor or weaken the sole policy
authority.

## Complete authorization request

`TxPolicy::authorise(&request)` combines class permission with healthy selected
state, an established configuration identity, exact RX/TX frequencies,
modulation, bandwidth, conducted milliwatts, a positive timeout no longer than
60,000 ms, the existing versioned regional rules, operator attestations, and
independent hardware/calibration envelopes. Optional grants also restrict
otherwise built-in rules. Every applicable overlapping range contributes its
strictest power/bandwidth limit; malformed ranges deny regardless of order.
Power is rejected rather than silently clamped.

Each envelope has at most four FM ranges, positive frequency resolution, and
non-zero evidence and target/unit identities. Both must match the requested
unit; neither absence nor synthetic host fixtures establish target limits.
The PMR446 model additionally requires the exact sixteen-channel raster,
simplex RX/TX, and affirmative mobile, non-airborne, and receiver-capable facts.
Equipment eligibility remains a separate operator/adapter assertion.

The token borrows both policy and request. Rust prevents mutation/replacement
of those borrowed values while the token is used. `matches` compares every
current request field, including instance, retained generation, selection
revision, unit, grant, and evidence. Volatile generation zero is valid, but
instance/revision must be non-zero and must never be reused. Adapters must
build the current request from authoritative state immediately before TX;
comparing an old snapshot cannot discover that hardware or external facts
changed. Channel-controller bundles also borrow the selected controller.

`TXCTRL-054` adds the shared [PTT lifecycle](tx-control.md) above this policy.
It rechecks complete authority at each entry effect, measures a deterministic
deadline from the press, and orders PA/TX shutdown before receive restoration.
The simulator records logical preparation and PA state; the chip driver still
models only tuning and mode entry. Physical power application, microphone,
PA/RF switching, and safe target shutdown require independent target evidence.
Neither handset gains a TX path, permission persistence, firmware identity, or
physical compliance claim.
