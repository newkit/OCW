#!/usr/bin/env bash
# ANT+ on the Devices page without hardware (spec 14 Phase 17, A3): runs
# the desktop app headless (drive.sh, X11 or Wayland) with a simulated ANT
# stick (`--ant-sim`) and screenshots each step, in three runs with three
# different riders (look, kit, bike):
#
#   A  the key is set: scan (the simulated straps appear), connect two
#      straps, Devices -> ANT+, the second strap's page, *Use for heart
#      rate*, the ride with its heart rate in the HUD;
#   B  no key yet: Devices -> ANT+ says "network key needed", the key
#      field being typed (masked);
#   C  a remembered strap reconnects at the start: the ride with the
#      heart rate in the HUD, the camera on the rider.
#
#   scripts/test-ant-devices.sh [shot|shot-wayland] [OUT_DIR]
#
# Each run has its own config and data directories (OCW_CONFIG_DIR,
# OCW_DATA_DIR). The network key is a made-up test key, not the ANT+ key
# (the simulated stick takes any key; the ANT+ key is never in this
# repository, spec 14 Phase 17 Q1 (b)).
set -euo pipefail
export LC_NUMERIC=C
cd "$(dirname "$0")/.."
mode=${1:-shot}
out=${2:-/tmp/ocw-ant-devices}
rm -rf "$out"
mkdir -p "$out"
export OCW_SIZE=${OCW_SIZE:-1600x900}
KEY="0A1B2C3D4E5F6071" # made up (ocw_devices::ant::NetworkKey::TEST)

# run NAME RIDERS_TOML SECRETS DEVICES_TOML SECONDS ARGS... (the timeline
# in the global array `at`).
run() {
  local name=$1 riders=$2 secrets=$3 devices=$4 secs=$5
  shift 5
  local dir="$out/$name"
  mkdir -p "$dir/config" "$dir/data"
  printf '%s' "$riders" >"$dir/config/riders.toml"
  if [ -n "$secrets" ]; then
    printf '%s' "$secrets" >"$dir/config/secrets.toml"
    chmod 600 "$dir/config/secrets.toml"
  fi
  if [ -n "$devices" ]; then printf '%s' "$devices" >"$dir/config/devices.toml"; fi
  OCW_CONFIG_DIR="$dir/config" OCW_DATA_DIR="$dir/data" \
    .claude/skills/run-ocw/drive.sh "$mode" "$out/$name-end.jpg" "$secs" -- \
    --bots 2 "$@" "${at[@]}" >"$dir/drive.txt" 2>&1 || {
    tail -40 "$dir/drive.txt"
    return 1
  }
  grep -hE "ANT|ant\.|heart|devices|window system" "$dir/drive.txt" | grep -v "$KEY" | tail -30 || true
}

# A: Bea Climber – slim, auburn hair, gravel bike in green, chevron kit.
at=(
  --at "2=menu devices"
  --at "2.5=devices.scan 6"
  --at "5=screenshot $out/A-1-scan.png"
  --at "9=devices.connect ant:120:12345:1"
  --at "9.2=devices.connect ant:120:23456:1"
  --at "14=screenshot $out/A-2-paired.png"
  --at "14.5=menu ant"
  --at "15.5=screenshot $out/A-3-ant.png"
  --at "16=menu devices"
  # Scan, Share with a browser, ANT+, the first strap, the second.
  --at "16.5=menu down" --at "16.8=menu down" --at "17.1=menu down" --at "17.4=menu down"
  --at "17.8=menu select"
  --at "18.5=screenshot $out/A-4-strap-page.png"
  --at "19=menu down" --at "19.3=menu down" --at "19.6=menu down" --at "19.9=menu down"
  --at "20.3=menu select"
  --at "21.5=screenshot $out/A-5-use-for-heart-rate.png"
  --at "22=menu close"
  --at "25=screenshot $out/A-6-ride-hud.png"
)
run A "active = \"Bea Climber\"
[[rider]]
name = \"Bea Climber\"
weight_kg = 58.0
height_cm = 166.0
bike = \"gravel\"
frame_color = \"#2E7D32\"
jersey_color = \"#FDD835\"
kit = \"chevrons\"
kit_second = \"#1E1E22\"
helmet_color = \"#E53935\"
body_build = \"slim\"
skin = 2
hair = 3
hair_color = \"#8D3B1F\"
glasses = 1
" "[ant]
network_key = \"$KEY\"
" "" 45 --ant-sim "hr:12345:142,hr:23456:128"

# B: Cem Sprinter – strong, dark skin, short black hair, road bike in red.
at=(
  --at "2=menu ant"
  --at "3=screenshot $out/B-1-key-needed.png"
  --at "3.5=ant.key.edit"
  --at "4.5=screenshot $out/B-2-typing-the-key.png"
  --at "5=ant.key.cancel"
)
run B "active = \"Cem Sprinter\"
[[rider]]
name = \"Cem Sprinter\"
weight_kg = 86.0
height_cm = 188.0
bike = \"road-aero\"
frame_color = \"#C62828\"
jersey_color = \"#1565C0\"
kit = \"band\"
kit_second = \"#FFFFFF\"
helmet_color = \"#111111\"
body_build = \"strong\"
skin = 8
hair = 1
hair_color = \"#141414\"
" "" "" 14 --ant-sim "hr:12345:142"

# C: Dee Tempo – a remembered ANT+ strap reconnects; orbit camera on the rider.
at=(
  --at "4=camera.orbit 110 6 3.5"
  --at "9=screenshot $out/C-1-ride-hud-orbit.png"
)
run C "active = \"Dee Tempo\"
[[rider]]
name = \"Dee Tempo\"
weight_kg = 67.0
height_cm = 175.0
bike = \"tt\"
frame_color = \"#6A1B9A\"
jersey_color = \"#FF7043\"
kit = \"gradient\"
kit_second = \"#6A1B9A\"
helmet_color = \"#F2F2EE\"
skin = 5
hair = 5
hair_color = \"#D9B26F\"
glasses = 2
" "[ant]
network_key = \"$KEY\"
" "[[device]]
id = \"ant:120:34567:1\"
name = \"ANT+ heart rate 34567\"
kind = \"hr\"
transport = \"ant\"
" 22 --ant-sim "hr:34567:156"

ls "$out"
