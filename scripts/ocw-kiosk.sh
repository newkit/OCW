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
# Voice (spec §7.1.8): the game server starts the voice server it finds
# next to itself (OCW_KIOSK_DIR/livekit-server, installed by the deploy) or
# on PATH; riders who join talk to each other without any setup. Its
# ports: TCP 7880 / 7881 and UDP 7882 (`--voice-ports`).
# A game server already running on this machine (UDP port of
# OCW_KIOSK_LISTEN busy, e.g. one started by hand) is joined instead of
# starting another; OCW_KIOSK_SERVER=join always joins (waiting for it).
#
# A remote server (spec 11.5 *Join info*, 2026-11-26): OCW_KIOSK_SERVER=remote
# joins the server whose join info OCW_KIOSK_REMOTE names (default
# https://ocw-server.fly.dev/join.json, the Fly.io server). At every (re)start
# of the app the kiosk fetches it (curl, OCW_KIOSK_REMOTE_TIMEOUT_S, 10 s),
# takes its desktop join link (`join`: ocw://IPV4?cert=…&world=ID@VERSION over
# QUIC; servers without it: the WebTransport URL with the current certificate
# hash, https://IPV4:47801?cert-hash=…), finds the server's world among the
# packages in OCW_KIOSK_DIR by its SHA-256 (else by the manifest's id and
# version) and starts the app with --join LINK on that package, with --bots 0
# and no local ocw-server: the riders are the server's. A dropped connection
# or a failed join ("Disconnected: …" / "join failed: …" in the app log, e.g.
# the server restarted with a renewed certificate) restarts the app with
# freshly fetched join info. When the join info cannot be fetched or has no
# link, the kiosk tries again (OCW_KIOSK_REMOTE_TRIES times in a row, 3,
# after 5, 10, 20 s: OCW_KIOSK_REMOTE_DELAY_S); the same after joins that
# fail. When that runs out, or at once when the server's world package is not
# installed here (or differs), the kiosk logs why and falls back to the local
# mode (OCW_KIOSK_SERVER=1: its own game server with bots, or the app alone),
# and checks the join info again every OCW_KIOSK_REMOTE_RETRY_S (60 s); once
# the server can be joined it switches back. Voice: the server's grants name
# its voice URL (wss://ocw-server.fly.dev); nothing to set here.
#
# Settings file: OCW_KIOSK_DIR/kiosk.env (installed by scripts/deploy-test.sh
# from scripts/kiosk.env) and OCW_KIOSK_DIR/kiosk-local.env (machine-local,
# kept by deploys) hold OCW_KIOSK_*=VALUE lines; read at every kiosk start,
# kiosk-local.env first. A variable already set in the environment wins over
# both. A deploy that changes one restarts the kiosk on it.
#
#   ocw-kiosk.sh dry-run [--monitor SPEC] [-- app args...]
#       Print what `run` would start now (mode, join info, the app's command
#       line and arguments file) without starting anything.
set -uo pipefail

dir=${OCW_KIOSK_DIR:-$(cd "$(dirname "$0")" && pwd)}

# The settings files (OCW_KIOSK_*=VALUE; quotes around VALUE are removed):
# only variables the environment does not set. The names taken from the
# files are remembered, so a kiosk that restarts itself reads them again.
read_env_files() {
  local f line key value
  local -a from=()
  for key in ${OCW_KIOSK_ENV_FROM_FILE:-}; do unset "$key"; done
  for f in "$dir/kiosk-local.env" "$dir/kiosk.env"; do
    [ -f "$f" ] || continue
    while IFS= read -r line || [ -n "$line" ]; do
      [[ $line =~ ^[[:space:]]*(OCW_KIOSK_[A-Z0-9_]+)=(.*)$ ]] || continue
      key=${BASH_REMATCH[1]} value=${BASH_REMATCH[2]}
      [ "$key" = OCW_KIOSK_DIR ] && continue
      [ -n "${!key+set}" ] && continue
      value=${value%"${value##*[![:space:]]}"}
      if [[ $value =~ ^\"(.*)\"$ ]] || [[ $value =~ ^\'(.*)\'$ ]]; then value=${BASH_REMATCH[1]}; fi
      export "$key=$value"
      from+=("$key")
    done <"$f"
  done
  export OCW_KIOSK_ENV_FROM_FILE="${from[*]}"
}
read_env_files
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
self_sig() {
  local f
  for f in ocw-kiosk.sh kiosk.env kiosk-local.env; do
    printf '%s ' "$(stat -c '%Y %i %s' "$dir/$f" 2>/dev/null || echo none)"
  done
}

# The server mode: local (default), off, join or remote; remote falls back
# to local while the remote server cannot be joined (FALLBACK=1).
mode() {
  case "${OCW_KIOSK_SERVER:-1}" in
    0) echo off ;;
    join) echo join ;;
    remote) [ "${FALLBACK:-0}" = 1 ] && echo local || echo remote ;;
    *) echo local ;;
  esac
}

