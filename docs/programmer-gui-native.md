# Native programmer and flashing editor

`afik-studio` is a native cross-platform editor for channels, banks, the global
radio configuration, and guarded firmware and EEPROM operations. It is built on
`eframe`/`egui` and runs as a local desktop application; it opens no network
socket and is not a service.

```sh
cargo run --package radio-programmer-gui-native --bin afik-studio -- --help
cargo run --package radio-programmer-gui-native --bin afik-studio -- --sim
cargo run --package radio-programmer-gui-native --bin afik-studio -- \
  --project plan.afik
```

Start-up connection is optional. `--sim` connects the deterministic simulator,
`--device PATH --baud BAUD` connects an explicit serial port, and `--project
FILE` loads a canonical AFIK configuration image. `--device` and `--baud` must
be used together and cannot be combined with `--sim`.

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
  bank membership, and transmit class.
- **Banks:** the sixteen addressable banks with names and scan participation.
- **Radio:** squelch, backlight, scan resume mode and timings, dual watch,
  battery save, and the global behaviour flags.
- **Device:** connect the simulator or a serial port, refresh the object
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

The EEPROM backup is read-only and refuses to overwrite an existing file. Every
firmware write still requires its exact confirmation phrases, a non-zero
transaction identifier, the retained known-good recovery image, and, for the
UV-K5 V1 path, the retained EEPROM backup, the negotiated firmware version, and
the operator-entered image CRC-32. See `docs/k5-flashing.md` and
`docs/k1-bring-up.md` for what each guard means and `RISK-029` for the editor's
accepted local-tool exposure.
