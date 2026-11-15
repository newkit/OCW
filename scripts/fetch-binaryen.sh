#!/usr/bin/env bash
# binaryen's wasm-opt for the browser app's size-optimised builds (spec §14
# Phase 10 B8a, `make web-preview`):
#
#   scripts/fetch-binaryen.sh        prints the path of a wasm-opt to use
#
# A wasm-opt on PATH of at least the pinned version is used as it is;
# otherwise the pinned release of binaryen (GitHub, WebAssembly/binaryen)
# is downloaded once, checked against its SHA-256 and unpacked into
# ${OCW_TOOLS:-~/.cache/ocw/tools}/binaryen-version_N/ (no sudo; the build
# machines and the CI runner do the same). Debian's `apt install binaryen`
# is too old for the modules rustc writes now.
#
# Licence: binaryen is Apache-2.0; it is a build tool, nothing of it is
# shipped. Version and hashes change together, here only.
set -euo pipefail

version=133

if have=$(command -v wasm-opt 2>/dev/null); then
  v=$("$have" --version 2>/dev/null | sed -n 's/.*version \([0-9]*\).*/\1/p')
  if [ -n "$v" ] && [ "$v" -ge "$version" ]; then
    echo "$have"
    exit 0
  fi
fi

# sha256 of binaryen-version_$version-<platform>.tar.gz (the release's
# .sha256 files).
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=x86_64-linux
    sha256=2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489 ;;
  Darwin-arm64) platform=arm64-macos
    sha256=ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8 ;;
  Darwin-x86_64) platform=x86_64-macos
    sha256=13a9b90be775c6389ce3d1f879cb8627bea56708ba8c122983941d53a8199b95 ;;
  *) echo "fetch-binaryen: no pinned binaryen for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
tools=${OCW_TOOLS:-$HOME/.cache/ocw/tools}
dir=$tools/binaryen-version_$version
bin=$dir/bin/wasm-opt
if [ ! -x "$bin" ]; then
  name=binaryen-version_$version-$platform.tar.gz
  mkdir -p "$tools"
  tmp=$(mktemp -d "$tools/.binaryen.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  echo "fetching binaryen version_$version ($platform)" >&2
  curl -sSfL --retry 3 -o "$tmp/$name" \
    "https://github.com/WebAssembly/binaryen/releases/download/version_$version/$name"
  got=$( (sha256sum "$tmp/$name" 2>/dev/null || shasum -a 256 "$tmp/$name") | cut -d' ' -f1)
  if [ "$got" != "$sha256" ]; then
    echo "binaryen version_$version: the download does not match its SHA-256" >&2
    exit 1
  fi
  tar -xzf "$tmp/$name" -C "$tmp"
  rm -rf "$dir"
  mv "$tmp/binaryen-version_$version" "$dir"
fi
echo "$bin"
