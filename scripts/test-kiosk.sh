#!/usr/bin/env bash
# Tests ocw-kiosk.sh with a fake app (no display, GPU or build needed).
set -euo pipefail
cd "$(dirname "$0")/.."
t=$(mktemp -d)
www=""
trap 'kill $kiosk 2>/dev/null || true; [ -z "$www" ] || kill $www 2>/dev/null || true; pkill -f "sleep 60[01]" 2>/dev/null || true; rm -rf "$t"' EXIT
export OCW_KIOSK_DIR=$t/deploy HOME=$t/home XDG_STATE_HOME=$t/state XDG_CONFIG_HOME=$t/config
export OCW_KIOSK_POLL_S=0.2 OCW_KIOSK_SETTLE_S=0.2 OCW_KIOSK_QUICK_S=1
mkdir -p "$OCW_KIOSK_DIR" "$HOME"
fake() { # fake app $1: records its version, arguments and arguments file, then runs
  printf '#!/bin/sh\necho "%s $*${OCW_ARGS_FILE:+ argsfile=$OCW_ARGS_FILE}" >> %s\nexec sleep 600\n' "$1" "$t/runs" > "$OCW_KIOSK_DIR/.new"
  chmod 755 "$OCW_KIOSK_DIR/.new"
  mv -f "$OCW_KIOSK_DIR/.new" "$OCW_KIOSK_DIR/ocw-desktop"
}
wait_for() { for _ in $(seq 50); do eval "$1" && return 0; sleep 0.2; done; echo "FAIL: $2"; exit 1; }
runs() { [ -f "$t/runs" ] && wc -l < "$t/runs" || echo 0; }
fail=0

echo '[[device]]' > "$OCW_KIOSK_DIR/devices.toml"
fake v1
scripts/ocw-kiosk.sh run --monitor HDMI -- --virtual &
kiosk=$!
wait_for '[ "$(runs)" -eq 1 ]' "app started"
grep -q "v1 --monitor HDMI --virtual" "$t/runs" || { echo "FAIL: arguments"; fail=1; }
[ -f "$XDG_CONFIG_HOME/ocw/devices.toml" ] || { echo "FAIL: devices.toml seeded"; fail=1; }

# A second kiosk is refused.
if scripts/ocw-kiosk.sh run 2>/dev/null; then echo "FAIL: second instance ran"; fail=1; fi

# New build → restart with the new binary.
fake v2
wait_for '[ "$(runs)" -eq 2 ]' "restart after deployment"
tail -1 "$t/runs" | grep -q '^v2 ' || { echo "FAIL: new build started"; fail=1; }

# Crash → restart (after the 3 s pause).
pkill -f "sleep 600" || true
for _ in $(seq 40); do [ "$(runs)" -ge 3 ] && break; sleep 0.2; done
[ "$(runs)" -ge 3 ] || { echo "FAIL: restart after crash"; fail=1; }

# Stopping the kiosk stops the app.
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
sleep 0.5
if pgrep -f "sleep 600" >/dev/null; then echo "FAIL: app still running"; pkill -f "sleep 600"; fail=1; fi
grep -q "new build deployed" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: log"; fail=1; }


# A new kiosk script restarts the kiosk itself (self-update).
cp scripts/ocw-kiosk.sh "$OCW_KIOSK_DIR/ocw-kiosk.sh"
rm -f "$t/runs"
OCW_KIOSK_SERVER=0 "$OCW_KIOSK_DIR/ocw-kiosk.sh" run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "kiosk from the deploy dir"
{ cat scripts/ocw-kiosk.sh; echo "# v2"; } > "$OCW_KIOSK_DIR/.k" && chmod 755 "$OCW_KIOSK_DIR/.k" && mv -f "$OCW_KIOSK_DIR/.k" "$OCW_KIOSK_DIR/ocw-kiosk.sh"
fake v2b
wait_for 'grep -q "new kiosk script deployed" "$XDG_STATE_HOME/ocw/kiosk.log"' "kiosk restarted itself"
wait_for '[ "$(runs)" -ge 2 ]' "app started by the new kiosk"
# Subshells of the kiosk share its command line for a moment: the fewest
# of a few samples is the number of kiosks.
n=9; for _ in 1 2 3 4 5; do c=$(pgrep -fc "$OCW_KIOSK_DIR/ocw-kiosk.sh run" || true); [ "$c" -lt "$n" ] && n=$c; sleep 0.1; done
[ "$n" -le 1 ] || { echo "FAIL: $n kiosks: $(pgrep -af "$OCW_KIOSK_DIR/ocw-kiosk.sh run" | tr "\n" ";")"; fail=1; }
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
pkill -f "sleep 600" 2>/dev/null || true
rm -f "$OCW_KIOSK_DIR/ocw-kiosk.sh"

