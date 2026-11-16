#!/usr/bin/env bash
# LiveKit's voice server for the bundled voice chat (spec §7.1.1 (a),
# §7.1.7 Q1, Q2; milestone V1):
#
#   scripts/fetch-livekit-server.sh [PLATFORM] [OUT_DIR]
#
# PLATFORM: linux-amd64, linux-arm64, windows-amd64 (default: this machine's)
# or darwin-arm64. OUT_DIR (default target/voice-server/PLATFORM) gets
# livekit-server (livekit-server.exe on Windows) and its licence as
# livekit-server.LICENSE.txt. `ocw-server` finds the binary next to its own
# executable or on PATH (or --voice-server PATH); packaging it into the
# release packages is milestone V6.
#
# Linux and Windows: LiveKit's GitHub release archive of the pinned version,
# checked against its SHA-256 (the digests GitHub lists for the release
# assets, also measured on marge 2026-11-16) and kept in target/voice-vendor/
# (fetched once).
#
# macOS (decided 2026-11-16, Q2): the release has no darwin binary, so it is
# built from LiveKit's pinned source tag on the Mac build machine, like our
# other release binaries (not Homebrew): needs Go (go.mod of v1.13.10 asks
# for Go 1.26). It clones the tag, checks that it is the pinned commit and
# runs
#   CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 go build -trimpath \
#     -ldflags "-s -w" -o livekit-server ./cmd/server
# The binary is not signed here (the app bundle's signing does it, V6).
#
# Licence: LiveKit server is Apache-2.0 (LiveKit, Inc.); its LICENSE goes
# next to the binary. The licences of its Go dependencies (pion and others)
# for the packages come with V6 (go-licenses). Version, hashes and commit
# change together, here and in ocw_server::voice::launcher::PINNED_VERSION.
set -euo pipefail
cd "$(dirname "$0")/.."

version=1.13.10
commit=e5655ac20267fa5005dccaf3f0e3bfc3ea688c07   # tag v1.13.10
base=https://github.com/livekit/livekit/releases/download/v$version

sha256_of() {
  case "$1" in
    linux-amd64) echo c25a3e45fb922e9a93c6443d10c0ed041ba64342097e38632331736630283165 ;;
    linux-arm64) echo ff60607e1931ff078576882fa11c087aa5821d635ac0c47f56ff77e203e32006 ;;
    windows-amd64) echo fd4b7c2c3c1e4c71adaa4f6a046ad1c0abaaaf1e09fae904b35bf231ffae11d2 ;;
    *) return 1 ;;
  esac
}

host_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    MINGW*|MSYS*|CYGWIN*) os=windows ;;
    *) echo "unknown system $(uname -s)" >&2; return 1 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch=amd64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) echo "unknown machine $(uname -m)" >&2; return 1 ;;
  esac
  echo "$os-$arch"
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

platform=${1:-$(host_platform)}
out=${2:-target/voice-server/$platform}
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ "$platform" = darwin-arm64 ]; then
  command -v go >/dev/null || { echo "darwin-arm64: needs Go (the source build of Q2)" >&2; exit 1; }
  echo "building livekit-server $version from source (tag v$version)"
  git -c advice.detachedHead=false clone -q --depth 1 --branch "v$version" \
    https://github.com/livekit/livekit "$tmp/src"
  got=$(git -C "$tmp/src" rev-parse HEAD)
  [ "$got" = "$commit" ] || { echo "tag v$version is $got, expected $commit" >&2; exit 1; }
  (cd "$tmp/src" && CGO_ENABLED=0 GOOS=darwin GOARCH=arm64 \
    go build -trimpath -ldflags "-s -w" -o "$tmp/livekit-server" ./cmd/server)
  cp "$tmp/livekit-server" "$out/livekit-server"
  cp "$tmp/src/LICENSE" "$out/livekit-server.LICENSE.txt"
  echo "livekit-server $version (source build) -> $out/livekit-server"
  exit 0
fi

want=$(sha256_of "$platform") || { echo "unknown platform $platform" >&2; exit 2; }
case "$platform" in
  windows-*) asset=livekit_${version}_${platform/-/_}.zip; exe=livekit-server.exe ;;
  *) asset=livekit_${version}_${platform/-/_}.tar.gz; exe=livekit-server ;;
esac
cache=target/voice-vendor
archive=$cache/$asset
mkdir -p "$cache"
ok() { [ -f "$1" ] && [ "$(sha256 "$1")" = "$want" ]; }
if ! ok "$archive"; then
  echo "fetching $asset (GitHub release v$version)"
  curl -sSfL --retry 3 -o "$archive.part" "$base/$asset"
  if ! ok "$archive.part"; then
    echo "$asset: the download does not match its SHA-256" >&2
    rm -f "$archive.part"
    exit 1
  fi
  mv "$archive.part" "$archive"
fi
case "$asset" in
  *.zip)
    if command -v unzip >/dev/null 2>&1; then unzip -q -o "$archive" "$exe" LICENSE -d "$tmp"
    else python3 -I -m zipfile -e "$archive" "$tmp"; fi ;;
  *) tar -xzf "$archive" -C "$tmp" "$exe" LICENSE ;;
esac
cp "$tmp/$exe" "$out/$exe"
chmod +x "$out/$exe"
cp "$tmp/LICENSE" "$out/livekit-server.LICENSE.txt"
if [ "$platform" = "$(host_platform 2>/dev/null || true)" ]; then
  "$out/$exe" --version | grep -q " $version\$" || { echo "$out/$exe: not version $version" >&2; exit 1; }
fi
echo "livekit-server $version -> $out/$exe"
