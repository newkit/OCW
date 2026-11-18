#!/usr/bin/env bash
# Headless screenshots of OCW Studio's Models workspace (spec §14 Phase 16,
# 16B / 16F E3) on Wayland (headless weston), through
# .claude/skills/run-ocw/drive.sh:
#
#   scripts/models-workspace-shots.sh [OUT_DIR]      (default test-output/models-e3)
#
# The rider with its anchors, a bike with its hotspots, the wireframe and
# the normals with a material highlighted, the level-of-detail switch (auto
# by the distance, and lod1 chosen), a helmet tried on a heavy and a slim
# rider, the rider on a bike in the drops, a plant in the small scene in
# autumn and winter, a prop, and the validator panel on a file shaped like
# a careless Blender export (made here from a committed helmet: a `.001`
# node, a material in Blender case, an unused material that is no slot, no
# extras, no copyright). Writes OUT_DIR/index.md with one row per picture.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/models-e3}
mkdir -p "$out/fixture/wear"
python3 -I - artwork/3dmodels/wear/helmet-road-vented.glb "$out/fixture/wear/helmet-broken.glb" <<'EOF'
import json, struct, sys
src, dst = sys.argv[1], sys.argv[2]
b = open(src, 'rb').read()
assert b[:4] == b'glTF', 'run git lfs pull'
n = struct.unpack('<I', b[12:16])[0]
doc = json.loads(b[20:20 + n])
rest = b[20 + n:]
doc['nodes'][0]['name'] += '.001'
doc['materials'][0]['name'] = doc['materials'][0]['name'].capitalize() + '.002'
doc['materials'].append({'name': 'gold', 'pbrMetallicRoughness': {'baseColorFactor': [1, 0.8, 0.2, 1]}})
doc['asset'].pop('extras', None)
doc['asset'].pop('copyright', None)
t = json.dumps(doc, separators=(',', ':')).encode()
t += b' ' * (-len(t) % 4)
open(dst, 'wb').write(b'glTF' + struct.pack('<III', 2, 20 + len(t) + len(rest), len(t)) + b'JSON' + t + rest)
EOF
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900}
drive=.claude/skills/run-ocw/drive.sh
printf '# OCW Studio – Models workspace (spec §14 Phase 16, 16B, E3)\n\n| Picture | What it shows | Arguments |\n|---|---|---|\n' >"$out/index.md"
shot() {
  local name=$1 what=$2; shift 2
  "$drive" shot-wayland "$out/$name.jpg" 10 -- "$@"
  printf '| [%s.jpg](%s.jpg) | %s | `%s` |\n' "$name" "$name" "$what" "$*" >>"$out/index.md"
}
shot rider "The rider (riders/rider-body) with its anchors and labels, the stats and the validator" \
  --model riders/rider-body
shot bike "A bike (bikes/bike-road-aero) with the workshop's hotspots hs_*" \
  --model bikes/bike-road-aero
shot wireframe "The TT bike as a wireframe" \
  --model bikes/bike-tt --cmd "model show wireframe" --cmd "model show gizmos off"
shot normals-highlight "The gravel bike's normals, the frame material highlighted" \
  --model bikes/bike-gravel --cmd "model show normals" --cmd "model select material frame" \
  --cmd "model show gizmos off" --cmd "model orbit 70 10 1.4"
shot lod-auto-far "Level of detail by distance: past 40 m the bike shows its lod1" \
  --model bikes/bike-road-endurance --cmd "model zoom 48" --cmd "model show gizmos off"
shot lod-near "The same bike up close: the near model" \
  --model bikes/bike-road-endurance --cmd "model zoom 3" --cmd "model show gizmos off"
shot lod1-chosen "lod1 chosen up close: the far bike with its seated rider" \
  --model bikes/bike-road-endurance --cmd "model lod lod1" --cmd "model show gizmos off"
shot lod-tree-lod1 "A tree's lod1 chosen in the level switch" \
  --model flora/tree-oak-broad-b --cmd "model lod lod1"
shot helmet-tryon-fat "Try it on: the aero helmet on a heavy (Fat), 1.90 m rider" \
  --model wear/helmet-aero-road --cmd "model try rider fat 190"
shot helmet-tryon-slim "Try it on: the same helmet on a slim 1.55 m rider" \
  --model wear/helmet-aero-road --cmd "model try rider skinny 155"
shot ride-drops "Try it on: the rider on the aero bike in the drops, pedalling" \
  --model riders/rider-body --cmd "model try ride road-aero drops" --cmd "model show gizmos off"
shot plant-scene-autumn "Try it on: a beech in the small scene in autumn, overcast light" \
  --model flora/tree-beech-round-a --cmd "model try scene autumn" --cmd "model light overcast"
shot plant-scene-winter "The same scene in winter: the leaves off, snow on the ground (the files hold only the summer shape, 15I: no bare branches)" \
  --model flora/tree-beech-round-a --cmd "model try scene winter"
shot prop "A prop (props/phone-slab) at dusk" \
  --model props/phone-slab --cmd "model light dusk"
shot validator-panel "The validator panel on a careless Blender export: errors and warnings with their fixes" \
  --model "$out/fixture/wear/helmet-broken.glb"
shot night "The rider at night (the Phase 9 light model)" \
  --model riders/rider-body --cmd "model light night" --cmd "model show gizmos off"
echo "screenshots in $out (index.md)"
