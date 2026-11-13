#!/usr/bin/env bash
# Downloads the source data of a world region (spec §4.1, §4.3) into data/
# (git-ignored), the best open data there is (spec Phase 8 Q9). Run on the
# build machine:
#
#   scripts/fetch-pilot.sh [REGION]
#
# Regions:
#   liechtenstein (default)  the Phase 3 pilot, into data/pilot/
#     - OpenStreetMap extract of Liechtenstein from Geofabrik.
#     - swissALTI3D 2 m (swisstopo, GeoTIFF, 1 km tiles; Switzerland and
#       Liechtenstein, bare earth) around the country: swissalti3d/.
#     - SRTM tile N47E009 (the Austrian side, fallback).
#   blackforest              the Black Forest (Schwarzwald), into data/blackforest/
#     - OpenStreetMap extracts of the Regierungsbezirke Freiburg and
#       Karlsruhe (Baden-Württemberg) from Geofabrik, same day: the three
#       Black Forest packages (south, middle, north; spec §4.7) are cut out
#       of both with `world-builder package --bbox` (build-blackforest.sh).
#     - LGL DGM1 (1 m digital terrain model of Baden-Württemberg, XYZ in
#       2 km zips) for the three package boxes, imported to 2 m GeoTIFFs
#       (dgm1/) with `world-builder dem-import`; the zips are deleted after
#       the import unless OCW_KEEP_RAW=1.
#     - SRTM tiles N47E007, N47E008, N48E007, N48E008 (fallback: France,
#       Switzerland, gaps).
#
# Environment: OCW_FETCH_FRESH=1 downloads the OSM extracts again (the
# freshest Geofabrik extract, daily); elevation tiles are kept.
#
# Licences (spec §14 Phase 8A, decided Q1): OSM ODbL 1.0 "© OpenStreetMap
# contributors"; LGL DGM1 Datenlizenz Deutschland – Namensnennung 2.0
# ("Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0"); swissALTI3D swisstopo
# OGD (free use incl. commercial, source "© swisstopo"); SRTM public domain
# (AWS terrain tiles, skadi layout). Each region folder gets a SOURCES.md
# licence table. Never public OSM tile servers (CLAUDE.md, spec §4.2).
set -euo pipefail
cd "$(dirname "$0")/.."
region=${1:-liechtenstein}
geofabrik=https://download.geofabrik.de
skadi=https://s3.amazonaws.com/elevation-tiles-prod/skadi
lgl=https://opengeodata.lgl-bw.de/data/dgm
stac=https://data.geo.admin.ch/api/stac/v0.9/collections/ch.swisstopo.swissalti3d/items
# Package boxes of the Black Forest (S,W,N,E; the same as in
# scripts/build-blackforest.sh), each with a 0.04° margin for the context
# roads, the margin tiles and the feathering into SRTM.
BF_BOXES=(47.68,7.58,48.16,8.34 48.08,7.66,48.49,8.44 48.41,7.81,48.84,8.49)
# Liechtenstein's .poly box with the same margin.
LI_BOX=47.00,9.42,47.31,9.69
case "$region" in
  liechtenstein)
    out=data/pilot
    osm=("europe/liechtenstein:liechtenstein")
    hgt=(N47E009)
    title="Pilot region (Liechtenstein)"
    ;;
  blackforest)
    out=data/blackforest
    osm=("europe/germany/baden-wuerttemberg/freiburg-regbez:freiburg-regbez"
         "europe/germany/baden-wuerttemberg/karlsruhe-regbez:karlsruhe-regbez")
    hgt=(N47E007 N47E008 N48E007 N48E008)
    title="Black Forest"
    ;;
  *) echo "unknown region '$region' (liechtenstein, blackforest)" >&2; exit 1 ;;
esac
mkdir -p "$out"
fetch() { # url file [force]
  if [ -s "$out/$2" ] && [ -z "${3:-}" ]; then echo "have $2"; return; fi
  echo "fetching $2"
  curl -fsSL --retry 3 -o "$out/$2.part" "$1"
  mv "$out/$2.part" "$out/$2"
}
rows=""
for o in "${osm[@]}"; do
  path=${o%%:*}; file=${o##*:}
  fetch "$geofabrik/$path-latest.osm.pbf" "$file.osm.pbf" "${OCW_FETCH_FRESH:-}"
  fetch "$geofabrik/$path.poly" "$file.poly"
  # Geofabrik's dated file name behind -latest (the OSM data's day).
  dated=$(curl -fsSIL "$geofabrik/$path-latest.osm.pbf" | tr -d '\r' |
          awk 'tolower($1)=="location:"{print $2}' | tail -1)
  rows+="| $file.osm.pbf | $geofabrik/$path-latest.osm.pbf (${dated##*/}, downloaded $(date -u -r "$out/$file.osm.pbf" +%F)) | ODbL 1.0, © OpenStreetMap contributors |"$'\n'
  rows+="| $file.poly | $geofabrik/$path.poly (extract boundary) | ODbL 1.0, © OpenStreetMap contributors |"$'\n'
done
for t in "${hgt[@]}"; do
  fetch "$skadi/${t:0:3}/$t.hgt.gz" "$t.hgt.gz"
  [ -s "$out/$t.hgt" ] || gunzip -k "$out/$t.hgt.gz"
  rows+="| $t.hgt | $skadi/${t:0:3}/$t.hgt.gz (SRTM 1″ via AWS Terrain Tiles; fallback outside the DTM) | SRTM: public domain (NASA/USGS) |"$'\n'
done

WB=target/release/world-builder
build_wb() { cargo build -q --release -p world-builder; }

case "$region" in
  liechtenstein)
    # swissALTI3D 2 m GeoTIFFs: the newest year of every 1 km tile in the
    # box (STAC API of data.geo.admin.ch).
    mkdir -p "$out/swissalti3d"
    # STAC wants west,south,east,north.
    IFS=, read -r S W N E <<<"$LI_BOX"
    python3 -I - "$stac?bbox=$W,$S,$E,$N&limit=100" > "$out/swissalti3d-urls.txt" <<'PY'
