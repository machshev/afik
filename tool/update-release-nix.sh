#!/usr/bin/env bash
# Pins `nix/release.json` to a published GitHub release, taking every asset's
# hash from the release's SHA256SUMS, so the flake's `afik-bin` package
# installs that release's binaries.
#
#   tool/update-release-nix.sh [VERSION]
#
# VERSION defaults to the workspace version in Cargo.toml (ADR-097).
# AFIK_RELEASES overrides the release download base, for trying a local copy.
set -euo pipefail

version="${1:-$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)}"
[[ -n "$version" ]] || { echo "no version given or found in Cargo.toml" >&2; exit 2; }
base="${AFIK_RELEASES:-https://github.com/machshev/afik/releases/download}"
url="$base/v$version/SHA256SUMS"

sums="$(curl --fail --silent --show-error --location "$url")" || {
  echo "could not fetch $url; is v$version published?" >&2
  exit 1
}

hashes="{}"
while read -r hex name; do
  [[ -n "$hex" ]] || continue
  sri="$(nix hash convert --hash-algo sha256 --to sri "$hex")"
  hashes="$(jq --arg name "${name#\*}" --arg sri "$sri" '. + {($name): $sri}' <<<"$hashes")"
done <<<"$sums"

jq -n --arg version "$version" --argjson hashes "$hashes" \
  '{version: $version, hashes: $hashes}' > nix/release.json
echo "pinned nix/release.json to v$version ($(jq length <<<"$hashes") assets)"
