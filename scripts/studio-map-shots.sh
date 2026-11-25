#!/usr/bin/env bash
# Screenshots of OCW Studio's 2D map with the neighbouring packages, the
# world's border and the aerial-photo layer (spec 14 Phase 16, 16H Q15 (a))
# on Wayland (headless weston) through .claude/skills/run-ocw/drive.sh:
#
#   scripts/studio-map-shots.sh [OUT_DIR]      (default test-output/studio-map-q15)
#
# Black Forest middle with both neighbours (north, south) dimmed and its
# border, Liechtenstein with only its border, and the aerial photo off and
# on in both. No riders are drawn in the studio. Needs the world packages
# (OCW_WORLD_DATA, default ~/src/OCW/data, as on marge). Writes
# OUT_DIR/index.md with one row per picture.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/studio-map-q15}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
mkdir -p "$out"
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900}
export OCW_PROFILE=${OCW_PROFILE:-release}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# A fresh autosave folder: no restore offer over the map.
drive=.claude/skills/run-ocw/drive.sh
bf=$data/blackforest/blackforest-middle.ocwpkg
li=$data/pilot/liechtenstein.ocwpkg
# Where the aerial pictures look (package metres, m per point).
bf_at=${BF_AT:-9570 -39670 1.5}  # Haslach im Kinzigtal
li_at=${LI_AT:--3386 -892 1.5}  # Schaan
printf '# OCW Studio – the 2D map: neighbours, border, aerial photo (spec 16H Q15 (a))\n\n| Picture | What it shows | Arguments |\n|---|---|---|\n' >"$out/index.md"
shot() {
  local name=$1 what=$2; shift 2
  "$drive" shot-wayland "$out/$name.jpg" 14 -- --autosave-dir "$work/autosave-$name" "$@"
  local args="$*"
  printf '| [%s.jpg](%s.jpg) | %s | `%s` |\n' "$name" "$name" "$what" "${args//$data/DATA}" >>"$out/index.md"
}
shot bf-middle-neighbours "Black Forest middle with its neighbours north and south dimmed (their roads and region), its own border as a thin line" \
  --world "$bf" --cmd "studio map-fit all"
shot bf-middle-border "Black Forest middle fitted to itself: the border line, the neighbours' roads beyond it" \
  --world "$bf" --cmd "studio map-fit"
shot bf-middle-aerial-off "Black Forest middle zoomed in, aerial photo off (the default)" \
  --world "$bf" --cmd "studio map $bf_at"
shot bf-middle-aerial-on "The same view with the aerial photo on (studio layer aerial on; the orthophoto corridor along the roads)" \
  --world "$bf" --cmd "studio map $bf_at" --cmd "studio layer aerial on"
shot li-border "Liechtenstein: no neighbouring package, so only its border – nothing beyond it" \
  --world "$li" --cmd "studio map-fit all"
shot li-aerial-off "Liechtenstein zoomed in, aerial photo off" \
  --world "$li" --cmd "studio map $li_at"
shot li-aerial-on "The same view with the aerial photo on" \
  --world "$li" --cmd "studio map $li_at" --cmd "studio layer aerial on"
echo "screenshots in $out (index.md)"
