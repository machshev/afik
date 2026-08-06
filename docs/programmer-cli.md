# Programmer CLI

`afik-programmer` is the thin process front end for `radio-programmer`. Run
`afik-programmer --help` for the stable command summary. Operational commands
require exactly one backend:

- `--sim` creates a fresh deterministic `SimDevice` for that invocation.
- `--device PATH --baud BAUD` configures and opens one explicit Linux serial
  path. Supported baud values are listed in help; there is no default.

The commands are:

- `info` negotiates and prints every device capability.
- `list` prints the stable generation-tagged object listing.
- `compile OUTPUT [--force] --bank SPEC...` negotiates capabilities, compiles
  without device mutation, and writes one canonical image.
- `write --bank SPEC...` compiles, transactionally writes, reads the complete
  configuration back, and requires an exact generation/object match.
- `backup OUTPUT [--force]` reads one stable snapshot, validates and reports it
  in the programmer library, and writes its canonical image.
- `restore INPUT` reads at most 8 MiB, fully validates the image against
  negotiated capabilities, transactionally writes it, and requires exact
  read-back.

A generated-bank specification is
`ID:NAME:BASE_HZ:SPACING_HZ:COUNT:TX_CLASS`. All integer and domain constructors
are checked. Duplicate IDs remain a compiler error, so the CLI does not
reimplement stable-object identity rules.

## Files and process behavior

Compile and backup use create-new semantics by default. An existing output is
an operation failure; `--force` is the only path that deliberately truncates
and replaces it. Input reads are capped during streaming as well as by initial
metadata, preventing a growing file from bypassing the 8 MiB bound.

Successful output is stable line-oriented `key=value` text. Exit status `0`
means success, `1` means compiler, image, file, transport, protocol, device, or
verification failure, and `2` means invalid CLI usage. Errors go only to
standard error with an `error:` prefix.

## Serial boundary

The shared host-only `radio-programmer-serial` adapter runs
`stty -F PATH raw -echo min 0 time 1 BAUD`, then opens the path for ordered reads
and writes through `ProtocolTransport`. The CLI and GUI select this same
adapter; neither reimplements serial setup. It contains no unsafe code and adds
no raw-object or raw-memory command. This host adapter does not establish target
pins, boot mode, baud, protocol availability, timeout suitability, recovery, or
physical programming success; those remain open in `RISK-009`.
