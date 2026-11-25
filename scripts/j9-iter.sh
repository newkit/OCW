#!/usr/bin/env bash
# One quick J9 round on a build machine (spec 4.8.8 Rule, J9 working tool):
# the live package's road condition (once, cached in $HOME/j9), this
# checkout's junctions, the lint and the side findings by cause.
# Usage: scripts/j9-iter.sh TAG [PKG]   (default pilot/liechtenstein)
set -euo pipefail
export LC_ALL=C
tag=$1 p=${2:-pilot/liechtenstein}
D=${OCW_DATA:-$HOME/src/OCW/data} W=$HOME/j9
dir=${p%/*} name=${p#*/}
mkdir -p "$W"
cargo build --release -p world-builder 2>&1 | tail -1
cargo build --release -p ocw-worldgen --example side_dump 2>&1 | tail -1
WB=target/release/world-builder
pbf=()
for f in "$D/$dir"/*.osm.pbf; do [ -e "$f" ] && pbf+=(--pbf "$f"); done
[ -s "$W/$name.cond.ocwpkg" ] || "$WB" condition --world "$D/$p.ocwpkg" --out "$W/$name.cond.ocwpkg" "${pbf[@]}" > "$W/$name.cond.log" 2>&1
s=$(date +%s)
"$WB" junctions --world "$W/$name.cond.ocwpkg" --out "$W/$name.$tag.ocwpkg" "${pbf[@]}" > "$W/$name.$tag.junctions.log" 2>&1
echo "junctions $(( $(date +%s) - s )) s"
"$WB" lint --world "$W/$name.$tag.ocwpkg" > "$W/$name.$tag.lint.txt" 2>&1
grep -E '^(road\.vertical\.(kink|step|grade_over_limit)|junction\.(seam|overlap|mouth|no_surface)|side\.)' "$W/$name.$tag.lint.txt" | sed 's/, [0-9]* m$//' || true
target/release/examples/side_dump "$W/$name.$tag.ocwpkg" "$W/$name.$tag.jsonl" 2>&1 | grep -E "^patches" || true
python3 scripts/side-classify.py "$W/$name.$tag.jsonl" | grep -v "^{"
