#!/usr/bin/env bash
# Rebuilds the live world packages into data/j5-build/ (spec 4.8.8 Rule: the
# sides of a road are as smooth as the road, v5.55) on a build machine with
# the steps of scripts/j4-build.sh: the road condition from OSM (`world-builder
# condition`), the junctions (`world-builder junctions`: J4's creases, planes
# and never-above cut, now with J5's sides – outlines along the real edge
# lines with rounded corners, mouth rows leaning with the main road or the
# blended plane, the sides smoothed along the outline), the light terrain
# smoothing (`world-builder smooth-terrain`) and the street lamps mapped in
# OSM (`world-builder street-lamps`). The manifests and the route files
# beside them; the lint of the new package next to the J4 one's
# (data/j4-build, linted again with this lint: the `side.*` measure changed).
# The live packages are only read.
# Usage: scripts/j5-build.sh [PKG...]  (default: all five)
#   OCW_J5_OUT=DIR  another output folder, LINT=0 no lint
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J5_OUT:-$D/j5-build}
J4=${OCW_J4_OUT:-$D/j4-build}
WB=${WB:-target/release/world-builder}
[ -n "${WB_PREBUILT:-}" ] || cargo build --release -p world-builder
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
keys='^(road\.vertical\.(kink|step|grade_over_limit)|junction\.(seam|measured|overlap|mouth|no_surface)|side\.|lamp\.(foot|osm\.placed|placed))'
for p in "${pkgs[@]}"; do
  dir=${p%/*}; name=${p#*/}
  mkdir -p "$OUT/$dir"
  cp -p "$D/$dir"/*.route "$OUT/$dir/" 2>/dev/null || true
  echo "== $p"
  start=$(date +%s.%N)
  pbf=()
  for f in "$D/$dir"/*.osm.pbf; do [ -e "$f" ] && pbf+=(--pbf "$f"); done
  "$WB" condition --world "$D/$p.ocwpkg" --out "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.condition.log" 2>&1
  grep -E "^road condition" "$OUT/$p.condition.log" || tail -3 "$OUT/$p.condition.log"
  "$WB" junctions --world "$OUT/$p.ocwpkg" --out "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.junctions.log" 2>&1
  grep -E "^junctions" "$OUT/$p.junctions.log" || true
  "$WB" smooth-terrain --world "$OUT/$p.ocwpkg" --out "$OUT/$p.ocwpkg" > "$OUT/$p.smooth.log" 2>&1
  grep -E "^clean: terrain smoothed" "$OUT/$p.smooth.log" || tail -3 "$OUT/$p.smooth.log"
  "$WB" street-lamps --world "$OUT/$p.ocwpkg" --out "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.lamps.log" 2>&1
  grep -E "street lamps|mapped street lamps" "$OUT/$p.lamps.log" || tail -3 "$OUT/$p.lamps.log"
  end=$(date +%s.%N)
  echo "build $(echo "$end - $start" | bc) s"
  ls -l "$D/$p.ocwpkg" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$OUT/$p.ocwpkg"
  if [ "${LINT:-1}" = 1 ]; then
    "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j5.txt" 2>&1
    "$WB" lint --world "$J4/$p.ocwpkg" > "$OUT/$p.lint-j4.txt" 2>&1
    for f in j4 j5; do
      echo "-- lint $f"
      grep -E "$keys" "$OUT/$p.lint-$f.txt" | sed 's/, [0-9]* m$//' || true
    done
  fi
done
