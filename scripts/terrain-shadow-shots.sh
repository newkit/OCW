#!/usr/bin/env bash
# Pictures of the mountains' shadows (spec 4.5 *Terrain shadows*, v7.09):
# a camera approaching a valley in the evening shadow of a ridge in
# Andorra from 10 km to 1 km, a picture every 500 m, each with the terrain
# shadow off (the look before: only the cascades near the camera, the
# terrain casting nothing) and on; the same with the sun's shadow maps off
# (as on the phones) at a few distances; and wide views at sunset and in
# the morning. Release build, Wayland (headless weston, drive.sh), into
# OUT_DIR with index.md:
#
#   scripts/terrain-shadow-shots.sh [OUT_DIR]   (default test-output/terrain-shadow)
#
# The camera (the unicorn's flight camera, 80 m behind it and 10° aside)
# follows the path from `terrain-shadow-probe --pick` (a target deep
# in the shadow, sunlit ground near it, in sight from the whole path, the
# sun to the side). Needs the Andorra package (OCW_ANDORRA, default
# ~/src/OCW/data/andorra/andorra.ocwpkg or /var/tmp/ocw/andorra.ocwpkg).
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/terrain-shadow}
mkdir -p "$out"
out=$(cd "$out" && pwd)
rm -f "$out"/*.jpg "$out"/*.log "$out"/*-probe.txt
pkg=${OCW_ANDORRA:-}
for p in "$HOME/src/OCW/data/andorra/andorra.ocwpkg" /var/tmp/ocw/andorra.ocwpkg; do
  [ -z "$pkg" ] && [ -f "$p" ] && pkg=$p
done
[ -f "$pkg" ] || { echo "no Andorra package" >&2; exit 2; }
export OCW_PROFILE=${OCW_PROFILE:-release}
export OCW_SIZE=${OCW_SIZE:-1600x900}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-900}
export OCW_FPS=0
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
drive=.claude/skills/run-ocw/drive.sh
cargo build -q --release -p ocw-world --example terrain-shadow-probe
probe=target/release/examples/terrain-shadow-probe
printf '# Terrain shadows (spec 4.5, v7.09)\n\nAndorra, the same camera path with the terrain shadow off (before: the cascades reach 250 m, the terrain casts nothing) and on.\n\n| Picture | What it shows |\n|---|---|\n' >"$out/index.md"
row() { printf '| [%s](%s) | %s |\n' "$1" "$1" "$2" >>"$out/index.md"; }

# The camera at a pose (TIME X Z HEIGHT YAW PITCH) into the array `pose`:
# the unicorn 80 m ahead of it, 20° right of the view to the target, the
# flight camera right behind it looking down at the pitch – the target
# 20° left of the picture's middle, apart from the unicorn.
unicorn_at() {
  local t=$1 ux uz uh uyaw
  read -r ux uz uh uyaw < <(awk -v x="$2" -v z="$3" -v h="$4" -v yaw="$5" -v p="$6" 'BEGIN {
    d = 80; r = atan2(0, -1) / 180; a = (yaw + 20) * r; q = -p * r
    printf "%.0f %.0f %.0f %.1f\n", x + d * cos(q) * sin(a), z - d * cos(q) * cos(a), h - d * sin(q), yaw + 20 }')
  pose=(--at "$t=god.pose $ux $uz $uh $uyaw $6" --at "$t.2=camera.orbit 0 ${6#-} 80")
}

# One approach: LABEL, local date and time, UTC time, extra --at commands,
# distances to keep (all when empty).
approach() {
  local label=$1 date=$2 local_t=$3 utc=$4 extra=$5 keep=${6:-}
  "$probe" "$pkg" --utc "$utc" --pick 1 --map 0 >"$out/$label-probe.txt"
  local path
  path=$(grep '^PATH 1 ' "$out/$label-probe.txt" || true)
  [ -n "$path" ] || { echo "$label: no path found" >&2; return 0; }
  read -r _ _ tx tz th bearing alt <<<"$path"
  local args=(--world "$pkg" --free-ride --start "$tx,$tz" --bots 0 --power 0
    --at "0.5=time pause" --at "0.5=time $date $local_t" --at "0.5=weather clear"
    --at "0.5=hud off" --at "0.5=hud.profile off" --at "0.5=set display.quality high"
    --at "1.5=god.flight hover" --at "2=god on")
  [ -n "$extra" ] && args+=(--at "2.6=$extra")
  local t=4 n=0
  while read -r _ _ d x z h yaw pitch; do
    if [ -n "$keep" ] && ! grep -qw "$d" <<<"$keep"; then continue; fi
    unicorn_at "$t" "$x" "$z" "$h" "$yaw" "$pitch"
    args+=("${pose[@]}")
    args+=(--at "$((t + 6))=terrain.shadow off" --at "$((t + 7))=screenshot $out/$label-$d-before.jpg")
    args+=(--at "$((t + 8))=terrain.shadow on" --at "$((t + 9))=screenshot $out/$label-$d-after.jpg")
    t=$((t + 10)); n=$((n + 1))
  done < <(grep '^POSE 1 ' "$out/$label-probe.txt")
  "$drive" shot-wayland "$out/$label-last.jpg" $((t + 4)) -- "${args[@]}"
  if grep -q "failed to process shader" "$out/$label-last.log"; then
    grep -A6 "failed to process shader" "$out/$label-last.log" | head -20 >&2
    exit 3
  fi
  for f in "$out/$label"-*-before.jpg; do
    [ -f "$f" ] || continue
    local d=${f%-before.jpg}; d=${d##*-}
    row "$label-$d-before.jpg" "$label, $date $local_t: camera $d m from a valley point in the shadow of a ridge (target $tx,$tz, $th m; camera at $alt m from bearing $bearing°), terrain shadow off (before)"
    row "$label-$d-after.jpg" "the same with the terrain shadow on (after)"
    if command -v ffmpeg >/dev/null; then
      ffmpeg -loglevel error -y -i "$f" -i "$out/$label-$d-after.jpg" -filter_complex hstack "$out/$label-$d-compare.jpg" &&
        row "$label-$d-compare.jpg" "before | after side by side"
    fi
  done
}

approach evening 2026-10-08 18:45 2026-10-08T16:45 ""
approach morning 2026-10-08 08:45 2026-10-08T06:45 ""
approach evening-noshadowmaps 2026-10-08 18:45 2026-10-08T16:45 "terrain.shadow --sun-shadows off" "10000 5000 3000 2000 1000"

# Wide views: high above the country looking away from the low sun (the
# shadows stretch away from the camera), terrain shadow off and on.
wide() {
  local label=$1 date=$2 local_t=$3 x=$4 z=$5 h=$6 yaw=$7 pitch=$8 extra=${9:-}
  local args=(--world "$pkg" --free-ride --start "$x,$z" --bots 0 --power 0
    --at "0.5=time pause" --at "0.5=time $date $local_t" --at "0.5=weather clear"
    --at "0.5=hud off" --at "0.5=hud.profile off" --at "0.5=set display.quality high"
    --at "1.5=god.flight hover" --at "2=god on")
  unicorn_at 3 "$x" "$z" "$h" "$yaw" "$pitch"
  args+=("${pose[@]}")
  [ -n "$extra" ] && args+=(--at "3.1=$extra")
  args+=(--at "12=terrain.shadow off" --at "13=screenshot $out/$label-before.jpg")
  args+=(--at "14=terrain.shadow on" --at "15=screenshot $out/$label-after.jpg")
  "$drive" shot-wayland "$out/$label-last.jpg" 19 -- "${args[@]}"
  row "$label-before.jpg" "$label: $date $local_t, $h m above the ground at $x,$z looking $yaw°, terrain shadow off (before)"
  row "$label-after.jpg" "the same, terrain shadow on: the valleys in the mountains' shadow"
}

# Above Andorra la Vella looking east up the Valira d'Orient at sunset
# (the sun in the west behind the camera), and over Encamp looking west
# into the morning (the sun in the east behind).
wide sunset-vella 2026-10-08 19:05 -7600 3300 1500 75 -12
wide sunset-vella-noshadowmaps 2026-10-08 19:05 -7600 3300 1500 75 -12 "terrain.shadow --sun-shadows off"
wide morning-encamp 2026-10-08 08:40 2500 -400 1500 255 -12
echo "pictures in $out (index.md)"
