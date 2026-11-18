#!/usr/bin/env bash
# Headless run of the Strava recordings made without Strava set up (spec
# §2.8.1 *Recording without Strava set up, uploaded later*), on Tenerife,
# made-up rider Ada Rider, all in a throwaway folder (OCW_DATA_DIR,
# OCW_CONFIG_DIR, OCW_ACTIVITIES_DIR) – no network: Strava is the app's
# stand-in (OCW_STRAVA_FAKE=1).
#
# 1. Without credentials: four S recordings, the stop notice (saved, will be
#    uploaded once Strava is set up), the ride saved, its page in the
#    training history: *waiting for Strava set-up*.
# 2. The app runs again without credentials; a few seconds in, the
#    credentials appear in strava.toml (and a made-up linked account): the
#    app starts its Strava thread, asks before uploading the four waiting
#    recordings (the question), *Yes* – the notice "4 recordings uploaded to
#    Strava", then the ride's page: *on Strava*.
#
#   scripts/test-strava-pending.sh OUT_DIR [x11|wayland]
#
# Writes OUT_DIR/{stop,waiting,question,uploaded,on-strava}-<ws>.jpg, the
# logs and OUT_DIR/state-<ws>.txt (the sync state after each run).
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
export OCW_STRAVA_FAKE=1
export OCW_TEST_OUTPUT=off OCW_SIZE=${OCW_SIZE:-1920x1080}
mkdir -p "$OCW_DATA_DIR" "$OCW_CONFIG_DIR"
common=(--world "$pkg" --route "$route" --name "Ada Rider" --virtual --power 300
  --bots 0 --no-hearts --record true --strava-dry-run
  --cmd "time golden-hour" --cmd "weather clear")
# Game times: software rendering on X11 runs slower than the wall clock.
wall=${OCW_STRAVA_WALL_S:-$([ "$ws" = x11 ] && echo 300 || echo 90)}
export OCW_SHOT_TIMEOUT=$((wall + 300))
# 1. Four recordings without the set-up.
recs=()
for k in 0 1 2 3; do
  a=$((4 + k * 7))
  recs+=(--at "$a=strava.record start" --at "$((a + 5))=strava.record stop")
done
.claude/skills/run-ocw/drive.sh "$mode" "$out/waiting-$ws-end.jpg" "$wall" -- "${common[@]}" \
  "${recs[@]}" --at "34=screenshot $out/stop-$ws.jpg" \
  --at "38=ride.save" --at "40=menu ride 1" --at "43=screenshot $out/waiting-$ws.jpg"
{
  echo "# After run 1 ($ws): no credentials"
  cat "$OCW_DATA_DIR/rides/strava-sync.toml" 2>/dev/null || echo "(no sync state)"
} > "$out/state-$ws.txt"
# 2. The credentials come while the app runs (made-up application and
# account; the stand-in Strava answers).
cat > "$OCW_CONFIG_DIR/strava-token.toml" <<'EOF'
access_token = "made-up"
refresh_token = "made-up"
expires_at = 4102444800
athlete = "Ada Rider"
EOF
log2=$out/uploaded-$ws-end.log
rm -f "$log2"
(
  # Once the app is up (it says Strava is not set up), write strava.toml.
  for _ in $(seq 1 1800); do
    grep -q "no API application set up" "$log2" 2>/dev/null && break
    sleep 1
  done
  sleep 3
  printf 'client_id = "12345"\nclient_secret = "made-up"\n' > "$OCW_CONFIG_DIR/strava.toml"
) &
writer=$!
.claude/skills/run-ocw/drive.sh "$mode" "$out/uploaded-$ws-end.jpg" "$wall" -- "${common[@]}" \
  --at "30=screenshot $out/question-$ws.jpg" --at "32=strava.waiting yes" --at "33=menu close" \
  --at "35=screenshot $out/uploaded-$ws.jpg" --at "40=menu ride 1" \
  --at "43=screenshot $out/on-strava-$ws.jpg"
wait "$writer" || true
{
  echo "# After run 2 ($ws): the credentials came while it ran"
  cat "$OCW_DATA_DIR/rides/strava-sync.toml" 2>/dev/null || echo "(no sync state)"
} >> "$out/state-$ws.txt"
grep -hE 'Strava|strava' "$out/waiting-$ws-end.log" "$log2" | sed "s|$work/||g" | tail -30 || true
cat "$out/state-$ws.txt"
