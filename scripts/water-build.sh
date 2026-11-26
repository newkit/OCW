#!/usr/bin/env bash
# Rebuilds the live world packages into data/water-build/ with the
# shorelines and rivers of spec 4.6 *Water surfaces* (v6.58): `world-builder
# shores`, then `world-builder pylons` (the towers of dismantled power lines
# out of OBST), on the J9 packages (data/j9-build/, Andorra's from data/andorra/)
# – the terrain fitted to the OSM outlines of lakes, riverbanks and the
# sea, the rivers and streams as ribbons along their course (section
# WATR). The manifests and the route files beside them; the lint of the
# new package next to the J9 one's. The input packages are only read.
# Usage: scripts/water-build.sh [PKG...]  (default: all six)
#   OCW_WATER_OUT=DIR  another output folder, LINT=0 no lint
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_WATER_OUT:-$D/water-build}
WB=${WB:-target/release/world-builder}
[ -n "${WB_PREBUILT:-}" ] || cargo build --release -p world-builder
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest andorra/andorra)
keys='^(water\.|road\.vertical\.(kink|step|grade_over_limit)|junction\.(seam|measured|overlap|mouth|no_surface)|side\.|lamp\.(foot|osm\.placed|placed)|building\.)'
for p in "${pkgs[@]}"; do
  dir=${p%/*}
  src="$D/j9-build/$p.ocwpkg"
  [ -e "$src" ] || src="$D/$p.ocwpkg"
  mkdir -p "$OUT/$dir"
  cp -p "$D/$dir"/*.route "$OUT/$dir/" 2>/dev/null || true
  cp -p "$(dirname "$src")"/*.route "$OUT/$dir/" 2>/dev/null || true
  echo "== $p (from $src)"
  start=$(date +%s.%N)
  pbf=()
  for f in "$D/$dir"/*.osm.pbf; do [ -e "$f" ] && pbf+=(--pbf "$f"); done
  "$WB" shores --world "$src" --out "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.shores.log" 2>&1
  grep -E "^(clean: shorelines|clean: rivers|shore offset|worst wide)" "$OUT/$p.shores.log" || tail -5 "$OUT/$p.shores.log"
  # The towers of dismantled power lines (v6.58: the Envalira's phantom pylons).
  "$WB" pylons --world "$OUT/$p.ocwpkg" --out "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.pylons.log" 2>&1
  grep -E "^pylons:" "$OUT/$p.pylons.log" || tail -3 "$OUT/$p.pylons.log"
  end=$(date +%s.%N)
  echo "build $(echo "$end - $start" | bc) s"
  ls -l "$src" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$OUT/$p.ocwpkg"
  if [ "${LINT:-1}" = 1 ]; then
    "$WB" lint --world "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.lint-water.txt" 2>&1
    [ -e "$OUT/$p.lint-before.txt" ] || "$WB" lint --world "$src" "${pbf[@]}" > "$OUT/$p.lint-before.txt" 2>&1
    for f in before water; do
      echo "-- lint $f"
      grep -E "$keys" "$OUT/$p.lint-$f.txt" | sed 's/, [0-9]* m$//' || true
    done
  fi
done
