#!/usr/bin/env bash
# ANT+ FE-C trainers without hardware (spec 14 Phase 17, 17E): runs the
# desktop app headless (drive.sh) with a simulated ANT stick that has a
# simulated FE-C trainer (`--ant-sim fec:N:W`), with two different riders
# (look, kit, bike):
#
#   A  (screenshots) scan, connect the FE-C trainer and a strap: the
#      Devices page with the trainer in use ("trainer, ANT+, connected"),
#      then the ride in ERG with the trainer's power in the HUD;
#   B  (video) a remembered FE-C trainer reconnects at the start; ERG
#      steps 180 W -> 300 W -> 220 W, the simulated trainer's power
#      following each target.
#
#   scripts/test-ant-fec.sh [shot|shot-wayland] [OUT_DIR] [video|video-wayland]
#
# OCW_FEC_RUNS=A (or B) makes only that run. The `--at` times are the
# app's own clock, which runs slow under software rendering: run A waits
# 75 s of wall time for its last screenshot.
#
# Each run has its own config and data directories. The network key is a
# made-up test key, not the ANT+ key (the simulated stick takes any key;
# the ANT+ key is never in this repository, spec 14 Phase 17 Q1 (b)).
set -euo pipefail
export LC_NUMERIC=C
cd "$(dirname "$0")/.."
mode=${1:-shot}
out=${2:-test-output/ant-fec}
vmode=${3:-video}
runs=${OCW_FEC_RUNS:-AB}
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
KEY="0A1B2C3D4E5F6071" # made up (ocw_devices::ant::NetworkKey::TEST)

# prepare NAME RIDERS_TOML DEVICES_TOML: the run's config and data dirs.
prepare() {
  local dir="$out/$1"
  mkdir -p "$dir/config" "$dir/data"
  printf '%s' "$2" >"$dir/config/riders.toml"
  printf '[ant]\nnetwork_key = "%s"\n' "$KEY" >"$dir/config/secrets.toml"
  chmod 600 "$dir/config/secrets.toml"
  if [ -n "$3" ]; then printf '%s' "$3" >"$dir/config/devices.toml"; fi
}

# A: Fay Rouleur – medium build, red curls, road bike in teal, stripes.
prepare A "active = \"Fay Rouleur\"
[[rider]]
name = \"Fay Rouleur\"
weight_kg = 63.0
height_cm = 170.0
bike = \"road\"
frame_color = \"#00897B\"
jersey_color = \"#F4511E\"
kit = \"stripes\"
kit_second = \"#263238\"
helmet_color = \"#FAFAFA\"
skin = 1
hair = 4
hair_color = \"#B23A1E\"
glasses = 1
" ""
if [[ $runs == *A* ]]; then
OCW_SIZE=${OCW_SIZE:-1600x900} OCW_CONFIG_DIR="$out/A/config" OCW_DATA_DIR="$out/A/data" \
  .claude/skills/run-ocw/drive.sh "$mode" "$out/A-end.jpg" 75 -- \
  --bots 2 --ant-sim "fec:5005:180:88,hr:12345:138" \
  --at "2=menu devices" \
  --at "2.5=devices.scan 5" \
  --at "8=devices.connect ant:17:5005:5" \
  --at "8.3=devices.connect ant:120:12345:1" \
  --at "9=erg.target 240" \
  --at "15=screenshot $out/A-1-devices-fe-c-trainer.png" \
  --at "15.5=menu close" \
  --at "22=screenshot $out/A-2-ride-erg-240w.png" \
  >"$out/A/drive.txt" 2>&1 || {
  tail -40 "$out/A/drive.txt"
  exit 1
}
grep -hE "ANT|FE-C|trainer|devices" "$out/A/drive.txt" | grep -v "$KEY" | tail -20 || true
fi

# B: Gus Climber – strong, dark skin, short black hair, climbing bike in yellow.
prepare B "active = \"Gus Climber\"
[[rider]]
name = \"Gus Climber\"
weight_kg = 78.0
height_cm = 183.0
bike = \"road-climbing\"
frame_color = \"#FBC02D\"
jersey_color = \"#283593\"
kit = \"band\"
kit_second = \"#FBC02D\"
helmet_color = \"#212121\"
body_build = \"strong\"
skin = 9
hair = 2
hair_color = \"#141414\"
glasses = 2
" "[[device]]
id = \"ant:17:5005:5\"
name = \"ANT+ trainer 5005\"
kind = \"trainer\"
transport = \"ant\"
"
if [[ $runs == *B* ]]; then
OCW_SIZE=${OCW_VIDEO_SIZE:-1280x720} OCW_CONFIG_DIR="$out/B/config" OCW_DATA_DIR="$out/B/data" \
  .claude/skills/run-ocw/drive.sh "$vmode" "$out/B-ride-erg.mp4" 14 -- \
  --bots 2 --ant-sim "fec:5005:170:92" \
  --at "0.5=erg.target 180" \
  --at "5=erg.target 300" \
  --at "11=erg.target 220" \
  --at "4=camera.orbit 120 6 3.5" \
  >"$out/B/drive.txt" 2>&1 || {
  tail -40 "$out/B/drive.txt"
  exit 1
}
grep -hE "ANT|FE-C|trainer|devices" "$out/B/drive.txt" | grep -v "$KEY" | tail -20 || true
fi
ls -la "$out"
