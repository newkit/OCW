#!/usr/bin/env bash
# Demo of GPX import and route rides (spec §2.7): makes a GPX track from a
# package route (shortest main-road way between two points, sampled every
# 10 m with ~5 m of GPS-like noise), imports it with `world-builder gpx` into
# a copy of the package and prints the match report.
#
#   scripts/demo-gpx-route.sh [PKG] [OUT_PKG]
#
# Then ride it:  ocw-desktop --world OUT_PKG --route "Demo GPX"
set -euo pipefail
cd "$(dirname "$0")/.."
pkg=${1:-data/pilot/liechtenstein.ocwpkg}
out=${2:-data/pilot/liechtenstein-routes.ocwpkg}
wb() { cargo run --release -q -p world-builder -- "$@"; }
cp "$pkg" "$out"
# Vaduz centre to Schaan along the main roads.
wb route --world "$out" --from 47.1395,9.5220 --to 47.1650,9.5090 --name "Demo shortest"
wb routes --world "$out" --export-gpx
src="$(dirname "$out")/Demo_shortest.gpx"
noisy="$(dirname "$out")/demo-noisy.gpx"
# GPS noise: about ±5 m (deterministic).
awk 'BEGIN { srand(7) }
  /<trkpt/ {
    match($0, /lat="[^"]*"/); lat = substr($0, RSTART + 5, RLENGTH - 6)
    match($0, /lon="[^"]*"/); lon = substr($0, RSTART + 5, RLENGTH - 6)
    lat += (rand() - 0.5) * 9e-5; lon += (rand() - 0.5) * 1.3e-4
    printf "      <trkpt lat=\"%.7f\" lon=\"%.7f\"/>\n", lat, lon; next }
  { print }' "$src" > "$noisy"
wb routes --world "$out" --remove "Demo shortest"
wb gpx --world "$out" --gpx "$noisy" --name "Demo GPX" --route-file "$(dirname "$out")/demo.ocwroute"
echo "ride it: ocw-desktop --world $out --route \"Demo GPX\""
