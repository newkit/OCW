#!/usr/bin/env bash
# A screenshot of one place from a fixed camera (a guided camera path held
# still), HUD off, midsummer day (OCW_SHOT_TIME, default 13:00), clear sky – for comparing world
# packages at a junction (spec 4.8.8 Q6 (d) / Q7). Run on a build machine
# with a release build of ocw-desktop (via scripts/remote.sh MACHINE run
# '…'); Wayland (headless weston).
#
#   scripts/junction-shots.sh OUT.jpg PACKAGE X Y Z HEADING PITCH START_X,START_Z
#
# X Y Z: the camera in local metres (x east, y up – the package's heights –,
# z south), HEADING compass degrees (0 north, 90 east), PITCH degrees (up).
# START: the free ride's start (the tiles load round the rider there).
set -euo pipefail
out=${1:?OUT.jpg}; pkg=${2:?PACKAGE}; x=${3:?X}; y=${4:?Y}; z=${5:?Z}
heading=${6:?HEADING}; pitch=${7:?PITCH}; start=${8:?START}
cd "$(dirname "$0")/.."
key="camera.path key $x $y $z $heading $pitch"
OCW_PROFILE=release OCW_FPS=0 OCW_TEST_OUTPUT=off OCW_SIZE=${OCW_SIZE:-1920x1080} \
  .claude/skills/run-ocw/drive.sh shot-wayland "$out" "${OCW_SHOT_DELAY:-12}" -- \
  --world "$pkg" --no-neighbours --free-ride --start "$start" --bots 0 --power 0 \
  --at "0.5=time pause" --at "0.5=time 2026-06-21 ${OCW_SHOT_TIME:-13:00}" --at "0.5=weather clear" \
  --at "0.5=hud off" --at "0.5=hud.profile off" \
  --at "2=$key" --at "2=$key" --at "2.5=camera.path play --seconds 600"
