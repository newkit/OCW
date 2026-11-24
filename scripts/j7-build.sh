#!/usr/bin/env bash
# Rebuilds the live world packages into data/j7-build/ (spec 4.8.8 Rule: the
# sides of a road are as smooth as the road, J7) on a build machine with the
# steps of scripts/j6-build.sh: the road condition from OSM (`world-builder
# condition`), the junctions (`world-builder junctions`: J6's, with paths
# along a road drawn as a band of its side and taking no part in the
# junctions, corners cut short turning round), the light terrain smoothing
# (`world-builder smooth-terrain`) and the street lamps mapped in OSM
# (`world-builder street-lamps`). The manifests and the route files beside
# them; the lint of the new package next to the J6 one's (data/j6-build,
# its lint as J6's build wrote it). The live packages are only read.
# Usage: scripts/j7-build.sh [PKG...]  (default: all five)
#   OCW_J7_OUT=DIR  another output folder, LINT=0 no lint
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J7_OUT:-$D/j7-build}
J6=${OCW_J6_OUT:-$D/j6-build}
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
    "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j7.txt" 2>&1
    # The J6 package's lint as J6's build wrote it (this lint reads its
    # junction section, layout 2, as outdated and would compute J7's).
    cp "$J6/$p.lint-j6.txt" "$OUT/$p.lint-j6.txt" 2>/dev/null || true
    for f in j6 j7; do
      echo "-- lint $f"
      grep -E "$keys" "$OUT/$p.lint-$f.txt" | sed 's/, [0-9]* m$//' || true
    done
  fi
done
