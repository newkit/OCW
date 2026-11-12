#!/usr/bin/env bash
# Downloads the source data of a world region (spec §4.1, §4.3) into data/
# (git-ignored). Run on the build machine.
#
#   scripts/fetch-pilot.sh [REGION]
#
# Regions:
#   liechtenstein (default)  the Phase 3 pilot, into data/pilot/
#     - OpenStreetMap extract of Liechtenstein from Geofabrik.
#     - SRTM tile N47E009.
#   blackforest              the Black Forest (Schwarzwald), into data/blackforest/
#     - OpenStreetMap extract of the Regierungsbezirk Freiburg (Baden-
#       Württemberg) from Geofabrik, about 160 MB: the southern and central
#       Black Forest (Freiburg, Schauinsland, Feldberg, Titisee) lie in it;
#       the package is cut out of it with `world-builder package --bbox`
#       (spec §4.7, "Black Forest").
#     - SRTM tiles N47E007, N47E008, N48E007, N48E008.
#
# OSM data: ODbL 1.0, "© OpenStreetMap contributors". SRTM (1 arc-second,
# public domain) from the open AWS terrain tiles dataset (skadi layout,
# gzip-compressed HGT). Each region folder gets a SOURCES.md licence table.
# Never public OSM tile servers (CLAUDE.md, spec §4.2).
set -euo pipefail
cd "$(dirname "$0")/.."
region=${1:-liechtenstein}
geofabrik=https://download.geofabrik.de
skadi=https://s3.amazonaws.com/elevation-tiles-prod/skadi
case "$region" in
  liechtenstein)
    out=data/pilot
    pbf=europe/liechtenstein-latest.osm.pbf; pbf_file=liechtenstein.osm.pbf
    poly=europe/liechtenstein.poly; poly_file=liechtenstein.poly
    hgt=(N47E009)
    title="Pilot region"
    ;;
  blackforest)
    out=data/blackforest
    pbf=europe/germany/baden-wuerttemberg/freiburg-regbez-latest.osm.pbf; pbf_file=freiburg-regbez.osm.pbf
    poly=europe/germany/baden-wuerttemberg/freiburg-regbez.poly; poly_file=freiburg-regbez.poly
    hgt=(N47E007 N47E008 N48E007 N48E008)
    title="Black Forest"
    ;;
  *) echo "unknown region '$region' (liechtenstein, blackforest)" >&2; exit 1 ;;
esac
mkdir -p "$out"
fetch() { # url file
  if [ -s "$out/$2" ]; then echo "have $2"; return; fi
  echo "fetching $2"
  curl -fsSL --retry 3 -o "$out/$2.part" "$1"
  mv "$out/$2.part" "$out/$2"
}
fetch "$geofabrik/$pbf" "$pbf_file"
fetch "$geofabrik/$poly" "$poly_file"
rows=""
for t in "${hgt[@]}"; do
  fetch "$skadi/${t:0:3}/$t.hgt.gz" "$t.hgt.gz"
  [ -s "$out/$t.hgt" ] || gunzip -k "$out/$t.hgt.gz"
  rows+="| $t.hgt | $skadi/${t:0:3}/$t.hgt.gz (SRTM via AWS Terrain Tiles) | SRTM: public domain (NASA/USGS) |"$'\n'
done
# The date of the OSM data (Geofabrik's file name behind -latest).
cat > "$out/SOURCES.md" <<TXT
# $title sources

Downloaded by scripts/fetch-pilot.sh $region on $(date -u +%F) (OSM data as of the Geofabrik extract of that day).

| File | Source | Licence |
|---|---|---|
| $pbf_file | $geofabrik/$pbf | ODbL 1.0, © OpenStreetMap contributors |
| $poly_file | $geofabrik/$poly (extract boundary) | ODbL 1.0, © OpenStreetMap contributors |
${rows}
TXT
ls -la "$out"
