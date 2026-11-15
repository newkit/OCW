#!/usr/bin/env bash
# Tests ocw-kiosk.sh with a fake app (no display, GPU or build needed).
set -euo pipefail
cd "$(dirname "$0")/.."
t=$(mktemp -d)
trap 'kill $kiosk 2>/dev/null || true; pkill -f "sleep 60[01]" 2>/dev/null || true; rm -rf "$t"' EXIT
export OCW_KIOSK_DIR=$t/deploy HOME=$t/home XDG_STATE_HOME=$t/state XDG_CONFIG_HOME=$t/config
export OCW_KIOSK_POLL_S=0.2 OCW_KIOSK_SETTLE_S=0.2 OCW_KIOSK_QUICK_S=1
mkdir -p "$OCW_KIOSK_DIR" "$HOME"
fake() { # fake app $1: records its version and arguments, then runs
  printf '#!/bin/sh\necho "%s $*" >> %s\nexec sleep 600\n' "$1" "$t/runs" > "$OCW_KIOSK_DIR/.new"
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
printf '#!/bin/sh\necho "srv $*" >> %s\necho "Clients connect with:"\necho "  ocw-desktop --server host --server-cert ab12"\nexec sleep 601\n' "$t/srv" > "$OCW_KIOSK_DIR/ocw-server"
chmod 755 "$OCW_KIOSK_DIR/ocw-server"
fake v3
OCW_KIOSK_LISTEN=0.0.0.0:47999 OCW_KIOSK_BOTS=4 scripts/ocw-kiosk.sh run &
kiosk=$!
wait_for '[ "$(runs)" -ge 1 ]' "app started with the server"
grep -q -- "--world /w/tenerife.ocwpkg --route Puerto de la Cruz - Teide" "$t/srv" || { echo "FAIL: server course: $(cat "$t/srv")"; fail=1; }
grep -q -- "--listen 0.0.0.0:47999" "$t/srv" && grep -q -- "--bots 4" "$t/srv" || { echo "FAIL: server options"; fail=1; }
tail -1 "$t/runs" | grep -q -- "--server 127.0.0.1:47999 --insecure-dev" || { echo "FAIL: app not a client: $(tail -1 "$t/runs")"; fail=1; }
grep -q "ab12" "$XDG_STATE_HOME/ocw/server-connect.txt" || { echo "FAIL: connect line"; fail=1; }
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
[ "$fail" -eq 0 ] && echo "kiosk tests passed" || exit 1
