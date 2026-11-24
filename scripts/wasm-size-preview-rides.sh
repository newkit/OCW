#!/usr/bin/env bash
# The phone's numbers with the module a phone gets (spec 14 Phase 10, 10G
# *Memory on phones*): the public preview's profile and wasm-opt passes
# (`web-preview-release`, -Oz) but the dev server's trust, built into
# target/web-pv/ next to `make web`'s worlds; then the 5-minute WebKit
# iPhone 15 rides (Loop, Hills, Liechtenstein Vaduz – Malbun) and the
# uncapped frame rate of headless Chromium on WebGPU and WebGL 2, into
# test-output/wasm-size/LABEL/{ios-pv,web-pv}/. Needs `make web-worlds`.
#   scripts/remote.sh marge run 'scripts/wasm-size-preview-rides.sh LABEL'
set -uo pipefail
cd "$(dirname "$0")/.."
label=${1:?usage: scripts/wasm-size-preview-rides.sh LABEL}
out=test-output/wasm-size/$label
dir=target/web-pv
mkdir -p "$out"
WEB_PROFILE=web-preview-release WEB_WASM_OPT=-Oz WEB_OUT=$dir scripts/build-web.sh 2>&1 | tail -12
cp "$dir/sizes.txt" "$out/web-pv-sizes.txt"
[ -d target/web/worlds ] || make web-worlds 2>&1 | tail -3
[ -e "$dir/worlds" ] || cp -al target/web/worlds "$dir/worlds"
scripts/test-web-ios.py --dir "$dir" --out "$out/ios-pv" --minutes 5 --route loop --label loop 2>&1 | tail -4
scripts/test-web-ios.py --dir "$dir" --out "$out/ios-pv" --minutes 5 --route hills --label hills 2>&1 | tail -4
scripts/test-web-ios.py --dir "$dir" --out "$out/ios-pv" --minutes 5 --world liechtenstein --route vaduz-malbun --label liechtenstein 2>&1 | tail -4
uncapped="--disable-gpu-vsync --disable-frame-rate-limit"
OCW_CHROMIUM_FLAGS=$uncapped scripts/test-web.py --dir "$dir" --out "$out/web-pv" --fps-seconds 10 2>&1 | tail -2
OCW_CHROMIUM_FLAGS=$uncapped scripts/test-web.py --dir "$dir" --out "$out/web-pv" --gpu webgl2 --fps-seconds 10 2>&1 | tail -2
echo "done: $out"
