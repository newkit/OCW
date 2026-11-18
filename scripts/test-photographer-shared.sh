#!/usr/bin/env bash
# The ride photographer seen by another rider (spec §2.11 Q2, v4.33) – a
# headless run with two desktop apps on one game server, on marge (the GPU,
# headless weston and Xvfb):
#
#   scripts/test-photographer-shared.sh [OUT_DIR] [SECONDS] [x11|wayland]
#       (default test-output/photographer-shared, 300 s, the second rider's
#       view on X11)
#
# Starts `ocw-server --listen` on Tenerife (Buenavista - Masca - Santiago
# del Teide: cliffs and the sea), then two riders on it, both on the
# virtual trainer at 300 W from the start:
#
#   Ada Rider  (the photographed one, Wayland) – the photographer's time 10×
#              faster (the server's too), off until `photographer drone`
#              after 30 s (Bea has joined by then): a drone flies around
#              her and takes the photo, saved as on the desktop
#   Bea Climber (the second rider, X11 or Wayland) – her own photographer
#              off; whenever Ada's photographer, drone or flash is in her
#              view her window is saved (--photo-chase-shots)
#
# Writes to OUT_DIR:
#   a/end.jpg, a/end.log, a/photos/   Ada's last frame, log and photos
#   b/end.jpg, b/end.log, b/chase/    Bea's last frame, log and her views
#                                     with Ada's photographer: chase-NN-
#                                     other-photographer|other-drone|other-flash.jpg
#   server.log, index.md              the server's log, what is where
#
# OCW_WORLD_DATA: the folder with tenerife/ (default ~/src/OCW/data).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-test-output/photographer-shared}
seconds=${2:-300}
second_ws=${3:-x11}
data=${OCW_WORLD_DATA:-$HOME/src/OCW/data}
pkg=$data/tenerife/tenerife.ocwpkg
route="Buenavista - Masca - Santiago del Teide"
[ -f "$pkg" ] || { echo "no $pkg (OCW_WORLD_DATA)" >&2; exit 2; }
mkdir -p "$out/a" "$out/b"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
server_pid=""
cleanup() {
  [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# Each package on its own, as drive.sh builds the app (one build of
# both unifies their features differently: drive.sh would build again
# while the server already runs).
cargo build -q --release -p ocw-desktop
cargo build -q --release -p ocw-server

port=$((47900 + RANDOM % 80))
target/release/ocw-server --listen "127.0.0.1:$port" --bots 0 --world "$pkg" \
  --route "$route" --seconds $((seconds + 240)) --tls-dir "$work/tls" --photo-time-scale 10 \
  >"$out/server.log" 2>&1 &
server_pid=$!
fp=""
for _ in $(seq 240); do
  fp=$(grep -oE -- '--server-cert [0-9a-f]{64}' "$out/server.log" 2>/dev/null | head -n 1 | awk '{print $2}' || true)
  [ -n "$fp" ] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.5
done
[ -n "$fp" ] || { echo "the server did not start (see $out/server.log)" >&2; exit 1; }
echo "server up on UDP $port"

# One rider: $1 a|b, $2 drive.sh mode, the rest the app's own arguments.
rider() {
  local who=$1 mode=$2; shift 2
  local dir=$work/$who
  mkdir -p "$dir/data" "$dir/config"
  OCW_DATA_DIR=$dir/data OCW_CONFIG_DIR=$dir/config OCW_PHOTOS_DIR=$dir/Pictures/OCW OCW_ACTIVITIES_DIR=$dir/Activities/OCW OCW_TEST_OUTPUT=off \
    OCW_PROFILE=release OCW_SHOT_TIMEOUT=$((seconds + 240)) OCW_SIZE=${OCW_SIZE:-1920x1080} \
    .claude/skills/run-ocw/drive.sh "$mode" "$out/$who/end.jpg" "$seconds" -- \
    --server "127.0.0.1:$port" --server-cert "$fp" --world "$pkg" --route "$route" \
    --virtual --power 300 --no-hearts --cmd "weather clear" "$@" \
    >"$out/$who/drive.log" 2>&1
}

rider a shot-wayland --name "Ada Rider" --record true --photo-time-scale 10 \
  --cmd "time golden-hour" --cmd "photographer off" --at "30=photographer drone" &
a_pid=$!
sleep 4
b_mode=shot
# X11 renders on the CPU (lavapipe): a smaller window and the Low preset
# keep its frame rate (and so its game time) up.
b_low=(--window 1280x720 --cmd "quality low")
if [ "$second_ws" = wayland ]; then
  b_mode=shot-wayland
  b_low=()
fi
rider b "$b_mode" "${b_low[@]}" --name "Bea Climber" --record false --cmd "photographer off" \
  --photo-chase-shots "$out/b/chase" &
b_pid=$!
status=0
wait "$a_pid" || status=$?
wait "$b_pid" || status=$?

rm -rf "$out/a/photos"
mkdir -p "$out/a/photos"
find "$work/a" \( -path '*/photos/*' -o -path '*/Pictures/OCW/*' \) -type f -exec cp {} "$out/a/photos/" \;
# No home paths in what is kept.
for f in "$out"/*.log "$out"/*/*.log; do
  [ -f "$f" ] && sed -i "s|$HOME|~|g; s|$work|WORK|g" "$f"
done
photos=$(find "$out/a/photos" -name '*.jpg' | wc -l)
chase=$(find "$out/b/chase" -name '*.jpg' 2>/dev/null | wc -l)
flash=$(find "$out/b/chase" -name '*other-flash*.jpg' 2>/dev/null | wc -l)
{
  echo "# The ride photographer seen by another rider (spec §2.11 Q2, v4.33)"
  echo
  echo "Tenerife, $route; two desktop apps on one \`ocw-server\`, $seconds s, $(date -u +%Y-%m-%dT%H:%MZ)."
  echo "Ada Rider (Wayland) is photographed by a drone; Bea Climber ($second_ws) rides with her, her own photographer off."
  echo
  echo "- Ada's photos: $photos (a/photos/)"
  echo "- Bea's views with Ada's photographer: $chase (b/chase/), $flash of them with the flash"
  echo
  echo "| File | What |"
  echo "|---|---|"
  for f in "$out"/b/chase/*.jpg; do
    [ -f "$f" ] || continue
    echo "| b/chase/$(basename "$f") | Bea's view: $(basename "$f" .jpg | sed 's/chase-[0-9]*-//; s/-/ /g') |"
  done
  for f in "$out"/a/photos/*.jpg; do
    [ -f "$f" ] || continue
    echo "| a/photos/$(basename "$f") | Ada's photo |"
  done
  echo "| a/end.jpg, b/end.jpg | the last frames |"
} >"$out/index.md"
echo "photos: $photos, Bea's views with Ada's photographer: $chase ($flash with the flash)"
grep -hE 'ride photo|chase view|photo shot' "$out/a/end.log" "$out/b/end.log" 2>/dev/null | tail -30 || true
exit "$status"
