#!/usr/bin/env bash
# Drive the Open Cycling World apps headlessly. Run from the repository root.
#
#   drive.sh shot OUT.png [DELAY_S] [-- desktop args...]
#       Launch the desktop app on a virtual X11 display (Xvfb, software Vulkan),
#       ride for DELAY_S seconds (default 8), save a screenshot to OUT.png, exit.
#       Extra args go to ocw-desktop, e.g. --cmd "teleport 1.2" --cmd "console open".
#       The app log is written next to the screenshot as OUT.log.
#   drive.sh shot-wayland OUT.png [DELAY_S] [-- desktop args...]
#       Same, but on a headless Wayland compositor (weston) instead of X11.
#   drive.sh video OUT.mp4 [SECONDS] [-- desktop args...]
#       Record SECONDS (default 6) of riding on X11 as a smooth 25 fps video:
#       the app saves every frame itself with game time advancing exactly
#       1/25 s per frame (--capture), however slowly it renders; ffmpeg encodes.
#       Rides 3 s of game time first. Also writes OUT.png (last frame), OUT.log.
#   drive.sh video-wayland OUT.mp4 [SECONDS] [-- desktop args...]
#       Same on Wayland (headless weston).
#   Every successful shot/video is also archived (see archive() below) under
#   test-output/YYYY-MM-DD/ with commit and mode in the file name, plus a row
#   in test-output/index.md. OCW_TEST_OUTPUT=DIR changes the folder,
#   OCW_TEST_OUTPUT=off disables archiving.
#   drive.sh spike REPORT.json [-- spike args...]
#       Graphics spike benchmark (apps/spike, release build) on a headless
#       weston with the GPU renderer (real GPU required, e.g. on marge).
#       Writes REPORT.json, REPORT.png (last frame) and REPORT.log; archived.
#   drive.sh spike-video OUT.mp4 [SECONDS] [-- spike args...]
#       Smooth 25 fps video of the spike on the GPU (frame capture: game time
#       advances 1/25 s per frame, so it is not a benchmark). Archived.
#   drive.sh server [args...]
#       Run the headless game server, e.g. --bots 3 --seconds 10 --fast.
set -euo pipefail

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

APP_ENV=(WGPU_BACKEND=vulkan "RUST_LOG=${RUST_LOG:-info,wgpu=warn,naga=warn}")

