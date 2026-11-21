#!/usr/bin/env bash
# Before / after pictures of junctions with steep side roads (spec 4.8.8
# J2b): each junction drawn from the J2 package (data/j2-build, the blend
# alone) and the J2b package (data/j2b-build, the grade limits applied
# again), the same build, on Wayland and X11, into test-output/junctions-j2b/.
# Usage: scripts/j2b-shots.sh   (on a build machine with both packages)
set -uo pipefail
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OUT:-test-output/junctions-j2b}
mkdir -p "$OUT"
export OCW_PROFILE=release OCW_TEST_OUTPUT=off
# name  start x,z  camera (yaw pitch distance)
scenes=(
  "v3929 -3603.0,-5897.8 90 18 26"
  "v1855 -3686.6,-6244.7 90 18 26"
  "v14 -528.1,-2842.6 90 18 26"
)
for s in "${scenes[@]}"; do
  read -r name start yaw pitch dist <<<"$s"
  for build in j2 j2b; do
    pkg=$D/$build-build/pilot/liechtenstein.ocwpkg
    for ws in wayland x11; do
      mode=shot; [ $ws = wayland ] && mode=shot-wayland
      f=$OUT/$name-$build-$ws.jpg
      .claude/skills/run-ocw/drive.sh $mode "$f" 10 -- --virtual --window 1920x1080 \
        --world "$pkg" --no-neighbours --free-ride --start "$start" --bots 0 --power 0 \
        --at "0.5=time pause" --at "0.5=time 2026-06-20 14:00" --at "0.5=hud off" \
        --at "0.5=hud.profile off" --at "1=camera.orbit $yaw $pitch $dist" > "$OUT/$name-$build-$ws.log" 2>&1
      echo "$f: $?"
    done
  done
done
