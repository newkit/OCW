#!/usr/bin/env bash
# Drive the Open Cycling World apps headlessly. Run from the repository root.
#
#   drive.sh shot OUT.png [DELAY_S] [-- desktop args...]
#       Launch the desktop app on a virtual X display (Xvfb, software Vulkan),
#       ride for DELAY_S seconds (default 8), save a screenshot to OUT.png, exit.
#       Extra args go to ocw-desktop, e.g. --cmd "teleport 1.2" --cmd "console open".
#       The app log is written next to the screenshot as OUT.log.
#   drive.sh server [args...]
#       Run the headless game server, e.g. --bots 3 --seconds 10 --fast.
set -euo pipefail

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

cmd=${1:-}; shift || true
case "$cmd" in
  shot)
    out=${1:?usage: drive.sh shot OUT.png [DELAY_S] [-- args...]}; shift
    delay=8
    if [ $# -gt 0 ] && [ "$1" != "--" ]; then delay=$1; shift; fi
    [ "${1:-}" = "--" ] && shift
    cargo build -q -p ocw-desktop
    log="${out%.*}.log"
    rm -f "$out"
    timeout 180 xvfb-run -a -s "-screen 0 1280x720x24" \
      env WGPU_BACKEND=vulkan RUST_LOG="${RUST_LOG:-info,wgpu=warn,naga=warn}" \
      ./target/debug/ocw-desktop --screenshot "$out" --screenshot-delay "$delay" "$@" \
      >"$log" 2>&1 || { echo "app failed, see $log" >&2; tail -20 "$log" >&2; exit 1; }
    [ -s "$out" ] || { echo "no screenshot written, see $log" >&2; exit 1; }
    echo "screenshot: $out"
    echo "log:        $log"
    grep -E "console|joined session|rejected|ERROR|panicked" "$log" | sed 's/\x1b\[[0-9;]*m//g' || true
    ;;
  server)
    cargo run -q -p ocw-server -- "$@"
    ;;
  *)
    sed -n '2,12p' "$0"
    exit 2
    ;;
esac
