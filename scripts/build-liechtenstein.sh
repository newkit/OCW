#!/usr/bin/env bash
# Builds the Liechtenstein pilot package (spec §4.7) from the data of
# `scripts/fetch-pilot.sh` (liechtenstein). Run on a build machine:
#
#   scripts/remote.sh marge run scripts/build-liechtenstein.sh
#
# Writes into data/pilot/ (git-ignored): liechtenstein.ocwpkg (+ manifest)
# with the package route "Vaduz - Malbun", and vaduz-malbun.route (the same
# route as an ocw-route file). Terrain: swissALTI3D 2 m (data/pilot/
# swissalti3d; Liechtenstein and Switzerland), SRTM on the Austrian side
# (feathered into it, spec §4.3).
#
# Environment: OCW_SIGN_KEY  signing key for the manifest (default: unsigned).
#              OCW_SRTM_ONLY=1  build without swissALTI3D into data/pilot/srtm/
#                         (no routes), to compare with the SRTM terrain.
#
# Region calibration (spec §14 Phase 8 T3): when data/pilot/imagery holds the
# imagery of `scripts/fetch-imagery.sh liechtenstein` for this package, the package is
# calibrated after its routes (palette, roof colours, seasonal tints;
# OCW_NO_CALIBRATE=1 skips it, OCW_SEASON_CELLS sets the tint cells per tile).
set -euo pipefail
cd "$(dirname "$0")/.."
D=data/pilot
for f in liechtenstein.osm.pbf liechtenstein.poly N47E009.hgt; do
  [ -f "$D/$f" ] || { echo "$D/$f missing: run scripts/fetch-pilot.sh" >&2; exit 1; }
done
SRC=(--pbf "$D/liechtenstein.osm.pbf" --poly "$D/liechtenstein.poly" --hgt "$D/N47E009.hgt")
ATTR=()
OUT=$D
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  OUT=$D/srtm
  mkdir -p "$OUT"
elif compgen -G "$D/swissalti3d/*.tif" > /dev/null; then
  SRC+=(--dem "$D/swissalti3d")
  ATTR=(--dem-attribution swissalti3d --dem-attribution srtm)
else
  echo "warning: no swissALTI3D in $D/swissalti3d (fetch-pilot.sh): SRTM only" >&2
fi
cargo build -q --release -p world-builder
WB=target/release/world-builder
# Region calibration (spec §14 Phase 8 T3) if the imagery covers the package.
calibrate() { # package pbf...
  local pkg=$1; shift
  if [ -z "${OCW_NO_CALIBRATE:-}" ] && grep -qx "$(basename "$pkg")" "$D/imagery/packages.txt" 2>/dev/null; then
    local pbf=()
    for p in "$@"; do pbf+=(--pbf "$p"); done
    $WB calibrate --world "$pkg" --imagery "$D/imagery" "${pbf[@]}" --season-cells "${OCW_SEASON_CELLS:-50}"
  fi
}
out=$OUT/liechtenstein.ocwpkg
# Build time and builder peak memory go to $OUT/measure.log.
scripts/measure.py --label "package liechtenstein" --out "$OUT/measure.log" -- \
  $WB package "${SRC[@]}" --name Liechtenstein "${ATTR[@]}" \
  ${OCW_SIGN_KEY:+--sign-key "$OCW_SIGN_KEY"} --out "$out"
if [ -z "${OCW_SRTM_ONLY:-}" ]; then
  # Vaduz (455 m) - Triesenberg - Steg tunnel - Malbun (1,600 m).
  $WB route "${SRC[@]}" --name "Vaduz - Malbun" --out "$D/vaduz-malbun.route"
  $WB route --world "$out" --name "Vaduz - Malbun" --from 47.1395,9.5220 --to 47.1013,9.6094
  calibrate "$out" "$D/liechtenstein.osm.pbf"
  if [ -n "${OCW_SIGN_KEY:-}" ]; then
    $WB sign --key "$OCW_SIGN_KEY" "$out"
  fi
  $WB routes --world "$out"
fi
$WB check --world "$out" | grep -E "terrain above|land use|buildings"
ls -la "$out"*
