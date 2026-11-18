#!/usr/bin/env bash
# Frame-rate cost of single render features (spec §4.6.2 ground detail,
# §4.6 *Road damage*, §3.2 navigation map): for each scene, preset and
# window size, alternating rounds of `all off`, each feature alone on and
# `all on`, measured with scripts/measure-fps.sh on the GPU (headless
# weston, no vsync, riding at 250 W, fixed time and weather, the dynamic
# render distance off). Before every run it waits until the machine is
# quiet (no rustc / cargo / other ocw-desktop, at most OCW_FC_BUSY cores
# busy over 5 s; at the start of each scene block also a 1-minute load
# below OCW_FC_LOAD), so the numbers are not taken under load.
#
#   scripts/feature-fps-cost.sh OUT.tsv [ROUNDS] [SCENE_GLOB]
#
# One TSV line per run: size, preset, scene, config, round, mean fps, mean
# frame ms, p50 ms, p95 ms, load at the start. Needs the release build and
# the world packages (OCW_WORLD_DATA, default ~/src/OCW/data).
# OCW_FC_SIZES ("1280x720 1920x1080"), OCW_FC_PRESETS ("high medium"),
# OCW_FC_SECONDS (46) and OCW_FC_WARMUP (15) change the matrix.
set -euo pipefail
cd "$(dirname "$0")/.."
# Numbers with a decimal point whatever the locale (awk reads and prints them).
export LC_ALL=C
out=${1:?OUT.tsv}
rounds=${2:-3}
only=${3:-*}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
sizes=${OCW_FC_SIZES:-1280x720 1920x1080}
presets=${OCW_FC_PRESETS:-high medium}
seconds=${OCW_FC_SECONDS:-46}
warmup=${OCW_FC_WARMUP:-15}
busy_max=${OCW_FC_BUSY:-1.0}
load_max=${OCW_FC_LOAD:-1.5}
logs=${out%.tsv}-logs
mkdir -p "$logs"
[ -f "$out" ] || printf 'size\tpreset\tscene\tconfig\tround\tfps\tms\tp50\tp95\tload\n' >"$out"

tf=$data/tenerife/tenerife.ocwpkg
bf=$data/blackforest/blackforest-north.ocwpkg
li=$data/pilot/liechtenstein.ocwpkg
# name | package | ride arguments (;-separated) | start commands (;-separated) | damage when on
scenes=(
  "blackforest-mummelsee-km7|$bf|--route;Baden-Baden - Mummelsee|teleport 7|auto"
  "tenerife-masca-km9|$tf|--route;Buenavista - Masca - Santiago del Teide|teleport 9|auto"
  "liechtenstein-vaduz-free|$li|--free-ride;--start;-2430,2220||auto"
  "tenerife-puerto-teide-km4-horrible|$tf|--route-file;$data/tenerife/puerto-teide.route|teleport 4|horrible"
)
# config | ground detail | road damage (on: the scene's) | map
configs=("off|off|off|off" "gd|on|off|off" "rd|off|on|off" "map|off|off|on" "all|on|on|on")

