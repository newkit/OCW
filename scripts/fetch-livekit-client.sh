#!/usr/bin/env bash
# LiveKit's JS SDK for the browser app's voice (spec §7, §14 Phase 10 B5):
#
#   scripts/fetch-livekit-client.sh [OUT_DIR]     (default target/web)
#
# Copies livekit-client's UMD build (window.LivekitClient) and its licence
# into OUT_DIR as livekit-client.umd.js and livekit-client.LICENSE.txt. The
# package is the npm registry's tarball of the pinned version, checked
# against its SHA-256 and kept in target/web-vendor/ (fetched once). It is
# not in the repository: the page loads it from our own server (Q4), never
# from a CDN at runtime.
#
# Licence: livekit-client is Apache-2.0 (LiveKit, Inc.); the licence text is
# served next to it. Version and hash change together, here only.
set -euo pipefail
cd "$(dirname "$0")/.."

version=2.23.0
# sha256 of https://registry.npmjs.org/livekit-client/-/livekit-client-2.23.0.tgz
# (npm integrity sha512-j9Kg86nJjBjNFUD+ukp2jwMCGa0k+YD5gU//RzAM+ZY3pbG631QCte7fQjwqWMmtBzhDJL9nKYuOLMv/9IbAxg==)
sha256=993eafb83afb4c566bb388108ad6469bf499d76200014c96c35fe3b39fee859d

out=${1:-target/web}
cache=target/web-vendor
tgz=$cache/livekit-client-$version.tgz
mkdir -p "$cache" "$out"
ok() { [ -f "$1" ] && [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$sha256" ]; }
if ! ok "$tgz"; then
  echo "fetching livekit-client $version (npm registry)"
  curl -sSfL --retry 3 -o "$tgz.part" \
    "https://registry.npmjs.org/livekit-client/-/livekit-client-$version.tgz"
  if ! ok "$tgz.part"; then
    echo "livekit-client $version: the download does not match its SHA-256" >&2
    rm -f "$tgz.part"
    exit 1
  fi
  mv "$tgz.part" "$tgz"
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xzf "$tgz" -C "$tmp" package/dist/livekit-client.umd.js package/LICENSE
cp "$tmp/package/dist/livekit-client.umd.js" "$out/livekit-client.umd.js"
cp "$tmp/package/LICENSE" "$out/livekit-client.LICENSE.txt"
echo "livekit-client $version -> $out/livekit-client.umd.js"
