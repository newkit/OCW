#!/usr/bin/env bash
# Builds the Linux release assets of a GitHub release (spec §11.3) on the
# Linux build machine (marge):
#
#   scripts/release-linux.sh [OUT_DIR]     (default: ~/ocw-release-<version>)
#
# run through `scripts/remote.sh marge run 'scripts/release-linux.sh'` from a
# checkout of the "Release X.Y.Z" commit (`make release-prepare`). Writes
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-x86_64.AppImage
#       ocw-desktop with voice and the material library
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-linux-x86_64.tar.xz
#       ocw-desktop, ocw-server, world-builder, ocw-studio (stripped),
#       materials/, LICENSE-MIT, LICENSE-APACHE, secrets.example.toml (the
#       Strava credentials of spec 3.2.4; strava.toml is migrated into it)
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-linux-x86_64/   the tarball's
#       folder, also used by the Windows zip (materials, licences)
#
# The commit comes from OCW_BUILD_ID (scripts/remote.sh passes it, the copy
# has no .git) or OCW_SHA, else git. Voice needs clang-21
# (`make deps-llvm-debian`); appimagetool comes from `make deps-appimage`.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
sha=${OCW_SHA:-${OCW_BUILD_ID:-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)}}
case $sha in *-dirty) echo "refusing a release from a dirty tree ($sha)" >&2; exit 1 ;; esac
out=${1:-$HOME/ocw-release-$version}
mkdir -p "$out"
out=$(cd "$out" && pwd)
base=OpenCyclingWorld-$version-$sha
features=ocw-desktop/voice

export CC=clang-21 CXX=clang++-21
command -v clang-21 >/dev/null || { echo "clang-21 needed for the voice feature" >&2; exit 1; }

t0=$(date +%s)
cargo build --release -p ocw-desktop -p ocw-server -p world-builder -p ocw-studio --features "$features"
echo "cargo build: $(( $(date +%s) - t0 )) s"

# AppImage (the desktop app with the material library).
OCW_FEATURES=$features scripts/build-appimage.sh "$out/$base-x86_64.AppImage"

# Tarball.
dir=$out/$base-linux-x86_64
rm -rf "$dir"
mkdir -p "$dir"
for b in ocw-desktop ocw-server world-builder ocw-studio; do
  install -m 755 "target/release/$b" "$dir/$b"
  strip "$dir/$b"
done
scripts/install-materials.sh "$dir/materials"
install -m 644 LICENSE-MIT LICENSE-APACHE packaging/secrets.example.toml "$dir/"
rm -f "$out/$base-linux-x86_64.tar.xz"
tar -C "$out" -cf - "$base-linux-x86_64" | xz -T0 -6 >"$out/$base-linux-x86_64.tar.xz"

(cd "$out" && sha256sum "$base-x86_64.AppImage" "$base-linux-x86_64.tar.xz")
echo "release assets: $out ($(( $(date +%s) - t0 )) s)"
