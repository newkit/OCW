#!/usr/bin/env bash
# Before/after comparison of the world cleaning (spec §4.8): builds a package
# with `--no-clean` and with the cleaning, and lints both. Run on a build
# machine (scripts/remote.sh marge run scripts/world-clean-compare.sh li|bf).
#
#   li   Liechtenstein from data/pilot/ (or $OCW_PILOT_DIR)
#   bf   the southern Black Forest from data/blackforest/ (or $OCW_BF_DIR),
#        box and origin as scripts/build-blackforest.sh
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
    pbf=$D/liechtenstein.osm.pbf
    SRC=(--pbf "$pbf" --poly "$D/liechtenstein.poly" --hgt "$D/N47E009.hgt"
         --id europe/liechtenstein --name Liechtenstein)
    ;;
  bf)
    D=${OCW_BF_DIR:-data/blackforest}
    name=blackforest
    pbf=$D/freiburg-regbez.osm.pbf
    SRC=(--pbf "$pbf" --poly "$D/freiburg-regbez.poly"
         --hgt "$D/N47E007.hgt" --hgt "$D/N47E008.hgt" --hgt "$D/N48E007.hgt" --hgt "$D/N48E008.hgt"
         --bbox 47.72,7.62,48.12,8.30 --origin 47.92,7.96
         --id europe/germany/black-forest --name "Black Forest")
    ;;
  *) echo "unknown package $which" >&2; exit 1 ;;
esac
$WB package "${SRC[@]}" --no-clean --out "data/clean/$name-before.ocwpkg" > "data/clean/$name-before.log" 2>&1
$WB package "${SRC[@]}" --out "data/clean/$name.ocwpkg" > "data/clean/$name.log" 2>&1
cat "data/clean/$name.log"
for x in "$name-before" "$name"; do
  $WB lint --world "data/clean/$x.ocwpkg" --pbf "$pbf" > "data/clean/$x.lint" 2>&1 &
done
wait
$WB check --world "data/clean/$name.ocwpkg" | tail -3
$WB verify --allow-unsigned "data/clean/$name.ocwpkg" || true
for x in "$name-before" "$name"; do echo "== $x"; grep -v '^    ' "data/clean/$x.lint"; done
