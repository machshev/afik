# Banked receive control

`radio-channel-control::banked` owns receive-side selection and scanning over
explicit channel records. It is `no_std`, allocation-free, has no clock or bus,
and mints no transmit authority. Every input returns a `ReceiveUpdate` holding
the tuning request to apply, whether audio should reach the output, and one
logical timer directive for an external scheduler.

## Memory and bank filtering

`ChannelMemory<CHANNELS>` is a fixed-capacity store kept in channel-identifier
order. Insertion replaces a channel with the same identifier in place and fails
closed when the store is full.

The controller holds an optional bank filter. With a filter set, only channels
whose membership mask contains that bank are selectable, navigable, or
scannable. Setting a filter which selects no channel is refused and leaves the
current selection untouched.

## Modes

`ReceiveMode::Memory` follows the selected banked channel. `ReceiveMode::Vfo`
tunes manually from the settings of the channel that was active when VFO was
entered, moving by that channel's step or jumping to an exact frequency.
Manual tuning past the representable range is refused rather than wrapped.

The resolved `ChannelReceiveSetup` honours the channel's reverse flag, so a
reversed channel receives on its stored transmit frequency.

## Audio gating

An observation carries the carrier squelch result and, for a tone-coded
channel, whether the tone matched. Audio opens when the carrier is open and the
tone matched, or whenever monitoring is on. Monitoring also forces the
effective squelch level to the open level in the emitted setup, so the RF
adapter and the controller agree about what the operator asked for.

## Scanning

Scanning runs over the eligible, non-scan-skipped channels of the active bank
and requires memory mode. The dwell timer advances to the next such channel;
finding a busy channel starts the hold timer. The three resume behaviours from
the stored radio configuration are:

- **after hold:** resume when the hold timer expires;
- **when carrier drops:** hold while busy and resume on the first idle
  observation;
- **stop on signal:** leave scanning immediately and stay on the channel.

Timer tokens are opaque and monotonic. An expiry carrying a stale token changes
nothing, so a late timer from a cancelled phase cannot move the scan.

## Dual watch

Dual watch requires the stored configuration to enable it and one other
eligible channel. The controller alternates between the two channels on each
dwell expiry, and holds on the current channel for as long as audio is open so
an active conversation is never interrupted by the alternation.
