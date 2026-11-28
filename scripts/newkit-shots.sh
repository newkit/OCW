#!/usr/bin/env bash
# Headless screenshots of the "newk it" bike (spec 14 Phase 15, *The "newk it"
# bike*: the user's own design, used with permission) in OCW Studio's Models
# workspace on Wayland (headless weston), through
# .claude/skills/run-ocw/drive.sh:
#
#   scripts/newkit-shots.sh [OUT_DIR]      (default test-output/newkit)
#
# The side view (the left side, front to the left, as in the user's photos),
# the unicorn on the head tube up close, a front three-quarter view low over
# the ground, the far model and the livery on the standard road frame are in
# the game's scenes (scripts/scenes.toml, `newkit-*`). Writes OUT_DIR/index.md
# with one row per picture.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/newkit}
mkdir -p "$out"
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900}
drive=.claude/skills/run-ocw/drive.sh
printf '# The "newk it" bike – OCW Studio, Models workspace (spec 14 Phase 15)\n\n| Picture | What it shows | Arguments |\n|---|---|---|\n' >"$out/index.md"
shot() {
  local name=$1 what=$2; shift 2
  "$drive" shot-wayland "$out/$name.jpg" 10 -- "$@"
  printf '| [%s.jpg](%s.jpg) | %s | `%s` |\n' "$name" "$name" "$what" "$*" >>"$out/index.md"
}
bike=bikes/bike-road-aero-newkit
shot studio-side "The \"newk it\" bike from its left side, front to the left (as the user's photo 1; Studio draws the slots plain – the flake and its magenta fade are the game's): \"newk it\" in pink along the down tube, pink inner fork, pink front rim letters and panel, yellow rear letters, pink tape, two black bottles" \
  --model "$bike" --cmd "model show gizmos off" --cmd "model orbit -90 4 2.3"
shot studio-unicorn "The unicorn on the head tube up close (as photo 2): white head with a closed eye, golden horn, rainbow mane; the pink inner fork below" \
  --model "$bike" --cmd "model show gizmos off" --cmd "model target 0 0.765 -0.335" --cmd "model orbit 0 8 0.24"
shot studio-front34 "Front three-quarter view low over the ground (as photo 6): the pink inner fork legs, the integrated bar with the computer out front, green axle ends" \
  --model "$bike" --cmd "model show gizmos off" --cmd "model target 0 0.45 -0.15" --cmd "model orbit -32 6 1.9"
shot studio-drive-side "The drive side (front to the right): the letters read from the rear to the front, the rear rim's yellow letters and block, the green lock rings" \
  --model "$bike" --cmd "model show gizmos off" --cmd "model orbit 90 4 2.3"
shot studio-hotspots "The \"newk it\" bike with its workshop hotspots, stats and validator panel" \
  --model "$bike"
echo "screenshots in $out (index.md)"
