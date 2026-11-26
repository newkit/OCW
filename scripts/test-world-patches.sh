#!/usr/bin/env bash
# Tests scripts/world-patches.sh (spec 16H Q10) in a temporary folder: the
# patches of a package from worlds/<region>/patches/ in order, the old place
# data/<region>/patches/ with a notice, neighbouring packages kept apart,
# OCW_NO_PATCHES. No build, no network.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
cd "$t"
. "$here/world-patches.sh"
fail() { echo "test-world-patches: $*" >&2; exit 1; }

mkdir -p worlds/blackforest/patches data/blackforest/patches data/pilot/patches
world_patches blackforest data/blackforest blackforest 2>/dev/null
[ ${#PATCH_ARGS[@]} -eq 0 ] || fail "no patches: ${PATCH_ARGS[*]}"

touch worlds/blackforest/patches/blackforest.ocwpatch \
      worlds/blackforest/patches/blackforest.020-bridge.ocwpatch \
      worlds/blackforest/patches/blackforest.010-surface.ocwpatch \
      worlds/blackforest/patches/blackforest-north.ocwpatch \
      data/blackforest/patches/blackforest.ocwpatch
world_patches blackforest data/blackforest blackforest 2>"$t/err"
want="--patch worlds/blackforest/patches/blackforest.ocwpatch --patch worlds/blackforest/patches/blackforest.010-surface.ocwpatch --patch worlds/blackforest/patches/blackforest.020-bridge.ocwpatch --strict-patch"
[ "${PATCH_ARGS[*]}" = "$want" ] || fail "stack order: ${PATCH_ARGS[*]}"
! grep -q notice "$t/err" || fail "notice for the new place"

world_patches blackforest data/blackforest blackforest-north 2>/dev/null
[ "${PATCH_ARGS[*]}" = "--patch worlds/blackforest/patches/blackforest-north.ocwpatch --strict-patch" ] \
  || fail "neighbour: ${PATCH_ARGS[*]}"
world_patches blackforest data/blackforest blackforest-middle 2>/dev/null
[ ${#PATCH_ARGS[@]} -eq 0 ] || fail "middle has none: ${PATCH_ARGS[*]}"

# The old place, with a notice, only when the new one has none.
touch data/pilot/patches/liechtenstein.ocwpatch
world_patches liechtenstein data/pilot liechtenstein 2>"$t/err"
[ "${PATCH_ARGS[*]}" = "--patch data/pilot/patches/liechtenstein.ocwpatch --strict-patch" ] || fail "old place: ${PATCH_ARGS[*]}"
grep -q "notice: .*old place.*worlds/liechtenstein/patches/" "$t/err" || fail "no notice: $(cat "$t/err")"

OCW_NO_PATCHES=1 world_patches blackforest data/blackforest blackforest 2>/dev/null
[ ${#PATCH_ARGS[@]} -eq 0 ] || fail "OCW_NO_PATCHES: ${PATCH_ARGS[*]}"
echo "world patch tests passed (order, neighbours, old place with notice, OCW_NO_PATCHES)"
