#!/usr/bin/env bash
# wasm-opt's pass sets on one module (spec 14 Phase 10, 10G *Memory on
# phones*): binds RAW.wasm with wasm-bindgen as `make web` does, then prints
# the raw and gzip -9 sizes after each set of passes.
#   scripts/wasm-opt-compare.sh RAW.wasm ["-Oz" "-Oz --converge" "-Os" ...]
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
raw=${1:?usage: scripts/wasm-opt-compare.sh RAW.wasm [PASSES...]}
shift
[ $# -gt 0 ] || set -- "-Oz" "-Oz --converge" "-Os" "-O2" "-O3"
wasm_opt=$(scripts/fetch-binaryen.sh)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
  --out-name m --out-dir "$tmp" "$raw"
b=$tmp/m_bg.wasm
echo "bound: $(stat -c %s "$b") bytes, gzip -9 $(gzip -9 -c "$b" | wc -c)"
for p in "$@"; do
  t0=$(date +%s)
  # shellcheck disable=SC2086
  "$wasm_opt" $p --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    --enable-mutable-globals --enable-reference-types --enable-multivalue --enable-simd -o "$tmp/o.wasm" "$b"
  echo "$p: $(stat -c %s "$tmp/o.wasm") bytes, gzip -9 $(gzip -9 -c "$tmp/o.wasm" | wc -c), $(( $(date +%s) - t0 )) s"
done
