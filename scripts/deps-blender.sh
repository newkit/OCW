#!/usr/bin/env bash
# Installs the pinned official Blender release into the user's home, without
# sudo (spec 14 Phase 15, 15G Q4: Blender on marge for the round-trip test and
# the .blend sources). `make deps-blender` runs it.
#
#   ~/.local/opt/blender-$BLENDER_VERSION/   the unpacked release tarball
#   ~/.local/bin/blender                      a symlink to its `blender`
#
# The tarball comes from download.blender.org and must match the pinned
# SHA-256 (taken from the release's own blender-X.Y.Z.sha256 list). Running
# it again does nothing but check the symlink and print the version; a new
# pin installs next to the old one and moves the symlink. Blender is a tool
# here: nothing of it is linked or shipped.
#
# The pin lives here and only here (Linux x64; Blender has LTS releases per
# year, 5.2 is the LTS of 2026, maintained until 2028).
set -euo pipefail

BLENDER_VERSION=5.2.4
BLENDER_SHA256=2fd14c50987426897ba60237ee996edf2b4151f4b1a72a460ca20be58c1d2d86

# Test hooks (scripts/test-deps-blender.sh): another pin, mirror or prefix.
version=${OCW_BLENDER_VERSION:-$BLENDER_VERSION}
sha256=${OCW_BLENDER_SHA256:-$BLENDER_SHA256}
mirror=${OCW_BLENDER_MIRROR:-https://download.blender.org/release}
opt=${OCW_BLENDER_OPT:-$HOME/.local/opt}
bin=${OCW_BLENDER_BIN:-$HOME/.local/bin/blender}

if [ "${1:-}" = "--print-version" ]; then
  echo "$BLENDER_VERSION"
  exit 0
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "deps-blender: no pinned Blender build for $(uname -sm) (Linux x64 only; elsewhere install Blender $version from blender.org)" >&2; exit 1 ;;
esac

name=blender-$version-linux-x64
series=${version%.*}
dir=$opt/blender-$version
url=$mirror/Blender$series/$name.tar.xz

if [ -x "$dir/blender" ] && [ "$(cat "$dir/.ocw-sha256" 2>/dev/null)" = "$sha256" ]; then
  echo "Blender $version already installed: $dir"
else
  mkdir -p "$opt"
  # Downloads and unpacks next to the target (never /tmp: deb's is RAM),
  # then moves it in place in one step.
  work=$(mktemp -d "$opt/.blender-$version.XXXXXX")
  trap 'rm -rf "$work"' EXIT
  echo "downloading $url"
  curl --proto "=https,file" --tlsv1.2 -fsSL --retry 3 -o "$work/$name.tar.xz" "$url"
  got=$(sha256sum "$work/$name.tar.xz" | cut -d' ' -f1)
  if [ "$got" != "$sha256" ]; then
    echo "deps-blender: $name.tar.xz has SHA-256 $got, expected $sha256" >&2
    exit 1
  fi
  tar -xJf "$work/$name.tar.xz" -C "$work"
  [ -x "$work/$name/blender" ] || { echo "deps-blender: no $name/blender in the tarball" >&2; exit 1; }
  echo "$sha256" >"$work/$name/.ocw-sha256"
  rm -rf "$dir"
  mv "$work/$name" "$dir"
fi

mkdir -p "$(dirname "$bin")"
ln -sfn "$dir/blender" "$bin"
echo "$bin -> $dir/blender"
"$bin" --background --factory-startup --version 2>/dev/null | head -n 1
