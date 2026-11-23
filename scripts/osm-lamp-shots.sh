#!/usr/bin/env bash
# Pictures of the street lamps mapped in OSM (world look *Street lamps*:
# `highway=street_lamp` nodes, section OBST) and of a rider bumping into a
# lamp pole (Phase 12, `dev.lamp`): Vaduz and Santa Cruz de Tenerife at
# night, with the packages rebuilt with the mapped lamps
# (OCW_LAMP_DATA, default ~/src/OCW/data/lamps-build), and the bump in
# Vaduz by day with collisions on. Wayland (headless weston) with the
# release build of ocw-desktop (drive.sh; archived with index.md).
#
#   scripts/osm-lamp-shots.sh [OUT_DIR] [SCENE_GLOB]
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
out=${1:-test-output/osm-lamps}
only=${2:-*}
data=${OCW_LAMP_DATA:-$HOME/src/OCW/data/lamps-build}
mode=${OCW_LAMP_MODE:-shot-wayland}
mkdir -p "$out"
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1920x1080}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-400}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"

li=$data/pilot/liechtenstein.ocwpkg
tf=$data/tenerife/tenerife.ocwpkg
night="0.5=time pause;0.5=time 2026-11-09 22:00"
day="0.5=time pause;0.5=time 2026-07-15 12:00"
quiet="0.5=hud off;0.5=hud.profile off"
at() { echo "--world $1 --no-neighbours --free-ride --start $2 --bots 0 --power 0"; }
bump="0.5=ride.collisions on;1=camera.orbit 75 14 7;5=rider.wild on;5.5=dev.lamp"

# name | package | args | seconds | commands
places=(
  "vaduz-night|$li|$(at "$li" -2862,1652)|12|$night;$quiet;1=camera.orbit 160 30 16"
  "vaduz-night-wide|$li|$(at "$li" -2862,1652)|12|$night;$quiet;1=camera.orbit 160 22 60"
  "santacruz-night|$tf|$(at "$tf" 26682,-19815)|16|$night;$quiet;1=camera.orbit 130 25 18"
  "santacruz-night-wide|$tf|$(at "$tf" 26682,-19815)|16|$night;$quiet;1=camera.orbit 130 22 60"
  "vaduz-lamp-bump|$li|$(at "$li" -2862,1652)|11|$day;$bump"
)

table=$out/index.md
[ -f "$table" ] || printf '| scene | window system | picture |\n|---|---|---|\n' >"$table"
ws=x11; [ "$mode" = shot-wayland ] && ws=wayland
for p in "${places[@]}"; do
  IFS='|' read -r name pkg args secs when <<<"$p"
  # shellcheck disable=SC2053
  [[ $name == $only ]] || continue
  [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
  cmds=()
  IFS=';' read -ra more <<<"$when"
  for c in "${more[@]}"; do cmds+=(--at "$c"); done
  file=$out/$name-$ws.jpg
  # shellcheck disable=SC2086
  .claude/skills/run-ocw/drive.sh "$mode" "$file" "$secs" -- \
    $args --name "Ada Rider" "${cmds[@]}" >"$work/run.log" 2>&1 || {
    echo "FAILED $file" >&2; tail -20 "$work/run.log" >&2; continue; }
  grep -E "street lamp|riding at" "$work/run.log" | tail -3 || true
  printf '| %s | %s | [%s](%s) |\n' "$name" "$ws" "$(basename "$file")" "$(basename "$file")" >>"$table"
  echo "$file"
done
