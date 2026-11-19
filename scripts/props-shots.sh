#!/usr/bin/env bash
# Pictures of the props from their model files (spec 15H MX / M2: point
# obstacles and street lamps as files in artwork/3dmodels/props/): the same
# scenes with the build before the switch (OCW_BEFORE_APP: the generated
# meshes) and after (the files) – bollards on a road in Liechtenstein (road
# barriers shown), the street lamps in Vaduz and beside the bollards, posts,
# gates, a lift gate, a cycle barrier, concrete blocks and a pylon, by day
# and (bollards, lamps, posts) by night; on X11 and Wayland with the release
# build of ocw-desktop (drive.sh; archived with index.md). Needs the world
# packages (OCW_WORLD_DATA, default ~/src/OCW/data, as on marge).
#
#   OCW_BEFORE_APP=target/before/ocw-desktop scripts/props-shots.sh [OUT_DIR] [SCENE_GLOB]
#
# OCW_PROPS_MODES="shot shot-wayland" (default) picks the window systems,
# OCW_PROPS_BUILDS="before after" (default) the builds.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
out=${1:-test-output/props-files}
only=${2:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
modes=${OCW_PROPS_MODES:-shot shot-wayland}
builds=${OCW_PROPS_BUILDS:-before after}
before=${OCW_BEFORE_APP:-}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1920x1080}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-400}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"

li=$data/pilot/liechtenstein.ocwpkg
day="0.5=time pause;0.5=time 2026-07-15 12:00"
night="0.5=time pause;0.5=time 2026-11-09 22:00"
quiet="0.5=hud off;0.5=hud.profile off"
bollards="--world $li --no-neighbours --free-ride --start -3386,-892 --bots 0 --power 0"
lamps="--world $li --route-file $data/pilot/vaduz-malbun.route --no-neighbours --bots 0 --power 0"
at() { echo "--world $li --no-neighbours --free-ride --start $1 --bots 0 --power 0"; }
posts=$(at -3399,-941)

# name | package | args | seconds | timed commands (;-separated)
scenes=(
  "bollards-day|$li|$bollards|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 150 22 14"
  "bollards-close|$li|$bollards|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 120 12 6"
  "bollards-night|$li|$bollards|12|$night;$quiet;0.5=road_barriers on;1=camera.orbit 150 22 14"
  "lamps-day|$li|$lamps|10|$day;$quiet;2.5=teleport 0.25;2.5=camera.orbit 20 14 12"
  "lamps-night|$li|$lamps|10|$night;$quiet;2.5=teleport 0.25;2.5=camera.orbit 20 14 12"
  "lamps-street-day|$li|$bollards|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 60 6 20"
  "lamps-street-night|$li|$bollards|12|$night;$quiet;0.5=road_barriers on;1=camera.orbit 60 6 20"
  "posts-day|$li|$posts|12|$day;$quiet;1=camera.orbit 150 14 16"
  "posts-night|$li|$posts|12|$night;$quiet;1=camera.orbit 150 14 16"
  "gate-day|$li|$(at -3561,-871)|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 150 18 10"
  "gate-wide-day|$li|$(at -3668,-897)|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 150 18 10"
  "lift-gate-day|$li|$(at -2444,-460)|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 150 18 10"
  "cycle-barrier-day|$li|$(at -3505,-900)|12|$day;$quiet;0.5=road_barriers on;1=camera.orbit 150 18 10"
  "blocks-day|$li|$(at -3391,-543)|12|$day;$quiet;1=camera.orbit 150 18 12"
  "pylon-day|$li|$(at -4086,-2478)|12|$day;$quiet;1=camera.orbit 150 12 40"
)

table=$out/index.md
[ -f "$table" ] || printf '| scene | build | window system | picture |\n|---|---|---|---|\n' >"$table"
for mode in $modes; do
  ws=x11; [ "$mode" = shot-wayland ] && ws=wayland
  for s in "${scenes[@]}"; do
    IFS='|' read -r name pkg args secs extra <<<"$s"
    # shellcheck disable=SC2053
    [[ $name == $only ]] || continue
    [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
    for build in $builds; do
      env=()
      if [ "$build" = before ]; then
        [ -n "$before" ] || { echo "no OCW_BEFORE_APP for $name" >&2; continue; }
        env=(OCW_APP="$before")
      fi
      cmds=()
      IFS=';' read -ra more <<<"$extra"
      for c in "${more[@]}"; do cmds+=(--at "$c"); done
      file=$out/$name-$build-$ws.jpg
      # shellcheck disable=SC2086
      env "${env[@]}" .claude/skills/run-ocw/drive.sh "$mode" "$file" "$secs" -- \
        $args --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
        echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
      printf '| %s | %s | %s | [%s](%s) |\n' "$name" "$build" "$ws" "$(basename "$file")" "$(basename "$file")" >>"$table"
      echo "$file"
    done
  done
done
