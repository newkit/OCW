#!/usr/bin/env bash
# Headless run of "only S … S goes to Strava, automatically or by hand"
# (spec 2.8.1 Q6), on Tenerife, made-up rider Ada Rider, all in a
# throwaway folder (OCW_DATA_DIR, OCW_CONFIG_DIR, OCW_ACTIVITIES_DIR) – no
# network: Strava is the app's stand-in (OCW_STRAVA_FAKE=1), the account a
# made-up linked one, the setting strava.auto_upload off (settings.toml).
#
# A Strava recording (S … S), the ride saved, a few seconds more without
# S, saved again; then the Strava page (Navi → Strava: the row *Automatic
# upload: off*), the ride with the recording (*not uploaded – upload now*)
# and the ride without (*no Strava recording – press S while riding*).
#
#   scripts/test-strava-q6.sh OUT_DIR [x11|wayland]
#
# Writes OUT_DIR/{settings,ride-with-s,ride-without-s}-<ws>.jpg, the log and
# OUT_DIR/state-<ws>.txt (the sync state: nothing uploaded, no whole ride).
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
cat > "$OCW_CONFIG_DIR/strava-token.toml" <<'EOF'
access_token = "made-up"
refresh_token = "made-up"
expires_at = 4102444800
athlete = "Ada Rider"
EOF
printf 'client_id = "12345"\nclient_secret = "made-up"\n' > "$OCW_CONFIG_DIR/strava.toml"
printf '[strava]\nauto_upload = false\n' > "$OCW_CONFIG_DIR/settings.toml"
common=(--world "$pkg" --route "$route" --name "Ada Rider" --virtual --power 300
  --bots 0 --no-hearts --record true --strava-dry-run
  --cmd "time golden-hour" --cmd "weather clear")
# Game times: software rendering on X11 runs slower than the wall clock.
wall=${OCW_STRAVA_WALL_S:-$([ "$ws" = x11 ] && echo 300 || echo 90)}
export OCW_SHOT_TIMEOUT=$((wall + 300))
.claude/skills/run-ocw/drive.sh "$mode" "$out/q6-$ws-end.jpg" "$wall" -- "${common[@]}" \
  --at "4=strava.record start" --at "12=strava.record stop" --at "14=ride.save" \
  --at "22=ride.save" \
  --at "24=menu strava" --at "27=screenshot $out/settings-$ws.jpg" \
  --at "29=menu ride 2" --at "32=screenshot $out/ride-with-s-$ws.jpg" \
  --at "34=menu ride 1" --at "37=screenshot $out/ride-without-s-$ws.jpg"
{
  echo "# After the run ($ws): strava.auto_upload off, the account linked (stand-in)"
  echo "## settings.toml"
  cat "$OCW_CONFIG_DIR/settings.toml"
  echo "## strava-sync.toml"
  cat "$OCW_DATA_DIR/rides/strava-sync.toml" 2>/dev/null || echo "(no sync state: nothing sent)"
  echo "## rides"
  (cd "$OCW_DATA_DIR/rides" && find . -name '*.fit' | sort)
} > "$out/state-$ws.txt"
grep -hE 'Strava|strava' "$out/q6-$ws-end.log" | sed "s|$work/||g" | tail -30 || true
cat "$out/state-$ws.txt"
