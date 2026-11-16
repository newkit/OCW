#!/usr/bin/env bash
# Phase 15 M1 measurement (spec §14 Phase 15): start-up time, peak memory and
# render rate of two ocw-desktop builds on the Black Forest's Schauinsland
# climb on a summer day, interleaved (A B A B, the machine is shared), plus
# the second build with the plants drawn from their model files
# (OCW_FLORA=files). Run on a GPU machine (marge) after building both:
#
#   scripts/measure-models.sh BEFORE_BIN AFTER_BIN [ROUNDS]
#
# Prints `measure: …` (wall, peak RSS) and `…: N fps mean` lines per run.
set -euo pipefail
cd "$(dirname "$0")/.."
before=${1:?before binary} after=${2:?after binary} rounds=${3:-2}
bf=${OCW_BLACKFOREST:-$HOME/src/OCW/data/blackforest}
args=(--world "$bf/blackforest.ocwpkg" --route-file "$bf/freiburg-schauinsland.route"
  --no-neighbours --bots 2 --power 300
  --at "0.5=time pause" --at "0.5=time 2026-06-21 13:00" --at "2.5=teleport 12.0")
out=$(mktemp)
trap 'rm -f "$out"' EXIT
ls -l "$before" "$after" | awk '{print "binary", $5, $NF}'
for r in $(seq "$rounds"); do
  for run in before after after-files; do
    case $run in
      before) bin=$before; env=() ;;
      after) bin=$after; env=() ;;
      after-files) bin=$after; env=(OCW_FLORA=files) ;;
    esac
    # Start-up: until the app quits 1 s of game time after the world loaded.
    env "${env[@]}" scripts/measure.py --label "startup $run #$r" --out "$out" -- \
      scripts/measure-fps.sh --app "$bin" --seconds 1 --warmup 0 --label "startup $run" -- "${args[@]}" \
      >/dev/null 2>&1 || true
    tail -n 1 "$out"
    env "${env[@]}" scripts/measure.py --label "ride $run #$r" -- \
      scripts/measure-fps.sh --app "$bin" --seconds 40 --warmup 12 --label "fps $run #$r" -- "${args[@]}" || true
  done
done 2>&1 | grep -E '^(binary|measure:|fps|startup)'
