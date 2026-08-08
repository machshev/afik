# Channel plans

Two encodings are implemented, and one `GeneratedBank` expresses both. It stores
a bank ID, bounded printable name, channel-name designator and first number,
base frequency, positive spacing, channel count, trusted TX class, one
`ChannelTemplate`, an optional calling-channel index, and a signed transmit
offset. Construction checks the final generated frequency and both ends of the
transmit range, so every valid index can be expanded without overflow.

`encoding()` follows the offset rather than being stored beside it: zero is
`LinearSimplex`, non-zero is `LinearFixedOffset`. A plan therefore cannot claim
an encoding its own contents contradict, and the capability bit a host
negotiates is always the one the plan needs.

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
- a derived name, the plan's designator followed by the channel's own number,
  which is what an operator matches against a published band plan: a plan
  designated `S` numbered from 8 expands to `S8` through `S23`. The plan name
  stays the editor's label for the bank and is never truncated into a channel.
  A plan given no designator derives one from the leading word of its name, so
  `PMR446` expands to `PMR 1` upwards;
- `ChannelFlags::CALLING` on the one index `calling_index` names, whose derived
  name gains a `CALL` suffix, so `S20` becomes `S20 CALL`. The flag means the
  same on an explicit record, so a radio implements go-to-calling and the
  default dual-watch partner once rather than once per channel kind;
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

## What bounds a plan

Nothing is materialised, so nothing about a plan's size costs memory: a plan of
two thousand channels and a plan of ten are the same object. The bounds are
therefore only these, and no image adds an arbitrary one on top:

- `MAX_GENERATED_CHANNELS` per plan, which is the eleven bits the identifier
  packing has for an index;
- the `u16` selection index space, which `ProgrammedMemory::install` checks
  across every stored channel and every installed plan;
- storage, which each device advertises as `configuration_bytes`, `max_objects`,
  and its per-kind object limits.

An expanded identifier packs its bank and index, so `generated_channel_parts`
resolves one arithmetically and `ChannelSource::member_at` answers bank
membership without building a record at all. A filtered view over a band-sized
plan therefore costs arithmetic per channel rather than a full expansion per
channel, which is what makes a bound on expanded channels unnecessary as well as
unwanted.

## Why the presets are plans

Every default set the editor ships is arithmetic, so every one of them is stored
as a plan rather than as channels: PMR446 (16 channels, 12.5 kHz from
446.00625), UK 2 m simplex (S8 to S23, calling S20), UK 70 cm simplex (SU16 to
SU23, calling SU20), UK 2 m repeaters (RV48 upwards, inputs 600 kHz below), and
the 25 kHz civil airband (760 AM channels, classified `Never`). The simplex set
is forty channels in three stored objects against twelve channels in fifteen.

Marine VHF and UK business radio are deliberately absent: marine numbering is
two interleaved runs with duplex ship/shore pairs, and business allocations are
spot frequencies. Neither is arithmetic, so both need a table encoding rather
than a linear plan, and inventing one would be worse than omitting them.

The protocol capability bit for an encoding is `1 << encoding_discriminant`.
The remaining declared encodings are model vocabulary only and cannot yet be
compiled or installed.
