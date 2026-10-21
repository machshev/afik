# AFIK

A ground-up Rust firmware and programming platform for the Quansheng UV-K5
(V1, DP32G030) and UV-K1 handhelds. It is not a port of the stock firmware or
its forks: the radio crates are hardware-independent, embedded code is
`no_std` and heap-free, and every transmit goes through one policy crate.

**Try it:** [AFIK Studio in the browser](https://machshev.github.io/afik/)
programs a radio over Web Serial in Chrome, Edge or a recent Firefox. Desktop
builds for Linux, macOS and Windows are on the
[releases page](https://github.com/machshev/afik/releases).

## What works

Both radios run the AFIK application, built from one shared application crate.

- **Receive:** FM, AM and USB through the BK4819 (BK4829 on the K1), with
  CTCSS/DCS, squelch, an S meter, and FM broadcast through the BK1080.
- **Memory:** up to 254 banks, addressed by bank and position, and bank
  groups that set what the radio scans. A generated bank stores a whole
  channel plan in one small object, which the radio expands as it is used.
- **Operation:** memory and VFO tuning, dual watch with two slots on screen,
  duplex channels heard on both sides, and fast memory scan.
- **Transmit:** only inside the TX ranges the project programs (frequency
  span, power ceiling, bandwidth, raster), in channel and VFO mode alike; a
  radio with none programmed does not transmit. On the K5 the hardware
  envelope is 400–470 MHz at up to 500 mW, and 2 m stays refused until its
  bias is calibrated. K1 transmit is implemented and still being bench
  checked.

What has and has not been observed on each radio is recorded per task in
`TASKS.md`, with the hardware facts and their confidence in
`docs/hardware-evidence.md`.

## AFIK Studio

Studio is the editor for projects: channels, banks and groups, the radio
configuration, TX ranges, and starting slots and VFOs. It adds banks from
default sets and from generators near a Maidenhead locator (repeaters,
beacons, gateways, airports, marine channels, satellites), shows them on a
map, writes the compiled image to the radio and reads it back, and tests the
serial link. Its Flash tab detects the radio and writes AFIK firmware.

The project is compiled and validated before it is written; the radio takes
it as memory blocks over the stock programming protocol and validates it
again before using it (`ADR-100`).

```sh
afik-studio            # desktop
afik-studio --sim      # with a simulated radio
afik-studio --help
```

The browser Studio is the same crate built for WebAssembly (`ADR-119`). It
keeps projects in the page's storage. Firmware flashing and the station,
satellite and map downloads are desktop-only for now. See
`docs/programmer-gui.md`.

## Releases and installation

Each `vX.Y.Z` tag publishes the K5 and K1 firmware images and the host tools
(`afik-studio`, `afik-flasher`, `afik-programmer`) for Linux x86_64/aarch64,
macOS Apple Silicon/Intel and Windows x86_64, with `SHA256SUMS`. The macOS
and Windows binaries are unsigned. Serial ports are named as the system
names them: `/dev/ttyUSB0`, `/dev/cu.usbserial-110` or `COM3`.

With Nix, `nix run github:machshev/afik` starts Studio from the pinned
release, and `nix shell github:machshev/afik` puts the tools on the path,
with the firmware images in `share/afik/firmware`.

## Flashing firmware

`afik-flasher` (and Studio's Flash tab) writes AFIK to either radio from its
stock bootloader. It reads the bootloader beacon to tell the K5 V1 `2.*` and
K1 `7.03.*` protocols apart and refuses an image built for the other radio:

```sh
afik-flasher identify
afik-flasher flash-afik [IMAGE]
afik-flasher link-test
```

Without IMAGE it uses the packaged image for the radio that answered. Only
the application region is written; the bootloader and EEPROM are left alone,
so an image that does not boot is recovered by flashing another. The K5's
recovery-gated paths (`flash-afik --recovery`, `restore-eeprom`) need a
complete EEPROM backup, a known-good raw image and exact confirmations. A
bootloader beacon proves only the protocol family, not the board. Read
`docs/k5-flashing.md` before connecting a K5; V2/V3 radios and bootloader v5
are not supported.

## Other tools

`afik-programmer` is the command-line programmer for AFIK's own framed
protocol, usable against the deterministic simulator:

```sh
afik-programmer --sim info
afik-programmer --sim write --bank '1:PMR446:446006250:12500:16'
```

See `docs/programmer-cli.md` and `docs/simulator.md`.

## Development

Everything builds in the pinned Nix development shell (`nix develop`, or
`direnv allow` once). Host checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Firmware images:

```sh
tool/build-k5.sh --release
tool/build-k1.sh --release
tool/test-k1-image.sh
```

A release build verifies the ELF and packages the raw image
(`afik-k5-v1.raw`, `radio-firmware-k1.raw`) beside it, which
`afik-flasher flash-afik` picks up by default. The K5 image is exactly the
60 KiB V1 application region.

The browser Studio:

```sh
cd crates/radio-programmer-gui && trunk serve --release
```

The `pages` workflow publishes it from `main`, and the `release` workflow
builds the release packages from a `v*` tag. After a release, pin it for Nix
with `tool/update-release-nix.sh` and commit `nix/release.json`. Firmware and
tool versions come from the workspace `version` in `Cargo.toml`.

`STATUS.md` holds the current work and the verified commands, `TASKS.md` the
tasks, `DECISIONS.md` the decisions, `RISKS.md` the open risks, and
`docs/architecture.md` the crate boundaries.
