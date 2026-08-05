# Architecture

The first milestone has this dependency direction:

```text
radio-domain
     ↓
radio-channel-plan
     ↓
radio-storage

radio-tx-policy       radio-protocol
                              ↓
                       radio-programmer
                              ↓
                         radio-sim
```

`radio-domain` owns checked integer radio types and classifications.
`radio-channel-plan` owns bounded generated-bank definitions and expansion.
`radio-storage` owns stable object envelopes and candidate transactions.
`radio-tx-policy` is an independent authority boundary. `radio-protocol` owns
only bytes and wire semantics. `radio-programmer` compiles host projects and
speaks the protocol through a transport. `radio-sim` supplies the deterministic
device and in-memory transport.

No host crate is a dependency of an embedded crate. Board, PAC, HAL, display,
keypad, BK4819, firmware, Renode, CLI, and GUI crates are intentionally deferred.
