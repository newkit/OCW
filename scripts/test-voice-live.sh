#!/usr/bin/env bash
# Live voice with the real desktop app as a rider (spec §7.1.8; `make
# test-voice-live`, on marge with livekit-server in ~/.local/bin):
#
#   scripts/test-voice-live.sh [LIVEKIT_SERVER]
#
# 1. `ocw-server --listen 127.0.0.1:PORT --echo-bot` starts the bundled
#    livekit-server by itself (no OCW_LIVEKIT_* variable).
# 2. A headless `ocw-bot` rider joins the game, so the session has two human
#    riders and the app's voice joins by itself (§2.4).
# 3. The echo participant (crates/ocw-voice/examples/echo_participant.rs)
#    joins the room as the session's Echo rider, with this start's keys
#    (read from livekit-server's environment, as test-voice-bundled does).
# 4. The desktop app with the `voice` feature (debug build, Xvfb via
#    .claude/skills/run-ocw/drive.sh, the real LiveKit backend; fresh
#    settings, microphone switched on) joins the game server with
#    --server: it gets a grant, connects to the room and publishes its
#    microphone; LiveKit lists it with a microphone track, and the echo
#    participant hears it.
# The other paths have their own tests: two headless riders on the bundled
# server (test-voice-bundled), the app hosting (test-host-voice), two
# browsers (test-web-livekit). Skipped (exit 0) without a livekit-server
# binary. Needs clang >= 21 on Linux for the voice build. Results:
# test-output/voice-live/summary.txt, the logs and the app's screenshot.
set -u
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
lks=${1:-livekit-server}
case "$lks" in
  */*) ;;
  *) lks=$(command -v "$lks" || echo "$HOME/.local/bin/$lks") ;;
esac
if [ ! -x "$lks" ]; then
  echo "skipped: no livekit-server (scripts/fetch-livekit-server.sh)"
  exit 0
fi
if command -v clang++-21 >/dev/null 2>&1; then export CC=clang-21 CXX=clang++-21; fi
out=test-output/voice-live
mkdir -p "$out"
work=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; sleep 0.5
            for p in "${pids[@]}"; do kill -9 "$p" 2>/dev/null; done; rm -rf "$work"; }
trap cleanup EXIT
fails=0
summary=$out/summary.txt
: >"$summary"
check() { # NAME OK(0/1) [DETAIL]
  if [ "$2" = 0 ]; then echo "PASS $1" | tee -a "$summary"
  else echo "FAIL $1 ${3:-}" | tee -a "$summary"; fails=$((fails + 1)); fi
}
plain() { sed 's/\x1b\[[0-9;]*m//g' "$@"; }

echo "building ocw-server, ocw-bot, the echo participant and ocw-desktop (voice)"
cargo build -q -p ocw-server -p ocw-bot || exit 1
cargo build -q -p ocw-voice --features livekit --example echo_participant || exit 1
cargo build -q -p ocw-desktop --features voice || exit 1

free_ports() { python3 -I -c '
import socket
s=[socket.socket(socket.AF_INET, t) for t in (socket.SOCK_DGRAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_DGRAM)]
for x in s: x.bind(("0.0.0.0", 0))
print(" ".join(str(x.getsockname()[1]) for x in s))'; }
read -r game sig tcp udp < <(free_ports)
export XDG_DATA_HOME=$work/data   # the voice server's log and pid file
unset OCW_LIVEKIT_URL OCW_LIVEKIT_API_KEY OCW_LIVEKIT_API_SECRET

# 1. The game server with its own voice server and the Echo rider.
target/debug/ocw-server --listen "127.0.0.1:$game" --bots 0 --test-route hills --echo-bot \
  --voice-server "$lks" --voice-ports "$sig,$tcp,$udp" >"$work/server" 2>&1 &
gs=$!; pids+=("$gs")
for _ in $(seq 1 600); do grep -q "Clients connect with" "$work/server" && break; kill -0 "$gs" 2>/dev/null || break; sleep 0.1; done
grep -q "voice chat on (bundled LiveKit" "$work/server"; check "ocw-server starts the bundled voice server" $? "$(tail -3 "$work/server")"
fp=$(sed -n 's/.*--server-cert \([0-9a-fA-F:]*\).*/\1/p' "$work/server" | head -n 1)
echo_id=$(plain "$work/server" | sed -n 's/.*echo rider at the start.*identity=rider-\([0-9]*\).*/\1/p' | head -n 1)
lk=$(pgrep -P "$gs" | head -n 1)
key=$(tr '\0' '\n' <"/proc/$lk/environ" 2>/dev/null | sed -n 's/^LIVEKIT_KEYS=\([^:]*\): .*/\1/p')
secret=$(tr '\0' '\n' <"/proc/$lk/environ" 2>/dev/null | sed -n 's/^LIVEKIT_KEYS=[^:]*: //p')
[ -n "$echo_id" ] && [ -n "$key" ]; check "the Echo rider and this start's keys are known" $?

