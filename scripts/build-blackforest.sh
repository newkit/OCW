#!/usr/bin/env bash
# Builds the Black Forest world packages (spec §4.7, "Black Forest") from
# the data of `scripts/fetch-pilot.sh blackforest`. Run on a build machine:
#
#   scripts/remote.sh marge run scripts/build-blackforest.sh [PART...]
#
# PART: south, middle, north (default: all three). Writes into
# data/blackforest/ (git-ignored):
#   blackforest.ocwpkg         south: Freiburg, Schauinsland, Feldberg,
#                              Titisee, Belchen, Schluchsee (47.72–48.12 N)
#   blackforest-middle.ocwpkg  middle: Elzach, Triberg, Hornberg, Wolfach,
#                              Schramberg (48.12–48.45 N)
#   blackforest-north.ocwpkg   north: Baden-Baden, Schwarzwaldhochstraße
#                              (B500), Mummelsee, Hornisgrinde,
#                              Sasbachwalden, Allerheiligen, Ruhestein,
#                              Bühlerhöhe, Kniebis (48.45–48.80 N)
#   (+ .manifest.json each) and the route files of their routes.
#
# All three are cut with --bbox from the same OSM extracts (Regierungsbezirke
# Freiburg and Karlsruhe, one day) and share the projection origin and the
# tile grid, so they meet without a step and ride as one world (spec
# §2.2.2): each cuts the roads exactly on the shared border lines 48.12 N
# and 48.45 N. Terrain: LGL DGM1 at 2 m (data/blackforest/dgm1), SRTM
# outside Baden-Württemberg.
#
# Environment: OCW_SIGN_KEY  signing key for the manifests (default: unsigned).
#              OCW_SRTM_ONLY=1  build without the DGM1 into data/blackforest/srtm/
#                         (no routes), to compare with the SRTM terrain.
#
# Region calibration (spec §14 Phase 8 T3): when data/blackforest/imagery holds the
# imagery of `scripts/fetch-imagery.sh blackforest` for this package, the package is
# calibrated after its routes (palette, roof colours, seasonal tints and the
# orthophoto corridor ORTC, spec §14 Phase 8 T7;
# OCW_NO_CALIBRATE=1 skips it, OCW_SEASON_CELLS sets the tint cells per tile).
set -euo pipefail
cd "$(dirname "$0")/.."
D=data/blackforest
ORIGIN=47.92,7.96
# Keep in sync with BF_BOXES in scripts/fetch-pilot.sh (those add a margin).
declare -A BBOX=([south]=47.72,7.62,48.12,8.30 [middle]=48.12,7.70,48.45,8.40 [north]=48.45,7.85,48.80,8.45)
declare -A FILE=([south]=blackforest [middle]=blackforest-middle [north]=blackforest-north)
declare -A ID=([south]=europe/germany/black-forest [middle]=europe/germany/black-forest-middle [north]=europe/germany/black-forest-north)
declare -A NAME=([south]="Black Forest" [middle]="Black Forest (middle)" [north]="Black Forest (north)")
parts=("$@")
[ ${#parts[@]} -gt 0 ] || parts=(south middle north)
for f in freiburg-regbez.osm.pbf freiburg-regbez.poly karlsruhe-regbez.osm.pbf karlsruhe-regbez.poly \
         N47E007.hgt N47E008.hgt N48E007.hgt N48E008.hgt; do
  [ -f "$D/$f" ] || { echo "$D/$f missing: run scripts/fetch-pilot.sh blackforest" >&2; exit 1; }
done
DEM=()
OUT=$D
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  OUT=$D/srtm
  mkdir -p "$OUT"
elif compgen -G "$D/dgm1/*.tif" > /dev/null; then
  DEM=(--dem "$D/dgm1" --dem-attribution lgl-dgm1 --dem-attribution srtm)
else
  echo "warning: no DGM1 in $D/dgm1 (fetch-pilot.sh blackforest): SRTM only" >&2
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
for part in "${parts[@]}"; do
  [ -n "${BBOX[$part]:-}" ] || { echo "unknown part $part (south, middle, north)" >&2; exit 1; }
  out=$OUT/${FILE[$part]}.ocwpkg
  SRC=(--pbf "$D/freiburg-regbez.osm.pbf" --pbf "$D/karlsruhe-regbez.osm.pbf"
       --poly "$D/freiburg-regbez.poly" --poly "$D/karlsruhe-regbez.poly"
       --hgt "$D/N47E007.hgt" --hgt "$D/N47E008.hgt" --hgt "$D/N48E007.hgt" --hgt "$D/N48E008.hgt"
       "${DEM[@]:0:2}" --bbox "${BBOX[$part]}" --origin "$ORIGIN")
  # Build time and builder peak memory go to $OUT/measure.log.
  scripts/measure.py --label "package $part" --out "$OUT/measure.log" -- \
    $WB package "${SRC[@]}" --name "${NAME[$part]}" --id "${ID[$part]}" \
    "${DEM[@]:2}" ${OCW_SIGN_KEY:+--sign-key "$OCW_SIGN_KEY"} --out "$out"
  # Signature routes (spec §2.7): route files for --route-file, and the same
  # start and end on the package's graph as package routes.
  route() { # name file from to
    $WB route "${SRC[@]}" --name "$1" --out "$D/$2" --from "$3" --to "$4"
    $WB route --world "$out" --name "$1" --from "$3" --to "$4"
  }
  [ -z "${OCW_SRTM_ONLY:-}" ] || { $WB check --world "$out" | grep -E "terrain above"; continue; }
  case $part in
    south)
      # Freiburg, Bertoldsbrunnen (278 m) - Günterstal - Schauinslandstraße - summit (~1,220 m road end).
      route "Freiburg - Schauinsland" freiburg-schauinsland.route 47.99485,7.84945 47.9110,7.8975
      # Titisee, Seestraße (850 m) - B317 - Feldberg-Passhöhe (1,231 m).
      route "Titisee - Feldberg" titisee-feldberg.route 47.9030,8.1560 47.8616,8.0337
      ;;
    middle)
      # Hornberg (380 m) - B33 / B500 Gutachtal - Triberg, at the waterfalls (~700 m).
      route "Hornberg - Triberg" hornberg-triberg.route 48.2105,8.2335 48.1318,8.2306
      ;;
    north)
      # Baden-Baden, Leopoldsplatz (160 m) - Geroldsau - B500
      # Schwarzwaldhochstraße via Bühlerhöhe, Sand, Hundseck - Mummelsee (1,030 m).
      route "Baden-Baden - Mummelsee" badenbaden-mummelsee.route 48.7606,8.2408 48.5977,8.2006
      # Sasbachwalden (260 m) - Brandmatt - Unterstmatt - Mummelsee below the Hornisgrinde.
      route "Sasbachwalden - Mummelsee" sasbachwalden-mummelsee.route 48.6170,8.1270 48.5977,8.2006
      # Bühl (135 m) - Bühlertal - Bühlerhöhe (760 m).
      route "Bühl - Bühlerhöhe" buehl-buehlerhoehe.route 48.6953,8.1350 48.6648,8.2240
      ;;
  esac
  calibrate "$out" "$D/freiburg-regbez.osm.pbf" "$D/karlsruhe-regbez.osm.pbf"
  # Adding the routes rewrote the manifest unsigned (next version): sign again.
  if [ -n "${OCW_SIGN_KEY:-}" ]; then
    $WB sign --key "$OCW_SIGN_KEY" "$out"
  fi
  $WB routes --world "$out"
  $WB check --world "$out" | grep -E "terrain above|land use|buildings"
  ls -la "$out"*
done