# Cores busy over 5 s (all processes; nothing of ours runs between runs).
busy_cores() {
  local a b
  read -ra a < <(head -1 /proc/stat)
  sleep 5
  read -ra b < <(head -1 /proc/stat)
  awk -v u1="${a[1]}" -v n1="${a[2]}" -v s1="${a[3]}" -v i1="${a[4]}" -v w1="${a[5]}" -v q1="${a[6]}" -v sq1="${a[7]}" \
      -v u2="${b[1]}" -v n2="${b[2]}" -v s2="${b[3]}" -v i2="${b[4]}" -v w2="${b[5]}" -v q2="${b[6]}" -v sq2="${b[7]}" \
      -v hz="$(getconf CLK_TCK)" 'BEGIN {
        busy = (u2 - u1) + (n2 - n1) + (s2 - s1) + (q2 - q1) + (sq2 - sq1)
        printf "%.2f", busy / hz / 5 }'
}
load1() { cut -d' ' -f1 /proc/loadavg; }
# Waits for a quiet machine: no build, no other app, at most busy_max
# cores in use by others over 5 s (nothing of ours runs between runs). With
# `strict` (at the start of every scene block) also a 1-minute load below
# load_max – between the runs of a block the load average still counts our
# own last run, so the 5-s CPU sample is the check there.
wait_quiet() {
  while :; do
    local busy; busy=$(busy_cores)
    if pgrep -x rustc >/dev/null || pgrep -x cargo >/dev/null || pgrep -x ocw-desktop >/dev/null; then
      echo "$(date +%T) busy: build or other app running, waiting" >&2; sleep 60; continue
    fi
    if awk -v b="$busy" -v m="$busy_max" 'BEGIN { exit !(b > m) }'; then
      echo "$(date +%T) busy: $busy cores in use, waiting" >&2; sleep 60; continue
    fi
    if [ "${1:-}" = strict ] && awk -v l="$(load1)" -v m="$load_max" 'BEGIN { exit !(l > m) }'; then
      echo "$(date +%T) load $(load1), waiting" >&2; sleep 30; continue
    fi
    return 0
  done
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for size in $sizes; do
  for preset in $presets; do
    for s in "${scenes[@]}"; do
      IFS='|' read -r name pkg ride start damage <<<"$s"
      # shellcheck disable=SC2053
      [[ $name == $only ]] || continue
      [ -f "$pkg" ] || { echo "no $pkg" >&2; continue; }
      IFS=';' read -ra ride_args <<<"$ride"
      wait_quiet strict
      for r in $(seq "$rounds"); do
        order=("${configs[@]}")
        if [ $((r % 2)) = 0 ]; then order=(); for ((i = ${#configs[@]} - 1; i >= 0; i--)); do order+=("${configs[$i]}"); done; fi
        for c in "${order[@]}"; do
          IFS='|' read -r cname gd rd map <<<"$c"
          [ "$rd" = on ] && rd=$damage
          wait_quiet
          load=$(load1)
          rm -rf "$work/cfg" "$work/data"; mkdir -p "$work/cfg" "$work/data"
          cmds=(--cmd "quality $preset" --cmd "view auto off" --cmd "time 2026-06-20 11:00" --cmd "weather clear"
                --cmd "ground_detail $gd" --cmd "road.damage $rd" --cmd "hud.map $map")
          IFS=';' read -ra more <<<"$start"
          for m in "${more[@]}"; do [ -n "$m" ] && cmds+=(--cmd "$m"); done
          # Again once the settings' start values are applied, and the state at the end
          # (not `hud.map`: without an argument it toggles the map).
          cmds+=(--at "2=quality $preset" --at "2.5=view auto off" --at "3=ground_detail $gd" --at "3=road.damage $rd" --at "3=hud.map $map")
          cmds+=(--at "$((seconds - 2))=quality" --at "$((seconds - 2))=view" --at "$((seconds - 2))=ground_detail"
                 --at "$((seconds - 2))=road.damage")
          label="$size $preset $name $cname r$r"
          log="$logs/${size}-${preset}-${name}-${cname}-r$r.log"
          line=$(OCW_CONFIG_DIR=$work/cfg OCW_DATA_DIR=$work/data \
            scripts/measure-fps.sh --seconds "$seconds" --warmup "$warmup" --size "$size" --label "$label" --log "$log" -- \
            --world "$pkg" "${ride_args[@]}" --no-neighbours --no-args-file --bots 0 --power 250 --name "Ada Rider" \
            --real-weather false --random-start false "${cmds[@]}" 2>&1) || true
          echo "$line" >&2
          # "LABEL: 87.3 fps mean (11.45 ms), min 80.1, p50 11.20 ms, p95 13.40 ms, 6 samples: …"
          v=$(sed -nE 's/.*: ([0-9.]+) fps mean \(([0-9.]+) ms\), min [0-9.]+, p50 ([0-9.]+) ms, p95 ([0-9.]+) ms,.*/\1\t\2\t\3\t\4/p' <<<"$line")
          [ -n "$v" ] || v=$'NA\tNA\tNA\tNA'
          printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$size" "$preset" "$name" "$cname" "$r" "$v" "$load" >>"$out"
        done
      done
    done
  done
done
