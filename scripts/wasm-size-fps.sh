#!/usr/bin/env bash
# The uncapped frame rate of built pages side by side (spec 14 Phase 10, 10G
# *Memory on phones*: does a smaller module draw slower?): headless Chromium
# without vsync and frame-rate cap on WebGPU and on WebGL 2, ROUNDS times
# per page directory, alternating, into test-output/wasm-size/LABEL/fps/.
#   scripts/wasm-size-fps.sh LABEL ROUNDS DIR...
set -uo pipefail
cd "$(dirname "$0")/.."
label=${1:?usage: scripts/wasm-size-fps.sh LABEL ROUNDS DIR...}
rounds=${2:?rounds}
shift 2
out=test-output/wasm-size/$label/fps
mkdir -p "$out"
export OCW_CHROMIUM_FLAGS="--disable-gpu-vsync --disable-frame-rate-limit"
for r in $(seq 1 "$rounds"); do
  for dir in "$@"; do
    for gpu in webgpu webgl2; do
      o=$out/$(basename "$dir")-$gpu-$r
      scripts/test-web.py --dir "$dir" --out "$o" --gpu "$gpu" --fps-seconds 10 >"$o.txt" 2>&1
      fps=$(python3 -I -c 'import glob, json, sys
fs = sorted(glob.glob(sys.argv[1] + "/*.json"))
d = json.load(open(fs[-1])) if fs else {}
print(d.get("fps"), d.get("ok"))' "$o")
      echo "$(basename "$dir") $gpu round $r: fps $fps" | tee -a "$out/summary.txt"
    done
  done
done
