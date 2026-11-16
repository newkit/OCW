#!/usr/bin/env bash
# The Devices page with a real Bluetooth stack (spec §3.2.1 *Devices*):
# runs the desktop app headless (drive.sh, X11 or Wayland), opens
# Settings -> Trainer -> Devices, scans, opens the first device found and
# connects it; screenshots of each step; then shows the devices.toml the
# app wrote. Uses its own config directory (XDG_CONFIG_HOME), so the
# user's and the kiosk's remembered devices are not touched.
#
#   scripts/test-devices-menu.sh [shot|shot-wayland] [OUT_DIR]
#
# Bluetooth addresses are masked in what it prints. With no device in
# range the list stays empty and the connect steps do nothing.
set -euo pipefail
# awk prints decimal points, not the locale's commas.
export LC_NUMERIC=C
cd "$(dirname "$0")/.."
mode=${1:-shot}
out=${2:-/tmp/ocw-devices-menu}
rm -rf "$out"
mkdir -p "$out/config"
export XDG_CONFIG_HOME="$out/config"
unset OCW_CONFIG_DIR
mask() { sed -E 's/([0-9A-Fa-f]{2}[:_]){5}[0-9A-Fa-f]{2}/XX:XX:XX:XX:XX:XX/g'; }

at=()
t=2
step() { at+=(--at "$t=$1"); t=$(awk "BEGIN { print $t + ${2:-0.4} }"); }
step "menu open"
for _ in 1 2 3 4 5; do step "menu down"; done # Settings
step "menu select"
step "menu down"; step "menu down"            # Trainer
step "menu select"
step "menu down"; step "menu down"            # Devices
step "menu select"
step "menu select" 3                          # Scan for devices
step "screenshot $out/1-scanning.png" 8
step "screenshot $out/2-found.png" 0.5
step "menu down"                              # the first device
step "menu select"
for _ in 1 2 3 4; do step "menu down"; done   # Connect
step "menu select" 2
step "screenshot $out/3-connecting.png" 25
step "screenshot $out/4-connected.png" 0.5
step "menu back" 1
step "screenshot $out/5-list.png" 1
step "devices.list" 1
OCW_SIZE=${OCW_SIZE:-1600x900} .claude/skills/run-ocw/drive.sh "$mode" "$out/6-end.jpg" \
  "$(awk "BEGIN { print int($t + 30) }")" -- --bots 2 "${at[@]}" >"$out/drive.txt" 2>&1 || {
  mask <"$out/drive.txt" | tail -40
  exit 1
}
grep -hE "remembered devices file|scan|connect|remembered|failed|devices|Wahoo|KICKR|window system" "$out/drive.txt" "$out/6-end.log" | mask | tail -40 || true
echo "--- $XDG_CONFIG_HOME/ocw/devices.toml:"
mask <"$XDG_CONFIG_HOME/ocw/devices.toml" 2>/dev/null || echo "(not written)"
ls "$out"
