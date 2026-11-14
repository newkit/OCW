#!/usr/bin/env bash
# Publishes world packages for the browser app's dev server (spec §14
# Phase 10 B2, `make web-worlds`):
#
#   scripts/publish-web-worlds.sh [PKG...]    (default: data/pilot/liechtenstein.ocwpkg)
#
# into target/web/worlds/<id>/<version>/ and <id>/latest (`world-builder
# publish`; WEB_WORLDS overrides the directory), each signed with a signing
# key of a development root kept in target/web-keys/ (created on the first
# run; git-ignored, never for production). The package files themselves
# are not changed: a copy is signed in target/web-keys/stage/.
#
# The root's public key goes to target/web-keys/root.hex: dev pages (on
# localhost) trust it with ?trust-root=HEX, which `make web-serve` prints
# and scripts/test-web.py adds.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

keys=target/web-keys
out=${WEB_WORLDS:-target/web/worlds}
mkdir -p "$keys" "$out"
cargo build -q --release -p world-builder
wb=target/release/world-builder
[ -x "$wb" ] || wb=target/release/world-builder.exe

if [ ! -f "$keys/root.key" ]; then
  "$wb" keys init-root --name "OCW web dev server root (development only)" --dir "$keys"
fi
if [ ! -f "$keys/web-dev.key" ]; then
  "$wb" keys new-signing --name web-dev --root "$keys/root.key" --days 3650 --out "$keys/web-dev.key"
fi
python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["public_key"])' \
  "$keys/root.pub.json" > "$keys/root.hex"

[ $# -gt 0 ] || set -- data/pilot/liechtenstein.ocwpkg
for pkg in "$@"; do
  [ -f "$pkg" ] || { echo "$pkg: no such package" >&2; exit 1; }
  stage=$keys/stage
  rm -rf "$stage"
  mkdir -p "$stage"
  name=$(basename "$pkg")
  # A hard link where possible (the package is not written), else a copy.
  ln "$pkg" "$stage/$name" 2>/dev/null || cp "$pkg" "$stage/$name"
  # The old manifest keeps id, version and attribution.
  [ -f "$pkg.manifest.json" ] && cp "$pkg.manifest.json" "$stage/$name.manifest.json"
  "$wb" sign "$stage/$name" --key "$keys/web-dev.key"
  "$wb" publish "$stage/$name" --out "$out"
done
echo "trust-root: $(cat "$keys/root.hex") (dev pages: ?trust-root=…)"
