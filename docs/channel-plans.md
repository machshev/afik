# Channel plans

The first implemented encoding is `LinearSimplex`. It stores a bank ID,
bounded printable name, base frequency, positive spacing, channel count,
trusted TX class, and one `ChannelTemplate`. Construction checks the final
generated frequency so every valid index can be expanded without overflow.

Expansion is lazy: requesting channel `n` performs checked
`base + spacing * n` arithmetic and returns one `ActiveChannel`. Scanning can
therefore iterate a generated bank without allocating or decoding a flat
channel list.

## The channelised space-saving model

A plan is not a shorthand for channels an operator must also store. It is the
stored form: one object of `GENERATED_BANK_ENCODED_LEN` bytes holds a whole
bank however many channels it contains, against 42 bytes for each explicit
channel record.

`GeneratedBank::channel_record(index)` expands one complete `ChannelRecord`, so
an expanded channel is indistinguishable from a stored one at the point of use.
It carries:

- the plan's `ChannelTemplate` — tones, modulation, bandwidth, power, manual
  step, squelch, and behaviour flags, stored once for the whole bank;
- a derived name, the plan name truncated so the one-based position always fits
  the twelve-character field, for example `PMR446 01`;
- membership of the plan's own bank and no other, so a bank filter selects
  exactly the plan;
- an identifier from the reserved range at or above `GENERATED_CHANNEL_ID_BASE`
  (`0x8000`), packing the bank identifier and the index.

`ChannelRecord::new` refuses that reserved range, so only expansion mints those
identifiers and a stored channel can never collide with an expanded one. One
plan holds at most `MAX_GENERATED_CHANNELS` channels, which is what the
identifier packing bounds.

Work Package 7 adds `radio-channel-control` over this lazy expansion. It checks
initial and manual indexes, wraps next/previous navigation at exact bank bounds,
and emits at most one `ChannelActivation` per selection or scan advance. The
controller holds one copied `GeneratedBank`; it does not materialize channels.

Scanning owns no clock. `ScanConfig` contains non-zero integer dwell and hold
durations selected by the application. Starting a scan returns a fresh opaque
`TimerToken`; only an expiry carrying the currently armed token can advance or
rearm state. Old, replaced, and cancelled tokens are harmless. Open squelch
starts or restarts hold; an expiry while the latest sample remains open rearms
hold without retuning, while a closed-squelch hold expiry advances once.

Manual selection stops scanning and cancels its timer. Controller-level TX
authorization is unavailable in every scanning phase. Selected state asks the
central `TxPolicy` for the current channel class and returns the exact channel
paired with the borrowed-driver capability; it never constructs authority
itself.

The protocol capability bit for an encoding is `1 << encoding_discriminant`.
The remaining declared encodings are model vocabulary only and cannot yet be
compiled or installed.
