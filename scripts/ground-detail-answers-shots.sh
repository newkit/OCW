#!/usr/bin/env bash
# Pictures of the ground detail answers (spec 4.6.2 Q1 – Q5, v4.99): a verge
# before (OCW_BEFORE_APP, the build before the change) and after (the
# spread to 14 m), the bits on the road, a wet road with darkened bits, an
# autumn Black Forest road with leaves beside and on it, and the setting's
# row (Display → Graphics → World detail); on X11 and Wayland, with the
# release build of ocw-desktop (drive.sh; archived in test-output/ with
# index.md). Needs the world packages (OCW_WORLD_DATA, default
# ~/src/OCW/data, as on marge).
#
#   OCW_BEFORE_APP=target/before/ocw-desktop scripts/ground-detail-answers-shots.sh [OUT_DIR] [SCENE_GLOB]
#
# OCW_GD_MODES="shot shot-wayland" (default) picks the window systems.
set -euo pipefail
# Numbers with a decimal point whatever the locale (awk prints the menu times).
export LC_ALL=C
cd "$(dirname "$0")/.."
out=${1:-test-output/ground-detail-answers}
only=${2:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
modes=${OCW_GD_MODES:-shot shot-wayland}
before=${OCW_BEFORE_APP:-}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1920x1080}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-400}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"

bf=$data/blackforest/blackforest-north.ocwpkg
li=$data/pilot/liechtenstein.ocwpkg

# name | build (before / after) | package | route | km | seconds | commands (;-separated)
summer="time 2026-06-05 11:00;weather clear"
verge="camera.orbit 70 32 16"
road="camera.orbit 160 38 6"
scenes=(
  "verge-before|before|$li|Vaduz - Malbun|11|16|$summer;$verge"
  "verge-after|after|$li|Vaduz - Malbun|11|16|$summer;$verge"
  "road-bits|after|$li|Vaduz - Malbun|11|16|$summer;$road"
  "wet-road|after|$li|Vaduz - Malbun|11|16|time 2026-06-05 11:00;weather overcast;weather wetness=1 puddles=0;$road"
  "dry-road|after|$li|Vaduz - Malbun|11|16|time 2026-06-05 11:00;weather overcast;weather wetness=0 puddles=0;$road"
  "blackforest-autumn-road|after|$bf|Baden-Baden - Mummelsee|7|16|time 2026-11-02 12:00;weather clear;$road"
  "blackforest-autumn-verge|after|$bf|Baden-Baden - Mummelsee|7|16|time 2026-11-02 12:00;weather clear;$verge"
  "blackforest-autumn-wet|after|$bf|Baden-Baden - Mummelsee|7|16|time 2026-11-02 12:00;weather overcast;weather wetness=1 puddles=0;$road"
)

table=$out/index.md
[ -f "$table" ] || printf '| scene | build | window system | picture |\n|---|---|---|---|\n' >"$table"
shoot() { # name build file seconds args...
  local name=$1 build=$2 file=$3 secs=$4; shift 4
  local env=()
  if [ "$build" = before ]; then
    [ -n "$before" ] || { echo "no OCW_BEFORE_APP for $name" >&2; return 0; }
    env=(OCW_APP="$before")
  fi
  env "${env[@]}" .claude/skills/run-ocw/drive.sh "$mode" "$file" "$secs" -- "$@" >"$work/run.log" 2>&1 || {
    echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; return 0; }
  grep -oE 'ground detail [a-z]+:[^"]*' "${file%.*}.log" | tail -1 || true
  printf '| %s | %s | %s | [%s](%s) |\n' "$name" "$build" "$ws" "$(basename "$file")" "$(basename "$file")" >>"$table"
  echo "$file"
}
for mode in $modes; do
  ws=x11; [ "$mode" = shot-wayland ] && ws=wayland
  for s in "${scenes[@]}"; do
    IFS='|' read -r name build pkg route km secs extra <<<"$s"
    # shellcheck disable=SC2053
    [[ $name == $only ]] || continue
    [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
    cmds=(--cmd "teleport $km" --cmd "sim.power 0" --cmd "hud off" --cmd "hud.profile off")
    IFS=';' read -ra more <<<"$extra"
    for c in "${more[@]}"; do cmds+=(--cmd "$c"); done
    cmds+=(--at "$((secs - 2))=ground_detail")
    shoot "$name" "$build" "$out/$name-$ws.jpg" "$secs" \
      --world "$pkg" --route "$route" --no-neighbours --bots 0 --power 0 --name "Ada Rider" "${cmds[@]}"
  done
  # The setting's row: Settings → Display → Graphics → World detail, the
  # ground detail row focused.
  # shellcheck disable=SC2053
  if [[ menu-world-detail == $only ]]; then
    nav=(--at "2=menu open")
    t=2
    for a in down down down down down select down down down down select down down down down down select down down select down; do
      t=$(awk -v t="$t" 'BEGIN { printf "%.1f", t + 0.4 }')
      nav+=(--at "$t=menu $a")
    done
    shoot menu-world-detail after "$out/menu-world-detail-$ws.jpg" 14 \
      --route hills --bots 0 --name "Ada Rider" "${nav[@]}"
  fi
done
