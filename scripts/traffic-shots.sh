#!/usr/bin/env bash
# Road traffic pictures and videos (spec 14 Phase 18, 18E T7): the
# Liechtenstein package with its TRFC section, busy road traffic in Vaduz,
# varied riders, by day, at night and in the rain, on X11 and Wayland
# (headless, software Vulkan; drive.sh archives them in test-output/).
#
#   scripts/traffic-shots.sh [PKG] [shots|videos|all]
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
out=${OCW_TRAFFIC_OUT:-/tmp/ocw-traffic}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
base=(--world "$pkg" --no-neighbours --free-ride --start -2430,2220 --bots 3 --power 170)

# One scene: NAME TIME WEATHER RIDER BOTBIKE CAMERA
scene_args() {
  local time=$1 weather=$2 rider=$3 bike=$4 camera=$5
  printf '%s\n' --at "0.5=time pause" --at "0.5=time $time" --at "0.5=weather $weather" \
    --at "0.6=traffic road busy" --at "0.6=traffic manners mostly_polite" \
    --at "0.7=character $rider" --at "1=dev.bot_bike 1 --bike $bike" \
    --at "1=dev.bot_bike 2 --bike road" --at "1.2=hud.profile off" \
    --at "2=camera.orbit $camera" --at "10=traffic.boxes" --at "10.5=traffic"
}

shot() {
  local mode=$1 name=$2; shift 2
  mapfile -t extra < <(scene_args "$@")
  "$drive" "$mode" "$out/$name.jpg" "${OCW_SHOT_DELAY:-45}" -- "${base[@]}" "${extra[@]}"
}

video() {
  local mode=$1 name=$2 seconds=$3; shift 3
  mapfile -t extra < <(scene_args "$@")
  OCW_SIZE=1280x720 "$drive" "$mode" "$out/$name.mp4" "$seconds" -- "${base[@]}" "${extra[@]}" \
    --capture-warmup 10
}

if [ "$what" = shots ] || [ "$what" = all ]; then
  shot shot traffic-day-x11 "2026-06-10 12:30" clear \
    "--body b --hair ponytail --hair-colour #2B1B14 --skin 9 --glasses 2" gravel "200 14 26"
  shot shot-wayland traffic-day-wayland "2026-06-10 17:30" clear \
    "--body a --hair short --hair-colour #8D5524 --skin 5 --beard 2" mtb "160 10 22"
  shot shot traffic-night-x11 "2026-06-10 22:40" clear \
    "--body a --hair bun --hair-colour #B3482A --skin 2" road "210 12 24"
  shot shot-wayland traffic-night-wayland "2026-11-27 18:10" clear \
    "--body b --hair long --hair-colour auburn --skin 7 --glasses 1" gravel "150 9 20"
  shot shot traffic-rain-x11 "2026-06-10 17:40" rain \
    "--body a --hair short --hair-colour #3B2A1A --skin 4" mtb "190 11 22"
  shot shot-wayland traffic-rain-wayland "2026-11-27 16:20" heavy-rain \
    "--body b --hair ponytail --hair-colour #C9A35A --skin 3" road "170 9 18"
fi
if [ "$what" = videos ] || [ "$what" = all ]; then
  video video traffic-dusk-rain-video-x11 6 "2026-11-27 16:50" rain \
    "--body b --hair bun --hair-colour #2B1B14 --skin 8 --glasses 2" gravel "190 10 20"
  video video-wayland traffic-day-video-wayland 6 "2026-06-10 17:30" clear \
    "--body a --hair short --hair-colour #6A4E35 --skin 6" road "160 10 22"
fi
ls -la "$out"
