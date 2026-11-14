#!/usr/bin/env bash
# Physics engine spike (spec §14 Phase 12 Q5): compile time on top of Bevy,
# wasm32 build, and a ragdoll chain step, for Avian and Rapier.
set -uo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_DIR=/var/tmp/ocw-physics-spike-target
log=/tmp/ocw-spike-$$.log
t() {
  local s e rc
  s=$(date +%s)
  "$@" > "$log" 2>&1
  rc=$?
  e=$(date +%s)
  echo "$((e - s)) s (exit $rc): $*"
  if [ "$rc" != 0 ]; then tail -40 "$log"; fi
}
echo "== native (dev, tests)"
(cd base && t cargo build --tests)
(cd avian && t cargo build --tests)
(cd rapier && t cargo build --tests)
echo "== wasm32 (dev, lib)"
(cd base && t cargo build --lib --target wasm32-unknown-unknown)
(cd avian && t cargo build --lib --target wasm32-unknown-unknown)
(cd rapier && t cargo build --lib --target wasm32-unknown-unknown)
echo "== release step"
(cd avian && cargo test --release -q -- --nocapture 2>&1 | grep -E "avian|test result|panicked|error" | head)
(cd rapier && cargo test --release -q -- --nocapture 2>&1 | grep -E "rapier|test result|panicked|error" | head)
echo "== crates added (vs base)"
(cd base && cargo tree -e normal --prefix none -f '{p}' | sort -u > /tmp/ocw-spike-base.txt)
for d in avian rapier; do
  (cd $d && cargo tree -e normal --prefix none -f '{p} {l}' | sort -u > /tmp/ocw-spike-$d.txt)
  echo "$d: $(cut -d' ' -f1,2 /tmp/ocw-spike-$d.txt | sort -u | comm -23 - <(cut -d' ' -f1,2 /tmp/ocw-spike-base.txt | sort -u) | wc -l) new crates"
  cut -d' ' -f1,2 /tmp/ocw-spike-$d.txt | sort -u | comm -23 - <(cut -d' ' -f1,2 /tmp/ocw-spike-base.txt | sort -u) | while read -r n v; do grep -m1 "^$n $v " /tmp/ocw-spike-$d.txt; done
done
