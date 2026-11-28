#!/usr/bin/env bash
# Spec 14 Phase 18, 18E T11: the open traffic counts into the world
# packages. Run on the build machine that holds the worlds (marge), from a
# checkout: fetches each world's counting-station data once (cached in
# DATA/<world>/traffic-counts/, see tools/traffic-counts/fetch_counts.py),
# then writes the packages with the counts' flow factors to OUT (default
# DATA/traffic-build/) – only TRFC and the manifest change – with a map
# (SVG) and a station report (TSV) per package in MAPS. Nothing is
# installed. Andorra has no open counts (18D) and is left out.
#
#   scripts/build-traffic-counts.sh [DATA [OUT [MAPS]]]
#
# PYTHON: an interpreter with pyogrio (Liechtenstein's station positions
# are a file geodatabase), default python3.
set -euo pipefail
cd "$(dirname "$0")/.."
D=${1:-$HOME/src/OCW/data}
O=${2:-$D/traffic-build}
M=${3:-$O/maps}
PY=${PYTHON:-python3}
mkdir -p "$O" "$M"
for w in "tenerife tenerife" "liechtenstein pilot" "blackforest blackforest"; do
  set -- $w
  "$PY" -I tools/traffic-counts/fetch_counts.py "$1" "$D/$2"
done
cargo build -q --release -p world-builder
WB=$PWD/target/release/world-builder
run() {
  local name=$1 world=$2; shift 2
  if ! "$WB" traffic-counts --world "$world" --out "$O/$name.ocwpkg" \
      --svg "$M/$name.svg" --report "$M/$name.tsv" "$@" > "$O/$name.counts.log" 2>&1; then
    tail -20 "$O/$name.counts.log"; exit 1
  fi
  echo "== $name"; grep -v '^  ' "$O/$name.counts.log"
  # Every byte but TRFC's flows as before.
  python3 -I tools/traffic-counts/same_outside_trfc.py "$world" "$O/$name.ocwpkg"
}
run liechtenstein "$D/pilot/liechtenstein.ocwpkg" --pbf "$D/pilot/liechtenstein.osm.pbf" \
  --counts "$D/pilot/traffic-counts/li-miv.counts.csv" \
  --counts "$D/pilot/traffic-counts/astra-svz.counts.csv"
run tenerife "$D/tenerife/tenerife.ocwpkg" --pbf "$D/tenerife/canary-islands.osm.pbf" \
  --counts "$D/tenerife/traffic-counts/cabildo-tenerife-imd.counts.csv"
for b in blackforest blackforest-north blackforest-middle; do
  run "$b" "$D/blackforest/$b.ocwpkg" --pbf "$D/blackforest/freiburg-regbez.osm.pbf" \
    --pbf "$D/blackforest/karlsruhe-regbez.osm.pbf" \
    --counts "$D/blackforest/traffic-counts/bw-svz.counts.csv"
done
(cd "$O" && sha256sum liechtenstein.ocwpkg tenerife.ocwpkg blackforest.ocwpkg \
  blackforest-north.ocwpkg blackforest-middle.ocwpkg)
