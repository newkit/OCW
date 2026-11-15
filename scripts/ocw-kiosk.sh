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
#
# Game server (user, 2026-11-15): when OCW_KIOSK_DIR/ocw-server exists (the
# deploy installs it), the kiosk first starts it - the same world and route
# as the app (from ocw-desktop.args), bots, UDP OCW_KIOSK_LISTEN (default
# 0.0.0.0:47800), a fixed certificate in ~/.local/state/ocw/server-tls - and
# then the app as its client (--server 127.0.0.1:PORT --insecure-dev), so
# other riders can join the kiosk's ride over the network with the line in
# ~/.local/state/ocw/server-connect.txt (`ocw-kiosk.sh status` prints it). A
# new build or a crash of either restarts both. OCW_KIOSK_SERVER=0: no
# server, the app rides alone as before; OCW_KIOSK_BOTS (default 10) and
# OCW_KIOSK_SERVER_ARGS (more ocw-server options) tune it.
set -uo pipefail

dir=${OCW_KIOSK_DIR:-$(cd "$(dirname "$0")" && pwd)}
bin=$dir/ocw-desktop
server_bin=$dir/ocw-server
listen=${OCW_KIOSK_LISTEN:-0.0.0.0:47800}
ready_s=${OCW_KIOSK_SERVER_READY_S:-180}   # world packages take a while to load
state=${XDG_STATE_HOME:-$HOME/.local/state}/ocw
log=$state/kiosk.log
applog=$state/app.log
serverlog=$state/server.log
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

signature() {
  echo "$(stat -c '%Y %i %s' "$bin" 2>/dev/null || echo missing) $(stat -c '%Y %i %s' "$server_bin" 2>/dev/null || echo none)"
}

# The kiosk script itself: a deploy that replaces it makes the running
# kiosk start the new one (exec), so changes to the kiosk need no login.
self_sig() { stat -c '%Y %i %s' "$dir/ocw-kiosk.sh" 2>/dev/null || echo none; }

server_wanted() { [ "${OCW_KIOSK_SERVER:-1}" != 0 ] && [ -x "$server_bin" ]; }

# The last value of OPTION in the app's ocw-desktop.args (quotes removed).
args_value() {
  sed -n "s/^$1[[:space:]]\{1,\}//p" "$dir/ocw-desktop.args" 2>/dev/null | tail -n 1 |
    sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'\$/\1/"
}

# Starts the game server for the app's world and route; sets SERVER_PID and
# SERVER_ARGS (the app's options to join it), or leaves both empty.
start_server() {
  SERVER_PID="" SERVER_ARGS=()
  server_wanted || return 0
  local world route course=() extra=()
  world=$(args_value --world)
  route=$(args_value --route)
  if [ -n "$world" ]; then
    course=(--world "$world")
    [ -n "$route" ] && course+=(--route "$route")
  else
    course=(--test-route hills)   # the app's own default route
  fi
  # shellcheck disable=SC2206 # options split on purpose
  [ -n "${OCW_KIOSK_SERVER_ARGS:-}" ] && extra=($OCW_KIOSK_SERVER_ARGS)
  mkdir -p "$state/server-tls"
  : >"$state/server.out"
  say "starting the game server (${course[*]}, $listen)"
  "$server_bin" --listen "$listen" --tls-dir "$state/server-tls" --bots "${OCW_KIOSK_BOTS:-10}" \
    "${course[@]}" "${extra[@]}" >"$state/server.out" 2>>"$serverlog" &
  SERVER_PID=$!
  echo "$SERVER_PID" >"$state/server.pid"
  local waited=0
  until grep -q "ocw-desktop --server" "$state/server.out" 2>/dev/null; do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      say "game server exited at start (see $serverlog); the app rides alone"
      SERVER_PID=""
      return 0
    fi
    if [ "$waited" -ge "$ready_s" ]; then
      say "game server not ready after ${ready_s} s; the app rides alone"
      stop_app "$SERVER_PID"; SERVER_PID=""
      return 0
    fi
    sleep 1; waited=$((waited + 1))
  done
  cat "$state/server.out" >>"$serverlog"
  grep -A2 "Clients connect with" "$state/server.out" >"$state/server-connect.txt"
  local port=${listen##*:}
  SERVER_ARGS=(--server "127.0.0.1:$port" --insecure-dev)
  say "game server ready after ${waited} s: $(sed -n 2p "$state/server-connect.txt" | sed 's/^ *//')"
}

stop_app() {
  local pid=$1
  kill -TERM "$pid" 2>/dev/null || return 0
  for _ in $(seq 50); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.2; done
  say "app did not stop, killing it"
  kill -KILL "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
}

run() {
  local orig_args=("$@")
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
  SERVER_PID="" SERVER_ARGS=()
  trap 'say "kiosk stopping"; [ -n "$child" ] && stop_app "$child"; [ -n "$SERVER_PID" ] && stop_app "$SERVER_PID"; exit 0' TERM INT
  say "kiosk started (dir $dir, args: ${APP_ARGS[*]:-none})"
  local me
  me=$(self_sig)
  while true; do
    until [ -x "$bin" ]; do sleep "$poll"; done
    local sig started reason=exited
    sig=$(signature)
    started=$(date +%s)
    say "starting $(cat "$dir/VERSION" 2>/dev/null || echo 'unknown build')"
    start_server
    "$bin" "${APP_ARGS[@]}" "${SERVER_ARGS[@]}" >>"$applog" 2>&1 &
    child=$!
    while kill -0 "$child" 2>/dev/null; do
      sleep "$poll"
      if [ -n "$SERVER_PID" ] && ! kill -0 "$SERVER_PID" 2>/dev/null; then
        say "game server exited, restarting both"
        SERVER_PID=""
        stop_app "$child"
        break
      fi
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
    [ -n "$SERVER_PID" ] && { stop_app "$SERVER_PID"; SERVER_PID=""; }
    if [ "$reason" = update ]; then
      fails=0
      if [ "$(self_sig)" != "$me" ] && [ -x "$dir/ocw-kiosk.sh" ]; then
        say "new kiosk script deployed, restarting the kiosk"
        exec 9>&-
        exec "$dir/ocw-kiosk.sh" run "${orig_args[@]}"
      fi
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
    if kill -0 "$(cat "$state/server.pid" 2>/dev/null || echo 0)" 2>/dev/null; then
      echo "game server: running; riders join with:"
      sed -n '2,3p' "$state/server-connect.txt" 2>/dev/null
    else
      echo "game server: not running"
    fi
    [ -f "$log" ] && tail -n 10 "$log"
    ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
