#!/usr/bin/env bash
# Renders the logos of artwork/logos/ to the PNG icon sizes (rsvg-convert,
# librsvg; on marge: scripts/remote.sh marge run 'scripts/render-logos.sh'):
#
#   scripts/render-logos.sh [LOGO...]     (default: ocw ocw-studio)
#
# Writes artwork/logos/LOGO-<size>.png for 16, 32, 48, 64, 128, 256 and
# 512 px from LOGO.svg; 16 and 32 px come from LOGO-small.svg when there is
# one (a simplified variant that stays legible at that size).
set -euo pipefail
cd "$(dirname "$0")/../artwork/logos"
command -v rsvg-convert >/dev/null || { echo "rsvg-convert not found (librsvg2-bin)" >&2; exit 1; }
[ $# -gt 0 ] || set -- ocw ocw-studio
for logo in "$@"; do
  for s in 16 32 48 64 128 256 512; do
    src=$logo.svg
    if [ "$s" -le 32 ] && [ -f "$logo-small.svg" ]; then src=$logo-small.svg; fi
    rsvg-convert -w "$s" -h "$s" "$src" -o "$logo-$s.png"
    echo "$logo-$s.png  <- $src"
  done
done
