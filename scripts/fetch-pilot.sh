#!/usr/bin/env bash
# Downloads the Phase 3 pilot region (spec §4.1, §4.3) into data/pilot/
# (git-ignored). Run on the build machine.
#
#   scripts/fetch-pilot.sh
#
# - OpenStreetMap extract of Liechtenstein from Geofabrik (ODbL,
#   "© OpenStreetMap contributors").
# - SRTM elevation tile N47E009 (1 arc-second, public domain) from the
#   open AWS terrain tiles dataset (skadi layout, gzip-compressed HGT).
set -euo pipefail
cd "$(dirname "$0")/.."
out=data/pilot
mkdir -p "$out"
fetch() { # url file
  if [ -s "$out/$2" ]; then echo "have $2"; return; fi
  echo "fetching $2"
  curl -fsSL --retry 3 -o "$out/$2.part" "$1"
  mv "$out/$2.part" "$out/$2"
}
fetch https://download.geofabrik.de/europe/liechtenstein-latest.osm.pbf liechtenstein.osm.pbf
fetch https://download.geofabrik.de/europe/liechtenstein.poly liechtenstein.poly
fetch https://s3.amazonaws.com/elevation-tiles-prod/skadi/N47/N47E009.hgt.gz N47E009.hgt.gz
[ -s "$out/N47E009.hgt" ] || gunzip -k "$out/N47E009.hgt.gz"
cat > "$out/SOURCES.md" <<'TXT'
# Pilot region sources

| File | Source | Licence |
|---|---|---|
| liechtenstein.osm.pbf | https://download.geofabrik.de/europe/liechtenstein-latest.osm.pbf | ODbL 1.0, © OpenStreetMap contributors |
| liechtenstein.poly | https://download.geofabrik.de/europe/liechtenstein.poly (extract boundary) | ODbL 1.0, © OpenStreetMap contributors |
| N47E009.hgt | https://s3.amazonaws.com/elevation-tiles-prod/skadi/N47/N47E009.hgt.gz (SRTM via AWS Terrain Tiles) | SRTM: public domain (NASA/USGS) |
TXT
ls -la "$out"
