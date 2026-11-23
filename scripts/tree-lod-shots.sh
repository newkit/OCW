#!/usr/bin/env bash
# Pictures of the trees' leaf state at every distance (the bug of
# 2026-11-23: "the trees turn bald all of a sudden when getting closer")
# and of the unicorn's shadow flying low and high: first-person views from
# the unicorn (hover flight, HUD off) at a forest edge from several
# distances, on several dates; Wayland (headless weston) with the release
# build of ocw-desktop (drive.sh; archived with index.md). Needs the world
# packages (OCW_WORLD_DATA, default ~/src/OCW/data, as on marge).
#
#   scripts/tree-lod-shots.sh [OUT_DIR] [SCENE_GLOB] [TAG]
#
# OCW_BEFORE_APP=PATH renders with another build (TAG names it in the
# file names, default "after"). OCW_TREE_SCENES=FILE takes the scenes from
# a file: scripts/tree-lod-scenes-forest.txt (a wood in the Rhine valley
# from 600 m to 60 m on four dates, the Black Forest),
# scripts/tree-lod-scenes-unicorn.txt (the unicorn low over a wood),
# scripts/plant-files-scenes.txt (the plants near and far in Liechtenstein,
# the Black Forest and on Tenerife, July and November: spec 15G Q11 (a)).
# OCW_FLORA=files draws the plants from their model files.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
out=${1:-test-output/tree-lod}
only=${2:-*}
tag=${3:-after}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1600x900}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-400}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
env=()
[ -n "${OCW_BEFORE_APP:-}" ] && env=(OCW_APP="$OCW_BEFORE_APP")

li=$data/pilot/liechtenstein.ocwpkg
bf=$data/blackforest/blackforest-north.ocwpkg
tf=$data/tenerife/tenerife.ocwpkg
bfs=$data/blackforest/blackforest.ocwpkg
quiet="0.5=hud off;0.5=hud.profile off;0.5=weather clear"
at() { echo "--world $1 --no-neighbours --free-ride --start $2 --bots 0 --power 0"; }

# name | package | start | date and time | the unicorn: x z height yaw
# pitch | view (first = from its head, flight = behind it, chase = the
# rider's chase camera, no unicorn) | more timed
# commands (;-separated, e.g. "2.1=god.flight plane;4.3=god.cruise 0.5")
scenes=()
# OCW_TREE_SCENES=FILE: other scenes, one a line (LI / BF / BFS / TF for the packages).
if [ -n "${OCW_TREE_SCENES:-}" ]; then
  while IFS= read -r line; do
    line=${line//\{LI\}/$li}
    line=${line//\{BF\}/$bf}
    line=${line//\{TF\}/$tf}
    line=${line//\{BFS\}/$bfs}
    [ -n "$line" ] && scenes+=("$line")
  done <"$OCW_TREE_SCENES"
fi
if [ ${#scenes[@]} -eq 0 ]; then
  for date in 2026-05-10 2026-07-15 2026-10-28 2026-11-23; do
    for d in 600 330 250 150 60; do
      scenes+=("li-edge-$date-${d}m|$li|-2430,2220|$date 13:00|-2430 $((2220 + d)) 6 180 2|first|")
    done
  done
fi

table=$out/index.md
[ -f "$table" ] || printf '| scene | build | picture |\n|---|---|---|\n' >"$table"
for s in "${scenes[@]}"; do
  IFS='|' read -r name pkg start date pose view extra <<<"$s"
  # shellcheck disable=SC2053
  [[ $name == $only ]] || continue
  [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
  cmds=()
  # "now": the world clock and the weather as they come (the user's ride).
  when="0.5=time pause;0.5=time $date;$quiet"
  [ "$date" = now ] && when="0.5=hud off;0.5=hud.profile off"
  # View "chase": no unicorn – the rider's chase camera at the start, set
  # by the extra commands (camera.orbit; the pose is not used).
  if [ "$view" = chase ]; then
    IFS=';' read -ra more <<<"$when${extra:+;$extra}"
  else
    IFS=';' read -ra more <<<"$when;2=god.flight hover;3=god on;4=god.view $view;4.2=god.pose $pose${extra:+;$extra}"
  fi
  for c in "${more[@]}"; do cmds+=(--at "$c"); done
  file=$out/$name-$tag.jpg
  # shellcheck disable=SC2086
  env "${env[@]}" .claude/skills/run-ocw/drive.sh shot-wayland "$file" "${OCW_TREE_SECS:-11}" -- \
    $(at "$pkg" "$start") --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
    echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
  printf '| %s | %s | [%s](%s) |\n' "$name" "$tag" "$(basename "$file")" "$(basename "$file")" >>"$table"
  echo "$file"
done
