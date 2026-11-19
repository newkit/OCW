#!/usr/bin/env bash
# Tests scripts/deps-blender.sh against a fake release on a file:// mirror
# (no network, no Blender): install, the symlink, idempotence (no second
# download), a wrong SHA-256 refused without leaving anything, a new pin
# installed next to the old one. Part of `make test-blender`'s logic tests.
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0
check() { if [ "$1" = "$2" ]; then echo "ok   $3"; else echo "FAIL $3 (got '$1', want '$2')"; fail=1; fi; }

if [ "$(uname -s)-$(uname -m)" != Linux-x86_64 ]; then
  echo "skipped: deps-blender has a pin only for Linux x64"
  exit 0
fi
d=$(mktemp -d "${TMPDIR:-/tmp}/test-deps-blender.XXXXXX")
trap 'rm -rf "$d"' EXIT

fake_release() { # version → prints its SHA-256
  local v=$1 n=blender-$1-linux-x64
  mkdir -p "$d/src/$n" "$d/mirror/Blender${v%.*}"
  printf '#!/bin/sh\necho "Blender %s (fake)"\n' "$v" >"$d/src/$n/blender"
  chmod +x "$d/src/$n/blender"
  tar -cJf "$d/mirror/Blender${v%.*}/$n.tar.xz" -C "$d/src" "$n"
  sha256sum "$d/mirror/Blender${v%.*}/$n.tar.xz" | cut -d' ' -f1
}
run() { # version sha → output
  OCW_BLENDER_VERSION=$1 OCW_BLENDER_SHA256=$2 OCW_BLENDER_MIRROR="file://$d/mirror" \
    OCW_BLENDER_OPT="$d/opt" OCW_BLENDER_BIN="$d/bin/blender" scripts/deps-blender.sh 2>&1
}

sum=$(fake_release 9.8.1)
out=$(run 9.8.1 "$sum")
check "$(echo "$out" | tail -n 1)" "Blender 9.8.1 (fake)" "installs and prints the version"
check "$(readlink "$d/bin/blender")" "$d/opt/blender-9.8.1/blender" "symlink into ~/.local/opt/blender-X.Y.Z"
check "$(ls -A "$d/opt" | tr '\n' ' ')" "blender-9.8.1 " "no download left behind"

rm "$d/mirror/Blender9.8/blender-9.8.1-linux-x64.tar.xz"
out=$(run 9.8.1 "$sum")
check "$(echo "$out" | grep -c 'already installed')" 1 "second run: already installed"
check "$(echo "$out" | grep -c downloading || true)" 0 "second run: no download"
check "$(echo "$out" | tail -n 1)" "Blender 9.8.1 (fake)" "second run prints the version"

rm "$d/bin/blender"
run 9.8.1 "$sum" >/dev/null
check "$(readlink "$d/bin/blender")" "$d/opt/blender-9.8.1/blender" "a missing symlink is restored"

fake_release 9.9.0 >/dev/null
if out=$(run 9.9.0 0000000000000000000000000000000000000000000000000000000000000000); then st=0; else st=$?; fi
check "$st" 1 "a wrong SHA-256 fails"
check "$(echo "$out" | grep -c 'expected 0000')" 1 "the mismatch is named"
check "$(ls -A "$d/opt" | tr '\n' ' ')" "blender-9.8.1 " "nothing unpacked after a mismatch"
check "$(readlink "$d/bin/blender")" "$d/opt/blender-9.8.1/blender" "symlink kept after a mismatch"

sum2=$(fake_release 9.9.1)
out=$(run 9.9.1 "$sum2")
check "$(echo "$out" | tail -n 1)" "Blender 9.9.1 (fake)" "a new pin installs"
check "$(readlink "$d/bin/blender")" "$d/opt/blender-9.9.1/blender" "the symlink moves to the new pin"
check "$(ls -A "$d/opt" | tr '\n' ' ')" "blender-9.8.1 blender-9.9.1 " "the old version stays"

check "$(scripts/deps-blender.sh --print-version)" "$(sed -n 's/^BLENDER_VERSION=//p' scripts/deps-blender.sh)" "--print-version is the pin"

exit $fail