# LiveKit's server API with this start's keys: METHOD JSON → JSON.
lk_api() {
  OCW_K="$key" OCW_S="$secret" OCW_P="$sig" python3 -I -c '
import base64, hashlib, hmac, json, os, sys, time, urllib.request
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=").decode()
now = int(time.time())
h = b(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
p = b(json.dumps({"iss": os.environ["OCW_K"], "sub": "ocw-test", "nbf": now - 5, "exp": now + 60,
                  "video": {"roomList": True, "roomAdmin": True,
                            "room": json.loads(sys.argv[2]).get("room", "")}}).encode())
t = h + "." + p + "." + b(hmac.new(os.environ["OCW_S"].encode(), (h + "." + p).encode(), hashlib.sha256).digest())
req = urllib.request.Request("http://127.0.0.1:%s/twirp/livekit.RoomService/%s" % (os.environ["OCW_P"], sys.argv[1]),
                             data=sys.argv[2].encode(),
                             headers={"Authorization": "Bearer " + t, "Content-Type": "application/json"})
print(json.dumps(json.load(urllib.request.urlopen(req, timeout=5))))' "$1" "$2" 2>&1
}

# 2. A second human rider (a bot client), so the app's voice joins.
target/debug/ocw-bot --server "127.0.0.1:$game" --server-cert "$fp" --riders 1 --seconds 90 \
  --route hills --name-prefix "Dora Descender" >"$work/bot" 2>&1 &
pids+=("$!")

# 3. The desktop app (headless, real LiveKit backend), microphone on (rides
#    are joined muted, spec §7.1.7 Q7) and heard by all nearby (the default
#    is no one, Q8).
(OCW_APP=target/debug/ocw-desktop OCW_TEST_OUTPUT=off .claude/skills/run-ocw/drive.sh shot "$out/desktop.jpg" 40 -- \
   --server "127.0.0.1:$game" --server-cert "$fp" --route hills --name "Cleo Climber" \
   --settings-file "$work/settings.toml" --cmd "set voice.heard_by nearby" --cmd "voice.mute-mic off" --cmd "voice.status" >"$work/drive" 2>&1) &
dr=$!; pids+=("$dr")
connected=1
for _ in $(seq 1 1200); do
  plain "$out/desktop.log" 2>/dev/null | grep -q "voice: connected to" && { connected=0; break; }
  kill -0 "$dr" 2>/dev/null || break; sleep 0.1
done
check "the app gets a grant and connects to the voice room" $connected "$(plain "$out/desktop.log" 2>/dev/null | grep -i voice | tail -3)"
room=$(plain "$out/desktop.log" 2>/dev/null | sed -n 's/.*voice: connected to \([^ ]*\).*/\1/p' | head -n 1)

# 4. The echo participant joins as the Echo rider and must hear the app.
if [ -n "$room" ]; then
  OCW_LIVEKIT_URL="ws://127.0.0.1:$sig" OCW_LIVEKIT_API_KEY="$key" OCW_LIVEKIT_API_SECRET="$secret" \
    target/debug/examples/echo_participant "$room" "$echo_id" 300 >"$work/echo" 2>&1 &
  pids+=("$!")
fi
heard=1
for _ in $(seq 1 300); do grep -q "hearing Cleo Climber (rider-" "$work/echo" 2>/dev/null && { heard=0; break; }; sleep 0.1; done
check "the echo participant hears the app's rider" $heard "$(tail -3 "$work/echo" 2>/dev/null)"
sleep 2
listing=$(lk_api ListParticipants "{\"room\": \"$room\"}")
OCW_L="$listing" python3 -I -c '
import json, os
ps = json.loads(os.environ["OCW_L"]).get("participants", [])
for p in ps:
    print(p.get("identity"), repr(p.get("name")), p.get("state", "JOINING"),
          [(t.get("source"), "muted" if t.get("muted") else "on") for t in p.get("tracks", [])])
app = [p for p in ps if p.get("name") == "Cleo Climber"]
ok = app and app[0].get("state") == "ACTIVE" and any(t.get("source") == "MICROPHONE" and not t.get("muted") for t in app[0].get("tracks", []))
raise SystemExit(0 if ok else 1)' >"$work/listing" 2>&1
check "LiveKit lists the app's rider, active, with its microphone on" $? "$(cat "$work/listing")"
cat "$work/listing" | tee -a "$summary"

wait "$dr" 2>/dev/null
cp "$work/server" "$out/ocw-server.log"; cp "$work/bot" "$out/bot.log"; cp "$work/echo" "$out/echo.log" 2>/dev/null
cp "$XDG_DATA_HOME"/ocw/voice/livekit-*.log "$out/livekit-server.log" 2>/dev/null
! grep -qF "$secret" "$out"/*.log; check "the secret is in no log" $?

echo "failures: $fails" | tee -a "$summary"
[ "$fails" = 0 ]