# Game server (2026-11-15): with ocw-server in the kiosk dir, the kiosk starts
# it for the app's world and route, then the app as its client; a server
# crash restarts both.
rm -f "$t/runs" "$t/srv"
printf '%s\n' '# test world' '--world /w/tenerife.ocwpkg' '--route "Puerto de la Cruz - Teide"' > "$OCW_KIOSK_DIR/ocw-desktop.args"
printf '#!/bin/sh\necho "srv $*" >> %s\necho "Join link (in the app: Ride -> Join; or ocw-desktop --join LINK):"\necho "  ocw://192.0.2.7:47999?cert=ab12&route=hills"\necho "Clients connect with:"\necho "  ocw-desktop --server host --server-cert ab12"\nexec sleep 601\n' "$t/srv" > "$OCW_KIOSK_DIR/ocw-server"
chmod 755 "$OCW_KIOSK_DIR/ocw-server"
fake v3
OCW_KIOSK_LISTEN=0.0.0.0:47999 OCW_KIOSK_BOTS=4 scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started with the server"
grep -q -- "--world /w/tenerife.ocwpkg --route Puerto de la Cruz - Teide" "$t/srv" || { echo "FAIL: server course: $(cat "$t/srv")"; fail=1; }
grep -q -- "--listen 0.0.0.0:47999" "$t/srv" && grep -q -- "--bots 4" "$t/srv" || { echo "FAIL: server options"; fail=1; }
tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1:47999 --insecure-dev" || { echo "FAIL: app not a client: $(tail -1 "$t/runs")"; fail=1; }
grep -q "ab12" "$XDG_STATE_HOME/ocw/server-connect.txt" || { echo "FAIL: connect line"; fail=1; }
grep -q "^  ocw://192.0.2.7:47999?cert=ab12&route=hills" "$XDG_STATE_HOME/ocw/server-connect.txt" || { echo "FAIL: join link: $(cat "$XDG_STATE_HOME/ocw/server-connect.txt")"; fail=1; }
grep -q "ocw://192.0.2.7" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: join link in the log"; fail=1; }
st=$(scripts/ocw-kiosk.sh status 2>&1); echo "$st" | grep -q "game server: running" || { echo "FAIL: status: $st"; fail=1; }
before=$(runs)
pkill -f "sleep 601" || true
wait_for '[ "$(runs)" -gt "$before" ]' "both restarted after a server crash"
[ "$(wc -l < "$t/srv")" -ge 2 ] || { echo "FAIL: server restarted"; fail=1; }
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
sleep 0.5
if pgrep -f "sleep 601" >/dev/null; then echo "FAIL: server still running"; pkill -f "sleep 601"; fail=1; fi
# A game server already listening on the port is joined, not started again.
python3 -c 'import socket,time; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.bind(("127.0.0.1",47998)); time.sleep(30)' &
udp=$!
sleep 0.5
rm -f "$t/runs" "$t/srv"
OCW_KIOSK_LISTEN=0.0.0.0:47998 scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started against the running server"
tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1:47998 --insecure-dev" || { echo "FAIL: did not join: $(tail -1 "$t/runs")"; fail=1; }
[ ! -f "$t/srv" ] || { echo "FAIL: started a second server"; fail=1; }
grep -q "joining the game server already running" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: join log"; fail=1; }
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
kill "$udp" 2>/dev/null || true
pkill -f "sleep 600" 2>/dev/null || true
# OCW_KIOSK_SERVER=0: the app rides alone.
rm -f "$t/runs"
OCW_KIOSK_SERVER=0 scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started without the server"
if grep -q -- "--server" "$t/runs"; then echo "FAIL: --server without a server"; fail=1; fi
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
pkill -f "sleep 600" 2>/dev/null || true

