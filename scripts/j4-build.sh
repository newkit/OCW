#!/usr/bin/env bash
# Rebuilds the live world packages into data/j4-build/ (spec 4.8.8 Q7 (d),
# v5.52) on a build machine, with everything the separate builds added, so
# one set has it all: the road condition from OSM (section RCND, `world-builder
# condition`, as data/j3-build), the junctions with Q7 (`world-builder
# junctions`: creases rounded into vertical curves, never above the main
# road, blended junctions on one plane; the mini-roundabouts), the light
# terrain smoothing (`world-builder smooth-terrain`) and the street lamps
# mapped in OSM (`world-builder street-lamps`, as data/lamps-build). The
# manifests and the route files beside them; the lint of the new package
# next to the J3 one's (data/j3-build, linted again with this lint for the
# new checks). The live packages are only read.
# Usage: scripts/j4-build.sh [PKG...]  (default: all five)
#   OCW_J4_OUT=DIR  another output folder, LINT=0 no lint
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J4_OUT:-$D/j4-build}
J3=${OCW_J3_OUT:-$D/j3-build}
WB=${WB:-target/release/world-builder}
[ -n "${WB_PREBUILT:-}" ] || cargo build --release -p world-builder
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
keys='^(road\.vertical\.(kink|step|grade_over_limit)|junction\.(seam|measured|overlap|mouth)|lamp\.(foot|osm\.placed|placed))'
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
  echo "build $(echo "$end - $start" | bc) s, junctions peak $(grep -E '^total ' "$OUT/$p.junctions.log" | awk '{print $NF}') GB"
  ls -l "$D/$p.ocwpkg" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$OUT/$p.ocwpkg"
  if [ "${LINT:-1}" = 1 ]; then
    "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j4.txt" 2>&1
    [ -s "$OUT/$p.lint-j3.txt" ] || "$WB" lint --world "$J3/$p.ocwpkg" > "$OUT/$p.lint-j3.txt" 2>&1
    for f in j3 j4; do
      echo "-- lint $f"
      grep -E "$keys" "$OUT/$p.lint-$f.txt" || true
    done
  fi
done
