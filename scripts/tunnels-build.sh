#!/usr/bin/env bash
# Rebuilds the live world packages with their tunnels shaped into the
# terrain and checked (`world-builder tunnels`, spec 4.3 *Tunnels and
# bridges in the terrain*; the step scripts/j9-build.sh runs too) into new
# folders – the live ones stay as they are – with the measure before and
# after and the lint's `tunnel.*`. On marge:
#
#   scripts/tunnels-build.sh [OUT_DIR] [PACKAGE_NAME...]
#
# OUT_DIR defaults to ~/src/OCW/data/tunnels-build; the live packages come
# from OCW_WORLD_DATA (default ~/src/OCW/data). Logs: OUT_DIR/*.log. Exits
# 1 when the tunnel check fails for a package (a tube still out of the
# terrain).
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
out=${1:-$HOME/src/OCW/data/tunnels-build}
shift || true
wb=${OCW_WORLD_BUILDER:-target/release/world-builder}
[ -x "$wb" ] || cargo build --release -p world-builder
bf="$data/blackforest/freiburg-regbez.osm.pbf $data/blackforest/karlsruhe-regbez.osm.pbf"
# name | live package | PBF extracts
worlds=(
  "andorra|andorra/andorra.ocwpkg|$data/andorra/andorra.osm.pbf"
  "liechtenstein|pilot/liechtenstein.ocwpkg|$data/pilot/liechtenstein.osm.pbf"
  "tenerife|tenerife/tenerife.ocwpkg|$data/tenerife/canary-islands.osm.pbf"
  "blackforest-north|blackforest/blackforest-north.ocwpkg|$bf"
  "blackforest-middle|blackforest/blackforest-middle.ocwpkg|$bf"
  "blackforest|blackforest/blackforest.ocwpkg|$bf"
)
want=("$@")
failed=0
for w in "${worlds[@]}"; do
  IFS='|' read -r name rel pbfs <<<"$w"
  if [ ${#want[@]} -gt 0 ] && [[ ! " ${want[*]} " == *" $name "* ]]; then
    continue
  fi
  src=$data/$rel
  dst=$out/$rel
  mkdir -p "$(dirname "$dst")"
  cp -p "$(dirname "$src")"/*.route "$(dirname "$dst")/" 2>/dev/null || true
  args=()
  for p in $pbfs; do [ -e "$p" ] && args+=(--pbf "$p"); done
  echo "== $name"
  if ! "$wb" tunnels --world "$src" "${args[@]}" --out "$dst" >"$out/$name.tunnels.log" 2>&1; then
    failed=1
  fi
  grep -E "^(tunnels:|tunnels shaped|after shaping|tunnel check|buried roads)" "$out/$name.tunnels.log" || tail -5 "$out/$name.tunnels.log"
  "$wb" tunnels --world "$dst" "${args[@]}" --report-only >"$out/$name.after.log" 2>&1 || true
  "$wb" lint --world "$dst" >"$out/$name.lint.log" 2>&1 || true
  grep -E "^ *tunnel\." "$out/$name.lint.log" || true
  sha256sum "$dst"
done
exit $failed
