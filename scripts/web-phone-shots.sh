#!/usr/bin/env bash
# Daylight screenshots of worlds as the reporting iPhone (WebKit at
# 430 x 932, WebGL 2, Low; spec 14 Phase 10, 10G *The first tiles' GPU
# memory*): each world with the phone's defaults and with reach=off (all
# 3 x 3 near tiles meshed, as before v7.02), 20 s into the ride at 13:00.
#
#   scripts/web-phone-shots.sh OUT WORLD_ID [WORLD_ID ...]
#
# Needs `make web`, the worlds published (scripts/publish-web-worlds.sh or
# scripts/web-load-memory.sh) and `make deps-webkit`.
set -uo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-phone-shots.sh OUT WORLD_ID ...}
shift
mkdir -p "$out"
for world in "$@"; do
  for q in "" "reach=off"; do
    tag=${q/=/-}
    label=$(basename "$world")-${tag:-default}
    scripts/test-web-ios.py --out "$out" --world "$world" --route "" --label "$label" \
      --minutes 0.34 --background-every 0 --no-context-loss --budget-mb 0 --viewport 430x932 \
      --query "gpu=webgl2&quality=low&bots=0${q:+&$q}" --console "time 13:00" > "$out/$label.txt" 2>&1
    echo "== $label: exit $?"
    grep -E "checks passed|  [0-9.]+ s end " "$out/$label.txt"
  done
done
