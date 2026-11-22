#!/usr/bin/env bash
# Pictures of the street lamp rule (spec: world look, *Street lamps*, the
# rule of 2026-11-22: lamps never stand in a road, only beside it): places
# the world lint found lamps standing in a road before the rule
# (`world-builder lint`: lamp.nominal.in_carriageway / .in_junction), by day
# and by night, with the build before the rule (OCW_BEFORE_APP) and after
# it; Wayland (headless weston) with the release build of ocw-desktop
# (drive.sh; archived with index.md). Needs the world packages
# (OCW_WORLD_DATA, default ~/src/OCW/data, as on marge).
#
#   OCW_BEFORE_APP=../OCW-before/target/release/ocw-desktop scripts/lamp-rule-shots.sh [OUT_DIR] [SCENE_GLOB]
#
# OCW_LAMP_MODES="shot-wayland" (default) picks the window systems,
# OCW_LAMP_BUILDS="before after" (default) the builds.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
out=${1:-test-output/lamp-rule}
only=${2:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
modes=${OCW_LAMP_MODES:-shot-wayland}
builds=${OCW_LAMP_BUILDS:-before after}
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
bf=$data/blackforest/blackforest-north.ocwpkg
tf=$data/tenerife/tenerife.ocwpkg
day="0.5=time pause;0.5=time 2026-07-15 12:00"
night="0.5=time pause;0.5=time 2026-11-09 22:00"
quiet="0.5=hud off;0.5=hud.profile off"
at() { echo "--world $1 --no-neighbours --free-ride --start $2 --bots 0 --power 0"; }

# name | package | args | seconds | camera (orbit yaw pitch distance)
places=(
  "vaduz-street|$li|$(at "$li" -2862,1652)|12|1=camera.orbit 160 30 16"
  "liechtenstein-junction|$li|$(at "$li" -4003,9991)|12|1=camera.orbit 160 35 18"
  "blackforest-village|$bf|$(at "$bf" -7891,-78220)|16|1=camera.orbit 160 35 18"
  "tenerife-town|$tf|$(at "$tf" 24860,-19030)|16|1=camera.orbit 160 30 16"
  "tenerife-junction|$tf|$(at "$tf" -34371,-5934)|16|1=camera.orbit 160 35 18"
)

table=$out/index.md
[ -f "$table" ] || printf '| scene | time | build | window system | picture |\n|---|---|---|---|---|\n' >"$table"
for mode in $modes; do
  ws=x11; [ "$mode" = shot-wayland ] && ws=wayland
  for p in "${places[@]}"; do
    IFS='|' read -r name pkg args secs camera <<<"$p"
    # shellcheck disable=SC2053
    [[ $name == $only ]] || continue
    [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
    for time in day night; do
      when=$day; [ "$time" = night ] && when=$night
      for build in $builds; do
        env=()
        if [ "$build" = before ]; then
          [ -n "$before" ] || { echo "no OCW_BEFORE_APP for $name" >&2; continue; }
          env=(OCW_APP="$before")
        fi
        cmds=()
        IFS=';' read -ra more <<<"$when;$quiet;$camera"
        for c in "${more[@]}"; do cmds+=(--at "$c"); done
        file=$out/$name-$time-$build-$ws.jpg
        # shellcheck disable=SC2086
        env "${env[@]}" .claude/skills/run-ocw/drive.sh "$mode" "$file" "$secs" -- \
          $args --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
          echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
        printf '| %s | %s | %s | %s | [%s](%s) |\n' "$name" "$time" "$build" "$ws" \
          "$(basename "$file")" "$(basename "$file")" >>"$table"
        echo "$file"
      done
    done
  done
done
