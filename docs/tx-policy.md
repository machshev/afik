# TX policy

`radio-tx-policy` is the sole safe constructor of `TxAuthorisation`. The token
has no public constructor. Hardware transmit interfaces must require a borrowed
token before enabling modulation or PA. `TXAUTH-053` replaces the historical
class-only token with authority for one complete immutable `TxRequest`.

The operator programs the ranges a radio may transmit in from the studio
(`ADR-101`). A `TxRange` names a service class, an inclusive span, its own
power ceiling in milliwatts, whether wide FM is allowed and an optional channel
raster (PMR446: 12.5 kHz, 500 mW, narrow). `TxPolicy::new` refuses a malformed
range (class `Never`, an empty or zero span, zero power) or more than
`MAX_TX_RANGES` (16); the default policy has no range and permits nothing.
The frequency decides, so a programmed channel and the VFO are checked alike;
the token reports the class of the permitting range. Channels carry no class.

The radio lowers the selected power to the highest level whose nominal
output the ranges at the frequency allow (`TxPolicy::max_mw_at`,
`transmit::level_within`); the policy itself still refuses, never clamps, a
request above a ceiling.

The handset can switch programmed classes off and on again
(`TxPolicy::restricted` with a `ClassSet`). It only drops ranges, so it can
never permit more than the studio programmed. The built-in regional snapshot,
grants, attestations and the hidden boot permission editor are gone.

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

`TxPolicy::authorise(&request)` combines the programmed ranges with healthy
selected state, an established configuration identity, exact RX/TX frequencies,
analog FM, bandwidth, conducted milliwatts, a positive timeout no longer than
`MAX_TX_TIMEOUT_MS`, and independent hardware/calibration envelopes. Every
programmed range containing the frequency must permit it, so a permissive
overlap cannot mask a strict one. Power is rejected rather than silently
clamped.

Each envelope has at most four FM ranges, positive frequency resolution, and
non-zero evidence and target/unit identities. Both must match the requested
unit; neither absence nor synthetic host fixtures establish target limits.

The token borrows both policy and request. Rust prevents mutation/replacement
of those borrowed values while the token is used. `matches` compares every
current request field, including instance, retained generation, selection
revision, unit, and evidence. Volatile generation zero is valid, but
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
