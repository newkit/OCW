#!/usr/bin/env bash
# The road traffic's frame time per quality level (spec 14 Phase 18, 18A
# budget table, 18E T7): busy traffic in Vaduz on the Liechtenstein
# package, the frame time with and without the traffic at Low, Medium and
# High on the machine's GPU (headless weston, scripts/measure-fps.sh),
# and the vehicles drawn (`traffic.boxes`: near, lod1, boxes, culled).
#
#   scripts/traffic-fps.sh [PKG] [SECONDS]
set -euo pipefail
cd "$(dirname "$0")/.."
pkg=${1:-}
seconds=${2:-40}
if [ -z "$pkg" ]; then
  for p in "$HOME/src/OCW/data/traffic-build/liechtenstein.ocwpkg" data/pilot/liechtenstein.ocwpkg; do
    [ -f "$p" ] && { pkg=$p; break; }
  done
fi
out=${OCW_TRAFFIC_OUT:-/tmp/ocw-traffic}
mkdir -p "$out"
base=(--world "$pkg" --no-neighbours --free-ride --start -2430,2220 --bots 3 --power 170
  --at "0.5=time pause" --at "0.5=time 2026-06-10 17:30" --at "0.5=weather clear"
  --at "2=camera.orbit 180 12 30")
for level in low medium high; do
  for traffic in busy off; do
    label="$level traffic=$traffic"
    log="$out/fps-$level-$traffic.log"
    scripts/measure-fps.sh --seconds "$seconds" --warmup 15 --label "$label" --log "$log" -- \
      "${base[@]}" --at "0.4=set display.quality $level" --at "0.6=traffic road $traffic" \
      --at "14=traffic.boxes" --at "$((seconds - 5))=traffic.boxes"
    sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -o 'traffic boxes: .*' | tail -1 || true
  done
done
