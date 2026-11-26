#!/usr/bin/env bash
# The workshop on an account, as a screenshot (spec 14 Phase 4, 4O): an
# accounts backend with fake sign-in (`ocw-server --auth-dev`, memory store)
# on the loopback, the desktop app signed in by a headless "browser" (curl
# following the fake providers' redirects, OCW_OPEN_URL), carbon wheels
# bought on the account and the bike painted, then a shot of the workshop
# showing the account's balance and the bought wheels on the bike.
#
#   scripts/workshop-account-shot.sh OUT.jpg [extra desktop args …]
#
# Runs on a build machine (marge, leela); drive.sh archives it.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: workshop-account-shot.sh OUT.jpg [desktop args …]}
shift
mkdir -p "$(dirname "$out")"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo build -q -p ocw-server
tmp=$(mktemp -d "${TMPDIR:-/tmp}/ocw-ws-shot.XXXXXX")
./target/debug/ocw-server --auth-dev 127.0.0.1:0 --auth-info-file "$tmp/info.json" --bots 0 \
  >"$tmp/server.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true; rm -rf "$tmp"' EXIT
for _ in $(seq 100); do [ -s "$tmp/info.json" ] && break; sleep 0.2; done
issuer=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["issuer"])' "$tmp/info.json")
echo "accounts backend (fake providers): $issuer"
# The sign-in and the account's files in the throw-away folder, never in
# this machine's keyring or data directory.
export OCW_ACCOUNT_FILE="$tmp/account.json"
export OCW_DATA_DIR="$tmp/data"
export OCW_OPEN_URL="curl -sL -o /dev/null --max-time 30"
export OCW_SIZE=${OCW_SIZE:-1280x720}
.claude/skills/run-ocw/drive.sh shot-wayland "$out" 24 -- \
  --account-server "$issuer" --virtual --bots 0 \
  --at "1=account signin google" \
  --at "2=character --body b --shape slim --age 30s --hair 3 --hair-colour #B5651D" \
  --at "3=kit.design --pattern chevrons --jersey #2E7D32 --second #FFD54F --number 27" \
  --at "8=workshop.buy wheels.road-carbon.mid" \
  --at "11=workshop.paint #C62828" \
  --at "14=workshop open" \
  "$@"
grep -E 'workshop|purchase|account' "$tmp/server.log" | tail -5 || true
