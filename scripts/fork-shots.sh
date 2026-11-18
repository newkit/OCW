#!/usr/bin/env bash
# Side views of every bike's fork (spec §14 Phase 15, bug fix: the fork
# raked forward), headless on X11 (Xvfb) and Wayland (weston) through
# .claude/skills/run-ocw/drive.sh:
#
#   scripts/fork-shots.sh OUT_DIR [LABEL]     (LABEL: before / after, default now)
#
# Per bike and window system: OCW Studio's model viewer from the drive side
# (the whole bike and a close-up crop of the front end), the workshop's bike
# stand and a riding view from the side. Writes OUT_DIR/index.md.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:?OUT_DIR}
label=${2:-now}
mkdir -p "$out"
drive=.claude/skills/run-ocw/drive.sh
export OCW_SIZE=${OCW_SIZE:-1600x900}
index="$out/index.md"
[ -f "$index" ] || printf '# Bike forks, side views (spec §14 Phase 15, fork rake fix)\n\n| Picture | What it shows |\n|---|---|\n' >"$index"
row() { printf '| [%s](%s) | %s |\n' "$1" "$1" "$2" >>"$index"; }
# file stem → profile id
bikes=(road-aero:road-aero road-climbing:road-climbing road-endurance:road gravel:gravel tt:tt mtb-hardtail:mtb)
cfg=$(mktemp -d)
trap 'rm -rf "$cfg"' EXIT
for mode in x11 wayland; do
  sub=shot; [ "$mode" = wayland ] && sub=shot-wayland
  for b in "${bikes[@]}"; do
    stem=${b%%:*} profile=${b##*:}
    name="$label-$stem-$mode"
    # Studio: the model from its right (drive) side, front to the right.
    OCW_PACKAGE=ocw-studio "$drive" "$sub" "$out/$name-studio.jpg" 10 -- \
      --model "bikes/bike-$stem" --cmd "model show gizmos off" --cmd "model show labels off" \
      --cmd "model orbit 90 0 2.4"
    row "$name-studio.jpg" "$label, $mode: OCW Studio, bikes/bike-$stem from the drive side (front to the right)"
    # The front end close up (the right part of the side view).
    ffmpeg -loglevel error -y -i "$out/$name-studio.jpg" \
      -vf "crop=iw*0.42:ih*0.78:iw*0.56:ih*0.12" -q:v 2 "$out/$name-fork.jpg"
    row "$name-fork.jpg" "$label, $mode: the fork of bike-$stem close up (crop of the side view)"
    # The game: the rider's profile picks the bike.
    rm -rf "$cfg"/*
    mkdir -p "$cfg/config" "$cfg/data" "$cfg/worlds"
    printf 'active = "Ada Rider"\n\n[[rider]]\nname = "Ada Rider"\nweight_kg = 66.0\nheight_cm = 172.0\nbike = "%s"\n' \
      "$profile" >"$cfg/config/riders.toml"
    export OCW_CONFIG_DIR="$cfg/config" OCW_DATA_DIR="$cfg/data" OCW_WORLD_CACHE="$cfg/worlds"
    "$drive" "$sub" "$out/$name-stand.jpg" 12 -- --route hills --bots 0 \
      --at "2.2=workshop open" --at "2.8=workshop select"
    row "$name-stand.jpg" "$label, $mode: the workshop's bike stand with the $profile bike"
    "$drive" "$sub" "$out/$name-ride.jpg" 12 -- --route hills --bots 0 --power 250 \
      --cmd "hud off" --at "1=camera.orbit 90 4 2.6"
    row "$name-ride.jpg" "$label, $mode: riding the $profile bike, the camera from the side"
    unset OCW_CONFIG_DIR OCW_DATA_DIR OCW_WORLD_CACHE
  done
done
echo "screenshots in $out (index.md)"
