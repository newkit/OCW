#!/usr/bin/env bash
# Tests the Makefile's version targets (`make version-next` / `version-bump`,
# used by `make release-prepare`) on a copy of the workspace manifests (no
# network, no git): patch (the default) / minor / major bumps change exactly the
# [workspace.package] version and the workspace crates in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cp Cargo.toml Cargo.lock "$tmp/"
for d in apps crates services tools; do
  [ -d "$d" ] && find "$d" -name Cargo.toml -not -path '*/target/*' -exec cp --parents {} "$tmp/" \;
done
mk() { make -s --no-print-directory "$@" VERSION_ROOT="$tmp"; }
v() { sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)".*/\1/p' "$tmp/Cargo.toml"; }
lockv() { awk -v n="$1" '/^name = "/{c=($0=="name = \"" n "\"")} c && /^version = /{gsub(/version = |"/,""); print; exit}' "$tmp/Cargo.lock"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
start=$(v); IFS=. read -r ma mi pa <<<"$start"
other=$(lockv serde)
[ "$(mk version)" = "$start" ] || fail "version"
[ "$(mk version-next)" = "$ma.$mi.$((pa+1))" ] || fail "version-next (default: patch)"
[ "$(v)" = "$start" ] || fail "version-next changed the file"
[ "$(mk version-bump)" = "$ma.$mi.$((pa+1))" ] || fail "patch (default)"
[ "$(v)" = "$ma.$mi.$((pa+1))" ] || fail "Cargo.toml patch"
[ "$(mk version-bump PART=minor)" = "$ma.$((mi+1)).0" ] || fail "minor"
[ "$(v)" = "$ma.$((mi+1)).0" ] || fail "Cargo.toml minor"
[ "$(lockv ocw-desktop)" = "$ma.$((mi+1)).0" ] || fail "Cargo.lock ocw-desktop"
[ "$(lockv world-builder)" = "$ma.$((mi+1)).0" ] || fail "Cargo.lock world-builder"
[ "$(lockv serde)" = "$other" ] || fail "a dependency's version changed"
[ "$(mk version-bump PART=patch)" = "$ma.$((mi+1)).1" ] || fail "patch after minor"
[ "$(mk version-bump PART=major)" = "$((ma+1)).0.0" ] || fail "major"
grep -q "^version = \"$((ma+1)).0.0\"" "$tmp/Cargo.toml" || fail "major in Cargo.toml"
echo "release version tests passed ($start -> $ma.$mi.$((pa+1)) by default, minor, patch, major; dependencies untouched)"
