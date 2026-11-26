#!/usr/bin/env bash
# Downloads the forest type and tree cover density of a world region (spec
# §14 Phase 8 T3) into data/<region>/forest/ (git-ignored): the Copernicus
# High Resolution Layers *Dominant Leaf Type 2018* and *Tree Cover Density
# 2018* (10 m, ETRS89-LAEA Europe) over the 1 km cells the built packages
# cover. Run on the build machine after the packages are built (they list
# the cells); the build scripts then put them into each package (section
# FRST, with `world-builder calibrate --forest` or `world-builder forest`):
#
#   scripts/fetch-forest.sh REGION [PACKAGE...]
#
# Regions: liechtenstein (data/pilot/), blackforest (data/blackforest/, all
# three parts by default), tenerife (data/tenerife/; the HRL covers the
# Canary Islands: Canary pine is coniferous, the laurel forest broadleaved),
# andorra (data/andorra/; Andorra is not an EEA member, the layers are read
# over its box like any other).
#
# Source (checked 2026-11-14): the EEA's public ArcGIS ImageServers
# image.discomap.eea.europa.eu/arcgis/rest/services/GioLandPublic/
# HRL_DominantLeafType2018 and HRL_TreeCoverDensity_2018, `exportImage` in
# EPSG:3035 at 10 m (nearest neighbour), no account needed (the CLMS
# portal's downloads need an EU Login; not used). Licence: Copernicus Land
# Monitoring Service data policy – free, full and open; the source must be
# named and modifications stated: "© European Union, Copernicus Land
# Monitoring Service 2018, European Environment Agency (EEA)".
#
# Environment: OCW_JOBS (parallel downloads, default 4). Each run appends
# the sources to data/<region>/SOURCES.md.
set -euo pipefail
cd "$(dirname "$0")/.."
region=${1:?usage: fetch-forest.sh liechtenstein|blackforest|tenerife|andorra [PACKAGE...]}
shift
pkgs=("$@")
case "$region" in
  liechtenstein) dir=data/pilot; def=("$dir/liechtenstein.ocwpkg") ;;
  blackforest)
    dir=data/blackforest
    def=("$dir/blackforest.ocwpkg" "$dir/blackforest-middle.ocwpkg" "$dir/blackforest-north.ocwpkg")
    ;;
  tenerife) dir=data/tenerife; def=("$dir/tenerife.ocwpkg") ;;
  andorra) dir=data/andorra; def=("$dir/andorra.ocwpkg") ;;
  *) echo "unknown region '$region' (liechtenstein, blackforest, tenerife, andorra)" >&2; exit 1 ;;
esac
if [ ${#pkgs[@]} -eq 0 ]; then
  for p in "${def[@]}"; do [ -f "$p" ] && pkgs+=("$p"); done
fi
[ ${#pkgs[@]} -gt 0 ] || { echo "no package of $region: build it first" >&2; exit 1; }
for pkg in "${pkgs[@]}"; do
  [ -f "$pkg" ] || { echo "$pkg missing: build the package first" >&2; exit 1; }
done
cargo build -q --release -p world-builder
WB=target/release/world-builder
out=$dir/forest
mkdir -p "$out"
for pkg in "${pkgs[@]}"; do
  $WB imagery-cells --world "$pkg" --crs laea --cell 1000 --margin 200
done | sort -u > "$out/cells-laea.txt"
echo "$(wc -l < "$out/cells-laea.txt") cells of 1 km (EPSG:3035) for ${pkgs[*]}"
python3 -I scripts/forest.py fetch --out "$out" --cells "$out/cells-laea.txt" --jobs "${OCW_JOBS:-4}"
mb=$(du -sm "$out" | cut -f1)
{
  echo
  echo "## Forest type and density (scripts/fetch-forest.sh $region, $(date -u +%F))"
  echo
  echo "| Folder | Source | Licence and notice |"
  echo "|---|---|---|"
  echo "| forest/leaf-type.u8 (10 m, EPSG:3035, raw 8-bit, with density.u8 $mb MB) | Copernicus HRL Dominant Leaf Type 2018: EEA ImageServer GioLandPublic/HRL_DominantLeafType2018 (exportImage) | Copernicus Land Monitoring Service data policy (free, full and open): \"© European Union, Copernicus Land Monitoring Service 2018, European Environment Agency (EEA)\" |"
  echo "| forest/density.u8 (10 m, EPSG:3035, raw 8-bit) | Copernicus HRL Tree Cover Density 2018: EEA ImageServer GioLandPublic/HRL_TreeCoverDensity_2018 (exportImage) | as above |"
} >> "$dir/SOURCES.md"
du -sh "$out"
