#!/usr/bin/env bash
# Plan views of junctions' drawn sides (spec 4.8.8 Rule) on a build machine:
# per "NAME X,Z [RADIUS]" argument the triangles round that point as the
# renderer draws them from PKG (`junction_verges_dump`, ocw-render's
# junction_seams test) plotted by scripts/verge-plot.py into OUT/NAME.png.
# Usage: scripts/side-plans.sh PKG OUT "NAME X,Z [R]"...
set -euo pipefail
pkg=$(realpath "$1") out=$(realpath -m "$2")
shift 2
mkdir -p "$out"
cargo test --release -p ocw-render --test junction_seams --no-run 2>&1 | tail -1
for spec in "$@"; do
  read -r name at r <<<"$spec"
  r=${r:-14}
  OCW_JUNCTION_WORLD="$pkg" OCW_JUNCTION_AT="$at" OCW_VERGE_RADIUS="$r" \
    OCW_VERGE_DUMP="$out/$name.csv" \
    cargo test --release -p ocw-render --test junction_seams junction_verges_dump \
    -- --exact --nocapture > "$out/$name.log" 2>&1 || { tail -5 "$out/$name.log"; continue; }
  python3 scripts/verge-plot.py "$out/$name.csv" "$out/$name.png" "$at" "$r"
  echo "$out/$name.png"
done
