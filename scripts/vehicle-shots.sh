#!/usr/bin/env bash
# One picture per traffic model (spec 14 Phase 18, 18E T6): OCW Studio's
# Models workspace (the game's renderer) shows each file of
# artwork/3dmodels/vehicles/ from a three-quarter front view, headless on
# Wayland (weston) or X11 (Xvfb) through .claude/skills/run-ocw/drive.sh:
#
#   scripts/vehicle-shots.sh OUT_DIR [wayland|x11] [LIGHT] [GLOB]
#
# LIGHT is the viewer's light preset (day, dusk, night, overcast; default
# day); GLOB picks the files (default `*`, e.g. `airliner-*`). Writes
# OUT_DIR/index.md.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:?OUT_DIR}
mode=${2:-wayland}
light=${3:-day}
glob=${4:-*}
mkdir -p "$out"
drive=.claude/skills/run-ocw/drive.sh
export OCW_SIZE=${OCW_SIZE:-1600x900}
sub=shot
[ "$mode" = wayland ] && sub=shot-wayland
index="$out/index.md"
[ -f "$index" ] || printf '# The traffic models in OCW Studio (spec 14 Phase 18, 18E T6)\n\n| Picture | What it shows |\n|---|---|\n' >"$index"
for f in artwork/3dmodels/vehicles/$glob.glb; do
  id=vehicles/$(basename "$f" .glb)
  name=$(basename "$f" .glb)-$light-$mode
  OCW_PACKAGE=ocw-studio "$drive" "$sub" "$out/$name.jpg" 10 -- \
    --model "$id" --cmd "model show gizmos off" --cmd "model show labels off" \
    --cmd "model light $light" --cmd "model orbit 35 14"
  printf '| [%s](%s) | %s, %s light, %s: OCW Studio Models workspace, three-quarter front view |\n' \
    "$name.jpg" "$name.jpg" "$id" "$light" "$mode" >>"$index"
done
echo "screenshots in $out (index.md)"
