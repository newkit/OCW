#!/usr/bin/env bash
# Videos of the ride photographer's drone (spec §2.11, v4.45; the art
# project's Day 6), on marge (GPU, headless weston):
#
#   scripts/capture-drone-videos.sh OUT_DIR probe [SECONDS]
#       A real-time run (screenshot at the end) that logs the drone's
#       phases (`photographer's drone t=… app_s=… phase=…`): when it takes
#       off, takes the photo and lands, in the app's game time (app_s).
#   scripts/capture-drone-videos.sh OUT_DIR outside|pov WARMUP SECONDS
#       Records SECONDS of game time after WARMUP (--capture: one fixed
#       game-time step per frame, the game sound in step) with the camera
#       following the drone (`camera.drone follow`) or from its camera
#       (`camera.drone pov`): OUT_DIR/drone-outside.mp4 or drone-pov.mp4
#       (H.264 yuv420p, AAC), its last frame (.jpg) and log.
#
# The ride: Tenerife, OCW_DRONE_ROUTE (default Buenavista - Masca -
# Santiago del Teide) from km OCW_DRONE_KM (default 9), a summer evening
# (OCW_DRONE_TIME, default 2026-07-15 20:30), clear, the virtual trainer
# at 230 W, the HUD off; `photographer drone` OCW_DRONE_AT (default 2) s
# into the ride. The same settings give the same flight in each run
# (stepped server, fixed start).
# OCW_WORLD_DATA: the folder with tenerife/ (default ~/src/OCW/data).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: capture-drone-videos.sh OUT_DIR probe|outside|pov [WARMUP] SECONDS}
what=${2:?usage: capture-drone-videos.sh OUT_DIR probe|outside|pov [WARMUP] SECONDS}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
pkg=$data/tenerife/tenerife.ocwpkg
route=${OCW_DRONE_ROUTE:-Buenavista - Masca - Santiago del Teide}
km=${OCW_DRONE_KM:-9}
at=${OCW_DRONE_AT:-2}
when=${OCW_DRONE_TIME:-2026-07-15 20:30}
[ -f "$pkg" ] || { echo "no $pkg (OCW_WORLD_DATA)" >&2; exit 2; }
mkdir -p "$out"
common=(--world "$pkg" --route "$route" --virtual --power 230 --no-hearts
  --cmd "weather clear" --cmd "time $when" --cmd "hud off" --cmd "teleport $km"
  --at "$at=photographer drone")
export OCW_SIZE=${OCW_SIZE:-1920x1080} OCW_PROFILE=release OCW_FPS=0 OCW_TEST_OUTPUT=off
export OCW_CAPTURE_FPS=${OCW_CAPTURE_FPS:-30} OCW_VIDEO_TIMEOUT=${OCW_VIDEO_TIMEOUT:-5400}
drive=.claude/skills/run-ocw/drive.sh
case $what in
  probe)
    seconds=${3:-200}
    log=$out/probe.log
    OCW_SHOT_TIMEOUT=$((seconds + 300)) "$drive" shot-wayland "$out/probe.jpg" "$seconds" -- "${common[@]}"
    ;;
  outside | pov)
    warmup=${3:?WARMUP}; seconds=${4:?SECONDS}
    mode=follow; [ "$what" = pov ] && mode=pov
    log=$out/drone-$what.log
    "$drive" video-wayland "$out/drone-$what.mp4" "$seconds" -- "${common[@]}" \
      --at "$at=camera.drone $mode" --capture-warmup "$warmup"
    ;;
  *) echo "probe, outside or pov, not '$what'" >&2; exit 2 ;;
esac
# No home paths in what is kept.
sed -i "s|$HOME|~|g" "$log"
sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "photographer's drone|ride photo|drone camera|capture finished" || true
