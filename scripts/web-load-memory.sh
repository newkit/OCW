#!/usr/bin/env bash
# A world's load in WebKit as the reporting iPhone, stage by stage (spec 14
# Phase 10, 10G *Bisecting a phone's crash*; the iPhone that closed the
# page while Andorra's and Tenerife's bases were loaded, 2026-11-27):
#
#   scripts/web-load-memory.sh OUT PKG:WORLD_ID [PKG:WORLD_ID ...]
#
# publishes the packages for the dev server (scripts/publish-web-worlds.sh,
# target/web/worlds) and loads each world in WebKit at 430 x 932, pixel
# ratio 3 (scripts/test-web-ios.py --viewport 430x932: Safari's user agent,
# WebGL 2, Low, the page log) until the ride runs, then rides 20 s. Prints
# each run's stage table (wasm memory, heap, its peak, the web process's
# RSS per "world load:" stage) and the web process's RSS high-water mark.
# WEB_LOAD_QUERY adds page options (e.g. base=all, memory=minimal),
# WEB_LOAD_NO_PUBLISH=1 uses the worlds already published.
# Needs `make web` and `make deps-webkit`.
set -uo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-load-memory.sh OUT PKG:WORLD_ID ...}
shift
mkdir -p "$out"
pkgs=()
for pw in "$@"; do pkgs+=("${pw%%:*}"); done
if [ -z "${WEB_LOAD_NO_PUBLISH:-}" ]; then
  scripts/publish-web-worlds.sh "${pkgs[@]}" > "$out/publish.txt" 2>&1 || { tail -5 "$out/publish.txt"; exit 1; }
fi
for pw in "$@"; do
  world=${pw#*:}
  label=$(basename "$world")${WEB_LOAD_LABEL:+-$WEB_LOAD_LABEL}
  scripts/test-web-ios.py --out "$out" --world "$world" --route "" --label "$label" \
    --minutes "${WEB_LOAD_MINUTES:-0.34}" --background-every 0 --no-context-loss --budget-mb 0 \
    --viewport 430x932 --query "gpu=webgl2&quality=low&log=1${WEB_LOAD_QUERY:+&$WEB_LOAD_QUERY}" \
    > "$out/$label.txt" 2>&1
  echo "== $label: exit $?"
  grep -E 'world load stages|^    +[0-9.]+ s |high-water|checks passed|FAIL' "$out/$label.txt"
done
