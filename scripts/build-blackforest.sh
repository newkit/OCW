#!/usr/bin/env bash
# Builds the Black Forest world package (spec §4.7, "Black Forest") from the
# data of `scripts/fetch-pilot.sh blackforest`. Run on a build machine:
#
#   scripts/remote.sh marge run scripts/build-blackforest.sh
#
# Writes into data/blackforest/ (git-ignored):
#   blackforest.ocwpkg (+ .manifest.json)  the southern Black Forest, cut out
#       of the Regierungsbezirk Freiburg extract with --bbox, with the routes
#       "Freiburg - Schauinsland" and "Titisee - Feldberg" (section ROUT)
#   freiburg-schauinsland.route, titisee-feldberg.route
#       the same routes as ocw-route files (like vaduz-malbun.route)
#
# Environment: OCW_SIGN_KEY  signing key for the manifest (default: unsigned).
#
# The box and the projection origin are fixed (spec §4.7): neighbouring
# Black Forest packages (north, east) are built with the same --origin, so
# their metres and tile grids coincide with this one's.
set -euo pipefail
cd "$(dirname "$0")/.."
D=data/blackforest
BBOX=47.72,7.62,48.12,8.30
ORIGIN=47.92,7.96
for f in freiburg-regbez.osm.pbf freiburg-regbez.poly N47E007.hgt N47E008.hgt N48E007.hgt N48E008.hgt; do
  [ -f "$D/$f" ] || { echo "$D/$f missing: run scripts/fetch-pilot.sh blackforest" >&2; exit 1; }
done
SRC=(--pbf "$D/freiburg-regbez.osm.pbf" --poly "$D/freiburg-regbez.poly"
     --hgt "$D/N47E007.hgt" --hgt "$D/N47E008.hgt" --hgt "$D/N48E007.hgt" --hgt "$D/N48E008.hgt"
     --bbox "$BBOX" --origin "$ORIGIN")
cargo build -q --release -p world-builder
WB=target/release/world-builder
$WB package "${SRC[@]}" --name "Black Forest" --id europe/germany/black-forest \
  ${OCW_SIGN_KEY:+--sign-key "$OCW_SIGN_KEY"} --out "$D/blackforest.ocwpkg"
# Signature routes (spec §2.7): route files for --route-file, and the same
# start and end on the package's graph as package routes.
route() { # name file from to
  $WB route "${SRC[@]}" --name "$1" --out "$D/$2" --from "$3" --to "$4"
  $WB route --world "$D/blackforest.ocwpkg" --name "$1" --from "$3" --to "$4"
}
# Freiburg, Bertoldsbrunnen (278 m) - Günterstal - Schauinslandstraße - summit (~1,220 m road end).
route "Freiburg - Schauinsland" freiburg-schauinsland.route 47.99485,7.84945 47.9110,7.8975
# Titisee, Seestraße (850 m) - B317 - Feldberg-Passhöhe (1,231 m).
route "Titisee - Feldberg" titisee-feldberg.route 47.9030,8.1560 47.8616,8.0337
# Adding the routes rewrote the manifest unsigned (next version): sign again.
if [ -n "${OCW_SIGN_KEY:-}" ]; then
  $WB sign --key "$OCW_SIGN_KEY" "$D/blackforest.ocwpkg"
fi
$WB routes --world "$D/blackforest.ocwpkg"
$WB check --world "$D/blackforest.ocwpkg" | head -5
ls -la "$D"/blackforest.ocwpkg* "$D"/*.route
