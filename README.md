# AFIK

A ground-up modular Rust radio firmware and programming platform, initially
targeting the DP32G030-based Quansheng UV-K5 family.

The first eleven architecture work packages are complete. They provide
hardware-independent domain types, channel plans, safe TX policy, protocol
framing, transactional storage, canonical logical images, a library-first
programmer, deterministic host simulation, the complete protocol command/error
matrix, and an evidence-backed minimal DP32G030 Rust image with a Renode
reset-path proof. They also provide an allocation-free boot-only TX-permission
UI, an evidence-bounded post-initialization BK4819 command driver, explicit-input
channel activation/scanning, and complete CLI and local-GUI programmer front
ends. Frequency Copy research defines a receive-only candidate and
experiment-gated design while deferring unverified BK4819 scan commands. No
target hardware-register adapter exists yet. Work Package 12 is adding a
recovery-gated UV-K5 V1 image and host flashing boundary while physical proof
is still pending. Display, keypad, timer, bus, board-RF, and on-air hardware
remain unimplemented.

## Programmer CLI

The CLI exposes the complete currently supported programmer surface over a
fresh deterministic simulator or an explicit Linux serial path and baud:

```sh
nix develop
cargo run --package radio-programmer-cli --bin afik-programmer -- --help
cargo run --package radio-programmer-cli --bin afik-programmer -- --sim info
cargo run --package radio-programmer-cli --bin afik-programmer -- \
  --sim write --bank '1:PMR446:446006250:12500:16:licence-free'
```

`compile`, `write`, `backup`, and `restore` use negotiated capabilities,
canonical images, validated transactions, and read-back verification from the
programmer library. Serial use requires both `--device PATH` and `--baud BAUD`;
no physical device/baud default or interoperability claim exists yet.

## Local programmer GUI

The GUI retains one simulator or explicit serial session and serves responsive
embedded assets only on a loopback IP address:

```sh
cargo run --package radio-programmer-gui --bin afik-programmer-gui -- --sim
```

It exposes the same negotiated capability, object, compile, verified write,
backup, and restore workflows as the programmer library. Mutation requires a
per-process token plus explicit replacement confirmation. This reduces
accidental local mutation but is not authentication; the GUI must not be bound
remotely or treated as a shared service. See `docs/programmer-gui.md`.

## Native editor and flashing front end

`afik-studio` is a native cross-platform desktop editor for channels, banks,
the global radio configuration, and guarded firmware and EEPROM operations:

```sh
cargo run --package radio-programmer-gui-native --bin afik-studio -- --help
cargo run --package radio-programmer-gui-native --bin afik-studio -- --sim
```

Operator input is validated before it can reach a canonical image, a device
transaction, or a radio. Configuration writes use the programmer library's
transactional write with read-back verification, and firmware writes reuse the
recovery-gated flasher workflows with every confirmation gate intact. The
editor is a local tool: it opens no network socket and holds no
authentication. See `docs/programmer-gui-native.md`.

## Channels, banks, and the receive path

Configuration storage holds explicit channel records, named banks, generated
plans, and one global radio configuration. Bank membership is a mask on the
channel, so a channel can belong to several of the sixteen addressable banks;
see `docs/storage-format.md`.

A generated plan is the channelised space-saving model: one 46-byte object holds
a bank of any size, carrying the arithmetic and the per-channel template every
channel of the bank shares, against 42 bytes for each explicit channel record.
A radio expands it into complete channel records on demand, so selection, bank
filtering, and scanning treat an expanded channel exactly like a stored one, and
the editor shows the channels a plan becomes before it is written. See
`docs/channel-plans.md`.

The receive path programs the BK4819 for FM, AM, and USB with filter
bandwidth, AGC, calibration-supplied squelch thresholds, CTCSS and CDCSS
decoding, RF filter path selection, and audio routing, and meters RSSI, glitch,
noise, carrier squelch, and tone status. Above it, a bounded controller
provides banked memory and VFO tuning, monitor, tone-aware audio gating,
scanning with skip and three resume modes, and dual watch. See
`docs/bk4819.md` and `docs/receive-control.md`. Register values and the K1
three-wire pinout come from the pinned reference firmware recorded in
`docs/hardware-evidence.md`; no AFIK receive register has yet been written on
hardware.

## UV-K5 V1 recovery and firmware flashing

`afik-flasher` is separate from the AFIK runtime configuration programmer. It
auto-selects exactly one USB serial candidate, classifies the bootloader from
its validated beacon, and currently supports K5 V1 recovery plus the pinned K1
recovery protocol:

```sh
cargo run --package radio-flasher-cli --bin afik-flasher -- identify
cargo run --package radio-flasher-cli --bin afik-flasher -- \
  --device /dev/ttyUSB0 identify
```

Zero or multiple USB candidates fail closed; a beacon proves only the protocol
family, not the physical board or MCU. `afik-k5` remains as the explicit-device
K5 compatibility workflow and supports an inspected UV-K5 V1 fitted with
DP32G030 and the stock version-2 bootloader:

```sh
cargo run --package radio-flasher-cli --bin afik-k5 -- --help
cargo run --package radio-flasher-cli --bin afik-k5 -- \
  inspect target/thumbv6m-none-eabi/debug/radio-firmware-dp32g030-k5-v1.raw
```

The hardware workflow requires a complete read-only EEPROM backup, a
vector-valid known-good raw recovery image, a same-unit recovery rehearsal,
and exact destructive confirmations. It never writes EEPROM or the preserved
stock bootloader and supports no V2/V3 radio or bootloader v5. See
`docs/k5-flashing.md` before connecting hardware. No physical flash or target
boot has yet been validated.

## Host checks

```sh
nix develop
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Minimal DP32G030 target checks

```sh
nix develop
tool/build-dp32g030.sh
tool/verify-dp32g030-image.sh
tool/package-k5-v1-image.sh
tool/test-k5-v1-package.sh
tool/test-renode.sh
```

The target build uses the locked Nix compiler and standard-library sources. It
creates a statically checked 60 KiB V1 application package but does not access
hardware or prove physical boot.

With direnv installed, run `direnv allow` once and the same development shell
loads automatically on entry. `flake.lock` pins the Nix package set; the CI
toolchain also checks the declared Rust 1.86 minimum.

See `STATUS.md` for the current work package and verified commands,
`TASKS.md` for bounded work, and `docs/architecture.md` for crate boundaries.
