#!/usr/bin/env bash
# Builds the browser app (spec §14 Phase 10, B1 / B7 / B8a) into target/web/:
#   ocw_web_bg.wasm, ocw_web.js                 the WebGPU build
#   ocw_web_webgl2_bg.wasm, ocw_web_webgl2.js   the WebGL 2 build (B7)
#   index.html (loads the one the browser can run), config.json (the page
#   configuration, B8a), sizes.txt,
#   ocw-voice.js and livekit-client.umd.js (voice, B5), ocw-gpu.js (the GPU
#   choice and the WebGPU watchdog, B7)
#
#   scripts/build-web.sh            the `web-release` profile (release, size-optimised)
#   scripts/build-web.sh --preview  the public preview (B8a, `make web-preview`)
#                                   into target/web-preview/: the
#                                   `web-preview-release` profile (fat LTO,
#                                   one codegen unit), the
#                                   `preview` feature (worlds signed by the web
#                                   preview's root only), wasm-opt required
#                                   (scripts/fetch-binaryen.sh), no LiveKit
#                                   (voice is off in the preview)
#   WEB_PROFILE=dev scripts/build-web.sh   a quick debug build
#   WEB_GPU=webgpu scripts/build-web.sh    only one of the builds (webgpu,
#                                          webgl2; default: both)
#   WEB_OUT=DIR, WEB_WASM_OPT="-O3"        another output directory, other
#                                          wasm-opt passes (default: -Oz for
#                                          the preview, -Os otherwise; off:
#                                          none)
#
# Bevy chooses WebGPU or WebGL 2 when it is compiled (its shaders, limits
# and bind group layouts differ), so there are two modules: the `webgpu` /
# `webgl2` features of ocw-web.
#
# Toolchain: the rustup target wasm32-unknown-unknown (rust-toolchain.toml)
# and wasm-bindgen-cli of exactly the wasm-bindgen version in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <version> --locked
# (`make deps-web` does that). wasm-opt (binaryen) runs on every release
# build (scripts/fetch-binaryen.sh fetches the pinned release when none is
# installed; required for --preview, else skipped with a warning when it
# cannot be had).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

preview=0
case ${1:-} in
  --preview) preview=1 ;;
  "") ;;
  *) echo "usage: scripts/build-web.sh [--preview]" >&2; exit 2 ;;
esac
if [ $preview = 1 ]; then
  profile=${WEB_PROFILE:-web-preview-release}
  out=${WEB_OUT:-target/web-preview}
  features=,preview
  opt_default=-Oz
else
  profile=${WEB_PROFILE:-web-release}
  # The old name of the profile (before v3.37).
  [ "$profile" = web ] && profile=web-release
  out=${WEB_OUT:-target/web}
  features=
  opt_default=-Os
fi
gpus=${WEB_GPU:-webgpu webgl2}
opt_passes=${WEB_WASM_OPT:-$opt_default}
# The version wasm-bindgen-cli must have: the crate's in Cargo.lock.
want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }' || true)
if [ "$have" != "$want" ]; then
  echo "wasm-bindgen-cli $want needed (found: ${have:-none}):" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked   (or: make deps-web)" >&2
  exit 1
fi

# wasm-opt: required for the preview, else used when it can be had (the
# pinned binaryen when none is installed).
wasm_opt=
if [ "$opt_passes" = off ]; then
  :
elif [ $preview = 1 ]; then
  wasm_opt=$(scripts/fetch-binaryen.sh)
elif [ "$profile" != dev ]; then
  # The pinned binaryen when none is installed (10G *Memory on phones*: the
  # test machines' modules were served without wasm-opt, a quarter bigger
  # than what the preview serves); without it the build goes on unoptimised.
  wasm_opt=$(scripts/fetch-binaryen.sh 2>/dev/null) ||
    { wasm_opt=; echo "warning: no wasm-opt (scripts/fetch-binaryen.sh failed): module not optimised" >&2; }
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
declare -A before=()
for gpu in $gpus; do
  case $gpu in
    webgpu) name=ocw_web ;;
    webgl2) name=ocw_web_webgl2 ;;
    *) echo "WEB_GPU: unknown $gpu (webgpu, webgl2)" >&2; exit 1 ;;
  esac
  t1=$(date +%s)
  cargo build -p ocw-web --target wasm32-unknown-unknown --profile "$profile" --features "$gpu$features"
  # Both builds write the same file: bind each before the next build.
  wasm-bindgen --target web --no-typescript "${strip[@]}" --out-name "$name" --out-dir "$out" "$wasm"
  before[$name]=$(stat -c %s "$out/${name}_bg.wasm")
  if [ -n "$wasm_opt" ]; then
    t2=$(date +%s)
    # The features rustc's wasm32 target enables (wasm-opt refuses a
    # module that uses one it was not told about).
    "$wasm_opt" $opt_passes --enable-bulk-memory --enable-nontrapping-float-to-int \
      --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
      -o "$out/${name}_bg.wasm" "$out/${name}_bg.wasm"
    echo "$gpu: wasm-opt $opt_passes $(( $(date +%s) - t2 )) s"
  fi
  echo "$gpu: $(( $(date +%s) - t1 )) s"
  names+=("$name")
done
cp apps/web/web/index.html apps/web/web/ocw-voice.js apps/web/web/ocw-gpu.js artwork/logos/ocw.svg "$out/"
# Add to Home Screen (spec 3.2.3): the web app manifest and its icons (the
# Apple touch icon is ocw-256.png).
cp apps/web/web/manifest.webmanifest artwork/logos/ocw-256.png artwork/logos/ocw-512.png "$out/"
# The page configuration (B8a): the dev server's page keeps every option
# (no server / voice / sign-in keys); the preview's site assembly
# (scripts/web-preview-site.py) writes its own with the worlds and rides.
if [ $preview = 1 ]; then
  printf '{\n  "channel": "preview",\n  "worlds": "worlds/",\n  "server": "",\n  "voice": "",\n  "signin": ""\n}\n' > "$out/config.json"
else
  printf '{\n  "channel": "dev",\n  "worlds": "worlds/"\n}\n' > "$out/config.json"
  # Voice (B5): LiveKit's JS SDK, pinned and hash-checked; without it the
  # page rides on, voice says "unavailable". The preview has no voice.
  scripts/fetch-livekit-client.sh "$out" ||
    echo "warning: livekit-client not fetched: voice is unavailable in this build" >&2
fi

# Sizes (spec §14 Phase 10, 10C: ≤ 30 MB compressed).
mib() { case $1 in n/a) echo n/a ;; *) awk -v b="$1" 'BEGIN { printf "%.1f MiB", b / 1048576 }' ;; esac; }
{
  echo "profile: $profile${features:+ (features$features)}"
  [ -n "$wasm_opt" ] && echo "wasm-opt: $("$wasm_opt" --version) $opt_passes"
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
    [ -n "$wasm_opt" ] && echo "${name}_bg.wasm before wasm-opt: ${before[$name]} bytes ($(mib "${before[$name]}"))"
    echo "${name}_bg.wasm raw: $raw bytes ($(mib "$raw"))"
    echo "${name}_bg.wasm gzip -9: $gz bytes ($(mib "$gz"))"
    echo "${name}_bg.wasm brotli -q 11: $br bytes ($(mib "$br"))"
    echo "$name.js: $(stat -c %s "$out/$name.js") bytes"
  done
  echo "build: $(( $(date +%s) - t0 )) s"
} | tee "$out/sizes.txt"
if [ $preview = 1 ]; then
  echo "built $out/ (the site: make web-preview-site)"
else
  echo "built $out/ (serve it: make web-serve)"
fi
