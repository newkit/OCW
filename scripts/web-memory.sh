#!/usr/bin/env bash
# The browser app's memory on the big worlds (spec §14 Phase 10 B2, the
# memory per resident tile): rides Liechtenstein, Tenerife and the northern
# Black Forest in headless Chromium on WebGPU and WebGL 2 (scripts/test-web.py
# --world, a teleport to km 6 to switch tiles) and prints a table of the
# peak wasm memory, the live heap and its peak and the GPU memory.
#
#   scripts/web-memory.sh OUT [WORLD:ROUTE ...]
#
# Needs `make web` and the worlds published (scripts/publish-web-worlds.sh
# with the packages); OUT gets each run's screenshots, log and JSON.
set -uo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-memory.sh OUT [WORLD:ROUTE ...]}
shift
[ $# -gt 0 ] || set -- liechtenstein:vaduz-malbun \
  europe/spain/tenerife:puerto-de-la-cruz-teide \
  europe/germany/black-forest-north:baden-baden-mummelsee
mkdir -p "$out"
for gpu in ${WEB_MEMORY_GPUS:-webgpu webgl2}; do
  for wr in "$@"; do
    world=${wr%%:*}
    route=${wr#*:}
    name=$(basename "$world")-$gpu
    mkdir -p "$out/$name"
    scripts/test-web.py --out "$out/$name" --world "$world" --route "$route" \
      --seconds "${WEB_MEMORY_SECONDS:-20}" --teleport 6 --teleport-wait "${WEB_MEMORY_WAIT:-20}" \
      --time noon --no-reload --gpu "$gpu" --max-wasm-mb 0 --timeout 300 \
      > "$out/$name/run.txt" 2>&1
    echo "$name: exit $?"
  done
done
python3 -I - "$out" <<'EOF'
import glob, json, os, sys
rows = []
for f in sorted(glob.glob(os.path.join(sys.argv[1], "*", "web-*.json"))):
    s = json.load(open(f))
    v = (s.get("visits") or [{}])[0]
    t = v.get("memory_timeline") or []
    h = ((v.get("memory") or {}).get("heap")) or {}
    last = t[-1] if t else {}
    rows.append((os.path.basename(os.path.dirname(f)), v.get("ok"), v.get("peak_wasm_mb"),
                 v.get("peak_heap_mb"), last.get("heap_mb"), last.get("resident"),
                 last.get("gpu_buffers_mb"), last.get("gpu_textures_mb"), v.get("errors")))
print("| run | ok | wasm peak MB | heap peak MB | heap end MB | tiles | GPU meshes MB | GPU textures MB |")
print("|---|---|---|---|---|---|---|---|")
for r in rows:
    print("| " + " | ".join("" if x is None else str(x) for x in r[:8]) + " |")
for r in rows:
    if r[8]:
        print(r[0], "errors:", str(r[8])[:300])
EOF
