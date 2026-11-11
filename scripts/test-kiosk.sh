#!/usr/bin/env bash
# Tests ocw-kiosk.sh with a fake app (no display, GPU or build needed).
set -euo pipefail
cd "$(dirname "$0")/.."
t=$(mktemp -d)
trap 'kill $kiosk 2>/dev/null || true; rm -rf "$t"' EXIT
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

[ "$fail" -eq 0 ] && echo "kiosk tests passed" || exit 1
