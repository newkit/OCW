#!/usr/bin/env bash
# Close-ups of the road damage (spec 4.6 *Road damage*, *Variety*, v5.68):
# a bad road in the Black Forest and on Tenerife, a medium one in
# Liechtenstein and a bad Vaduz street (`road.damage` sets the condition),
# dry and wet, at midday in clear weather, low and close above the road, on
# Wayland with the release build of ocw-desktop (drive.sh; archived in
# test-output/ with index.md). OCW_APP runs a saved binary (the build
# before a change), OCW_RD_TAG names it (before / after). Needs the world
# packages (OCW_WORLD_DATA, default ~/src/OCW/data).
#
#   scripts/road-damage-shots.sh [OUT_DIR] [SCENE_GLOB]
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-test-output/road-damage}
only=${2:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
tag=${OCW_RD_TAG:-after}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1920x1080}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-400}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"

tf=$data/tenerife/tenerife.ocwpkg
bf=$data/blackforest/blackforest-north.ocwpkg
li=$data/pilot/liechtenstein.ocwpkg

# name | package | ride arguments (;-separated) | start commands | condition | time
scenes=(
  "blackforest-bad|$bf|--route;Baden-Baden - Mummelsee|teleport 7|horrible|2026-06-20 12:30"
  "tenerife-bad|$tf|--route;Buenavista - Masca - Santiago del Teide|teleport 9|horrible|2026-06-20 13:00"
  "liechtenstein-medium|$li|--route;Vaduz - Malbun|teleport 11|intermediate|2026-06-20 12:30"
  "vaduz-street-bad|$li|--free-ride;--start;-2430,2220||very_bad|2026-06-20 12:30"
)
# OCW_RD_SCENES (lines of the same form; $bf, $tf, $li as above) replaces
# them, e.g. to sweep a route for a stretch with several holes.
if [ -n "${OCW_RD_SCENES:-}" ]; then
  scenes=()
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    line=${line//\$bf/$bf}; line=${line//\$tf/$tf}; line=${line//\$li/$li}
    scenes+=("$line")
  done <<<"$OCW_RD_SCENES"
fi
# weather | commands (;-separated); OCW_RD_WEATHERS picks some ("dry wet").
weathers=()
for w in "dry|weather clear;weather wetness=0 puddles=0" \
         "wet|weather clear;weather wetness=1 puddles=0.5"; do
  for want in ${OCW_RD_WEATHERS:-dry wet}; do [ "${w%%|*}" = "$want" ] && weathers+=("$w"); done
done
# view | camera command: low and close, looking down at the road beside
# the rider (about 3.8 m up, 3 - 10 m away); OCW_RD_VIEWS ("right left
# wide") picks some.
views=()
# (camera.orbit looks at the rider from the side or the front, far ahead
# from behind: yaw ±120 looks down about 45 degrees at the road on either
# side of the rider.)
for v in "right|camera.orbit 120 52 4.8" "left|camera.orbit -120 52 4.8" "wide|camera.orbit 200 40 6"; do
  for want in ${OCW_RD_VIEWS:-right left}; do [ "${v%%|*}" = "$want" ] && views+=("$v"); done
done

table=$out/index.md
[ -f "$table" ] || printf '| scene | weather | view | build | picture |\n|---|---|---|---|---|\n' >"$table"
for s in "${scenes[@]}"; do
  IFS='|' read -r name pkg ride start cond when <<<"$s"
  IFS=';' read -ra ride_args <<<"$ride"
  # shellcheck disable=SC2053
  [[ $name == $only ]] || continue
  [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
  for w in "${weathers[@]}"; do
    IFS='|' read -r wname wcmds <<<"$w"
    for v in "${views[@]}"; do
      IFS='|' read -r view cam <<<"$v"
      file=$out/$name-$wname-$view-$tag.jpg
      cmds=(--cmd "time pause" --cmd "time $when" --cmd "hud off" --cmd "sim.power 0")
      [ -n "$start" ] && cmds+=(--cmd "$start")
      IFS=';' read -ra more <<<"$wcmds"
      for c in "${more[@]}"; do cmds+=(--cmd "$c"); done
      # The condition after the weather and the settings' start values
      # (a weather change re-applies the road look).
      cmds+=(--cmd "$cam" --cmd "road.damage $cond" --at "4=road.damage $cond" --at "14=road.damage")
      .claude/skills/run-ocw/drive.sh shot-wayland "$file" 16 -- \
        --world "$pkg" "${ride_args[@]}" --no-neighbours --bots 0 --power 0 \
        --real-weather false --random-start false \
        --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
          echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
      printf '| %s | %s | %s | %s | [%s](%s) |\n' "$name" "$wname" "$view" "$tag" "$(basename "$file")" "$(basename "$file")" >>"$table"
      echo "$file"
    done
  done
done
