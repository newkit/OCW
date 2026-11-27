#!/usr/bin/env bash
# Buses on their lines and parked cars (spec 14 Phase 18, 18B): the
# Liechtenstein package with its TRFC section, normal road traffic in
# Vaduz by day, varied riders, on X11 and Wayland (headless, software
# Vulkan; drive.sh archives them in test-output/):
#   - a bus standing at its stop with the doors open, the camera beside it
#     (`traffic.bus watch` picks the bus and places the camera);
#   - parked cars along a residential street, the rider riding past;
#   - a short video at a stop: a bus comes in, opens its doors, closes
#     them and pulls out (`traffic.bus coming`).
#
#   scripts/bus-shots.sh [PKG] [shots|videos|all]
#
# PKG defaults to the traffic build of the package
# (~/src/OCW/data/traffic-build/liechtenstein.ocwpkg, else data/pilot/).
set -euo pipefail
cd "$(dirname "$0")/.."
pkg=${1:-}
what=${2:-all}
if [ -z "$pkg" ]; then
  for p in "$HOME/src/OCW/data/traffic-build/liechtenstein.ocwpkg" data/traffic-build/liechtenstein.ocwpkg data/pilot/liechtenstein.ocwpkg; do
    [ -f "$p" ] && { pkg=$p; break; }
  done
fi
[ -f "$pkg" ] || { echo "no Liechtenstein package" >&2; exit 1; }
drive=.claude/skills/run-ocw/drive.sh
out=${OCW_BUS_OUT:-$PWD/target/bus-shots}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
# Vaduz centre (the T7 pictures' start), and a residential street with
# parked cars (`traffic_bench --buses` lists the streets with the most).
vaduz=(--world "$pkg" --no-neighbours --free-ride --start -2430,2220 --bots 2)
street_at=${OCW_PARKED_AT:--2762,1859}

# Common: the time paused at TIME, clear, normal traffic, a rider's look.
common() {
  local time=$1 rider=$2 bike=$3
  printf '%s\n' --at "0.5=time pause" --at "0.5=time $time" --at "0.5=weather clear" \
    --at "0.6=traffic road normal" --at "0.7=character $rider" \
    --at "1=dev.bot_bike 1 --bike $bike" --at "1.2=hud.profile off"
}

# The camera beside a bus at its stop (`traffic.bus watch`: a fixed
# camera; `--at` times are game time, and software rendering on X11 runs at
# a few frames a second, so the camera is re-placed every 3 s until long
# after the picture is due: whenever it is taken, a bus was picked moments
# ago).
watch() {
  local mode=${1:-watch} t=8
  while [ "$t" -lt 240 ]; do
    printf '%s\n' --at "$t=traffic.bus $mode"
    t=$((t + 3))
  done
}

if [ "$what" = shots ] || [ "$what" = all ]; then
  mapfile -t a < <(common "2026-06-10 10:30" "--body b --hair ponytail --hair-colour #2B1B14 --skin 9 --glasses 2" gravel; watch)
  "$drive" shot "$out/bus-stop-doors-day-x11.jpg" 150 -- "${vaduz[@]}" --power 0 "${a[@]}"
  mapfile -t a < <(common "2026-06-10 15:10" "--body a --hair short --hair-colour #8D5524 --skin 5 --beard 2" road; watch)
  "$drive" shot-wayland "$out/bus-stop-doors-day-wayland.jpg" 40 -- "${vaduz[@]}" --power 0 "${a[@]}"
  mapfile -t a < <(common "2026-06-10 11:20" "--body b --hair bun --hair-colour #B3482A --skin 3" mtb)
  "$drive" shot "$out/parked-cars-street-day-x11.jpg" 90 -- --world "$pkg" --no-neighbours \
    --free-ride --start "$street_at" --bots 1 --power 140 "${a[@]}" --at "3=camera.orbit 120 12 14"
  mapfile -t a < <(common "2026-06-10 16:40" "--body a --hair long --hair-colour #6A4E35 --skin 7" gravel)
  "$drive" shot-wayland "$out/parked-cars-street-day-wayland.jpg" 24 -- --world "$pkg" --no-neighbours \
    --free-ride --start "$street_at" --bots 1 --power 120 "${a[@]}" --at "3=camera.orbit 230 30 26"
fi
if [ "$what" = videos ] || [ "$what" = all ]; then
  mapfile -t a < <(common "2026-06-10 10:50" "--body b --hair short --hair-colour #3B2A1A --skin 4" road)
  # The rider on a side street (it does not hold up the buses), the
  # camera at the stop of a bus coming.
  OCW_SIZE=1280x720 "$drive" video-wayland "$out/bus-stop-video-wayland.mp4" 60 -- --world "$pkg" \
    --no-neighbours --free-ride --start "$street_at" --bots 1 --power 0 "${a[@]}" \
    --at "8=traffic.bus coming" --capture-warmup 9
fi
ls -la "$out"
