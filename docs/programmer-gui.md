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

Start-up connection is optional. `--sim` connects a simulated radio,
`--device auto` connects the one detected USB serial device, `--device PATH`
connects an explicit port, and `--project FILE` opens a project file kept
elsewhere. `--baud` defaults to 38400 and accepts only the supported rates.
`--device` cannot be combined with `--sim`.

## Projects

Projects are kept by name in `$XDG_DATA_HOME/afik/projects`
(`~/.local/share/afik/projects`), one `NAME.afik` file each, and chosen from
the Project drop-down; no path is typed. The project opened last is recorded
in `$XDG_STATE_HOME/afik/last-project` and opened again at start-up. With no
projects yet, a project named `default` is opened, and its file is created on
the first Save. **New project** creates an empty project with the typed name
and opens it. A project with unsaved changes is never switched away from:
Save it or Revert it first.

A project file is the canonical format 4 object set, which every front end
reads. The editor also opens a format 5 radio image, and an older project
whose channels are in several banks, or in none, is grouped so each channel
is in one bank (its first, or a last bank named `Unbanked`).

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

A set is a starting point, not an authority. Applying one replaces every
bank, says how many it replaced, and asks the operator to confirm
every frequency against their own national band plan.
`tool/example-pmr-amateur-plan.sh` builds the same plan from the CLI, either to a
file or straight to a radio.

## Model boundary

Everything the editor decides lives in the library: `model` holds the editable
drafts and their validation, `session` holds the connection to a radio, and
`flash` holds the guarded firmware and EEPROM operations. The `app` module only
draws those decisions, so each is covered by host tests without a display.

Operator input is kept as drafts, including partially typed text. Validation is
the only path from a draft to a typed record, so an invalid field can never
reach a canonical image, a device transaction, or a radio. Failures are
reported with their row, field, and reason. Frequencies are parsed as exact
integer hertz from up to six decimal places of megahertz, never through
floating point.

## Tabs

- **Banks:** the project as a radio holds it: up to sixteen banks in order,
  each either a list of channels or a generated plan. Every channel is in
  exactly one bank, the one it is listed under, because the radio stores each
  channel in its bank (storage format 5, `ADR-100`). A bank's place in the
  list is its identifier and the order the radio shows it in; Move up and
  Move down change it. Banks open collapsed, showing only their name and
  summary; clicking the name opens one, a newly added bank opens for editing,
  and Delete on the header removes a bank with its channels (Revert undoes it
  until the project is saved). A channel bank carries a name and the scan flag, and
  its channels carry name, receive and transmit frequency, tones, modulation,
  bandwidth, power, step, squelch, flags and transmit class; channels are
  numbered by the radio from their position. A generated plan stores a base
  frequency, spacing, channel count, naming, transmit class and the
  per-channel template its channels share, and expands, in place, into the
  channels the radio will build from it. The tab says how many channels the
  radio can select and how much of its 7,424-byte image region
  (`0x0000..0x1D00`) the compiled image takes.
- **Radio:** squelch, backlight, scan resume mode and timings, dual watch,
  battery save, and the global behaviour flags.
- **Device:** detect and select a serial device, choose the baud, connect or
  disconnect it, connect the simulated radio, read the project back from the
  radio, and write it. Writing compiles the format 5 image, writes only the
  eight-byte blocks which differ into the radio's memory over the stock
  protocol, reads each back, then reads the whole image back
  (`radio_flasher::write_afik_image`). Only AFIK `7.0.17` or later is written
  to; stock firmware keeps its own settings where the image goes. The radio
  checks the image again a second after the last write and runs no channels
  from one which fails.
- **Flash:** the guided flash flow described below.

## Logging

Every failure shown on a status line is also logged with what was being done
(the device, operation, or file), refused requests as warnings with each field
that failed, and connections, writes, reads, backups and flashes as one info
line each. Lines go to stderr and are appended to
`$XDG_STATE_HOME/afik/studio.log` (`~/.local/state/afik/studio.log`), which is
moved to `studio.log.1` at start-up once it passes 1 MiB. `AFIK_LOG` sets the
level (`off`, `error`, `warn`, `info` by default, `debug`, `trace`); other
crates such as the windowing stack log only warnings below `debug`.

## Flashing

The Flash tab is a guided flow which does as much as it safely can by itself
and falls back to the operator wherever it cannot decide.

1. **Radio.** USB serial devices are scanned when the tab opens (and every few
   seconds while none is found) and the first is selected; another can be
   picked or a path typed. The radio is identified without writing to a
   bootloader: the tab listens for a bootloader beacon first and only sends the
   read-only hello when none arrives. A beacon names the family, K5/K6 (`2.*`)
   or K1 (`7.03.*`); running firmware names it from its version where it can
   (`AFIK-K5-`, `AFIK-K1-`, stock `k5_`), and otherwise the operator chooses.
   Until the radio is in flashing mode the tab says how to get there and checks
   again every few seconds. The family can always be overridden.
2. **EEPROM backup.** Backups are kept per family under
   `$XDG_DATA_HOME/afik/eeprom/<family>/` (`~/.local/share/afik/eeprom/...`);
   those taken earlier directly in `eeprom/` count for the K5/K6. With none
   kept, the tab asks for one before the first flash and offers **Back up now**
   when the running firmware can serve it (stock, or AFIK-K5 7.0.6 or later),
   but never insists: **Skip** goes on without one for the session, and the
   flash button then says it is flashing without a backup.
3. **Image.** The family's packaged image is chosen: the workspace build output
   (`tool/build-k5.sh --release` or `tool/build-k1.sh --release`) first, then
   `$XDG_DATA_HOME/afik/images/`. Typing a path overrides it; **Use packaged
   image** goes back. The size and CRC-32 are shown.
4. **Flash.** Enabled once the radio is in flashing mode and the image exists.
   It is the image-only write of `afik-flasher flash-afik` (`ADR-098`): the
   worker reads the beacon again, refuses an image built for the other radio
   before any page is sent, and writes only the application pages. Progress is
   shown per page; afterwards the tab identifies the radio again.

Only one worker ever has the port: a backup asked for during a check starts
when the check ends. **Advanced** keeps the K1 recovery write, the
recovery-gated K5 write and a backup to a chosen file, with every typed
confirmation and operator-entered CRC-32 they require; a CRC-32 that is not hex
is refused before the port is opened. Transaction identifiers are generated,
never typed. There is no firmware backup, and there cannot be one: the
bootloader protocols carry no flash-read command. See `docs/k5-flashing.md` and
`docs/k1-bring-up.md` for what each guard means and `RISK-029` for the editor's
accepted local-tool exposure.
