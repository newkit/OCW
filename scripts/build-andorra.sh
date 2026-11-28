#!/usr/bin/env bash
# Builds the Andorra world package (spec 4.7, "Fifth world: Andorra") from
# the data of `scripts/fetch-pilot.sh andorra`. Run on a build machine:
#
#   scripts/remote.sh leela run scripts/build-andorra.sh
#
# Writes into data/andorra/ (git-ignored):
#   andorra.ocwpkg (+ .manifest.json, andorra.recipe.toml: the build recipe,
#                  spec 16H Q17, scripts/world-recipe.sh)  the whole country,
#                  468 km², 840 – 2,943 m: Andorra la Vella, Escaldes,
#                  Sant Julià de Lòria, La Massana, Arinsal, Ordino, El
#                  Serrat, Canillo, Encamp, Soldeu, Grau Roig, Pas de la
#                  Casa; the Port d'Envalira (2,408 m), Arcalís, the Coll
#                  d'Ordino and the Coll de la Gallina
#   and the route files of its routes.
#
# Like Liechtenstein, the package is the country's Geofabrik extract inside
# its .poly (the roads end at the borders; Geofabrik's polygon has a margin
# of about 1 km, so the first metres of the roads into France and Spain are
# in it) and the box of its roads plus two terrain-only tiles (spec 4.7
# *Tile cover*: the French and Spanish slopes seen from the passes).
# Terrain: IGN MDT05 at 5 m (data/andorra/mdt05, Andorra and Spain), SRTM
# beyond (France). Time zone Europe/Andorra (from the origin, spec 14
# Phase 9I).
#
# Then the steps of the live worlds' J9 rebuild (scripts/j9-build.sh: the
# road condition, the junctions with the painted crossings kept in place,
# the light terrain smoothing and the OSM street lamps; spec 4.8.8 J9) into
# data/j9-build/andorra/ with its lint, and the result is installed as
# data/andorra/andorra.ocwpkg (OCW_NO_J9=1 skips this; the package as
# `world-builder package` wrote it stays as andorra.prej9.ocwpkg).
#
# Environment: OCW_SIGN_KEY  signing key for the manifest (default: unsigned).
#              OCW_NO_PATCHES=1  build without the world patches of
#                         worlds/<region>/patches/ (spec 16H Q10; the patches
#                         used go next to the package, 16H Q9).
#              OCW_SRTM_ONLY=1  build without the MDT05 into
#                         data/andorra/srtm/ (no routes), to compare.
#
# Region calibration (spec 14 Phase 8 T3): when data/andorra/imagery holds the
# imagery of `scripts/fetch-imagery.sh andorra` for this package, the package
# is calibrated after its routes (palette, roof colours, seasonal tints; no
# orthophoto corridor: there is no open orthophoto of Andorra, see
# SOURCES.md); with data/andorra/forest of `scripts/fetch-forest.sh` also the
# forest type and density FRST, T3 (OCW_NO_CALIBRATE=1 skips it,
# OCW_SEASON_CELLS sets the tint cells per tile).
set -euo pipefail
cd "$(dirname "$0")/.."
D=data/andorra
# The middle of the country (between Encamp and the Coll d'Ordino).
ORIGIN=42.54,1.60
# The summit of the Comapedrosa (lat,lon,m), the country's highest point:
# `check --peak` fails without it.
PEAK=42.5918,1.4437,2943
for f in andorra.osm.pbf andorra.poly N42E001.hgt; do
  [ -f "$D/$f" ] || { echo "$D/$f missing: run scripts/fetch-pilot.sh andorra" >&2; exit 1; }
done
DEM=()
OUT=$D
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  OUT=$D/srtm
  mkdir -p "$OUT"
elif compgen -G "$D/mdt05/*.tif" > /dev/null; then
  DEM=(--dem "$D/mdt05" --dem-attribution ign-mdt05-pb --dem-attribution srtm)
else
  echo "warning: no MDT05 in $D/mdt05 (fetch-pilot.sh andorra): SRTM only" >&2
