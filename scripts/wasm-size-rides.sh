#!/usr/bin/env bash
# The browser app's size and phone numbers in one go (spec 14 Phase 10, 10G
# *Memory on phones*): scripts/wasm-size.sh for the `web-release` profile,
# `make web` and the Liechtenstein world, then the 5-minute WebKit iPhone 15
# rides (Loop, Hills, Liechtenstein Vaduz – Malbun) and the frame rate of
# headless Chromium on WebGPU and on WebGL 2 – everything into
# test-output/wasm-size/LABEL/. `--preview` also measures the preview's
# profile (fat LTO, slow); `--no-rides` only the sizes. Runs on a build
# machine with WebKit (`make deps-webkit`) and the world packages, e.g.
#   scripts/remote.sh marge run 'scripts/wasm-size-rides.sh LABEL'
set -uo pipefail
cd "$(dirname "$0")/.."
label=${1:?usage: scripts/wasm-size-rides.sh LABEL [--preview] [--no-rides]}
shift
preview=0
rides=1
for a in "$@"; do
  case $a in
    --preview) preview=1 ;;
    --no-rides) rides=0 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done
out=test-output/wasm-size/$label
mkdir -p "$out"
scripts/wasm-size.sh --profile web-release --out "$out" 2>&1 | grep -v '^ *Compiling' | tail -40
make web 2>&1 | tail -12
cp target/web/sizes.txt "$out/web-sizes.txt"
if [ $rides = 1 ]; then
  make web-worlds 2>&1 | tail -3
  scripts/test-web-ios.py --out "$out/ios" --minutes 5 --route loop --label loop 2>&1 | tail -6
  scripts/test-web-ios.py --out "$out/ios" --minutes 5 --route hills --label hills 2>&1 | tail -6
  scripts/test-web-ios.py --out "$out/ios" --minutes 5 --world liechtenstein --route vaduz-malbun --label liechtenstein 2>&1 | tail -6
  scripts/test-web.py --out "$out/web" --fps-seconds 10 2>&1 | tail -8
  scripts/test-web.py --out "$out/web" --gpu webgl2 --fps-seconds 10 2>&1 | tail -8
  # The same without vsync and the frame-rate cap: the frame time itself.
  uncapped="--disable-gpu-vsync --disable-frame-rate-limit"
  OCW_CHROMIUM_FLAGS=$uncapped scripts/test-web.py --out "$out/web-uncapped" --fps-seconds 10 2>&1 | tail -3
  OCW_CHROMIUM_FLAGS=$uncapped scripts/test-web.py --out "$out/web-uncapped" --gpu webgl2 --fps-seconds 10 2>&1 | tail -3
fi
if [ $preview = 1 ]; then
  scripts/wasm-size.sh --profile web-preview-release --features preview --out "$out/preview" 2>&1 | grep -v '^ *Compiling' | tail -40
fi
echo "done: $out"
