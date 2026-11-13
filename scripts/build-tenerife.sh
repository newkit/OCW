#!/usr/bin/env bash
# Builds the Tenerife world package (spec §4.7, "Tenerife") from the data
# of `scripts/fetch-pilot.sh tenerife`. Run on a build machine:
#
#   scripts/remote.sh marge run scripts/build-tenerife.sh
#
# Writes into data/tenerife/ (git-ignored):
#   tenerife.ocwpkg (+ .manifest.json)  the whole island, 27.98–28.61 N,
#                   16.94–16.10 W: Santa Cruz, La Laguna, Anaga, La Orotava,
#                   Puerto de la Cruz, the Teide and Las Cañadas, Masca,
#                   Los Gigantes, Los Cristianos, Vilaflor, El Médano
#   and the route files of its routes.
#
# The tiles within 2 km of a road and every tile with land (--land-tiles,
# spec §4.7 "Tile cover"): the box of the roads is 64 % open sea, but the
# Teide's summit lies 3.3 km from the nearest road. Cut with --bbox from the Geofabrik extract of the Canary Islands (the
# other islands lie outside the box). Terrain: IGN MDT05 at 5 m
# (data/tenerife/mdt05), SRTM beyond. The sea comes from the OSM
# coastline (the island's closed ring, spec §4.6), level 0 m. Time zone
# Atlantic/Canary (from the origin, spec §14 Phase 9I).
#
# Environment: OCW_SIGN_KEY  signing key for the manifest (default: unsigned).
#              OCW_SRTM_ONLY=1  build without the MDT05 into
#                         data/tenerife/srtm/ (no routes), to compare.
set -euo pipefail
cd "$(dirname "$0")/.."
D=data/tenerife
# The island's centre (Las Cañadas lie 8 km west of it).
ORIGIN=28.29,-16.52
# Keep in sync with TF_BOX in scripts/fetch-pilot.sh (that adds a margin).
BBOX=27.98,-16.94,28.61,-16.10
# The summit of the Teide (lat,lon,m): `check --peak` fails without it.
TEIDE=28.2724,-16.6424,3715
for f in canary-islands.osm.pbf canary-islands.poly N28W017.hgt N27W017.hgt; do
  [ -f "$D/$f" ] || { echo "$D/$f missing: run scripts/fetch-pilot.sh tenerife" >&2; exit 1; }
done
DEM=()
OUT=$D
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  OUT=$D/srtm
  mkdir -p "$OUT"
elif compgen -G "$D/mdt05/*.tif" > /dev/null; then
  DEM=(--dem "$D/mdt05" --dem-attribution ign-mdt05 --dem-attribution srtm)
else
  echo "warning: no MDT05 in $D/mdt05 (fetch-pilot.sh tenerife): SRTM only" >&2
fi
cargo build -q --release -p world-builder
WB=target/release/world-builder
out=$OUT/tenerife.ocwpkg
SRC=(--pbf "$D/canary-islands.osm.pbf" --poly "$D/canary-islands.poly"
     --hgt "$D/N28W017.hgt" --hgt "$D/N27W017.hgt"
     "${DEM[@]:0:2}" --bbox "$BBOX" --origin "$ORIGIN")
# Build time and builder peak memory go to $OUT/measure.log.
scripts/measure.py --label "package tenerife" --out "$OUT/measure.log" -- \
  $WB package "${SRC[@]}" --name "Tenerife" --id europe/spain/tenerife --land-tiles \
  "${DEM[@]:2}" ${OCW_SIGN_KEY:+--sign-key "$OCW_SIGN_KEY"} --out "$out"
if [ -n "${OCW_SRTM_ONLY:-}" ]; then
  $WB check --world "$out" --peak "$TEIDE" | grep -E "terrain above|peak|tiles"
  exit 0
fi
# Signature routes (spec §2.7): route files for --route-file, and the same
# start and end on the package's graph as package routes.
route() { # name file from to
  $WB route "${SRC[@]}" --name "$1" --out "$D/$2" --from "$3" --to "$4"
  $WB route --world "$out" --name "$1" --from "$3" --to "$4"
}
# Puerto de la Cruz, Plaza del Charco (5 m) - La Orotava - TF-21 through
# Aguamansa and the pine forest - El Portillo (2,050 m) - Las Cañadas -
# Teide cable car base station (2,356 m).
route "Puerto de la Cruz - Teide" puerto-teide.route 28.4148,-16.5486 28.2554,-16.6165
# Los Cristianos (10 m) - Arona - TF-51 Vilaflor (1,400 m, the highest
# village of Spain) - TF-21 Boca de Tauce - Parador - cable car (2,356 m).
route "Los Cristianos - Teide" loscristianos-teide.route 28.0515,-16.7170 28.2554,-16.6165
# Masca (600 m, the roundabout on the TF-436) - the hairpins up to the
# Cherfe pass (1,100 m) - Santiago del Teide (TF-436 / TF-82 junction).
route "Masca - Santiago del Teide" masca-santiago.route 28.3053,-16.8397 28.2968,-16.8176
# The whole TF-436: Buenavista del Norte (El Palmar junction, 80 m) - El
# Palmar - Masca - Cherfe pass - Santiago del Teide.
route "Buenavista - Masca - Santiago del Teide" buenavista-masca.route 28.3699,-16.8498 28.2968,-16.8176
# La Laguna (550 m) - Las Mercedes - TF-12 through the laurel forest of
# Anaga - Cruz del Carmen (920 m) - the TF-114 junction to the Pico del Inglés.
route "La Laguna - Anaga" laguna-anaga.route 28.4876,-16.3157 28.5331,-16.2641
# The TF-21 through Las Cañadas, Boca de Tauce (2,050 m) - Parador -
# Roques de García - El Portillo: route file only (the scene
# showcase-tf-teide-iconic; at 18 and 24 km the road points at the summit).
$WB route "${SRC[@]}" --name "Las Cañadas (TF-21)" --out "$D/canadas.route" \
  --from 28.2045,-16.7040 --to 28.3045,-16.5650 --margin 300
# Its last part from km 24.3 (south of the Roques de García), where the road
# runs straight at the summit 6 km ahead: the riders start together there.
$WB route "${SRC[@]}" --name "Las Cañadas - El Portillo" --out "$D/canadas-teide.route" \
  --from 28.2184,-16.6277 --to 28.3045,-16.5650 --margin 300
# Adding the routes rewrote the manifest unsigned (next version): sign again.
if [ -n "${OCW_SIGN_KEY:-}" ]; then
  $WB sign --key "$OCW_SIGN_KEY" "$out"
fi
$WB routes --world "$out"
$WB check --world "$out" --peak "$TEIDE" | grep -E "terrain above|land use|buildings|peak|tiles"
ls -la "$out"*
