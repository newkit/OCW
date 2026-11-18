#!/usr/bin/env bash
# Headless run of the activities folder (spec §2.8.1 *Where an S recording
# is written*): Tenerife, a Strava recording (S) from 5 s, a photo by the
# ride photographer while it runs, S again – the stop notice names the
# recording's folder in Activities/OCW – then the ride saved and the simple
# upload, whose steps name that folder. Everything goes to a throwaway
# folder (OCW_DATA_DIR, OCW_CONFIG_DIR, OCW_ACTIVITIES_DIR):
# nothing lands in a real Activities folder. Made-up rider.
#
#   scripts/test-activities-folder.sh OUT_DIR [x11|wayland]
#
# Writes OUT_DIR/stop-<ws>.jpg (the stop notice), OUT_DIR/upload-<ws>.jpg
# (the upload's steps in the HUD), their logs, and OUT_DIR/listing-<ws>.txt:
# the test folder's Activities/OCW and rides/strava contents.
set -euo pipefail
out=${1:?OUT_DIR}
ws=${2:-x11}
case $ws in
  x11) mode=shot ;;
  wayland) mode=shot-wayland ;;
  *) echo "x11 or wayland, not $ws" >&2; exit 2 ;;
esac
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
pkg=$data/tenerife/tenerife.ocwpkg
[ -f "$pkg" ] || { echo "no $pkg (OCW_WORLD_DATA)" >&2; exit 2; }
route=${OCW_ROUTE:-"Puerto de la Cruz - Teide"}
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export OCW_DATA_DIR=$work/data OCW_CONFIG_DIR=$work/config
export OCW_ACTIVITIES_DIR=$work/Activities/OCW
export OCW_TEST_OUTPUT=off OCW_SIZE=${OCW_SIZE:-1920x1080}
export OCW_SHOT_TIMEOUT=${OCW_SHOT_TIMEOUT:-600}
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
common=(--world "$pkg" --route "$route" --name "Ada Rider" --virtual --power 350
  --bots 0 --no-hearts --record true --photo-time-scale 10 --strava-dry-run
  --cmd "time golden-hour" --cmd "weather clear" --cmd "photographer on"
  --at "5=strava.record start" --at "8=photographer now")
# 1. The stop notice: S again at 70 s, the shot 3 s later.
.claude/skills/run-ocw/drive.sh "$mode" "$out/stop-$ws.jpg" 73 -- "${common[@]}" \
  --at "70=strava.record stop"
# 2. The simple upload of that ride: its steps name the recording's folder.
rm -rf "$OCW_DATA_DIR" "$OCW_ACTIVITIES_DIR"
mkdir -p "$OCW_DATA_DIR"
.claude/skills/run-ocw/drive.sh "$mode" "$out/upload-$ws.jpg" 78 -- "${common[@]}" \
  --at "70=strava.record stop" --at "72=ride.save" --at "74=strava.upload-manual 1" \
  --at "76=screenshot"
{
  echo "# The test folder after the run ($ws; made-up rider Ada Rider)"
  for d in Activities data/rides/strava data/rides/strava-upload; do
    echo "## $d"
    (cd "$work" && [ -d "$d" ] && find "$d" -type f | sort) || echo "(none)"
  done
  echo "## photo sidecars"
  (cd "$work" && find data/rides -path '*/photos/*.toml' -exec grep -H '^path' {} \; | sed "s|$work/||") || true
  echo "## internal record's folder link"
  (cd "$work" && grep -H '^folder' data/rides/strava/*.toml | sed "s|$work/||") || true
} > "$out/listing-$ws.txt"
grep -E 'Strava recording|ride photo|Strava upload' "$out/stop-$ws.log" "$out/upload-$ws.log" \
  | sed "s|$work/||g" | tail -20 || true
cat "$out/listing-$ws.txt"
