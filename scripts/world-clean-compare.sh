#!/usr/bin/env bash
# Before/after comparison of the world cleaning (spec §4.8): builds a package
# with `--no-clean` and with the cleaning, and lints both. Run on a build
# machine (scripts/remote.sh marge run scripts/world-clean-compare.sh li|bf).
#
#   li   Liechtenstein from data/pilot/ (or $OCW_PILOT_DIR), with swissALTI3D
#        if present (as scripts/build-liechtenstein.sh)
#   bf   the southern Black Forest from data/blackforest/ (or $OCW_BF_DIR),
#        inputs, box and origin as scripts/build-blackforest.sh (DGM1 if present)
#
# Writes data/clean/<name>-before.ocwpkg, data/clean/<name>.ocwpkg, their
# build logs (.log) and lint reports (.lint).
set -euo pipefail
cd "$(dirname "$0")/.."
which=${1:?usage: world-clean-compare.sh li|bf}
cargo build -q --release -p world-builder
WB=target/release/world-builder
mkdir -p data/clean
case "$which" in
  li)
    D=${OCW_PILOT_DIR:-data/pilot}
    name=liechtenstein
    PBF=(--pbf "$D/liechtenstein.osm.pbf")
    SRC=("${PBF[@]}" --poly "$D/liechtenstein.poly" --hgt "$D/N47E009.hgt"
         --id europe/liechtenstein --name Liechtenstein)
    if compgen -G "$D/swissalti3d/*.tif" > /dev/null; then
      SRC+=(--dem "$D/swissalti3d" --dem-attribution swissalti3d --dem-attribution srtm)
    fi
    ;;
  bf)
    D=${OCW_BF_DIR:-data/blackforest}
    name=blackforest
    PBF=(--pbf "$D/freiburg-regbez.osm.pbf" --pbf "$D/karlsruhe-regbez.osm.pbf")
    SRC=("${PBF[@]}" --poly "$D/freiburg-regbez.poly" --poly "$D/karlsruhe-regbez.poly"
         --hgt "$D/N47E007.hgt" --hgt "$D/N47E008.hgt" --hgt "$D/N48E007.hgt" --hgt "$D/N48E008.hgt"
         --bbox 47.72,7.62,48.12,8.30 --origin 47.92,7.96
         --id europe/germany/black-forest --name "Black Forest")
    if compgen -G "$D/dgm1/*.tif" > /dev/null; then
      SRC+=(--dem "$D/dgm1" --dem-attribution lgl-dgm1 --dem-attribution srtm)
    fi
    ;;
  *) echo "unknown package $which" >&2; exit 1 ;;
esac
$WB package "${SRC[@]}" --no-clean --out "data/clean/$name-before.ocwpkg" > "data/clean/$name-before.log" 2>&1
$WB package "${SRC[@]}" --out "data/clean/$name.ocwpkg" > "data/clean/$name.log" 2>&1
cat "data/clean/$name.log"
for x in "$name-before" "$name"; do
  $WB lint --world "data/clean/$x.ocwpkg" "${PBF[@]}" > "data/clean/$x.lint" 2>&1 &
done
wait
$WB check --world "data/clean/$name.ocwpkg" | tail -3
$WB verify --allow-unsigned "data/clean/$name.ocwpkg" || true
for x in "$name-before" "$name"; do echo "== $x"; grep -v '^    ' "data/clean/$x.lint"; done
