#!/usr/bin/env bash
# Screenshots of the level-of-detail distances (spec 16H Q24 (a)): props
# switch to their lod1 at 60 m, the unicorn at 40 m, in the game and in OCW
# Studio's Models viewer (auto level), on Wayland (headless weston) with
# the release builds (drive.sh), into OUT_DIR with index.md:
#
#   scripts/lod-shots.sh [OUT_DIR]      (default test-output/lod-q24)
#
# The shipped props have no lod1 yet, so for the props' pictures the game is
# built with test copies of the street lamp files whose lod1 is the near
# model painted red (scripts/lod-demo-lamps.py): a lamp beyond 60 m is red.
# The lamp files are put back at the end. Needs the world packages
# (OCW_WORLD_DATA, default ~/src/OCW/data, as on marge).
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/lod-q24}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1600x900}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-500}
work=$(mktemp -d)
props=artwork/3dmodels/props
mkdir -p "$work/orig" "$work/demo"
cp "$props"/lamp-street-arm-*.glb "$work/orig/"
restore() { cp "$work/orig/"*.glb "$props/"; rm -rf "$work"; }
trap restore EXIT
python3 -I scripts/lod-demo-lamps.py "$work/demo" "$work/orig"
cp "$work/demo/"*.glb "$props/"
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
drive=.claude/skills/run-ocw/drive.sh
li=$data/pilot/liechtenstein.ocwpkg
printf '# Level-of-detail distances (spec 16H Q24 (a))\n\n| Picture | What it shows |\n|---|---|\n' >"$out/index.md"
row() { printf '| [%s](%s) | %s |\n' "$1" "$1" "$2" >>"$out/index.md"; }
crop() { # the middle of a picture, twice as large (the props are small)
  ffmpeg -loglevel error -y -i "$out/$1.jpg" -vf "crop=iw/2:ih/2:iw/4:ih/4,scale=iw*2:ih*2:flags=lanczos" "$out/$1-crop.jpg"
}
for d in 50 70; do
  # shellcheck disable=SC2086
  "$drive" shot-wayland "$out/lamps-$d.jpg" 12 -- --world "$li" --no-neighbours --free-ride \
    --start -3386,-892 --bots 0 --power 0 --name "Ada Rider" \
    --at "0.5=time pause" --at "0.5=time 2026-07-15 12:00" --at "0.5=hud off" \
    --at "0.5=hud.profile off" --at "0.5=road_barriers on" --at "1=camera.orbit 60 6 $d"
  crop "lamps-$d"
  row "lamps-$d.jpg" "Street lamps (test lod1 = red) round the rider, camera $d m away: near model up to 60 m, red lod1 beyond"
  row "lamps-$d-crop.jpg" "The middle of lamps-$d.jpg, twice as large"
done
for d in 30 50; do
  "$drive" shot-wayland "$out/unicorn-$d.jpg" 10 -- --world "$li" --no-neighbours --free-ride \
    --start -2430,2220 --bots 0 --name "Ada Rider" \
    --at "1=time 14:00" --at "2=god.flight hover" --at "3=god on" \
    --at "4=god.pose -2650 2330 70 80 -10" --at "5=camera.orbit 90 8 $d"
  crop "unicorn-$d"
  row "unicorn-$d.jpg" "The unicorn hovering, camera $d m away (switch at 40 m: near model at 30 m, lod1 at 50 m)"
  row "unicorn-$d-crop.jpg" "The middle of unicorn-$d.jpg, twice as large"
done
export OCW_PACKAGE=ocw-studio
for d in 50 70; do
  "$drive" shot-wayland "$out/studio-lamp-$d.jpg" 12 -- --model "$work/demo/lamp-street-arm-road.glb" \
    --cmd "model show gizmos off" --cmd "model zoom $d"
  row "studio-lamp-$d.jpg" "OCW Studio, Models, auto level: the test lamp at $d m (switch at 60 m)"
done
for d in 30 50; do
  "$drive" shot-wayland "$out/studio-unicorn-$d.jpg" 12 -- --model creatures/unicorn-winged \
    --cmd "model show gizmos off" --cmd "model zoom $d"
  row "studio-unicorn-$d.jpg" "OCW Studio, Models, auto level: the unicorn at $d m (switch at 40 m)"
done
"$drive" shot-wayland "$out/studio-workshop-70.jpg" 12 -- --model workshop/bench-work-wood \
  --cmd "model show gizmos off" --cmd "model zoom 70"
row "studio-workshop-70.jpg" "OCW Studio, Models, auto level: a workshop piece at 70 m: never switched"
echo "screenshots in $out (index.md)"
