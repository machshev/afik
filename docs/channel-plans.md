# Channel plans

The first implemented encoding is `LinearSimplex`. It stores a bank ID,
bounded printable name, base frequency, positive spacing, channel count, and
trusted TX class. Construction checks the final generated frequency so every
valid index can be expanded without overflow.

Expansion is lazy: requesting channel `n` performs checked
`base + spacing * n` arithmetic and returns one `ActiveChannel`. Scanning can
therefore iterate a generated bank without allocating or decoding a flat
channel list.

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
