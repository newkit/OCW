#!/usr/bin/env bash
# Downloads the open imagery of a world region for the region calibration
# (spec §14 Phase 8 T3, 8B) into data/<region>/imagery/ (git-ignored): only
# the cells the built package covers. Run on the build machine after the
# package is built (it lists the cells), then `world-builder calibrate`:
#
#   scripts/fetch-imagery.sh REGION [PACKAGE...]
#
# With several packages (the Black Forest's parts) the cells of all of them
# are fetched and the Sentinel-2 grid covers them all; imagery/packages.txt
# lists the packages covered (the build scripts calibrate only those).
#
# Regions and sources (licences checked on the dataset pages 2026-11-13):
#   liechtenstein  data/pilot/imagery/
#     1-li-orthophoto-2025: Orthophoto Liechtenstein 2025 (SWISSIMAGE,
#       10 cm, flown in summer 2025) of the Amt für Tiefbau und
#       Geoinformation, opendata.swiss "Opendata BY" (open use, source must
#       be named); WMS service.geo.llv.li layer li.atg.orthophoto2025, LV95,
#       requested at 0.5 m (the WMS covers the Swiss Rhine valley too).
#     2-swissimage: swisstopo SWISSIMAGE 10 cm (OGD, source must be named)
#       via wms.geo.admin.ch, LV95, 0.5 m: where the Liechtenstein service
#       has no data.
#   blackforest    data/blackforest/imagery/   (default: the south package)
#     1-lgl-dop20: LGL DOP20 (20 cm digital orthophotos of
#       Baden-Württemberg), Datenlizenz Deutschland – Namensnennung 2.0;
#       WMS owsproxy.lgl-bw.de WMS_LGL-BW_ATKIS_DOP_20_C, UTM 32 (ETRS89),
#       requested at 1 m.
#   tenerife       data/tenerife/imagery/
#     1-pnoa: IGN PNOA orthophotos of the latest flight ("máxima
#       actualidad", 25–50 cm), CC BY 4.0 scne.es; WMS
#       www.ign.es/wms-inspire/pnoa-ma layer OI.OrthoimageCoverage,
#       REGCAN95 / UTM 28N (EPSG:4083), requested at 1 m. GRAFCAN's
#       OrtoExpress needs an express authorisation: not used.
#   every region   sentinel-2: our own Sentinel-2 L2A seasonal median
#     composites (spring 1 Apr–31 May, summer 15 Jun–31 Aug, autumn
#     25 Sep–10 Nov, winter 1 Dec–28 Feb; 2023–2025, the 12 clearest scenes
#     per MGRS tile and season, clouds and shadows masked with the scene
#     classification) at 20 m from the true-colour COGs of the Copernicus
#     Sentinel-2 Collection 1 L2A products (Copernicus Sentinel data
#     licence: free, full and open; "Contains modified Copernicus Sentinel
#     data"). Read from the AWS Open Data copy (Earth Search STAC,
#     no account needed); the Copernicus Data Space Ecosystem serves the
#     same products but needs a registered account. No EOX mosaics
#     (2018–2025 are CC BY-NC-SA, spec 8A).
#
# Environment: OCW_S2_PIXEL (default 20 m), OCW_S2_SCENES (12 per MGRS tile
# and season), OCW_S2_YEARS (2023-2025), OCW_ORTHO_PIXEL (overrides the
# per-region resolution), OCW_JOBS (parallel downloads, default 4).
# Each run appends the sources to data/<region>/SOURCES.md.
set -euo pipefail
cd "$(dirname "$0")/.."
region=${1:?usage: fetch-imagery.sh liechtenstein|blackforest|tenerife [PACKAGE...]}
shift
pkgs=("$@")
WB=target/release/world-builder
year=$(date -u +%Y)
s2_pixel=${OCW_S2_PIXEL:-20}
jobs=${OCW_JOBS:-4}
case "$region" in
  liechtenstein)
    dir=data/pilot; def=$dir/liechtenstein.ocwpkg
    crs=lv95; epsg=2056; s2crs=utm32; s2epsg=32632
    ;;
  blackforest)
    dir=data/blackforest; def=$dir/blackforest.ocwpkg
    crs=utm32; epsg=25832; s2crs=utm32; s2epsg=32632
    ;;
  tenerife)
    dir=data/tenerife; def=$dir/tenerife.ocwpkg
    crs=utm28; epsg=4083; s2crs=utm28; s2epsg=32628
    ;;
  *) echo "unknown region '$region' (liechtenstein, blackforest, tenerife)" >&2; exit 1 ;;
