#!/usr/bin/env bash
# The workshop on an account, as a screenshot (spec 14 Phase 4, 4O): an
# accounts backend with fake sign-in (`ocw-server --auth-dev`, memory store)
# on the loopback, the desktop app signed in by a headless "browser" (curl
# following the fake providers' redirects), a bike bought on the account,
# then a shot of the workshop showing the account's balance and the bike.
#
#   scripts/workshop-account-shot.sh OUT.jpg [extra --at / --cmd args …]
#
# Runs on a build machine (marge); archived by drive.sh into test-output/.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: workshop-account-shot.sh OUT.jpg [desktop args …]}
shift
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo build -q -p ocw-server
tmp=$(mktemp -d "${TMPDIR:-/tmp}/ocw-ws-shot.XXXXXX")
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
target/debug/ocw-server --auth-dev "127.0.0.1:$port" --bots 0 >"$tmp/server.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true; rm -rf "$tmp"' EXIT
for _ in $(seq 100); do
  curl -fs "http://127.0.0.1:$port/health" >/dev/null 2>&1 && break
  sleep 0.2
done
export OCW_ACCOUNT_SERVER="http://127.0.0.1:$port"
export OCW_ACCOUNT_FILE="$tmp/account.json"
export OCW_DATA_DIR="$tmp/data"
export OCW_OPEN_URL="curl -sSL -o /dev/null -c $tmp/jar -b $tmp/jar"
.claude/skills/run-ocw/drive.sh shot-wayland "$out" 26 -- \
  --virtual \
  --at "1=account signin google" \
  --at "2=character --build strong --hair-colour #B5651D" \
  --at "3=kit.design --pattern chevrons --jersey #2E7D32 --second #FFD54F --number 27" \
  --at "9=bikes.buy frame.road.mid --with wheels.road-carbon.mid --with tyres.road.race" \
  --at "13=bikes.select road-1" \
  --at "16=workshop open" \
  "$@"
grep -E 'workshop|account' "$tmp/server.log" | tail -5 || true
