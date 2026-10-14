# Native programmer and flashing editor

`afik-studio` is a native cross-platform editor for channels, banks, the global
radio configuration, and guarded firmware and EEPROM operations. It is built on
`eframe`/`egui` and runs as a local desktop application; it opens no network
socket and is not a service.

Run it from the pinned environment. `winit` and `glutin` load the window-system
and GL libraries at run time, so the development shell puts Wayland, X11,
`libxkbcommon`, and `libGL` on `LD_LIBRARY_PATH`; outside it the editor exits
with a `winit EventLoopError` before drawing anything.

```sh
nix develop path:. -c \
  cargo run --package radio-programmer-gui --bin afik-studio -- --help
cargo run --package radio-programmer-gui --bin afik-studio -- --sim
cargo run --package radio-programmer-gui --bin afik-studio -- --device auto
cargo run --package radio-programmer-gui --bin afik-studio -- \
  --project plan.afik
```

Start-up connection is optional. `--sim` connects the deterministic simulator,
`--device auto` connects the one detected USB serial device, `--device PATH`
connects an explicit port, and `--project FILE` loads a canonical AFIK
configuration image. `--baud` defaults to 38400 and accepts only the supported
rates. `--device` cannot be combined with `--sim`.

## Device detection

The editor detects USB serial devices at start-up and whenever Detect is
pressed, using the same discovery the flasher CLI uses, and applies the same
fail-closed rule: one candidate becomes the selection, several are listed for
the operator to choose from, and none leaves the manually entered path in
charge. A manual path always overrides detection, so an unusual port stays
reachable. Detection opens nothing; connecting is a separate operator action,
and no write ever picks a device for itself. The flash tab detects the same way
but never preselects a flashing target, however few candidates there are.

## Default sets

The editor carries default channel sets so a first plan is not typed by hand: one
UK and EU simplex set of PMR446 plus 2 m and 70 cm amateur FM simplex, twelve
channels in three named banks, which is exactly what the K1 receive image holds;
and one PMR446 generated plan, which is one stored object a target expands into
sixteen channels.

A set is a starting point, not an authority. Applying one replaces every channel
and bank row, says how many rows it replaced, and asks the operator to confirm
every frequency against their own national band plan.
`tool/example-pmr-amateur-plan.sh` builds the same plan from the CLI, either to a
file or straight to a radio.

## Model boundary

Everything the editor decides lives in the library: `model` holds the editable
drafts and their validation, `session` holds the programmer connection, and
`flash` holds the guarded firmware and EEPROM operations. The `app` module only
draws those decisions, so each is covered by host tests without a display.

Operator input is kept as drafts, including partially typed text. Validation is
the only path from a draft to a typed record, so an invalid field can never
reach a canonical image, a device transaction, or a radio. Failures are
reported with their row, field, and reason. Frequencies are parsed as exact
integer hertz from up to six decimal places of megahertz, never through
floating point.

## Tabs

- **Channels:** identifier, name, receive and transmit frequency, receive and
  transmit tone, modulation, bandwidth, power, step, squelch, per-channel flags,
  bank membership, and transmit class. Rows collapse, can be duplicated, and
  name the bank each membership checkbox joins. Each row is one stored object;
  identifiers stop below the range reserved for channels a radio expands from a
  plan.
- **Banks:** the sixteen addressable bank identifiers. A row is either a named
  bank, which groups the channel rows claiming membership of it and carries the
  scan flag, or a generated plan, which stores a base frequency, channel
  spacing, channel count, transmit class, and the per-channel template every
  channel of the plan shares — tones, modulation, bandwidth, power, step,
  squelch, and flags — edited once for the whole bank. A generated row reports
  the span it covers and expands, in place, into the channels the radio will
  build from it: the same names, order, and frequencies the radio's own channel
  list shows, with what the plan costs in stored bytes against what those
  channels would have cost. The two kinds are separate stored objects, so one
  identifier can hold one of each; a channel row cannot join a plan's bank,
  because the plan already owns every channel in it. A target which advertises
  no plan encoding is named as such before a write is attempted.

Both tabs carry the same summary line: how many channels the radio can select,
how many of them are stored against expanded, the object and byte cost, and the
bytes the plans saved. It counts only rows which validate, so it always
describes a configuration which could be written.
- **Radio:** squelch, backlight, scan resume mode and timings, dual watch,
  battery save, and the global behaviour flags.
- **Device:** detect and select a serial device, choose the baud, connect or
  disconnect it, connect the simulator, refresh the object
  listing, read the project back from the radio, and write it. Writing compiles
  against the negotiated target capabilities and uses the programmer library's
  transactional write with read-back verification.
- **Flash:** firmware and EEPROM operations.

Project files are canonical AFIK configuration images, the same format the CLI
and the loopback GUI produce, so a project saved here can be restored by any
front end.

## Logging

Every failure shown on a status line is also logged with what was being done
(the device, operation, or file), refused requests as warnings with each field
that failed, and connections, writes, reads, backups and flashes as one info
line each. Lines go to stderr and are appended to
`$XDG_STATE_HOME/afik/studio.log` (`~/.local/state/afik/studio.log`), which is
moved to `studio.log.1` at start-up once it passes 1 MiB. `AFIK_LOG` sets the
level (`off`, `error`, `warn`, `info` by default, `debug`, `trace`); other
crates such as the windowing stack log only warnings below `debug`.

## Flashing boundary

Firmware and EEPROM operations reuse the `radio-flasher` workflows. The editor
collects input, validates the request before opening the serial device, runs the
operation on a worker thread, and reports page acknowledgements as they arrive.

There is no firmware backup, and there cannot be one: the bootloader protocols
carry no flash-read command, so a page acknowledgement is the only evidence a
write produced.

**Flash AFIK application** is the default write and matches
`afik-flasher flash-afik` (`ADR-098`). It takes the image and nothing else: when
the write starts it reads the bootloader beacon, refuses an image built for the
other radio before any page is sent, and writes only the application pages to a
K5 V1 (`2.*`) or K1 (`7.03.*`). An image that does not boot is recovered by
writing another one. The editor shows the image's size and CRC-32 beside it.

**Flash K1 recovery image** and **Flash K5 V1 application (recovery-gated)**
keep their typed confirmations and the operator-entered image CRC-32; the K5
form also needs the retained recovery image, EEPROM backup and negotiated
firmware version, each shown with its digest. **Identify radio** reads the
bootloader version read-only and fills it in for the K1 recovery write. A CRC-32
that is not hex is refused before the serial device is opened. The per-run
transaction identifier is generated, never typed, so one run's acknowledgements
cannot be mistaken for another's.

The EEPROM backup is read-only and refuses to overwrite an existing file. Its
output defaults to a timestamped file under `$XDG_DATA_HOME/afik/eeprom/`
(`~/.local/share/afik/eeprom/` without it), created on first use, and
**New default name** restores that default after an override. A finished
backup becomes the retained EEPROM backup the recovery-gated K5 write offers,
which otherwise starts as the newest backup in that directory. See
`docs/k5-flashing.md` and `docs/k1-bring-up.md` for what each guard means and
`RISK-029` for the editor's accepted local-tool exposure.