server_wanted() { case "$(mode)" in off | remote) return 1 ;; join) return 0 ;; esac; [ -x "$server_bin" ]; }

# --- Joining a remote server (OCW_KIOSK_SERVER=remote) ----------------------
remote_url=${OCW_KIOSK_REMOTE:-https://ocw-server.fly.dev/join.json}
remote_timeout=${OCW_KIOSK_REMOTE_TIMEOUT_S:-10}
remote_tries=${OCW_KIOSK_REMOTE_TRIES:-3}
remote_delay=${OCW_KIOSK_REMOTE_DELAY_S:-5}
remote_retry=${OCW_KIOSK_REMOTE_RETRY_S:-60}

# Reads the join info (untrusted: only checked values come out) and the
# world manifests. `info FILE` prints tab-separated lines: link, world,
# version, sha256, voice (or `error TEXT`); `manifest FILE` prints the id and
# version of a package manifest.
# shellcheck disable=SC2016
read -r -d '' KIOSK_PY <<'PY'
import json, re, sys, urllib.parse
URL = r"[0-9A-Za-z._~:/?&=%@,+\[\]-]"  # what a link may hold (no quotes, no spaces)
def out(k, v):
    print(k + "\t" + v)
cmd, path = sys.argv[1], sys.argv[2]
try:
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
except Exception as e:
    out("error", "not JSON (%s)" % type(e).__name__)
    sys.exit(0)
if cmd == "manifest":
    s = d.get("signed", d) if isinstance(d, dict) else {}
    print("%s\t%s" % (s.get("id", ""), s.get("version", "")))
    sys.exit(0)
if not isinstance(d, dict):
    out("error", "not a join info object")
    sys.exit(0)
def text(k, pat):
    v = d.get(k)
    if isinstance(v, int) and not isinstance(v, bool):
        v = str(v)
    return v if isinstance(v, str) and re.fullmatch(pat, v) else ""
world = text("world", r"[a-z0-9][a-z0-9/_.-]{0,99}")
version = text("world_version", r"[0-9]{1,12}")
sha = text("world_sha256", r"[0-9a-f]{64}")
route = d.get("route") if isinstance(d.get("route"), str) else ""
link = d.get("join") if isinstance(d.get("join"), str) else ""
if not re.fullmatch(r"ocw://[0-9A-Za-z.:\[\]-]+\?" + URL + "{1,1000}", link or ""):
    link = ""
    web = d.get("webtransport") if isinstance(d.get("webtransport"), str) else ""
    hashes = d.get("cert_hashes") if isinstance(d.get("cert_hashes"), list) else []
    h = hashes[0] if hashes and isinstance(hashes[0], str) else ""
    if re.fullmatch(r"https://[0-9A-Za-z.:\[\]-]+", web) and re.fullmatch(r"[0-9a-f]{64}", h):
        q = [("cert-hash", h)]
        if world:
            q.append(("world", world + ("@" + version if version else "")))
        if route:
            q.append(("route", route))
        link = web + "?" + "&".join(k + "=" + urllib.parse.quote(v, safe="@/") for k, v in q)
if not link:
    out("error", "no join link in it (neither join nor webtransport with cert_hashes)")
    sys.exit(0)
out("link", link)
out("world", world)
out("version", version)
out("sha256", sha)
out("voice", text("voice", r"wss?://" + URL + "{1,200}"))
PY

# The SHA-256 of a package, cached by its mtime, inode and size.
pkg_sha() {
  local sig cache=$state/world-sha.cache sha
  sig=$(stat -c '%Y %i %s' "$1" 2>/dev/null) || return 1
  sha=$(awk -F'\t' -v p="$1" -v s="$sig" '$1 == p && $2 == s { print $3 }' "$cache" 2>/dev/null | tail -n 1)
  if [ -z "$sha" ]; then
    sha=$(sha256sum "$1" | cut -d' ' -f1) || return 1
    mkdir -p "$state"
    { grep -v "^$1	" "$cache" 2>/dev/null; printf '%s\t%s\t%s\n' "$1" "$sig" "$sha"; } >"$cache.new" && mv -f "$cache.new" "$cache"
  fi
  echo "$sha"
}

# Fetches the join info and finds the server's world here. Sets
# REMOTE_LINK, REMOTE_WORLD (ID@VERSION), REMOTE_PKG (the local package, or
# empty for a server without a world) and REMOTE_VOICE; on failure
# REMOTE_WHY. Returns 0 (joinable), 1 (no join info) or 2 (world missing).
remote_info() {
  REMOTE_LINK="" REMOTE_WORLD="" REMOTE_PKG="" REMOTE_VOICE="" REMOTE_WHY=""
  local tmp k v world="" version="" sha="" p
  mkdir -p "$state"
  tmp=$state/join.json.new
  if ! command -v curl >/dev/null 2>&1; then REMOTE_WHY="curl is not installed"; return 1; fi
  if ! command -v python3 >/dev/null 2>&1; then REMOTE_WHY="python3 is not installed"; return 1; fi
  if ! curl -fsS --max-time "$remote_timeout" --max-filesize 65536 -o "$tmp" "$remote_url" 2>"$state/join.err"; then
    REMOTE_WHY="join info $remote_url not fetched: $(head -c 200 "$state/join.err" | tr '\n' ' ')"
    rm -f "$tmp"
    return 1
  fi
  while IFS=$'\t' read -r k v; do
    case "$k" in
      error) REMOTE_WHY="join info $remote_url: $v" ;;
      link) REMOTE_LINK=$v ;;
      world) world=$v ;;
      version) version=$v ;;
      sha256) sha=$v ;;
      voice) REMOTE_VOICE=$v ;;
    esac
  done < <(python3 -I -c "$KIOSK_PY" info "$tmp")
  mv -f "$tmp" "$state/join.json"
  [ -n "$REMOTE_LINK" ] || { REMOTE_WHY=${REMOTE_WHY:-"join info $remote_url: unreadable"}; return 1; }
  [ -n "$world" ] || return 0   # a test route: no package needed
  REMOTE_WORLD=$world${version:+@$version}
  for p in "$dir"/*.ocwpkg; do
    [ -f "$p" ] || continue
    if [ -n "$sha" ]; then
      [ "$(pkg_sha "$p")" = "$sha" ] && { REMOTE_PKG=$p; return 0; }
    elif [ -f "$p.manifest.json" ]; then
      IFS=$'\t' read -r k v < <(python3 -I -c "$KIOSK_PY" manifest "$p.manifest.json")
      [ "$k" = "$world" ] && { [ -z "$version" ] || [ "$v" = "$version" ]; } && { REMOTE_PKG=$p; return 0; }
    fi
  done
  REMOTE_WHY="the server rides the world $REMOTE_WORLD${sha:+ (SHA-256 ${sha:0:12}…)}, which is not installed in $dir (or differs: deploy the same package)"
  return 2
}

# The app's arguments file for joining: ocw-desktop.args without its course
# and server options, the server's package as --world and no bots.
remote_args_text() {
  echo "# Written by ocw-kiosk.sh: joining $remote_url (OCW_KIOSK_SERVER=remote)"
  grep -vE '^[[:space:]]*--(world|route|route-file|route-gpx|free-ride|start|test-route|bots|server|server-cert|insecure-dev|join)([[:space:]=]|$)' \
    "$dir/ocw-desktop.args" 2>/dev/null
  [ -n "$REMOTE_PKG" ] && printf -- "--world '%s'\n" "$REMOTE_PKG"
  echo "--bots 0"
}

# Sets up the app's options to join the remote server (SERVER_ARGS,
# APP_ENV); retries and falls back as the header says. Returns 0 when the
# app joins, 1 when it falls back to the local mode.
start_remote() {
  local rc
  while true; do
    remote_info
    rc=$?
    if [ "$rc" = 0 ]; then
      remote_args_text >"$state/remote.args"
      printf '%s\n' "$REMOTE_LINK" "world: ${REMOTE_WORLD:-none} ${REMOTE_PKG:+($REMOTE_PKG)}" "voice: ${REMOTE_VOICE:-none}" >"$state/remote.txt"
      APP_ENV=("OCW_ARGS_FILE=$state/remote.args")
      SERVER_ARGS=(--join "$REMOTE_LINK")
      REMOTE_ON=1
      say "joining the remote server: $REMOTE_LINK (world ${REMOTE_WORLD:-none}${REMOTE_PKG:+, $(basename "$REMOTE_PKG")}; voice ${REMOTE_VOICE:-none})"
      return 0
    fi
    [ "$rc" = 2 ] && break
    remote_fails=$((remote_fails + 1))
    [ "$remote_fails" -ge "$remote_tries" ] && break
    local wait=$((remote_delay << (remote_fails - 1)))
    say "$REMOTE_WHY; trying again in $wait s"
    pause "$wait"
  done
  fallback "$REMOTE_WHY"
  return 1
}

# Falls back to the local mode, checking the remote server every
# remote_retry seconds.
fallback() {
  FALLBACK=1
  next_probe=$(($(date +%s) + remote_retry))
  say "$1 - falling back to the local mode (OCW_KIOSK_SERVER=1) and checking $remote_url again every $remote_retry s"
}

# A sleep the kiosk's TERM trap interrupts.
pause() { sleep "$1" & wait $!; }

# What the app's log $1 says since byte $2: "dropped<TAB>LINE" (a lost
# connection or a failed join), "joined", or nothing.
app_link_state() {
  local new line
  new=$(tail -c +"$(($2 + 1))" "$1" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g')
  line=$(grep -a -m1 -oE '(Disconnected: |join failed: ).*' <<<"$new")
  [ -n "$line" ] && { printf 'dropped\t%s\n' "$line"; return; }
  grep -aqE 'joined [^ ]+: ' <<<"$new" && echo joined
}

# Whether something listens on UDP port $1 on this machine.
udp_busy() {
  command -v ss >/dev/null 2>&1 || return 1
  [ -n "$(ss -Hlun "sport = :$1" 2>/dev/null)" ]
}

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
  local port=${listen##*:}
  if [ "${OCW_KIOSK_SERVER:-1}" = join ] || udp_busy "$port"; then
    local waited=0
    until udp_busy "$port"; do
      [ "$waited" -ge "$ready_s" ] && { say "no game server on UDP $port after ${ready_s} s; the app rides alone"; return 0; }
      sleep 1; waited=$((waited + 1))
    done
    SERVER_ARGS=(--server "127.0.0.1:$port" --insecure-dev)
    say "joining the game server already running on UDP $port (the app must ride its world and route)"
    return 0
  fi
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
  # The join link(s) (Ride -> Join, ocw-desktop --join) and the command line.
  { grep -A3 "^Join link" "$state/server.out" | grep -v "^Clients" | grep -v "ocw-desktop --server"
    grep -A2 "Clients connect with" "$state/server.out"; } >"$state/server-connect.txt"
  SERVER_ARGS=(--server "127.0.0.1:$port" --insecure-dev)
  say "game server ready after ${waited} s: $(grep -m1 "ocw://" "$state/server-connect.txt" | sed 's/^ *//')"
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
  # The app keeps them in ocw.toml [devices] (spec 3.2.4): seed only once.
  if [ -f "$dir/devices.toml" ] && [ ! -f "$cfg" ] && ! grep -qs '^\[\[devices\.device\]\]' "$(dirname "$cfg")/ocw.toml"; then
    mkdir -p "$(dirname "$cfg")" && cp "$dir/devices.toml" "$cfg" && say "seeded $cfg"
  fi
  local child="" fails=0
  SERVER_PID="" SERVER_ARGS=() APP_ENV=() REMOTE_ON=0 FALLBACK=0
  remote_fails=0 next_probe=0
  trap 'say "kiosk stopping"; [ -n "$child" ] && stop_app "$child"; [ -n "$SERVER_PID" ] && stop_app "$SERVER_PID"; exit 0' TERM INT
  say "kiosk started (dir $dir, server mode $(mode)$([ "$(mode)" = remote ] && echo ": $remote_url"), args: ${APP_ARGS[*]:-none})"
  local me
  me=$(self_sig)
  while true; do
    until [ -x "$bin" ]; do sleep "$poll"; done
    local sig started reason=exited offset joined=0
    sig=$(signature)
    started=$(date +%s)
    say "starting $(cat "$dir/VERSION" 2>/dev/null || echo 'unknown build')"
    SERVER_PID="" SERVER_ARGS=() APP_ENV=() REMOTE_ON=0
    if [ "$(mode)" = remote ]; then
      start_remote || start_server
    else
      start_server
    fi
    offset=$(stat -c %s "$applog" 2>/dev/null || echo 0)
    env "${APP_ENV[@]}" "$bin" "${APP_ARGS[@]}" "${SERVER_ARGS[@]}" >>"$applog" 2>&1 &
    child=$!
    while kill -0 "$child" 2>/dev/null; do
      sleep "$poll"
      if [ "$REMOTE_ON" = 1 ]; then
        local link_state
        link_state=$(app_link_state "$applog" "$offset")
        case "$link_state" in
          dropped*)
            say "remote server: ${link_state#dropped	}; fetching the join info again"
            reason=remote
            stop_app "$child"
            break
            ;;
          joined)
            if [ "$joined" = 0 ]; then
              joined=1 remote_fails=0
              say "joined the remote server"
            fi
            ;;
        esac
      fi
      if [ "$FALLBACK" = 1 ] && [ "$(date +%s)" -ge "$next_probe" ]; then
        next_probe=$(($(date +%s) + remote_retry))
        if remote_info; then
          say "the remote server can be joined again ($REMOTE_LINK), switching to it"
          FALLBACK=0
          remote_fails=$((remote_tries - 1))   # one more failure falls back at once
          reason=switch
          stop_app "$child"
          break
        fi
      fi
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
    [ "$reason" = switch ] && { fails=0; continue; }
    if [ "$reason" = remote ]; then
      fails=0
      local wait=$remote_delay
      if [ "$joined" = 0 ]; then
        # The join itself failed (no answer, refused, other world).
        remote_fails=$((remote_fails + 1))
        if [ "$remote_fails" -ge "$remote_tries" ]; then
          fallback "$remote_fails joins of the remote server failed in a row"
          continue
        fi
        wait=$((remote_delay << (remote_fails - 1)))
      fi
      say "restarting the app in $wait s"
      pause "$wait"
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

