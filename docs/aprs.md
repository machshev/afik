# APRS complete-frame and discovery boundary

`radio-aprs` owns the hardware-independent receive layer authorized by
`APRS-011`. Hardware evidence and the physical defer verdict live in
`hardware-evidence.md` and `aprs-feasibility.md`.

## Input contract

The parser accepts one complete de-stuffed, octet-aligned AX.25 frame beginning
with the destination address and ending with its two FCS octets. Opening and
closing `0x7E` flags are absent. Carrier detection, audio/FSK demodulation,
symbol timing, NRZI decoding, flag detection, abort handling, and removal of
stuffed zero bits have already happened outside this API.

The input is bounded to:

- destination and source: exactly two seven-octet address subfields;
- path: zero through eight seven-octet APRS digipeater subfields;
- control: exactly `0x03` (UI);
- PID: exactly `0xF0` (no Layer 3);
- information: 1 through 256 octets;
- FCS: two octets, checked over the entire preceding frame.

Addresses use six shifted upper-case alphanumeric characters with trailing
space padding, a four-bit SSID, standard reserved bits, and an extension bit set
only on the final address. The parser preserves the source and each path entry;
it does not repeat, reply, or transmit.

The FCS uses the reflected representation of the ISO 3309/CRC-CCITT polynomial
(`0x8408`), starts at `0xFFFF`, and validates the complete received frame plus
FCS against residue `0xF0B8`. This byte-oriented rule is tested independently
from excluded on-air bit order.

## Supported APRS reports

The first information octet must be `;` for Object or `)` for Item. The initial
implementation accepts the uncompressed position form because it has exact
fixed fields and is the form specified for recommended voice-repeater Objects.
Compressed positions and all other APRS data types return explicit unsupported
errors rather than being guessed.

- Object: fixed nine-character printable case-sensitive name, live `*` or
  killed `_`, seven-character timestamp, position, and comment.
- Item: three through nine printable case-sensitive name, live `!` or killed
  `_`, position, and comment. APRS 1.1 discourages Item use on RF, but receiving
  the defined base format is bounded and does not transmit it.
- Position: validated fixed latitude/table/longitude/symbol fields. Progressive
  latitude spaces record ambiguity; longitude digits remain syntactically
  valid and the latitude ambiguity level applies to both axes as specified.

Only live reports with either a frequency Object name or a leading frequency
comment yield a repeater advertisement. Killed reports yield removal events by
identity and source without needing frequency fields.

## Advertisement fields

The candidate preserves only advertised data:

- report kind, case-sensitive name, and originating AX.25 source/SSID;
- raw validated uncompressed position and ambiguity level;
- advertised repeater output frequency from a frequency Object name, or the
  leading `FFF.FFFMHz`/`FFF.FF MHz` comment frequency;
- optional alternate input/cross-band frequency when both name and comment
  frequencies exist;
- optional advertised CTCSS/DCS token, narrow marker, signed offset in 10 kHz
  units, and nominal range/unit.

The parser never applies a regional “standard offset.” It does not calculate a
transmit frequency, expand truncated CTCSS values into `radio_domain::Tone`,
validate a DCS token against a trusted code table, or treat range as measured.
Malformed recognized fields fail explicitly; unrelated trailing comment text
is ignored.

## Deterministic table

The fixed-capacity table key is `(report kind, case-sensitive name, source)`.
This differs deliberately from APRS's general same-name takeover rule: it keeps
conflicting unauthenticated origins visible instead of allowing one to erase
another. It also agrees with the permanent-frequency-object restriction for
different origins.

Every input carries an explicit monotonic receive time:

- a newer live report replaces the same key;
- an identical report at the same time is unchanged;
- differing data at the same time is a conflict with no mutation;
- an older live or kill report is stale with no mutation;
- a current kill removes only the same key;
- inserting a new key into a full table fails without eviction;
- explicit `expire_before(cutoff)` removes entries older than the cutoff.

There is no wall clock, implicit timeout, allocation, persistence, channel-plan
conversion, or TX path. Tests and the host simulator supply frames and times
directly so identical scripts produce identical events and table state.
