#!/usr/bin/env bash
# Screenshots of the sign-in (spec §9.1) against the fake providers:
#
#   scripts/test-account-pages.sh [OUT_DIR]      (default test-output/account)
#
# Starts `ocw-server --auth-dev` (the accounts backend with in-process fake
# Google and Strava), then the desktop app headlessly (run-ocw's drive.sh on
# X11) three times:
#
#   OUT/account-signed-out.jpg   Settings -> Account, not signed in
#   OUT/account-signed-in.jpg    after `account signin google` (the "system
#                                browser" is curl following the redirects:
#                                OCW_BROWSER), Google linked
#   OUT/account-code.jpg         `account signin --code`: the kiosk's code
#                                and URL on the Account page
#
# and the backend's code page on the phone in headless Chromium:
#
#   OUT/device-page.png          /device?user_code=… (confirm with Google or
#                                Strava, CSRF-protected form)
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
out=${1:-test-output/account}
mkdir -p "$out"
cargo build -q -p ocw-server
info=$(mktemp)
./target/debug/ocw-server --auth-dev 127.0.0.1:0 --auth-info-file "$info" --bots 0 > "$out/server.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true; rm -f "$info" "${OCW_ACCOUNT_FILE:-}"' EXIT
for _ in $(seq 100); do [ -s "$info" ] && break; sleep 0.2; done
issuer=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["issuer"])' "$info")
echo "accounts backend (fake providers): $issuer"
drive=.claude/skills/run-ocw/drive.sh
menu=(--at "7=menu open" --at "7.5=menu down" --at "7.7=menu down" --at "7.9=menu down"
      --at "8.1=menu down" --at "8.3=menu down" --at "8.6=menu select" --at "9.2=menu select")
export OCW_SIZE=1280x720
# The sign-in in a throwaway file, never in this machine's keyring.
export OCW_ACCOUNT_FILE="$(mktemp -u)-ocw-account.json"
"$drive" shot "$out/account-signed-out.jpg" 12 -- --account-server "$issuer" --bots 2 "${menu[@]}"
OCW_BROWSER="curl -sL -o /dev/null --max-time 30" \
  "$drive" shot "$out/account-signed-in.jpg" 12 -- --account-server "$issuer" --bots 2 \
  --cmd "account signin google" "${menu[@]}"
rm -f "$OCW_ACCOUNT_FILE"
"$drive" shot "$out/account-code.jpg" 12 -- --account-server "$issuer" --bots 2 \
  --cmd "account signin --code" "${menu[@]}"
# The phone's side: a fresh code, its confirmation page.
uri=$(curl -s -d client_id=ocw-kiosk -d scope=openid "$issuer/device_authorization" |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["verification_uri_complete"])')
chromium --headless=new --no-sandbox --hide-scrollbars --window-size=480,860 \
  --screenshot="$PWD/$out/device-page.png" "$uri" > /dev/null 2>&1 || true
ls -l "$out"
