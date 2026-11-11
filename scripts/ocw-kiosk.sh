#!/usr/bin/env bash
# Keeps the OCW test app running on a test machine's desktop (e.g. marge).
# Run it as the user of the desktop session (not over SSH):
#
#   ocw-kiosk.sh run [--monitor SPEC] [-- app args...]
#       Start the app (borderless fullscreen on SPEC: index, WIDTHxHEIGHT or
#       name substring, see the "monitor" lines in the app log). Restart it
#       when a new build is deployed (scripts/deploy-test.sh) and when it
#       exits or crashes (pausing 30 s after three quick crashes in a row).
#   ocw-kiosk.sh install [--monitor SPEC] [-- app args...]
#       Start automatically at login (XDG autostart) and start it now.
#   ocw-kiosk.sh uninstall    remove the autostart entry and stop it
#   ocw-kiosk.sh status       show the deployed build and the kiosk log
#
# Files: the app and this script live in OCW_KIOSK_DIR (default: this
# script's directory, /var/tmp/ocw); the log and lock in
# ~/.local/state/ocw/. On first start, OCW_KIOSK_DIR/devices.toml (if any)
# seeds ~/.config/ocw/devices.toml, so the trainer is connected right away.
set -uo pipefail

dir=${OCW_KIOSK_DIR:-$(cd "$(dirname "$0")" && pwd)}
bin=$dir/ocw-desktop
state=${XDG_STATE_HOME:-$HOME/.local/state}/ocw
log=$state/kiosk.log
applog=$state/app.log
poll=${OCW_KIOSK_POLL_S:-2}        # how often to check for a new build
settle=${OCW_KIOSK_SETTLE_S:-2}    # wait after a change before restarting
quick=${OCW_KIOSK_QUICK_S:-15}     # an exit within this time counts as a crash
autostart=${XDG_CONFIG_HOME:-$HOME/.config}/autostart/ocw-kiosk.desktop

say() { echo "$(date '+%F %T') $*" | tee -a "$log"; }

# Parses [--monitor SPEC] [-- args...] into APP_ARGS.
parse_args() {
  APP_ARGS=()
  MONITOR=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --monitor) MONITOR=${2:?--monitor needs a value}; shift 2 ;;
      --) shift; APP_ARGS+=("$@"); break ;;
      *) APP_ARGS+=("$1"); shift ;;
    esac
  done
  [ -n "$MONITOR" ] && APP_ARGS=(--monitor "$MONITOR" "${APP_ARGS[@]}")
  return 0
}

signature() { stat -c '%Y %i %s' "$bin" 2>/dev/null || echo missing; }

stop_app() {
  local pid=$1
  kill -TERM "$pid" 2>/dev/null || return 0
  for _ in $(seq 50); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.2; done
  say "app did not stop, killing it"
  kill -KILL "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

run() {
  parse_args "$@"
  mkdir -p "$state"
  exec 9>"$state/kiosk.lock"
  if ! flock -n 9; then
    echo "kiosk already running (lock $state/kiosk.lock)" >&2
    exit 1
  fi
  local cfg=${XDG_CONFIG_HOME:-$HOME/.config}/ocw/devices.toml
  if [ -f "$dir/devices.toml" ] && [ ! -f "$cfg" ]; then
    mkdir -p "$(dirname "$cfg")" && cp "$dir/devices.toml" "$cfg" && say "seeded $cfg"
  fi
  local child="" fails=0
  trap 'say "kiosk stopping"; [ -n "$child" ] && stop_app "$child"; exit 0' TERM INT
  say "kiosk started (dir $dir, args: ${APP_ARGS[*]:-none})"
  while true; do
    until [ -x "$bin" ]; do sleep "$poll"; done
    local sig started reason=exited
    sig=$(signature)
    started=$(date +%s)
    say "starting $(cat "$dir/VERSION" 2>/dev/null || echo 'unknown build')"
    "$bin" "${APP_ARGS[@]}" >>"$applog" 2>&1 &
    child=$!
    while kill -0 "$child" 2>/dev/null; do
      sleep "$poll"
      if [ "$(signature)" != "$sig" ]; then
        sleep "$settle" # let the deployment finish
        reason=update
        say "new build deployed, restarting"
        stop_app "$child"
        break
      fi
    done
    wait "$child" 2>/dev/null
    local code=$?
    child=""
    if [ "$reason" = update ]; then
      fails=0
      continue
    fi
    if [ $(( $(date +%s) - started )) -lt "$quick" ]; then
      fails=$((fails + 1))
    else
      fails=0
    fi
    if [ "$fails" -ge 3 ]; then
      say "app exited (code $code) $fails times in a row quickly, waiting 30 s"
      sleep 30
      fails=0
    else
      say "app exited (code $code), restarting in 3 s"
      sleep 3
    fi
  done
}

install() {
  parse_args "$@"
  mkdir -p "$(dirname "$autostart")" "$state"
  local exec_line="$dir/ocw-kiosk.sh run"
  [ ${#APP_ARGS[@]} -gt 0 ] && exec_line+=" --$(printf ' %q' "${APP_ARGS[@]}")"
  cat >"$autostart" <<DESKTOP
[Desktop Entry]
Type=Application
Name=OCW test app (kiosk)
Comment=Keeps the Open Cycling World test build running
Exec=$exec_line
X-GNOME-Autostart-enabled=true
NoDisplay=true
DESKTOP
  say "autostart installed: $autostart"
  pkill -f "$dir/ocw-kiosk.sh run" 2>/dev/null && sleep 1
  nohup setsid "$dir/ocw-kiosk.sh" run "$@" >/dev/null 2>&1 &
  echo "started; log: $log"
}

case "${1:-}" in
  run) shift; run "$@" ;;
  install) shift; install "$@" ;;
  uninstall)
    rm -f "$autostart"
    pkill -f "$dir/ocw-kiosk.sh run" && echo "stopped"
    echo "autostart removed"
    ;;
  status)
    echo "build: $(cat "$dir/VERSION" 2>/dev/null || echo none) ($(signature))"
    echo "autostart: $([ -f "$autostart" ] && echo installed || echo not installed)"
    pgrep -af "ocw-kiosk.sh run" || echo "kiosk not running"
    [ -f "$log" ] && tail -n 10 "$log"
    ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
