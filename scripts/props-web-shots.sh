#!/usr/bin/env bash
# Browser pictures of the props from their model files (spec 15H MX / M2):
# headless Chromium streams Liechtenstein, rides Vaduz – Malbun and
# teleports into Vaduz (street lamps) by day and by night, with this
# checkout's target/web (after) and, with OCW_BEFORE_WEB=DIR, a checkout
# of the build before the switch (before). Run on marge after `make web`
# (and, in the before checkout, `make web` after copying this checkout's
# target/web-keys there).
#
#   OCW_BEFORE_WEB=../OCW-props-before scripts/props-web-shots.sh [OUT_DIR]
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-test-output/props-files/web}
before=${OCW_BEFORE_WEB:-}
make -s web-worlds
for build in after before; do
  dir=target/web
  if [ "$build" = before ]; then
    [ -n "$before" ] || continue
    dir=$before/target/web
    # The same published worlds (signed by this checkout's dev root, which
    # the before build was given: target/web-keys copied before its build).
    rm -rf "$dir/worlds" && cp -r target/web/worlds "$dir/worlds"
  fi
  for t in "noon|day" "2026-11-09 22:00|night"; do
    IFS='|' read -r time tag <<<"$t"
    scripts/test-web.py --dir "$dir" --out "$out/$build-$tag" --world liechtenstein \
      --route vaduz-malbun --seconds 15 --teleport 0.25 --no-reload --time "$time"
  done
done
