#!/usr/bin/env bash
# Before / after pictures of the near-field ground detail (spec §4.6.2):
# every scene with `ground_detail off` (the look before) and `on`, from the
# rider's view (chase camera) and a close orbit, on X11 and Wayland, with
# the release build of ocw-desktop (drive.sh; archived in test-output/ with
# index.md). Needs the world packages (OCW_WORLD_DATA, default
# ~/src/OCW/data, as on marge).
#
#   scripts/ground-detail-shots.sh [OUT_DIR] [SCENE_GLOB]
#
# OCW_GD_MODES="shot shot-wayland" (default) picks the window systems.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-test-output/ground-detail}
only=${2:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
modes=${OCW_GD_MODES:-shot shot-wayland}
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

# name | package | route | km | extra commands (;-separated)
scenes=(
  "tenerife-masca-km9|$tf|Buenavista - Masca - Santiago del Teide|9|time 2026-06-20 11:00;weather clear"
  "blackforest-forest-road|$bf|Baden-Baden - Mummelsee|7|time 2026-07-10 11:00;weather clear"
  "liechtenstein-meadow-road|$li|Vaduz - Malbun|11|time 2026-06-05 11:00;weather clear"
  "blackforest-autumn|$bf|Baden-Baden - Mummelsee|7|time 2026-10-30 12:00;weather clear"
  "liechtenstein-snow|$li|Vaduz - Malbun|13|time 2027-01-25 12:00;weather snow snowcover=0.7"
)
# view | camera commands
views=(
  "rider|"
  "orbit|camera.orbit 35 14 5"
)

table=$out/index.md
[ -f "$table" ] || printf '| scene | view | ground detail | window system | picture |\n|---|---|---|---|---|\n' >"$table"
for s in "${scenes[@]}"; do
  IFS='|' read -r name pkg route km extra <<<"$s"
  # shellcheck disable=SC2053
  [[ $name == $only ]] || continue
  [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
  for v in "${views[@]}"; do
    IFS='|' read -r view cam <<<"$v"
    for state in off on; do
      for mode in $modes; do
        ws=x11; [ "$mode" = shot-wayland ] && ws=wayland
        file=$out/$name-$view-$state-$ws.jpg
        cmds=(--cmd "ground_detail $state" --cmd "teleport $km" --cmd "sim.power 0")
        IFS=';' read -ra more <<<"$extra"
        for c in "${more[@]}"; do cmds+=(--cmd "$c"); done
        [ -n "$cam" ] && cmds+=(--cmd "$cam")
        cmds+=(--at "14=ground_detail")
        .claude/skills/run-ocw/drive.sh "$mode" "$file" 16 -- \
          --world "$pkg" --route "$route" --no-neighbours --bots 0 --power 0 \
          --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
            echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
        grep -oE 'ground detail (on|off):[^"]*' "${file%.*}.log" | tail -1 || true
        printf '| %s | %s | %s | %s | [%s](%s) |\n' "$name" "$view" "$state" "$ws" "$(basename "$file")" "$(basename "$file")" >>"$table"
        echo "$file"
      done
    done
  done
done