# Remote server (spec 11.5 *Join info*): OCW_KIOSK_SERVER=remote from the
# settings files fetches a fake join.json, joins with its link on the local
# package of the same SHA-256, no local server, no bots; re-fetches after a
# dropped connection; falls back to the local mode when the world differs or
# the join info cannot be fetched, and switches back when it can join.
mkdir -p "$t/www"
port=$((20000 + RANDOM % 20000))
python3 -I -m http.server --bind 127.0.0.1 --directory "$t/www" "$port" >/dev/null 2>&1 &
www=$!
url="http://127.0.0.1:$port/join.json"
printf 'kiosk world bytes\n' > "$OCW_KIOSK_DIR/kioskworld.ocwpkg"
printf 'other world bytes\n' > "$OCW_KIOSK_DIR/other.ocwpkg"
sha=$(sha256sum "$OCW_KIOSK_DIR/kioskworld.ocwpkg" | cut -d' ' -f1)
fp1=$(printf 'a%.0s' $(seq 64)) fp2=$(printf 'b%.0s' $(seq 64)) wt=$(printf 'c%.0s' $(seq 64))
join_json() { # $1 QUIC fingerprint ("" = no join key), $2 world SHA-256
  local j=""
  [ -n "$1" ] && j=", \"join\": \"ocw://203.0.113.7?cert=$1&world=kioskworld@3\""
  printf '{"webtransport": "https://203.0.113.7:47801", "cert_hashes": ["%s"], "not_after_unix_s": 1800000000, "voice": "wss://voice.example.org", "world": "kioskworld", "world_version": 3, "world_sha256": "%s", "query": "x"%s}\n' \
    "$wt" "$2" "$j" > "$t/www/.j" && mv -f "$t/www/.j" "$t/www/join.json"
}
join_json "$fp1" "$sha"
for _ in $(seq 25); do curl -fs -o /dev/null "$url" && break; sleep 0.2; done
printf '%s\n' '# shipped' 'OCW_KIOSK_SERVER=remote' 'OCW_KIOSK_REMOTE=http://127.0.0.1:1/wrong.json' > "$OCW_KIOSK_DIR/kiosk.env"
printf '%s\n' '# this machine' "OCW_KIOSK_REMOTE=\"$url\"" > "$OCW_KIOSK_DIR/kiosk-local.env"
export OCW_KIOSK_REMOTE_DELAY_S=0 OCW_KIOSK_REMOTE_RETRY_S=1 OCW_KIOSK_REMOTE_TRIES=2 OCW_KIOSK_REMOTE_TIMEOUT_S=3
# Dry run: prints the command, starts nothing.
rm -f "$t/runs"
dry=$(scripts/ocw-kiosk.sh dry-run 2>&1)
echo "$dry" | grep -q "server mode: remote" || { echo "FAIL: dry-run mode: $dry"; fail=1; }
echo "$dry" | grep -q -- "--join 'ocw://203.0.113.7?cert=$fp1&world=kioskworld@3'" || { echo "FAIL: dry-run link: $dry"; fail=1; }
echo "$dry" | grep -q "kioskworld.ocwpkg (SHA-256 matches)" || { echo "FAIL: dry-run world: $dry"; fail=1; }
[ "$(runs)" -eq 0 ] || { echo "FAIL: dry-run started the app"; fail=1; }
dry=$(OCW_KIOSK_SERVER=0 scripts/ocw-kiosk.sh dry-run 2>&1)
echo "$dry" | grep -q "server mode: off" || { echo "FAIL: the environment wins over kiosk.env: $dry"; fail=1; }
# Run: joins.
rm -f "$t/runs" "$t/srv"
scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started for the remote server"
r=$(tail -1 "$t/runs")
echo "$r" | grep -q -- "--join ocw://203.0.113.7?cert=$fp1&world=kioskworld@3" || { echo "FAIL: remote join args: $r"; fail=1; }
if echo "$r" | grep -q -- "--server"; then echo "FAIL: --server in remote mode: $r"; fail=1; fi
[ ! -f "$t/srv" ] || { echo "FAIL: a local server in remote mode"; fail=1; }
af=$XDG_STATE_HOME/ocw/remote.args
echo "$r" | grep -q "argsfile=$af" || { echo "FAIL: arguments file not passed: $r"; fail=1; }
grep -qx -- "--world '$OCW_KIOSK_DIR/kioskworld.ocwpkg'" "$af" && grep -qx -- "--bots 0" "$af" || { echo "FAIL: remote args: $(cat "$af")"; fail=1; }
if grep -qE -- "^--(route|world /w)" "$af"; then echo "FAIL: the kiosk's own course kept: $(cat "$af")"; fail=1; fi
# Joined, then the connection drops (a server restart with a new certificate):
# the join info is fetched again and the app joins with the new link.
echo "2026-11-26T10:00:00Z  INFO ocw_desktop::ride::join: joined 203.0.113.7:47800: kioskworld, free ride" >> "$XDG_STATE_HOME/ocw/app.log"
wait_for 'grep -q "joined the remote server" "$XDG_STATE_HOME/ocw/kiosk.log"' "joined logged"
join_json "$fp2" "$sha"
printf '\033[33mWARN\033[0m ocw_desktop::connection: Disconnected: the server closed the connection\n' >> "$XDG_STATE_HOME/ocw/app.log"
wait_for '[ "$(runs)" -ge 2 ]' "app restarted after the dropped connection"
tail -1 "$t/runs" | grep -q -- "cert=$fp2" || { echo "FAIL: not re-fetched: $(tail -1 "$t/runs")"; fail=1; }
grep -q "Disconnected: the server closed the connection; fetching the join info again" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: drop log"; fail=1; }
# Two failed joins in a row (OCW_KIOSK_REMOTE_TRIES=2) fall back to the
# local server; when the join info says the same again, the kiosk switches
# back to the remote server after OCW_KIOSK_REMOTE_RETRY_S.
before=$(runs)
echo "WARN ocw_desktop::ride::join: join failed: no answer from the server within 10 s" >> "$XDG_STATE_HOME/ocw/app.log"
wait_for '[ "$(runs)" -gt "$before" ]' "app restarted after a failed join"
before=$(runs)
echo "WARN ocw_desktop::ride::join: join failed: no answer from the server within 10 s" >> "$XDG_STATE_HOME/ocw/app.log"
wait_for 'grep -q "2 joins of the remote server failed in a row - falling back" "$XDG_STATE_HOME/ocw/kiosk.log"' "fallback after failed joins"
wait_for '[ "$(runs)" -gt "$before" ] && tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1"' "local mode after failed joins"
[ -f "$t/srv" ] || { echo "FAIL: no local server in the fallback"; fail=1; }
wait_for 'grep -q "the remote server can be joined again" "$XDG_STATE_HOME/ocw/kiosk.log"' "switch back to the remote server"
wait_for 'tail -1 "$t/runs" | grep -q -- "--join ocw://"' "joined again after the fallback"
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
pkill -f "sleep 60[01]" 2>/dev/null || true
# The world differs (another SHA-256): fall back at once, say why.
rm -f "$t/runs" "$t/srv"
join_json "$fp1" "$(printf 'd%.0s' $(seq 64))"
scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started on a world mismatch"
tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1" || { echo "FAIL: no local fallback on a world mismatch: $(tail -1 "$t/runs")"; fail=1; }
grep -q "the server rides the world kioskworld@3 .*which is not installed" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: mismatch log"; fail=1; }
# The right package arrives on the server side: the kiosk switches.
join_json "$fp1" "$sha"
wait_for 'tail -1 "$t/runs" | grep -q -- "--join ocw://"' "switch after the world matches"
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
pkill -f "sleep 60[01]" 2>/dev/null || true
# A server without the `join` key: the WebTransport link with the current hash.
join_json "" "$sha"
dry=$(scripts/ocw-kiosk.sh dry-run 2>&1)
echo "$dry" | grep -q -- "--join 'https://203.0.113.7:47801?cert-hash=$wt&world=kioskworld@3'" || { echo "FAIL: WebTransport link: $dry"; fail=1; }
# The join info cannot be fetched: retried, then the local mode.
kill "$www" 2>/dev/null || true; wait "$www" 2>/dev/null || true; www=""
dry=$(scripts/ocw-kiosk.sh dry-run 2>&1)
echo "$dry" | grep -q "not joinable: join info $url not fetched" || { echo "FAIL: fetch failure in dry-run: $dry"; fail=1; }
rm -f "$t/runs" "$t/srv"
scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started without the join info"
tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1" || { echo "FAIL: no local fallback without join info: $(tail -1 "$t/runs")"; fail=1; }
grep -q "not fetched.*trying again in 0 s" "$XDG_STATE_HOME/ocw/kiosk.log" || { echo "FAIL: no retry before the fallback"; fail=1; }
kill -TERM "$kiosk"; wait "$kiosk" 2>/dev/null || true
pkill -f "sleep 60[01]" 2>/dev/null || true
rm -f "$OCW_KIOSK_DIR/kiosk.env" "$OCW_KIOSK_DIR/kiosk-local.env" "$OCW_KIOSK_DIR"/*.ocwpkg
unset OCW_KIOSK_REMOTE_DELAY_S OCW_KIOSK_REMOTE_RETRY_S OCW_KIOSK_REMOTE_TRIES OCW_KIOSK_REMOTE_TIMEOUT_S
# The Windows VR kiosk's world files (scripts/deploy-test-windows.sh): the
# package (Black Forest: north first), its manifest, every route file.
l=$'a.route\nblackforest.ocwpkg\nblackforest-north.ocwpkg\nblackforest-north.ocwpkg.manifest.json\ntenerife.ocwpkg\nb.route'
wf() { scripts/deploy-test-windows.sh --world-files "$1" "$l" | tr '\n' ' '; }
[ "$(wf blackforest)" = "blackforest-north.ocwpkg blackforest-north.ocwpkg.manifest.json a.route b.route " ] || { echo "FAIL: world files blackforest: $(wf blackforest)"; fail=1; }
[ "$(wf tenerife)" = "tenerife.ocwpkg a.route b.route " ] || { echo "FAIL: world files tenerife: $(wf tenerife)"; fail=1; }
[ -z "$(wf liechtenstein)" ] || { echo "FAIL: world files of a missing package: $(wf liechtenstein)"; fail=1; }
[ "$fail" -eq 0 ] && echo "kiosk tests passed" || exit 1
