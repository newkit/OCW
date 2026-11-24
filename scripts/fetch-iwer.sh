#!/usr/bin/env bash
# The WebXR emulator for the browser test of VR (spec §14 Phase 14, 14I W1;
# `make test-web-xr`):
#
#   scripts/fetch-iwer.sh [OUT_DIR]     (default target/web-test)
#
# Copies the Immersive Web Emulation Runtime (iwer, Meta's JavaScript WebXR
# runtime – the core of the Immersive Web Emulator extension 2.x) as
# OUT_DIR/iwer.min.js (UMD: window.IWER) with its licence, and prints the
# file's path. The package is the npm registry's tarball of the pinned
# version, checked against its SHA-256 and kept in target/web-vendor/
# (fetched once). Test-only: never served with the app.
#
# Licence: iwer is MIT (Meta Platforms, Inc.). Version and hash change
# together, here only.
set -euo pipefail
cd "$(dirname "$0")/.."

version=2.7.0
# sha256 of https://registry.npmjs.org/iwer/-/iwer-2.7.0.tgz
sha256=d3c15cccd69399923fb92d4c365f15bb5fa7fbeeda7d2b66621e7f0835845481

out=${1:-target/web-test}
cache=target/web-vendor
tgz=$cache/iwer-$version.tgz
mkdir -p "$cache" "$out"
ok() { [ -f "$1" ] && [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$sha256" ]; }
if ! ok "$tgz"; then
  echo "fetching iwer $version (npm registry)" >&2
  curl -sSfL --retry 3 -o "$tgz.part" "https://registry.npmjs.org/iwer/-/iwer-$version.tgz"
  if ! ok "$tgz.part"; then
    echo "iwer $version: the download does not match its SHA-256" >&2
    rm -f "$tgz.part"
    exit 1
  fi
  mv "$tgz.part" "$tgz"
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$tgz" -C "$tmp" package/build/iwer.min.js package/LICENSE
cp "$tmp/package/build/iwer.min.js" "$out/iwer.min.js"
cp "$tmp/package/LICENSE" "$out/iwer.LICENSE.txt"
echo "iwer $version -> $out/iwer.min.js" >&2
echo "$out/iwer.min.js"
