#!/usr/bin/env bash
# Headless photo run of the ride photographer (spec §2.11) and the Strava
# recording (spec §2.8.1) – on marge (the GPU, headless weston):
#
#   scripts/test-photographer.sh [OUT_DIR] [SECONDS] [drone]
#       (default test-output/photographer, 480 s; `drone`: the photos by the
#       drone – `photographer drone` after 25 s and 170 s, up to speed)
#
# Rides Tenerife (Buenavista - Masca - Santiago del Teide: cliffs and the
# sea) on the virtual trainer at 350 W with the photographer's time 10×
# faster (--photo-time-scale 10: 2 minutes between photos instead of 20),
# a Strava recording from the start (--strava-dry-run: saved, never
# uploaded) and a throwaway data folder (OCW_DATA_DIR). Whenever the
# photographer or the drone is in the rider's view the window is saved
# (--photo-chase-shots). Writes to OUT_DIR:
#
#   photos/photo-N.jpg, photo-N.toml   the ride photos and their sidecars
#   chase/chase-NN-photographer|drone.jpg  the rider's view with them in it
#   strava/*.fit                        the Strava recording's activity
#   end.jpg, end.log                    the last frame and the app's log
#
# OCW_WORLD_DATA: the folder with tenerife/ (default ~/src/OCW/data).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-test-output/photographer}
seconds=${2:-480}
mode=${3:-}
extra=()
if [ "$mode" = drone ]; then
  extra=(--at "25=photographer drone" --at "170=photographer drone")
fi
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
pkg=$data/tenerife/tenerife.ocwpkg
[ -f "$pkg" ] || { echo "no $pkg (OCW_WORLD_DATA)" >&2; exit 2; }
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data
export OCW_CONFIG_DIR=$work/config
export OCW_TEST_OUTPUT=off
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
export OCW_SHOT_TIMEOUT=$((seconds + 240))
export OCW_SIZE=${OCW_SIZE:-1920x1080}
.claude/skills/run-ocw/drive.sh shot-wayland "$out/end.jpg" "$seconds" -- \
  --world "$pkg" --route "Buenavista - Masca - Santiago del Teide" \
  --name "Ada Rider" --virtual --power 350 --bots 0 --no-hearts \
  --record true --photo-time-scale 10 --strava-dry-run \
  --photo-chase-shots "$out/chase" \
  --cmd "time golden-hour" --cmd "weather clear" \
  --cmd "strava.record start" --cmd "photographer on" "${extra[@]}"
rm -rf "$out/photos" "$out/strava"
mkdir -p "$out/photos" "$out/strava"
find "$OCW_DATA_DIR" -path '*/photos/*' -type f -exec cp {} "$out/photos/" \;
find "$OCW_DATA_DIR" -path '*/rides/strava/*' -type f -exec cp {} "$out/strava/" \;
echo "photos: $(find "$out/photos" -name '*.jpg' | wc -l), chase views: $(find "$out/chase" -name '*.jpg' 2>/dev/null | wc -l), Strava activities: $(find "$out/strava" -name '*.fit' | wc -l)"
grep -E 'ride photo|chase view|Strava recording' "$out/end.log" | sed 's|/[^ ]*/data/|DATA/|g' | tail -40 || true
