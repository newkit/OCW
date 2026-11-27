#!/usr/bin/env bash
# ANT+ power meters and speed / cadence sensors on the Devices page without
# hardware (spec 14 Phase 17, 17E): runs the desktop app headless
# (drive.sh, X11 or Wayland) with a simulated ANT stick (`--ant-sim`) that
# has a crank torque power meter (torque pages only, no power-only page)
# and a combined speed / cadence sensor, and screenshots each step, in two
# runs with two different riders (look, kit, bike):
#
#   A  both remembered, reconnected at the start: the Devices list with
#      their readings, the meter calibrated (Calibrate: the zero offset),
#      the meter's page (power, cadence, Use for cadence, Calibrate, Auto
#      zero), the sensor's page (cadence and speed), the ride's HUD;
#   B  a scan finds both in their roles, both connected.
#
#   scripts/test-ant-power.sh [shot|shot-wayland] [OUT_DIR]
#
# The network key is a made-up test key, not the ANT+ key (the simulated
# stick takes any key; the ANT+ key is never in this repository, spec 14
# Phase 17 Q1 (b)).
set -euo pipefail
export LC_NUMERIC=C
cd "$(dirname "$0")/.."
mode=${1:-shot}
out=${2:-${TMPDIR:-/tmp}/ocw-ant-power}
rm -rf "$out"
mkdir -p "$out"
export OCW_SIZE=${OCW_SIZE:-1600x900}
KEY="0A1B2C3D4E5F6071" # made up (ocw_devices::ant::NetworkKey::TEST)
SIM="torque:4005:250:85,spdcad:2222:92:32"

# run NAME RIDERS_TOML SECRETS DEVICES_TOML SECONDS ARGS... (the timeline
# in the global array `at`).
run() {
  local name=$1 riders=$2 secrets=$3 devices=$4 secs=$5
  shift 5
  local dir="$out/$name"
  mkdir -p "$dir/config" "$dir/data"
  printf '%s' "$riders" >"$dir/config/riders.toml"
  printf '%s' "$secrets" >"$dir/config/secrets.toml"
  chmod 600 "$dir/config/secrets.toml"
  if [ -n "$devices" ]; then printf '%s' "$devices" >"$dir/config/devices.toml"; fi
  OCW_CONFIG_DIR="$dir/config" OCW_DATA_DIR="$dir/data" \
    .claude/skills/run-ocw/drive.sh "$mode" "$out/$name-end.jpg" "$secs" -- \
    --bots 2 "$@" "${at[@]}" >"$dir/drive.txt" 2>&1 || {
    tail -40 "$dir/drive.txt"
    return 1
  }
  grep -hE "ANT|ant\.|calibrat|cadence|devices|window system" "$dir/drive.txt" | grep -v "$KEY" | tail -30 || true
}

SECRETS="[ant]
network_key = \"$KEY\"
"

# A: Eli Rouleur – medium build, grey hair, road bike in teal, stripes kit.
at=(
  --at "6=menu devices"
  --at "8=screenshot $out/A-1-devices-list.png"
  --at "8.5=devices.calibrate ant:11:4005:5"
  --at "9=menu close"
  # A device page shows four rows at a time: Kind, State, Signal,
  # Remembered, then Next – the reading, Use for cadence, Calibrate and
  # Auto zero are on its second page.
  --at "11=menu device 2"
  --at "11.5=menu down" --at "11.8=menu down" --at "12.1=menu down" --at "12.4=menu down"
  --at "12.8=menu select"
  --at "13.8=screenshot $out/A-2-power-meter-page.png"
  --at "14.2=menu close"
  --at "15=menu device 1"
  --at "15.5=menu down" --at "15.8=menu down" --at "16.1=menu down" --at "16.4=menu down"
  --at "16.8=menu select"
  --at "17.8=screenshot $out/A-3-speed-cadence-page.png"
  --at "18.2=menu close"
  --at "23=screenshot $out/A-4-ride-hud.png"
)
run A "active = \"Eli Rouleur\"
[[rider]]
name = \"Eli Rouleur\"
weight_kg = 74.0
height_cm = 181.0
bike = \"road\"
frame_color = \"#00897B\"
jersey_color = \"#ECEFF1\"
kit = \"stripes\"
kit_second = \"#00897B\"
helmet_color = \"#263238\"
skin = 3
hair = 2
hair_color = \"#9E9E9E\"
glasses = 1
" "$SECRETS" "[[device]]
id = \"ant:121:2222:1\"
name = \"ANT+ speed/cadence 2222\"
kind = \"cadence\"
transport = \"ant\"

[[device]]
id = \"ant:11:4005:5\"
name = \"ANT+ power 4005\"
kind = \"power\"
transport = \"ant\"
" 28 --ant-sim "$SIM"

# B: Fay Climber – slim, dark skin, long black hair, MTB in orange.
at=(
  --at "2=menu devices"
  --at "2.5=devices.scan 6"
  --at "6=screenshot $out/B-1-scan.png"
  --at "9=devices.connect ant:121:2222:1"
  --at "9.3=devices.connect ant:11:4005:5"
  --at "14=screenshot $out/B-2-connected.png"
)
run B "active = \"Fay Climber\"
[[rider]]
name = \"Fay Climber\"
weight_kg = 55.0
height_cm = 163.0
bike = \"mtb\"
frame_color = \"#EF6C00\"
jersey_color = \"#3949AB\"
kit = \"band\"
kit_second = \"#FFFFFF\"
helmet_color = \"#FAFAFA\"
body_build = \"slim\"
skin = 9
hair = 4
hair_color = \"#141414\"
" "$SECRETS" "" 18 --ant-sim "$SIM"

ls "$out"
