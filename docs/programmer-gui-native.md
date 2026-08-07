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
  cargo run --package radio-programmer-gui-native --bin afik-studio -- --help
cargo run --package radio-programmer-gui-native --bin afik-studio -- --sim
cargo run --package radio-programmer-gui-native --bin afik-studio -- --device auto
cargo run --package radio-programmer-gui-native --bin afik-studio -- \
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
and one compact PMR446 generated plan for a target which expands plans.

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
  name the bank each membership checkbox joins.
- **Banks:** the sixteen addressable bank identifiers. A row is either a named
  bank, which groups the channel rows claiming membership of it and carries the
  scan flag, or a compact generated plan, which stores a base frequency,
  channel spacing, channel count, and transmit class the radio expands for
  itself. A generated row reports the span it covers. The two kinds are separate
  stored objects, so one identifier can hold one of each. A target which
  advertises no compact plan encoding is named as such before a write is
  attempted; the K1 receive image is one, so plans can be saved to a file but
  not written to it.
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

## Flashing boundary

Firmware and EEPROM operations reuse the recovery-gated `radio-flasher`
workflows unchanged. The editor collects input, validates the request before
opening the serial device, runs the operation on a worker thread, and reports
page acknowledgements as they arrive. It adds no shortcut and weakens no gate.

There is no firmware backup, and there cannot be one: the bootloader protocols
this crate drives carry no flash-read command, so a page acknowledgement is the
only evidence a write produced. What protects a unit is the retained known-good
recovery image and the retained EEPROM backup. Every write path requires them and
reports each file's size and CRC-32 before starting, because that digest is the
only evidence available that the files on disk are the pair kept for that unit.

**Identify radio** classifies the bootloader read-only and fills in the version
the write then checks against the radio, so a mistyped version or a K5 in
bootloader mode stops the run before any page is written. A K1 application write
additionally requires the retained EEPROM backup and an image CRC-32
confirmation, exactly as the flasher CLI's `flash-afik-k1` does. The per-run
transaction identifier is generated rather than typed: the bootloader ties every
acknowledgement to it, so reuse would make one run's acknowledgements
indistinguishable from another's.

The EEPROM backup is read-only and refuses to overwrite an existing file. Every
firmware write still requires its exact confirmation phrases, a non-zero
transaction identifier, the retained known-good recovery image, and, for the
UV-K5 V1 path, the retained EEPROM backup, the negotiated firmware version, and
the operator-entered image CRC-32. See `docs/k5-flashing.md` and
`docs/k1-bring-up.md` for what each guard means and `RISK-029` for the editor's
accepted local-tool exposure.
