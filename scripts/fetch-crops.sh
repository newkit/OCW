#!/usr/bin/env bash
# Downloads the measured crop map of a world region (spec §4.6.1, §14 Phase 8
# T8) into data/<region>/crops/ (git-ignored): OCW crop codes on a 10 m grid
# in ETRS89-LAEA Europe over the 1 km cells the built packages cover. Run on
# the build machine (GDAL Python bindings and numpy needed) after the
# packages are built; the build scripts then pass it to `world-builder
# vegetation --crops` (section VEGE):
#
#   scripts/fetch-crops.sh REGION [PACKAGE...]
#
# Regions and sources (checked 2026-11-15, no account needed):
#   blackforest  JRC EUCROPMAP 2022 (10 m, EU-27; CC BY 4.0, "© European
#                Union, Joint Research Centre"), read by HTTP range requests
#                from https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/EUCROPMAP/2022/
#   tenerife     Mapa de Cultivos de Canarias 2021, Tenerife (Gobierno de
#                Canarias, CC BY 4.0), shapefile from opendata.sitcan.es,
#                rasterised
#   liechtenstein  none: EUCROPMAP has no data there (not in the EU); OSM's
#                crops and the region's crop mix decide.
#
# Environment: OCW_JOBS (parallel range reads, default 4). Each run appends
# the sources to data/<region>/SOURCES.md.
set -euo pipefail
cd "$(dirname "$0")/.."
region=${1:?usage: fetch-crops.sh blackforest|tenerife [PACKAGE...]}
shift
pkgs=("$@")
case "$region" in
  blackforest)
    dir=data/blackforest
    def=("$dir/blackforest.ocwpkg" "$dir/blackforest-middle.ocwpkg" "$dir/blackforest-north.ocwpkg")
    ;;
  tenerife) dir=data/tenerife; def=("$dir/tenerife.ocwpkg") ;;
  liechtenstein) echo "no open crop map covers Liechtenstein (EUCROPMAP: EU only)"; exit 0 ;;
  *) echo "unknown region '$region' (blackforest, tenerife)" >&2; exit 1 ;;
esac
if [ ${#pkgs[@]} -eq 0 ]; then
  for p in "${def[@]}"; do [ -f "$p" ] && pkgs+=("$p"); done
fi
[ ${#pkgs[@]} -gt 0 ] || { echo "no package of $region: build it first" >&2; exit 1; }
cargo build -q --release -p world-builder
WB=target/release/world-builder
out=$dir/crops
mkdir -p "$out"
for pkg in "${pkgs[@]}"; do
  $WB imagery-cells --world "$pkg" --crs laea --cell 1000 --margin 200
done | sort -u > "$out/cells-laea.txt"
echo "$(wc -l < "$out/cells-laea.txt") cells of 1 km (EPSG:3035) for ${pkgs[*]}"
UA="OCW-world-builder/1 (open data import)"
case "$region" in
  blackforest)
    python3 -I scripts/crops.py eucropmap --out "$out" --cells "$out/cells-laea.txt" --jobs "${OCW_JOBS:-4}"
    src="JRC EUCROPMAP 2022, EU_CropMap_22_v1_stratum_EU27-HR.tif (HTTP range reads)"
    notice="EUCROPMAP 2022 © European Union, Joint Research Centre (JRC), CC BY 4.0"
    ;;
  tenerife)
    shp_dir=$dir/crops-src
    mkdir -p "$shp_dir"
    if [ ! -f "$shp_dir/TF_MCultivos_2021.shp" ]; then
      curl -fsSL -A "$UA" -o "$shp_dir/tf.zip" https://opendata.sitcan.es/upload/medio-rural/gobcan_mapa-cultivos_tf_shp.zip
      (cd "$shp_dir" && unzip -o -q tf.zip)
    fi
    python3 -I scripts/crops.py canarias --out "$out" --cells "$out/cells-laea.txt" --shp "$shp_dir/TF_MCultivos_2021.shp"
    src="Mapa de Cultivos de Canarias 2021, gobcan_mapa-cultivos_tf_shp.zip"
    notice="Mapa de Cultivos de Canarias 2021 © Gobierno de Canarias, CC BY 4.0"
    ;;
esac
{
  echo
  echo "## Crop map (scripts/fetch-crops.sh $region, $(date -u +%F))"
  echo
  echo "| Folder | Source | Licence and notice |"
  echo "|---|---|---|"
  echo "| crops/crops.u8 (10 m, EPSG:3035, OCW crop codes, $(du -sm "$out" | cut -f1) MB) | $src | CC BY 4.0: \"$notice\" |"
} >> "$dir/SOURCES.md"
du -sh "$out"
