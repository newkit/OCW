#!/usr/bin/env bash
# Builds the browser app (spec §14 Phase 10, B1) into target/web/:
#   ocw_web_bg.wasm, ocw_web.js (wasm-bindgen glue), index.html, sizes.txt
#
#   scripts/build-web.sh            the `web` profile (release, size-optimised)
#   WEB_PROFILE=dev scripts/build-web.sh   a quick debug build
#
# Toolchain: the rustup target wasm32-unknown-unknown (rust-toolchain.toml)
# and wasm-bindgen-cli of exactly the wasm-bindgen version in Cargo.lock:
#   cargo install wasm-bindgen-cli --version <version> --locked
# (`make deps-web` does that). wasm-opt (binaryen) is used when installed.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

profile=${WEB_PROFILE:-web}
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
cargo build -p ocw-web --target wasm32-unknown-unknown --profile "$profile"
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
wasm-bindgen --target web --no-typescript "${strip[@]}" --out-dir "$out" "$wasm"
if command -v wasm-opt >/dev/null && [ "$profile" != dev ]; then
  wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
    -o "$out/ocw_web_bg.wasm" "$out/ocw_web_bg.wasm"
fi
cp apps/web/web/index.html "$out/"

# Sizes (spec §14 Phase 10, 10C: ≤ 30 MB compressed).
bin=$out/ocw_web_bg.wasm
raw=$(stat -c %s "$bin")
gz=$(gzip -9 -c "$bin" | wc -c)
br=$(python3 -I -c 'import sys
try:
    import brotli
except ImportError:
    print("n/a"); sys.exit()
print(len(brotli.compress(open(sys.argv[1], "rb").read(), quality=11)))' "$bin" 2>/dev/null || echo n/a)
[ "$br" = n/a ] && command -v brotli >/dev/null && br=$(brotli -c -q 11 "$bin" | wc -c)
mib() { case $1 in n/a) echo n/a ;; *) awk -v b="$1" 'BEGIN { printf "%.1f MiB", b / 1048576 }' ;; esac; }
{
  echo "profile: $profile"
  echo "ocw_web_bg.wasm raw: $raw bytes ($(mib "$raw"))"
  echo "ocw_web_bg.wasm gzip -9: $gz bytes ($(mib "$gz"))"
  echo "ocw_web_bg.wasm brotli -q 11: $br bytes ($(mib "$br"))"
  echo "ocw_web.js: $(stat -c %s "$out/ocw_web.js") bytes"
  echo "build: $(( $(date +%s) - t0 )) s"
} | tee "$out/sizes.txt"
echo "built $out/ (serve it: make web-serve)"