fi
cargo build -q --release -p world-builder
WB=target/release/world-builder
# The project's own world patches (spec 16H Q10): worlds/<region>/patches/.
. scripts/world-patches.sh
# The build recipe shipped next to the package (spec 16H Q17).
. scripts/world-recipe.sh
recipe_start "$OUT"
# Region calibration (spec 14 Phase 8 T3) if the imagery covers the package.
# The forest type and density (section FRST, spec 14 Phase 8 T3) of
# `scripts/fetch-forest.sh` go in with the calibration, or alone without it
# (OCW_NO_FOREST=1 leaves them out).
calibrate() { # package pbf...
  local pkg=$1; shift
  local forest=()
  if [ -z "${OCW_NO_FOREST:-}" ] && [ -f "$D/forest/index.json" ]; then
    forest=(--forest "$D/forest")
  fi
  if [ -z "${OCW_NO_CALIBRATE:-}" ] && grep -qx "$(basename "$pkg")" "$D/imagery/packages.txt" 2>/dev/null; then
    local pbf=()
    for p in "$@"; do pbf+=(--pbf "$p"); done
    $WB calibrate --world "$pkg" --imagery "$D/imagery" "${pbf[@]}" --season-cells "${OCW_SEASON_CELLS:-50}" "${forest[@]}"
  elif [ ${#forest[@]} -gt 0 ]; then
    $WB forest --world "$pkg" "${forest[@]}"
  fi
  # The fields, plantations and mapped trees (section VEGE, spec 14 Phase 8
  # T8; OCW_NO_VEGETATION=1 leaves them out); renews the trees of OBST.
  if [ -z "${OCW_NO_VEGETATION:-}" ]; then
    local vp=()
    for p in "$@"; do vp+=(--pbf "$p"); done
    $WB vegetation --world "$pkg" "${vp[@]}"
  fi
}
out=$OUT/andorra.ocwpkg
world_patches andorra "$D" andorra
SRC=(--pbf "$D/andorra.osm.pbf" --poly "$D/andorra.poly" --hgt "$D/N42E001.hgt"
     "${DEM[@]:0:2}" --origin "$ORIGIN")
# Build time and builder peak memory go to $OUT/measure.log.
scripts/measure.py --label "package andorra" --out "$OUT/measure.log" -- \
  $WB package "${SRC[@]}" --name "Andorra" --id europe/andorra \
  "${DEM[@]:2}" ${OCW_SIGN_KEY:+--sign-key "$OCW_SIGN_KEY"} "${PATCH_ARGS[@]}" --out "$out"
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  $WB check --world "$out" --peak "$PEAK" | grep -E "terrain above|peak|tiles"
  exit 0
fi
# Signature climbs (spec 2.7): route files for --route-file, and the same
# start and end on the package's graph as package routes. Bikes may not
# use the Envalira tunnel (OSM bicycle=no: not in the graph), so the
# Envalira route takes the old road over the pass.
route() { # name file from to
  $WB route "${SRC[@]}" --name "$1" --out "$D/$2" --from "$3" --to "$4"
  $WB route --world "$out" --name "$1" --from "$3" --to "$4"
}
# Andorra la Vella, Plaça del Poble (1,050 m) - Escaldes - Encamp - Canillo
# - Soldeu - Grau Roig - Port d'Envalira (2,408 m, the highest paved pass
# of the Pyrenees): the CG-2.
route "Andorra la Vella - Port d'Envalira" andorra-envalira.route 42.5067,1.5223 42.5402,1.7194
# Ordino (1,300 m) - La Cortinada - El Serrat - the CG-3 hairpins to the
# Arcalís ski station (about 2,230 m), a Vuelta summit finish.
route "Ordino - Arcalís" ordino-arcalis.route 42.5562,1.5335 42.6295,1.4825
# Ordino (1,300 m) - CS-340 - Coll d'Ordino (1,980 m).
route "Ordino - Coll d'Ordino" ordino-coll.route 42.5562,1.5335 42.5561,1.5723
# Sant Julià de Lòria (910 m) - the CS-130 / CS-111 - Coll de la Gallina
# (1,910 m).
route "Sant Julià de Lòria - Coll de la Gallina" santjulia-gallina.route 42.4669,1.4923 42.4594,1.4500
calibrate "$out" "$D/andorra.osm.pbf"
recipe_write "$out" andorra andorra scripts/build-andorra.sh
# Adding the routes rewrote the manifest unsigned (next version): sign again.
if [ -n "${OCW_SIGN_KEY:-}" ]; then
  $WB sign --key "$OCW_SIGN_KEY" "$out"
fi
$WB routes --world "$out"
$WB check --world "$out" --peak "$PEAK" | grep -E "terrain above|land use|buildings|peak|tiles"
# The live worlds' J9 steps (spec 4.8.8 J9) and their lint.
if [ -z "${OCW_NO_J9:-}" ]; then
  OCW_DATA=$PWD/data WB=$WB WB_PREBUILT=1 scripts/j9-build.sh andorra/andorra
  j9=data/j9-build/andorra/andorra.ocwpkg
  cp -p "$out" "$OUT/andorra.prej9.ocwpkg"
  cp -p "$out.manifest.json" "$OUT/andorra.prej9.ocwpkg.manifest.json"
  cp -p "$j9" "$out.new" && mv -f "$out.new" "$out"
  cp -p "$j9.manifest.json" "$out.manifest.json.new" && mv -f "$out.manifest.json.new" "$out.manifest.json"
  if [ -n "${OCW_SIGN_KEY:-}" ]; then
    $WB sign --key "$OCW_SIGN_KEY" "$out"
  fi
  $WB routes --world "$out"
fi
# No plant on a road (the world-building rule of 2026-11-28): the
# installed package once more, failing the build with the first cases.
$WB plants-check --world "$out" --list 10
ls -la "$out"*
sha256sum "$out"