# Parses OUT [DELAY] [-- args...] into $out, $delay, $log and APP_ARGS.
parse_shot_args() {
  out=${1:?usage: drive.sh shot|shot-wayland OUT.png [DELAY_S] [-- args...]}; shift
  delay=8
  if [ $# -gt 0 ] && [ "$1" != "--" ]; then delay=$1; shift; fi
  [ "${1:-}" = "--" ] && shift
  APP_ARGS=(--screenshot "$out" --screenshot-delay "$delay" "$@")
  ARCHIVE_APP_ARGS=("$@")
  log="${out%.*}.log"
  rm -f "$out"
}

# Checks the result and that the app ran on the expected window system.
report() {
  local expected=$1 status=$2
  [ "$status" -eq 0 ] || { echo "app failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
  [ -s "$out" ] || { echo "no screenshot written, see $log" >&2; exit 1; }
  local actual
  actual=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -oE 'window_system="?[a-z0-9]+' | head -1 | cut -d= -f2 | tr -d '"' || true)
  [ "$actual" = "$expected" ] || {
    echo "expected window system '$expected', app reported '${actual:-nothing}'" >&2; exit 1; }
  echo "screenshot:    $out"
  echo "log:           $log"
  echo "window system: $actual"
  archive "$MODE" "$actual" "$out" "$log"
  sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "console|joined session|rejected|ERROR|panicked" || true
}

# Parses OUT.mp4 [SECONDS] [-- args...] into $out, $seconds, $log, $shot,
# $frames and VIDEO_APP_ARGS.
parse_video_args() {
  out=${1:?usage: drive.sh video|video-wayland OUT.mp4 [SECONDS] [-- args...]}; shift
  seconds=6
  if [ $# -gt 0 ] && [ "$1" != "--" ]; then seconds=$1; shift; fi
  [ "${1:-}" = "--" ] && shift
  log="${out%.*}.log"; shot="${out%.*}.png"
  frames=$(mktemp -d)
  VIDEO_APP_ARGS=(--capture "$frames" --capture-fps 25 --capture-seconds "$seconds" "$@")
  ARCHIVE_APP_ARGS=(--capture-seconds "$seconds" "$@")
  rm -f "$out" "$shot"
  command -v ffmpeg >/dev/null || { echo "ffmpeg not installed (apt-get install ffmpeg)" >&2; exit 1; }
}

# Encodes the captured frames and reports. $1: expected window system, $2: app exit status.
encode_video() {
  local expected=$1 status=$2 actual last
  [ "$status" -eq 0 ] || { echo "app failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
  last=$(find "$frames" -name 'frame_*.png' | sort | tail -1)
  [ -n "$last" ] || { echo "no frames captured, see $log" >&2; exit 1; }
  cp "$last" "$shot"
  ffmpeg -loglevel error -y -framerate 25 -i "$frames/frame_%05d.png" -c:v libx264 \
    -preset veryfast -crf 24 -pix_fmt yuv420p -movflags +faststart "$out"
  rm -rf "$frames"
  actual=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -oE 'window_system="?[a-z0-9]+' | head -1 | cut -d= -f2 | tr -d '"' || true)
  [ "$actual" = "$expected" ] || {
    echo "expected window system '$expected', app reported '${actual:-nothing}'" >&2; exit 1; }
  echo "video:         $out ($(du -h "$out" | cut -f1))"
  echo "last frame:    $shot"
  echo "log:           $log"
  echo "window system: $actual"
  archive "$MODE" "$actual" "$out" "$shot" "$log"
  sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "console|joined session|capture finished|rejected|ERROR|panicked" || true
}

# Starts a headless weston; sets $runtime and $socket. Stopped on exit.
start_weston() {
  # GPU renderer when the machine has a GPU render node: a Vulkan app on a
  # real GPU cannot present to a software-only (pixman) weston.
  local default=pixman
  [ -r /dev/dri/renderD128 ] && default=gl
  local logfile=$1 renderer=${2:-${WESTON_RENDERER:-$default}}
  command -v weston >/dev/null || { echo "weston not installed (apt-get install weston)" >&2; exit 1; }
  runtime=${XDG_RUNTIME_DIR:-}
  if [ -z "$runtime" ] || [ ! -w "$runtime" ]; then
    runtime=$(mktemp -d); chmod 700 "$runtime"
  fi
  socket="ocw-wayland-$$"
  XDG_RUNTIME_DIR=$runtime weston --backend=headless --renderer="$renderer" \
    --socket="$socket" --width="${WESTON_WIDTH:-1280}" --height="${WESTON_HEIGHT:-720}" \
    --idle-time=0 >"$logfile" 2>&1 &
  weston_pid=$!
  trap 'kill $weston_pid 2>/dev/null || true' EXIT
  for _ in $(seq 50); do [ -S "$runtime/$socket" ] && break; sleep 0.1; done
  [ -S "$runtime/$socket" ] || { echo "weston did not start, see $logfile" >&2; exit 1; }
}

# Copies the result (and its log) into the archive and adds an index row.
# $1: mode (shot, shot-wayland, video, video-wayland), $2: window system,
# remaining: files to archive.
archive() {
  local mode=$1 ws=$2; shift 2
  local root=${OCW_TEST_OUTPUT:-$PWD/test-output}
  [ "$root" = "off" ] && return 0
  local day sha dir stem f name saved=()
  day=$(date +%Y-%m-%d)
  if [ -n "${OCW_BUILD_ID:-}" ]; then
    # A copy made by scripts/remote.sh passes the commit it was copied from
    # (a stale .git there would name the wrong one).
    sha=${OCW_BUILD_ID// /-}
  elif sha=$(git rev-parse --short HEAD 2>/dev/null); then
    if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then sha="$sha-dirty"; fi
  else
    sha=nogit
  fi
  dir="$root/$day"
  mkdir -p "$dir"
  stem="$(date +%H%M%S)-$sha-$mode"
  for f in "$@"; do
    [ -f "$f" ] || continue
    name="$stem-$(basename "$f")"
    cp "$f" "$dir/$name"
    saved+=("$day/$name")
  done
  if [ ! -f "$root/index.md" ]; then
    printf '# OCW test output\n\n| Date | Time | Commit | Mode | Window system | App arguments | Files |\n|---|---|---|---|---|---|---|\n' >"$root/index.md"
  fi
  local links="" args
  for f in "${saved[@]}"; do links+="[$(basename "$f" | sed "s/^$stem-//")]($f) "; done
  args=$(printf '%s ' "${ARCHIVE_APP_ARGS[@]}" | sed 's/|/\\|/g; s/ *$//')
  printf '| %s | %s | %s | %s | %s | `%s` | %s|\n' "$day" "$(date +%H:%M:%S)" "$sha" "$mode" "$ws" \
    "${args:--}" "$links" >>"$root/index.md"
  echo "archived:      $dir/$stem-*"
}

cmd=${1:-}; shift || true
MODE=$cmd
ARCHIVE_APP_ARGS=()
case "$cmd" in
  shot)
    parse_shot_args "$@"
    cargo build -q -p ocw-desktop
    status=0
    timeout 180 xvfb-run -a -s "-screen 0 1280x720x24" \
      env -u WAYLAND_DISPLAY "${APP_ENV[@]}" ./target/debug/ocw-desktop "${APP_ARGS[@]}" \
      >"$log" 2>&1 || status=$?
    report x11 "$status"
    ;;
  shot-wayland)
    parse_shot_args "$@"
    cargo build -q -p ocw-desktop
    start_weston "${out%.*}.weston.log"
    status=0
    timeout 180 env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
      "${APP_ENV[@]}" ./target/debug/ocw-desktop "${APP_ARGS[@]}" >"$log" 2>&1 || status=$?
    report wayland "$status"
    ;;
  video)
    parse_video_args "$@"
    cargo build -q -p ocw-desktop
    status=0
    timeout 600 xvfb-run -a -s "-screen 0 1280x720x24" \
      env -u WAYLAND_DISPLAY "${APP_ENV[@]}" ./target/debug/ocw-desktop "${VIDEO_APP_ARGS[@]}" \
      >"$log" 2>&1 || status=$?
    encode_video x11 "$status"
    ;;
  video-wayland)
    parse_video_args "$@"
    cargo build -q -p ocw-desktop
    start_weston "${out%.*}.weston.log"
    status=0
    timeout 600 env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
      "${APP_ENV[@]}" ./target/debug/ocw-desktop "${VIDEO_APP_ARGS[@]}" >"$log" 2>&1 || status=$?
    encode_video wayland "$status"
    ;;
  spike)
    out=${1:?usage: drive.sh spike REPORT.json [-- args...]}; shift
    [ "${1:-}" = "--" ] && shift
    log="${out%.*}.log"; shot="${out%.*}.png"
    rm -f "$out" "$shot"
    ARCHIVE_APP_ARGS=("$@")
    cargo build -q --release -p ocw-spike
    WESTON_WIDTH=1920 WESTON_HEIGHT=1080 start_weston "${out%.*}.weston.log" gl
    status=0
    timeout 1800 env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
      WGPU_BACKEND=vulkan "RUST_LOG=${RUST_LOG:-info,wgpu=warn,naga=warn}" \
      ./target/release/ocw-spike --report "$out" --screenshot "$shot" "$@" >"$log" 2>&1 || status=$?
    [ "$status" -eq 0 ] || { echo "spike failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
    [ -s "$out" ] || { echo "no report written, see $log" >&2; exit 1; }
    sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "AdapterInfo|window_system|initial tiles|report written|panicked|ERROR" | cut -c1-220 || true
    echo "report:        $out"
    archive spike wayland "$out" "$shot" "$log"
    ;;
  spike-video)
    out=${1:?usage: drive.sh spike-video OUT.mp4 [SECONDS] [-- args...]}; shift
    seconds=8
    if [ $# -gt 0 ] && [ "$1" != "--" ]; then seconds=$1; shift; fi
    [ "${1:-}" = "--" ] && shift
    log="${out%.*}.log"; shot="${out%.*}.png"
    frames=$(mktemp -d)
    rm -f "$out" "$shot"
    ARCHIVE_APP_ARGS=(--capture-seconds "$seconds" "$@")
    cargo build -q --release -p ocw-spike
    WESTON_WIDTH=1920 WESTON_HEIGHT=1080 start_weston "${out%.*}.weston.log" gl
    status=0
    timeout 1800 env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
      WGPU_BACKEND=vulkan "RUST_LOG=${RUST_LOG:-info,wgpu=warn,naga=warn}" \
      ./target/release/ocw-spike --capture "$frames" --capture-seconds "$seconds" \
      --report "${out%.*}.json" "$@" >"$log" 2>&1 || status=$?
    [ "$status" -eq 0 ] || { echo "spike failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
    last=$(find "$frames" -name 'frame_*.png' | sort | tail -1)
    [ -n "$last" ] || { echo "no frames captured, see $log" >&2; exit 1; }
    cp "$last" "$shot"
    ffmpeg -loglevel error -y -framerate 25 -i "$frames/frame_%05d.png" -c:v libx264 \
      -preset veryfast -crf 24 -pix_fmt yuv420p -movflags +faststart "$out"
    rm -rf "$frames"
    echo "video:         $out ($(du -h "$out" | cut -f1))"
    archive spike-video wayland "$out" "$shot" "$log"
    ;;
  server)
    cargo run -q -p ocw-server -- "$@"
    ;;
  *)
    sed -n '2,30p' "$0"
    exit 2
    ;;
esac