import json, sys, urllib.request
url, best = sys.argv[1], {}
while url:
    d = json.load(urllib.request.urlopen(url))
    for f in d["features"]:
        _, year, tile = f["id"].split("_")
        for a in f["assets"].values():
            if a["href"].endswith("_2_2056_5728.tif") and best.get(tile, ("", ""))[0] < year:
                best[tile] = (year, a["href"])
    url = next((l["href"] for l in d["links"] if l["rel"] == "next"), None)
for t in sorted(best):
    print(best[t][1])
PY
    n=0
    while read -r u; do
      f=$out/swissalti3d/$(basename "$u")
      [ -s "$f" ] || curl -fsSL --retry 3 -o "$f" "$u"
      n=$((n + 1))
    done < "$out/swissalti3d-urls.txt"
    years=$(sed -E 's/.*swissalti3d_([0-9]{4})_.*/\1/' "$out/swissalti3d-urls.txt" | sort -u | tr '\n' ' ')
    rows+="| swissalti3d/ ($n GeoTIFFs, 1 km, 2 m, years $years) | swissALTI3D, $stac (bbox $LI_BOX) | swisstopo OGD: free use incl. commercial, source \"© swisstopo\" (in Liechtenstein the Amt für Tiefbau und Geoinformation holds the distribution rights for its customers) |"$'\n'
    ;;
  blackforest)
    build_wb
    mkdir -p "$out/dgm1-zip" "$out/dgm1"
    for b in "${BF_BOXES[@]}"; do
      $WB dem-tiles --crs utm32 --bbox "$b" --size 2000 --phase 1000,0
    done | sort -u > "$out/dgm1-tiles.txt"
    echo "DGM1: $(wc -l < "$out/dgm1-tiles.txt") tiles of 2 km"
    # 4 downloads at a time; 404 = outside Baden-Württemberg (marker file).
    sed 's/.*/dgm1_32_&_2_bw.zip/' "$out/dgm1-tiles.txt" |
      xargs -P 4 -I{} sh -c 'f="$1/{}"; [ -s "$f" ] || [ -e "$f.404" ] || [ -e "$1/../dgm1/.{}.done" ] || {
        code=$(curl -s --retry 3 -o "$f.part" -w "%{http_code}" "$2/{}")
        if [ "$code" = 200 ]; then mv "$f.part" "$f"; else rm -f "$f.part"; touch "$f.404"; fi; }' _ "$out/dgm1-zip" "$lgl"
    # Import: 1 m XYZ → 2 m GeoTIFF per km (block means), 4 zips at a time,
    # then drop the zip; the LGL metadata (capture date per km) is kept.
    mkdir -p "$out/dgm1/meta"
    import_zip() { # zip
      local name tmp
      name=$(basename "$1")
      tmp=$(mktemp -d)
      unzip -q -o "$1" '*.xyz' '*.csv' -d "$tmp"
      "$WB" dem-import --crs utm32 --cell 2 --out "$out/dgm1" "$tmp"/*/*.xyz > /dev/null
      cat "$tmp"/*/*.csv | grep -v '^Kachelname' > "$out/dgm1/meta/$name.csv" || true
      rm -rf "$tmp"
      touch "$out/dgm1/.$name.done"
      [ -n "${OCW_KEEP_RAW:-}" ] || rm -f "$1"
    }
    export -f import_zip
    export WB out
    find "$out/dgm1-zip" -name '*.zip' | sort | xargs -r -P 4 -I{} bash -c 'import_zip "$1"' _ {}
    cat "$out"/dgm1/meta/*.csv 2>/dev/null | sort -u > "$out/dgm1/metadata.csv" || true
    tifs=$(find "$out/dgm1" -name '*.tif' | wc -l)
    missing=$(find "$out/dgm1-zip" -name '*.404' | wc -l)
    years=$(cut -d';' -f2 "$out/dgm1/metadata.csv" | cut -c1-4 | sort | uniq -c | awk '{printf "%s: %s, ", $2, $1}')
    rows+="| dgm1/ ($tifs GeoTIFFs, 1 km, 2 m from 1 m; $missing of $(wc -l < "$out/dgm1-tiles.txt") 2 km tiles outside Baden-Württemberg) | LGL DGM1 (Digitales Geländemodell 1 m, airborne laser scanning), $lgl/dgm1_32_E_N_2_bw.zip; capture years ${years%, } (dgm1/metadata.csv) | Datenlizenz Deutschland – Namensnennung – Version 2.0 (www.govdata.de/dl-de/by-2-0): \"Datenquelle: LGL, www.lgl-bw.de, dl-de/by-2-0\"; averaged to 2 m and adapted to the roads (modified) |"$'\n'
    ;;
esac

cat > "$out/SOURCES.md" <<TXT
# $title sources

Downloaded by scripts/fetch-pilot.sh $region; this table written on $(date -u +%F).

| File | Source | Licence |
|---|---|---|
${rows}
TXT
ls -la "$out"