# What `run` would start now; starts nothing (writes only the join info and
# the package hash cache to the state folder).
dry_run() {
  parse_args "$@"
  local m
  m=$(mode)
  # Shell words, single-quoted where needed (readable links).
  q() {
    local w
    for w in "$@"; do
      if [[ $w =~ ^[A-Za-z0-9_./:=@,+-]+$ ]]; then printf ' %s' "$w"; else printf " '%s'" "${w//\'/\'\\\'\'}"; fi
    done
  }
  echo "kiosk dir: $dir"
  echo "server mode: $m${OCW_KIOSK_ENV_FROM_FILE:+ (settings from kiosk*.env: $OCW_KIOSK_ENV_FROM_FILE)}"
  if [ "$m" = remote ]; then
    echo "join info: $remote_url"
    remote_info
    case $? in
      0)
        echo "join link: $REMOTE_LINK"
        echo "world: ${REMOTE_WORLD:-none (a test route)}${REMOTE_PKG:+ -> $REMOTE_PKG (SHA-256 matches)}"
        echo "voice: ${REMOTE_VOICE:-none}"
        echo "no local ocw-server, no bots"
        echo "app arguments file ($state/remote.args):"
        remote_args_text | grep -v '^#' | sed 's/^/  /'
        echo "app command:"
        echo "  OCW_ARGS_FILE=$state/remote.args$(q "$bin" "${APP_ARGS[@]}" --join "$REMOTE_LINK")"
        return 0
        ;;
      *)
        echo "not joinable: $REMOTE_WHY"
        echo "run would retry, then fall back to the local mode:"
        FALLBACK=1 m=local
        ;;
    esac
  fi
  if [ "$m" = local ] && [ -x "$server_bin" ]; then
    echo "local game server: $server_bin --listen $listen --bots ${OCW_KIOSK_BOTS:-10} (world and route of ocw-desktop.args)"
    echo "app command:"
    echo " $(q "$bin" "${APP_ARGS[@]}" --server "127.0.0.1:${listen##*:}" --insecure-dev)"
  elif [ "$m" = join ]; then
    echo "app command:"
    echo " $(q "$bin" "${APP_ARGS[@]}" --server "127.0.0.1:${listen##*:}" --insecure-dev)"
  else
    echo "app command (alone, its own local server):"
    echo " $(q "$bin" "${APP_ARGS[@]}")"
  fi
}

case "${1:-}" in
  run) shift; run "$@" ;;
  dry-run) shift; dry_run "$@" ;;
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
    echo "server mode: $(mode)"
    if [ "$(mode)" = remote ]; then
      echo "remote server ($remote_url), last join:"
      sed 's/^/  /' "$state/remote.txt" 2>/dev/null || echo "  none yet"
    elif kill -0 "$(cat "$state/server.pid" 2>/dev/null || echo 0)" 2>/dev/null; then
      echo "game server: running; riders join with:"
      grep '^ ' "$state/server-connect.txt" 2>/dev/null
      cat "$state/server.out" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | grep -a -o "voice chat o[nf].*" | tail -n 1 | sed 's/^/voice: /'
    elif udp_busy "${listen##*:}"; then
      echo "game server: another one listens on UDP ${listen##*:} (the kiosk joins it)"
    else
      echo "game server: not running"
    fi
    [ -f "$log" ] && tail -n 10 "$log"
    ;;
  *) sed -n '2,20p' "$0"; exit 2 ;;
esac
