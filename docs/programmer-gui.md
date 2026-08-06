# Local programmer GUI

`afik-programmer-gui` is a host-only, single-user web front end over one
persistent `radio-programmer` session. Select exactly one backend:

```text
afik-programmer-gui --sim [--listen LOOPBACK:PORT]
afik-programmer-gui --device PATH --baud BAUD [--listen LOOPBACK:PORT]
```

The default listener is `127.0.0.1:8765`. `--listen` accepts only an explicit
loopback IP socket address, including an IPv6 loopback address. It rejects
wildcard, LAN, hostname, and other non-loopback values. The launcher does not
discover a radio, choose a serial default, or open a browser automatically.

## Supported workflow

The responsive embedded interface displays every negotiated capability, the
active generation, and the complete stable object listing. Generated banks use
the same strict `ID:NAME:BASE_HZ:SPACING_HZ:COUNT:TX_CLASS` fields as the CLI,
one per line, capped at 64 KiB. Domain validation and duplicate/capacity rules
remain in their authoritative libraries.

Compile downloads a canonical image without mutation. Backup reads and
validates the active snapshot before downloading its canonical image. Write and
restore use the programmer-owned transactional, generation-tagged read-back
verification. Restore accepts uploaded bytes capped at 8 MiB; no route accepts
a server filesystem path. One simulator process retains its state across all
requests, unlike the CLI's fresh simulator per command.

## Local service boundary

The dependency-free server handles one request at a time over the persistent
session. Headers are capped at 16 KiB and bodies at 8 MiB; chunked transfer,
ambiguous content lengths, malformed headers, and oversized requests are
rejected. Responses disable caching and MIME sniffing and include a restrictive
same-origin Content Security Policy.

The process obtains a random 256-bit session token from the host and injects it
into the same-origin document. Write and restore require both that token and an
explicit `replace-configuration` confirmation header. The delivered interface
sends the header only after its replacement checkbox is selected. These checks
reduce accidental or cross-origin mutation; they are not authentication. The
service must not be exposed remotely or treated as multi-user. `RISK-010`
records the remaining threat-model boundary.

This GUI proves deterministic simulator workflows and bounded local protocol
handling. It does not prove serial interoperability, target UART behavior,
physical programming success, safe firmware flashing, or RF behavior.
