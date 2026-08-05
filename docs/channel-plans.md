# Channel plans

The first implemented encoding is `LinearSimplex`. It stores a bank ID,
bounded printable name, base frequency, positive spacing, channel count, and
trusted TX class. Construction checks the final generated frequency so every
valid index can be expanded without overflow.

Expansion is lazy: requesting channel `n` performs checked
`base + spacing * n` arithmetic and returns one `ActiveChannel`. Scanning can
therefore iterate a generated bank without allocating or decoding a flat
channel list.

The protocol capability bit for an encoding is `1 << encoding_discriminant`.
The remaining declared encodings are model vocabulary only and cannot yet be
compiled or installed.
