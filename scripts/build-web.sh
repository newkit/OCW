#!/usr/bin/env bash
# Builds the browser app (spec §14 Phase 10, B1 / B7) into target/web/:
#   ocw_web_bg.wasm, ocw_web.js                 the WebGPU build
#   ocw_web_webgl2_bg.wasm, ocw_web_webgl2.js   the WebGL 2 build (B7)
#   index.html (loads the one the browser can run), sizes.txt,
#   ocw-voice.js and livekit-client.umd.js (voice, B5)
#
#   scripts/build-web.sh            the `web-release` profile (release, size-optimised)
#   WEB_PROFILE=dev scripts/build-web.sh   a quick debug build
#   WEB_GPU=webgpu scripts/build-web.sh    only one of the builds (webgpu,
#                                          webgl2; default: both)
#
# Bevy chooses WebGPU or WebGL 2 when it is compiled (its shaders, limits
# and bind group layouts differ), so there are two modules: the `webgpu` /
# `webgl2` features of ocw-web.
#
# Toolchain: the rustup target wasm32-unknown-unknown (rust-toolchain.toml)
# and wasm-bindgen-cli of exactly the wasm-bindgen version in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <version> --locked
# (`make deps-web` does that). wasm-opt (binaryen) is used when installed.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

profile=${WEB_PROFILE:-web-release}
# The old name of the profile (before v3.35).
[ "$profile" = web ] && profile=web-release
gpus=${WEB_GPU:-webgpu webgl2}
out=target/web
# The version wasm-bindgen-cli must have: the crate's in Cargo.lock.
want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [ "$have" != "$want" ]; then
  echo "wasm-bindgen-cli $want needed (found: ${have:-none}):" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked   (or: make deps-web)" >&2
  exit 1
fi

t0=$(date +%s)
dir=$profile
[ "$profile" = dev ] && dir=debug
wasm=target/wasm32-unknown-unknown/$dir/ocw_web.wasm

# A fresh build, but the published worlds (`make web-worlds`) stay.
mkdir -p "$out"
find "$out" -mindepth 1 -maxdepth 1 ! -name worlds -exec rm -rf {} +
# The name section (function names for the console's stack traces) is
# 60 % of the module: only debug builds keep it.
strip=(--remove-name-section --remove-producers-section)
[ "$profile" = dev ] && strip=()
names=()
for gpu in $gpus; do
  case $gpu in
    webgpu) name=ocw_web ;;
    webgl2) name=ocw_web_webgl2 ;;
    *) echo "WEB_GPU: unknown $gpu (webgpu, webgl2)" >&2; exit 1 ;;
  esac
  t1=$(date +%s)
  cargo build -p ocw-web --target wasm32-unknown-unknown --profile "$profile" --features "$gpu"
  # Both builds write the same file: bind each before the next build.
  wasm-bindgen --target web --no-typescript "${strip[@]}" --out-name "$name" --out-dir "$out" "$wasm"
  if command -v wasm-opt >/dev/null && [ "$profile" != dev ]; then
    wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
      -o "$out/${name}_bg.wasm" "$out/${name}_bg.wasm"
  fi
  echo "$gpu: $(( $(date +%s) - t1 )) s"
  names+=("$name")
done
cp apps/web/web/index.html apps/web/web/ocw-voice.js "$out/"
# Voice (B5): LiveKit's JS SDK, pinned and hash-checked; without it the
# page rides on, voice says "unavailable".
scripts/fetch-livekit-client.sh "$out" ||
  echo "warning: livekit-client not fetched: voice is unavailable in this build" >&2

# Sizes (spec §14 Phase 10, 10C: ≤ 30 MB compressed).
mib() { case $1 in n/a) echo n/a ;; *) awk -v b="$1" 'BEGIN { printf "%.1f MiB", b / 1048576 }' ;; esac; }
{
  echo "profile: $profile"
  for name in "${names[@]}"; do
    bin=$out/${name}_bg.wasm
    raw=$(stat -c %s "$bin")
    gz=$(gzip -9 -c "$bin" | wc -c)
    br=$(python3 -I -c 'import sys
try:
    import brotli
except ImportError:
    print("n/a"); sys.exit()
print(len(brotli.compress(open(sys.argv[1], "rb").read(), quality=11)))' "$bin" 2>/dev/null || echo n/a)
    [ "$br" = n/a ] && command -v brotli >/dev/null && br=$(brotli -c -q 11 "$bin" | wc -c)
    echo "${name}_bg.wasm raw: $raw bytes ($(mib "$raw"))"
    echo "${name}_bg.wasm gzip -9: $gz bytes ($(mib "$gz"))"
    echo "${name}_bg.wasm brotli -q 11: $br bytes ($(mib "$br"))"
    echo "$name.js: $(stat -c %s "$out/$name.js") bytes"
  done
  echo "build: $(( $(date +%s) - t0 )) s"
} | tee "$out/sizes.txt"
echo "built $out/ (serve it: make web-serve)"
