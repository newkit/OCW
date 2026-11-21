#!/usr/bin/env bash
# Rebuilds the live world packages' junctions into data/j2b-build/ (spec
# 4.8.8 J2b) on a build machine: `world-builder junctions` per package,
# the manifests and the route files beside them, the lint of the live and
# the rebuilt package. The live packages are only read.
# Usage: scripts/j2b-build.sh [PKG...]  (default: all five)
set -euo pipefail
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J2B_OUT:-$D/j2b-build}
WB=${WB:-target/release/world-builder}
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
for p in "${pkgs[@]}"; do
  dir=${p%/*}; name=${p#*/}
  mkdir -p "$OUT/$dir"
  cp -p "$D/$dir"/*.route "$OUT/$dir/" 2>/dev/null || true
  echo "== $p"
  "$WB" junctions --world "$D/$p.ocwpkg" --out "$OUT/$p.ocwpkg" > "$OUT/$p.junctions.log" 2>&1
  grep -E "^junctions:|^total" "$OUT/$p.junctions.log"
  ls -l "$D/$p.ocwpkg" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$OUT/$p.ocwpkg"
  if [ "${LINT:-1}" = 1 ]; then
    "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j2b.txt" 2>&1
    [ -s "$OUT/$p.lint-live.txt" ] || "$WB" lint --world "$D/$p.ocwpkg" > "$OUT/$p.lint-live.txt" 2>&1
    for f in live j2b; do
      echo "-- lint $f"
      grep -E "^(road.vertical|junction.seam|junction.measured|junction.overlap)" "$OUT/$p.lint-$f.txt" || true
    done
  fi
done
