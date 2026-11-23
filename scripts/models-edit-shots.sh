#!/usr/bin/env bash
# Headless screenshots of OCW Studio's Models workspace editing (spec §14
# Phase 16, 16B / 16F E4) on X11 (Xvfb) and Wayland (headless weston),
# through .claude/skills/run-ocw/drive.sh:
#
#   scripts/models-edit-shots.sh [OUT_DIR]      (default test-output/models-e4)
#
# A bike with a node selected and the move gizmo (and the rotate and scale
# gizmos), an anchor being added, collision proxies in both forms (a mesh
# and a scaled empty), the paint slot picker, and the diff shown before a
# save. Nothing is saved: the save dialog waits for *Save*. Writes
# OUT_DIR/index.md with one row per picture.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/models-e4}
mkdir -p "$out"
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900}
drive=.claude/skills/run-ocw/drive.sh
printf '# OCW Studio – Models workspace, editing (spec §14 Phase 16, 16B, E4)\n\n| Picture | Window system | What it shows | Arguments |\n|---|---|---|---|\n' >"$out/index.md"
shot() {
  local name=$1 what=$2; shift 2
  for ws in x11 wayland; do
    local mode=shot
    [ "$ws" = wayland ] && mode=shot-wayland
    "$drive" "$mode" "$out/$name-$ws.jpg" 12 -- "$@"
    printf '| [%s-%s.jpg](%s-%s.jpg) | %s | %s | `%s` |\n' "$name" "$ws" "$name" "$ws" "$ws" "$what" "$*" >>"$out/index.md"
  done
}
bike=(--model bikes/bike-road-endurance --cmd "model show labels off" --cmd "model orbit 125 18 2.4")
shot bike-node-gizmo "The road bike with its saddle selected (highlighted) and the move gizmo, moved up 2 cm with the numeric fields" \
  "${bike[@]}" --cmd "studio model select saddle" --cmd "studio model info" \
  --cmd "studio model nudge 0 0.02 0"
shot bike-rotate-gizmo "The handlebar selected with the rotate gizmo's rings" \
  "${bike[@]}" --cmd "studio model select handlebar" --cmd "studio model gizmo rotate"
shot bike-scale-gizmo "The front wheel selected with the scale gizmo (the node's own axes)" \
  "${bike[@]}" --cmd "studio model select wheel_front" --cmd "studio model gizmo scale"
shot anchor-added "An anchor being added: anchor_bottle on the down tube of the bike, selected, with its gizmo and the tree" \
  --model bikes/bike-road-endurance --cmd "model orbit 100 15 2.2" --cmd "studio model select bike" \
  --cmd "studio model anchor bottle 0 0.42 0.12"
shot proxy-mesh "A collision proxy added as a mesh (col_box around the bollard, red) on a prop" \
  --model props/bollard-round --cmd "model orbit 35 15 2.2" --cmd "studio model proxy box mesh"
shot proxy-empty "Collision proxies in the other form: a scaled empty col_cylinder (15H Q9: both forms stay)" \
  --model props/block-concrete-box --cmd "model orbit 35 20 3.2" --cmd "studio model proxy cylinder empty"
shot slot-picker "The paint slot picker of the bike's first material (84 slots with their default colours)" \
  --model bikes/bike-road-endurance --cmd "studio model picker 0" --cmd "model orbit 125 18 2.4"
shot diff-before-save "The diff before saving: the saddle moved and renamed, an anchor added, the manifest's author changed – Save or Cancel" \
  "${bike[@]}" --cmd "studio model select saddle" --cmd "studio model nudge 0 0.02 -0.01" --cmd "studio model rename saddle_comfort" \
  --cmd "studio model select bike" --cmd "studio model anchor bottle 0 0.42 0.12" \
  --cmd "studio model entry author Ada Rider" --cmd "studio model save"
echo "screenshots in $out (index.md)"
