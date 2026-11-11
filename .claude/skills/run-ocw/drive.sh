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
#       Record SECONDS (default 6) of the desktop app on X11 with ffmpeg, after
#       a 4 s warm-up after the window appears. Also writes the last frame as OUT.png and the log OUT.log.
#   drive.sh video-wayland OUT.mp4 [SECONDS] [-- desktop args...]
#       Same on Wayland: headless cage (wlroots kiosk compositor) + wf-recorder.
#       Runs the app with --fullscreen (cage shows exactly one fullscreen app).
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
  sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "console|joined session|rejected|ERROR|panicked" || true
}

# Parses OUT.mp4 [SECONDS] [-- args...] into $out, $seconds, $log, $shot.
parse_video_args() {
  out=${1:?usage: drive.sh video|video-wayland OUT.mp4 [SECONDS] [-- args...]}; shift
  seconds=6
  if [ $# -gt 0 ] && [ "$1" != "--" ]; then seconds=$1; shift; fi
  [ "${1:-}" = "--" ] && shift
  VIDEO_APP_ARGS=("$@")
  warmup=4
  log="${out%.*}.log"; shot="${out%.*}.png"
  rm -f "$out" "$shot"
}

# Waits until the app has a window plus the warm-up (the first second after
# window creation still shows shaders compiling).
wait_for_window() {
  for _ in $(seq 600); do
    grep -q "window created" "$log" 2>/dev/null && break
    kill -0 "$app_pid" 2>/dev/null || break
    sleep 0.1
  done
  sleep "$warmup"
  kill -0 "$app_pid" 2>/dev/null || { echo "app failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
}

# Waits for the app to exit by itself and reports the recording.
report_video() {
  local expected=$1 status=0 actual
  timeout 60 tail --pid="$app_pid" -f /dev/null || status=$?
  wait "$app_pid" || status=$?
  [ "$status" -eq 0 ] || { echo "app failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
  [ -s "$out" ] || { echo "no video written" >&2; exit 1; }
  actual=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -oE 'window_system="?[a-z0-9]+' | head -1 | cut -d= -f2 | tr -d '"' || true)
  [ "$actual" = "$expected" ] || {
    echo "expected window system '$expected', app reported '${actual:-nothing}'" >&2; exit 1; }
  echo "video:         $out ($(du -h "$out" | cut -f1))"
  echo "last frame:    $shot"
  echo "log:           $log"
  echo "window system: $actual"
  sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "console|joined session|rejected|ERROR|panicked" || true
}

cmd=${1:-}; shift || true
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
    command -v weston >/dev/null || { echo "weston not installed (apt-get install weston)" >&2; exit 1; }
    cargo build -q -p ocw-desktop
    runtime=${XDG_RUNTIME_DIR:-}
    if [ -z "$runtime" ] || [ ! -w "$runtime" ]; then
      runtime=$(mktemp -d); chmod 700 "$runtime"
    fi
    socket="ocw-wayland-$$"
    XDG_RUNTIME_DIR=$runtime weston --backend=headless --renderer=pixman \
      --socket="$socket" --width=1280 --height=720 --idle-time=0 \
      >"${out%.*}.weston.log" 2>&1 &
    weston_pid=$!
    trap 'kill $weston_pid 2>/dev/null || true' EXIT
    for _ in $(seq 50); do [ -S "$runtime/$socket" ] && break; sleep 0.1; done
    [ -S "$runtime/$socket" ] || { echo "weston did not start, see ${out%.*}.weston.log" >&2; exit 1; }
    status=0
    timeout 180 env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
      "${APP_ENV[@]}" ./target/debug/ocw-desktop "${APP_ARGS[@]}" >"$log" 2>&1 || status=$?
    report wayland "$status"
    ;;
  video)
    parse_video_args "$@"
    command -v ffmpeg >/dev/null || { echo "ffmpeg not installed (apt-get install ffmpeg)" >&2; exit 1; }
    cargo build -q -p ocw-desktop
    display=""
    for n in $(seq 90 120); do
      [ -e "/tmp/.X11-unix/X$n" ] || [ -e "/tmp/.X$n-lock" ] || { display=":$n"; break; }
    done
    [ -n "$display" ] || { echo "no free X display" >&2; exit 1; }
    Xvfb "$display" -screen 0 1280x720x24 -nolisten tcp >/dev/null 2>&1 &
    xvfb_pid=$!
    trap 'kill $xvfb_pid 2>/dev/null || true' EXIT
    for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X${display#:}" ] && break; sleep 0.1; done
    # The app exits by itself after the recording (via its screenshot timer,
    # which starts with the app; startup until the window appears takes ~1-3 s).
    env -u WAYLAND_DISPLAY DISPLAY="$display" "${APP_ENV[@]}" \
      ./target/debug/ocw-desktop --screenshot "$shot" \
      --screenshot-delay "$((warmup + seconds + 5))" "${VIDEO_APP_ARGS[@]}" >"$log" 2>&1 &
    app_pid=$!
    wait_for_window
    ffmpeg -loglevel error -y -f x11grab -draw_mouse 0 -framerate 20 -video_size 1280x720 -i "$display" \
      -t "$seconds" -c:v libx264 -preset veryfast -crf 26 -pix_fmt yuv420p \
      -movflags +faststart "$out"
    report_video x11
    ;;
  video-wayland)
    parse_video_args "$@"
    for tool in cage wf-recorder; do
      command -v "$tool" >/dev/null || { echo "$tool not installed (apt-get install $tool)" >&2; exit 1; }
    done
    cargo build -q -p ocw-desktop
    # Private runtime dir, so cage's socket is the only one in it.
    runtime=$(mktemp -d); chmod 700 "$runtime"
    trap 'rm -rf "$runtime"' EXIT
    XDG_RUNTIME_DIR=$runtime WLR_BACKENDS=headless WLR_RENDERER=pixman \
      WLR_LIBINPUT_NO_DEVICES=1 env -u DISPLAY -u WAYLAND_DISPLAY "${APP_ENV[@]}" \
      cage -- ./target/debug/ocw-desktop --fullscreen --screenshot "$shot" \
      --screenshot-delay "$((warmup + seconds + 5))" "${VIDEO_APP_ARGS[@]}" >"$log" 2>&1 &
    app_pid=$!
    wait_for_window
    socket=$(find "$runtime" -maxdepth 1 -type s -name 'wayland-*' -printf '%f\n' | head -1)
    [ -n "$socket" ] || { echo "cage socket not found" >&2; exit 1; }
    XDG_RUNTIME_DIR=$runtime WAYLAND_DISPLAY=$socket timeout -s INT "$seconds" \
      wf-recorder -y -r 20 -c libx264 -x yuv420p -p crf=26 -p preset=veryfast -f "$out" >"${out%.*}.recorder.log" 2>&1 || true
    report_video wayland
    ;;
  server)
    cargo run -q -p ocw-server -- "$@"
    ;;
  *)
    sed -n '2,20p' "$0"
    exit 2
    ;;
esac
