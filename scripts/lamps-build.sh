#!/usr/bin/env bash
# Rebuilds the live world packages with the street lamps mapped in OSM
# (`world-builder street-lamps`, world look *Street lamps*) into new
# folders – the live ones stay as they are – and lints each for the lamp
# rule (`lamp.*`). On marge:
#
#   scripts/lamps-build.sh [OUT_DIR] [PACKAGE_NAME...]
#
# OUT_DIR defaults to ~/src/OCW/data/lamps-build; the live packages come
# from OCW_WORLD_DATA (default ~/src/OCW/data). Logs: OUT_DIR/*.log.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
out=${1:-$HOME/src/OCW/data/lamps-build}
shift || true
wb=${OCW_WORLD_BUILDER:-target/release/world-builder}
[ -x "$wb" ] || cargo build --release -p world-builder
bf="$data/blackforest/freiburg-regbez.osm.pbf $data/blackforest/karlsruhe-regbez.osm.pbf"
# name | live package | PBF extracts
worlds=(
  "liechtenstein|pilot/liechtenstein.ocwpkg|$data/pilot/liechtenstein.osm.pbf"
  "tenerife|tenerife/tenerife.ocwpkg|$data/tenerife/canary-islands.osm.pbf"
  "blackforest-north|blackforest/blackforest-north.ocwpkg|$bf"
  "blackforest-middle|blackforest/blackforest-middle.ocwpkg|$bf"
  "blackforest|blackforest/blackforest.ocwpkg|$bf"
)
want=("$@")
for w in "${worlds[@]}"; do
  IFS='|' read -r name rel pbfs <<<"$w"
  if [ ${#want[@]} -gt 0 ] && [[ ! " ${want[*]} " == *" $name "* ]]; then
    continue
  fi
  src=$data/$rel
  dst=$out/$rel
  mkdir -p "$(dirname "$dst")"
  args=()
  for p in $pbfs; do args+=(--pbf "$p"); done
  echo "== $name"
  "$wb" street-lamps --world "$src" "${args[@]}" --out "$dst" >"$out/$name.build.log" 2>&1
  grep -E "street lamps|world obstacles|street lamp poles|mapped street lamps" "$out/$name.build.log" || true
  "$wb" lint --world "$dst" >"$out/$name.lint.log" 2>&1 || true
  grep -E "^ *lamp\." "$out/$name.lint.log" || grep -E "lamp" "$out/$name.lint.log" | head -20 || true
done
