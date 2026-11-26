#!/usr/bin/env bash
# Screenshots of OCW Studio planning with our own BRouter (spec 14 Phase 16,
# 16D; 16H Q21 (a), Q22 (a)) on Wayland (headless weston) through
# .claude/skills/run-ocw/drive.sh, with the server of
# scripts/brouter-test-server.sh running only meanwhile:
#
#   scripts/studio-brouter-shots.sh [OUT_DIR]   (default test-output/studio-brouter)
#
# 1. the question before the via-points first go to the server (the consent
#    dialog, asked once per server);
# 2. Vaduz - Malbun planned by our BRouter from the package route's start
#    and end, matched onto the Liechtenstein package (the yes remembered in
#    a fresh planner.json of the run).
# Needs the Liechtenstein package (BROUTER_PACKAGE in the BRouter test
# config). Writes OUT_DIR/index.md with one row per picture.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/studio-brouter}
if [ -z "${OCW_BROUTER_URL:-}" ]; then
  exec scripts/brouter-test-server.sh run "$0" "$@"
fi
mkdir -p "$out"
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900}
export OCW_PROFILE=${OCW_PROFILE:-release}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
drive=.claude/skills/run-ocw/drive.sh
li=$OCW_BROUTER_PACKAGE
# The package route Vaduz - Malbun's start and end (package metres of the
# Liechtenstein package of 2026-11; the live planner test prints them,
# BROUTER_FROM / BROUTER_TO override them).
from=${BROUTER_FROM:--2426 2219}
to=${BROUTER_TO:-4231 6452}
printf '# OCW Studio – planning with our own BRouter (spec 16H Q21 (a), Q22 (a))\n\n| Picture | What it shows |\n|---|---|\n' >"$out/index.md"
shot() {
  local name=$1 what=$2; shift 2
  # A fresh config folder per picture: its own planner.json.
  OCW_CONFIG_DIR=$work/config-$name "$drive" shot-wayland "$out/$name.jpg" 16 -- \
    --autosave-dir "$work/autosave-$name" --world "$li" --workspace routes "$@"
  printf '| [%s.jpg](%s.jpg) | %s |\n' "$name" "$name" "$what" >>"$out/index.md"
}
common=(--cmd "studio route new Vaduz - Malbun" --cmd "studio route plan $from $to"
  --cmd "studio route planner brouter" --cmd "studio route brouter url $OCW_BROUTER_URL"
  --cmd "studio route brouter profile trekking")
shot brouter-consent "The question before the via-points first go to our BRouter: the tick box \"Send the via-points to SERVER to plan (they leave this computer)\", Remember \"don't send\", Plan with BRouter / Don't send / Cancel – asked once per server" \
  "${common[@]}" --cmd "studio route brouter ask"
shot brouter-vaduz-malbun "Vaduz - Malbun planned by our own BRouter (segments from our Liechtenstein extract and SRTM tile) from the package route's start and end, matched onto the package's roads; the yes remembered for the server (Forget next to it)" \
  "${common[@]}" --cmd "studio route brouter ok" --cmd "studio route brouter plan" --cmd "studio route fit"
echo "screenshots in $out (index.md)"
