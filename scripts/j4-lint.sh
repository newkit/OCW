#!/usr/bin/env bash
# Lints the J4 packages again (data/j4-build, spec 4.8.8 Q7 and the Rule on
# the sides) with this checkout's world builder and prints the junction
# and side keys per package; LINT_J3=1 also the J3 packages.
#   scripts/j4-lint.sh [PKG...]
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_J4_OUT:-$D/j4-build}
WB=${WB:-target/release/world-builder}
cargo build --release -p world-builder 2>&1 | tail -1
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
keys='^(road\.vertical\.(kink|step|grade_over_limit)|junction\.(seam|overlap|mouth)|side\.|lamp\.(foot|osm\.placed))'
for p in "${pkgs[@]}"; do
  "$WB" lint --world "$OUT/$p.ocwpkg" > "$OUT/$p.lint-j4.txt" 2>&1
  if [ "${LINT_J3:-0}" = 1 ]; then
    "$WB" lint --world "$D/j3-build/$p.ocwpkg" > "$OUT/$p.lint-j3.txt" 2>&1
  fi
  echo "== $p"
  grep -E "$keys" "$OUT/$p.lint-j4.txt" | sed 's/, [0-9]* m$//'
done