esac
[ ${#pkgs[@]} -gt 0 ] || pkgs=("$def")
for pkg in "${pkgs[@]}"; do
  [ -f "$pkg" ] || { echo "$pkg missing: build the package first" >&2; exit 1; }
done
cargo build -q --release -p world-builder
out=$dir/imagery
mkdir -p "$out"
for c in "$crs" "$s2crs"; do
  for pkg in "${pkgs[@]}"; do
    $WB imagery-cells --world "$pkg" --crs "$c" --cell 1000
  done | sort -u > "$out/cells-$c.txt"
done
echo "$(wc -l < "$out/cells-$crs.txt") orthophoto cells of 1 km for ${pkgs[*]}"
py() { python3 -I scripts/imagery.py "$@"; }
rows=""
ortho() { # folder wms layer pixel id name licence notice url date
  local folder=$1 wms=$2 layer=$3 pixel=${OCW_ORTHO_PIXEL:-$4}
  py ortho --out "$out/$folder" --cells "$out/cells-$crs.txt" --crs "$crs" --epsg "$epsg" \
    --pixel "$pixel" --jobs "$jobs" --wms "$wms" --layer "$layer" \
    --id "$5" --name "$6" --licence "$7" --notice "$8" --url "$9" --date "${10}" || true
  local n mb
  n=$(find "$out/$folder" -name '*.jpg' | wc -l)
  mb=$(du -sm "$out/$folder" | cut -f1)
  rows+="| imagery/$folder/ ($n cells of 1 km, $pixel m/px, JPEG, $mb MB) | $6: WMS $wms layer $layer, EPSG:$epsg | $7: \"$8\" |"$'\n'
}
case "$region" in
  liechtenstein)
    ortho 1-li-orthophoto-2025 "https://service.geo.llv.li/service/wms" li.atg.orthophoto2025 0.5 \
      li-orthophoto-2025 "Orthophoto Liechtenstein 2025 (SWISSIMAGE)" \
      "Opendata BY (opendata.swiss: open use, source must be named)" \
      "Orthophoto Liechtenstein 2025: Amt für Tiefbau und Geoinformation, Fürstentum Liechtenstein (geodaten.llv.li) / swisstopo SWISSIMAGE; Opendata BY" \
      "https://opendata.swiss/de/dataset/orthophoto-liechtenstein-2025" "2025 (summer flight)"
    ortho 2-swissimage "https://wms.geo.admin.ch/" ch.swisstopo.swissimage 0.5 \
      swissimage "swisstopo SWISSIMAGE 10 cm" \
      "swisstopo OGD (open use, source must be named)" \
      "© swisstopo: SWISSIMAGE 10 cm" \
      "https://www.swisstopo.admin.ch/en/terms-of-use-free-geodata-and-geoservices" "latest flight"
    ;;
  blackforest)
    ortho 1-lgl-dop20 "https://owsproxy.lgl-bw.de/owsproxy/ows/WMS_LGL-BW_ATKIS_DOP_20_C" IMAGES_DOP_20_RGB 1 \
      lgl-dop20 "LGL DOP20 Baden-Württemberg (digital orthophotos)" "dl-de/by-2-0" \
      "LGL-BW ($year) Datenlizenz Deutschland - Namensnennung - Version 2.0, www.lgl-bw.de (DOP20, resampled to 1 m and measured)" \
      "https://www.govdata.de/dl-de/by-2-0" "latest flights"
    ;;
  tenerife)
    ortho 1-pnoa "https://www.ign.es/wms-inspire/pnoa-ma" OI.OrthoimageCoverage 1 \
      ign-pnoa "IGN PNOA orthophotos (máxima actualidad)" "CC BY 4.0" \
      "PNOA CC-BY 4.0 scne.es (Instituto Geográfico Nacional de España; resampled to 1 m and measured)" \
      "https://creativecommons.org/licenses/by/4.0/" "latest flights"
    ;;
esac
years=${OCW_S2_YEARS:-2023-2025}
py seasonal --out "$out/sentinel-2" --cells "$out/cells-$s2crs.txt" --crs "$s2crs" --epsg "$s2epsg" \
  --pixel "$s2_pixel" --jobs "$jobs" --years "$years" --scenes "${OCW_S2_SCENES:-12}" \
  --id sentinel-2-l2a-seasonal --name "Copernicus Sentinel-2 L2A (own seasonal medians)" \
  --licence "Copernicus Sentinel data licence (free, full and open)" \
  --notice "Contains modified Copernicus Sentinel data $years (Sentinel-2 L2A, ESA; seasonal median composites made by OCW)" \
  --url "https://sentinels.copernicus.eu/documents/247904/690755/Sentinel_Data_Legal_Notice" \
  --date "$years"
s2mb=$(du -sm "$out/sentinel-2" | cut -f1)
rows+="| imagery/sentinel-2/ (spring, summer, autumn, winter medians, $s2_pixel m, raw RGB, $s2mb MB; scene ids in <season>.json) | Copernicus Sentinel-2 Collection 1 L2A true-colour COGs and scene classification, Earth Search STAC (AWS Open Data), $years | Copernicus Sentinel data licence: \"Contains modified Copernicus Sentinel data $years\" |"$'\n'
{
  echo
  echo "## Imagery (scripts/fetch-imagery.sh $region, $(date -u +%F))"
  echo
  echo "| Folder | Source | Licence and notice |"
  echo "|---|---|---|"
  printf '%s' "$rows"
} >> "$dir/SOURCES.md"
for pkg in "${pkgs[@]}"; do basename "$pkg"; done | sort -u > "$out/packages.txt"
du -sh "$out"/*/
