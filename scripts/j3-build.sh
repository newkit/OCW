#!/usr/bin/env bash
# Rebuilds the live world packages' junctions into data/j3-build/ (spec
# 4.8.8 J3 with Q6 (d)) on a build machine: `world-builder junctions` per
# package, the manifests and the route files beside them, the lint of the
# rebuilt package next to the live one's and the J2b one's (if
# data/j2b-build has it). The live packages are only read.
# Usage: scripts/j3-build.sh [PKG...]  (default: all five)
#   OCW_J3_OUT=DIR  another output folder (iterations), LINT=0 no lint
set -euo pipefail
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J3_OUT:-$D/j3-build}
J2B=${OCW_J2B_OUT:-$D/j2b-build}
WB=${WB:-target/release/world-builder}
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
keys='^(road.vertical|junction\.(seam|measured|overlap|no_surface))'
for p in "${pkgs[@]}"; do
  dir=${p%/*}; name=${p#*/}
  mkdir -p "$OUT/$dir"
  cp -p "$D/$dir"/*.route "$OUT/$dir/" 2>/dev/null || true
  echo "== $p"
  start=$(date +%s.%N)
  "$WB" junctions --world "$D/$p.ocwpkg" --out "$OUT/$p.ocwpkg" > "$OUT/$p.junctions.log" 2>&1
  end=$(date +%s.%N)
  echo "build $(echo "$end - $start" | bc) s, peak $(grep -E '^total ' "$OUT/$p.junctions.log" | awk '{print $NF}') GB"
  grep -E "^junctions:|^total" "$OUT/$p.junctions.log" || true
  ls -l "$D/$p.ocwpkg" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$OUT/$p.ocwpkg"
  if [ "${LINT:-1}" = 1 ]; then
    "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j3.txt" 2>&1
    [ -s "$OUT/$p.lint-live.txt" ] || {
      if [ -s "$J2B/$p.lint-live.txt" ]; then cp "$J2B/$p.lint-live.txt" "$OUT/$p.lint-live.txt"
      else "$WB" lint --world "$D/$p.ocwpkg" > "$OUT/$p.lint-live.txt" 2>&1; fi
    }
    [ -s "$J2B/$p.lint-j2b.txt" ] && cp "$J2B/$p.lint-j2b.txt" "$OUT/$p.lint-j2b.txt"
    for f in live j2b j3; do
      [ -s "$OUT/$p.lint-$f.txt" ] || continue
      echo "-- lint $f"
      grep -E "$keys" "$OUT/$p.lint-$f.txt" || true
    done
  fi
done
