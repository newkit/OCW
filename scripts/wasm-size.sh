#!/usr/bin/env bash
# What the browser app's wasm module is made of (spec 14 Phase 10, 10G
# *Memory on phones*): builds ocw-web for wasm32 with a cargo profile and
# each GPU feature, then writes per build into OUT/:
#
#   <gpu>.raw.wasm         cargo's module (with its name section)
#   <gpu>.sizes.txt        raw, code, data and gzip -9 sizes; the module
#                          after wasm-bindgen with names and producers
#                          stripped (what `make web` serves) and, with
#                          wasm-opt, after `$WASM_SIZE_OPT`
#   <gpu>.top.txt          twiggy top -n 200 (the biggest functions)
#   <gpu>.crates.txt       code bytes summed by crate (demangled names)
#   <gpu>.dominators.txt   twiggy dominators -d 4 -r 400
#
#   scripts/wasm-size.sh [--profile web-release] [--gpu "webgpu webgl2"]
#                        [--out test-output/wasm-size/LABEL] [--features F]
#
# Needs twiggy (`cargo install twiggy --locked`) and wasm-bindgen-cli; runs
# on a build machine (scripts/remote.sh MACHINE run 'scripts/wasm-size.sh').
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

profile=web-release
gpus="webgpu webgl2"
out=
extra=
while [ $# -gt 0 ]; do
  case $1 in
    --profile) profile=$2; shift 2 ;;
    --gpu) gpus=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --features) extra=,$2; shift 2 ;;
    *) echo "usage: scripts/wasm-size.sh [--profile P] [--gpu \"webgpu webgl2\"] [--out DIR] [--features F]" >&2; exit 2 ;;
  esac
done
out=${out:-test-output/wasm-size/$profile-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
command -v twiggy >/dev/null || { echo "twiggy missing: cargo install twiggy --locked" >&2; exit 1; }
wasm_opt=$(command -v wasm-opt || true)
[ -z "$wasm_opt" ] && [ -x scripts/fetch-binaryen.sh ] && wasm_opt=$(scripts/fetch-binaryen.sh 2>/dev/null || true)
opt_passes=${WASM_SIZE_OPT:--Oz}
dir=$profile
[ "$profile" = dev ] && dir=debug

for gpu in $gpus; do
  t0=$(date +%s)
  cargo build -p ocw-web --target wasm32-unknown-unknown --profile "$profile" --features "$gpu$extra"
  echo "$gpu: build $(( $(date +%s) - t0 )) s"
  raw=$out/$gpu.raw.wasm
  cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/$dir/ocw_web.wasm" "$raw"
  bound=$(mktemp -d)
  wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
    --out-name ocw_web --out-dir "$bound" "$raw"
  served=$bound/ocw_web_bg.wasm
  python3 -I - "$raw" "$served" > "$out/$gpu.sizes.txt" <<'EOF'
import gzip, sys

def leb(b, i):
    r = s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7f) << s; s += 7
        if x < 0x80:
            return r, i

def sections(path):
    b = open(path, "rb").read()
    i, out = 8, {}
    names = {0: "custom", 1: "type", 2: "import", 3: "function", 4: "table", 5: "memory",
             6: "global", 7: "export", 8: "start", 9: "element", 10: "code", 11: "data", 12: "datacount"}
    while i < len(b):
        sid = b[i]; i += 1
        size, i = leb(b, i)
        key = names.get(sid, str(sid))
        if sid == 0:
            n, j = leb(b, i)
            key = "custom:" + b[j:j + n].decode("utf-8", "replace")
        out[key] = out.get(key, 0) + size
        i += size
    return b, out

for label, path in (("cargo (raw)", sys.argv[1]), ("served (wasm-bindgen, names stripped)", sys.argv[2])):
    b, s = sections(path)
    gz = len(gzip.compress(b, 9))
    print(f"{label}: {len(b)} bytes ({len(b) / 1048576:.2f} MiB), gzip -9 {gz} bytes ({gz / 1048576:.2f} MiB)")
    for k, v in sorted(s.items(), key=lambda kv: -kv[1]):
        print(f"  {k:28} {v:>10} bytes ({v / 1048576:6.2f} MiB)")
EOF
  if [ -n "$wasm_opt" ]; then
    t1=$(date +%s)
    "$wasm_opt" $opt_passes --enable-bulk-memory --enable-nontrapping-float-to-int \
      --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue --enable-simd \
      -o "$bound/opt.wasm" "$served"
    o=$(stat -c %s "$bound/opt.wasm"); g=$(gzip -9 -c "$bound/opt.wasm" | wc -c)
    echo "after wasm-opt $opt_passes ($(( $(date +%s) - t1 )) s): $o bytes ($(awk -v b="$o" 'BEGIN{printf "%.2f", b/1048576}') MiB), gzip -9 $g bytes ($(awk -v b="$g" 'BEGIN{printf "%.2f", b/1048576}') MiB)" >> "$out/$gpu.sizes.txt"
  fi
  rm -rf "$bound"
  twiggy top -n 200 "$raw" > "$out/$gpu.top.txt"
  twiggy dominators -d 4 -r 400 "$raw" > "$out/$gpu.dominators.txt"
  twiggy top -n 1000000 --format csv "$raw" > "$out/$gpu.all.csv"
  python3 -I - "$out/$gpu.all.csv" > "$out/$gpu.crates.txt" <<'EOF'
import csv, re, sys
from collections import Counter
by = Counter(); count = Counter(); total = 0
rows = csv.DictReader(open(sys.argv[1]))
for r in rows:
    name, size = r["Name"], int(r["ShallowSize"])
    total += size
    if name.startswith("data["):
        key = "<data segments>"
    elif name.startswith(("code[", "type[", "elem[", "global[", "import ", "export ", "table[", "memory[")) or name.startswith('"'):
        key = "<unnamed / sections>"
    else:
        # Leading generic impls: `<T as crate::...>` – take the first path.
        # v0 names: `crate[hash]::path`; the first crate named (a generic
        # impl `<A as B>::f` counts for A's crate).
        m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\[[0-9a-f]+\]::", name) or re.search(r"([A-Za-z_][A-Za-z0-9_]*)::", name)
        key = m.group(1) if m else "<other: " + name.split("(")[0][:30] + ">"
    by[key] += size; count[key] += 1
print(f"{'crate':40} {'bytes':>10} {'MiB':>7} {'items':>7}")
for k, v in by.most_common(120):
    print(f"{k:40} {v:>10} {v / 1048576:7.2f} {count[k]:>7}")
print(f"{'total':40} {total:>10} {total / 1048576:7.2f}")
EOF
  rm -f "$out/$gpu.all.csv"
  echo "$gpu: done in $(( $(date +%s) - t0 )) s -> $out/$gpu.*"
done
