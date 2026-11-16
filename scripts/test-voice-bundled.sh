#!/usr/bin/env bash
# The bundled voice server end to end (spec §7.1.5, milestone V1 of §7.1.6;
# `make test-voice-bundled`, on marge with livekit-server in ~/.local/bin):
#
#   scripts/test-voice-bundled.sh [LIVEKIT_SERVER]
#
# 1. `ocw-server --listen 127.0.0.1:PORT --voice-server LIVEKIT_SERVER` (no
#    OCW_LIVEKIT_* variable) starts livekit-server by itself; the API
#    secret is in its environment, never on its command line or in the logs.
# 2. Two headless riders (crates/ocw-voice/tests/bundled_voice_server.rs)
#    join the game, get voice grants from it and join the room; one hears
#    the other.
# 3. Killing ocw-server with SIGKILL (no clean-up runs) ends livekit-server
#    too (Linux: the parent-death signal).
# 4. --voice off and --livekit-url start no voice server.
# Skipped (exit 0) without a livekit-server binary. Voice builds need clang
# >= 21 on Linux (CC=clang-21 CXX=clang++-21, set here when installed).
# Results: test-output/voice-bundled/summary.txt.
set -uo pipefail
cd "$(dirname "$0")/.."
source ~/.cargo/env 2>/dev/null || true

lks=${1:-${LIVEKIT_SERVER:-livekit-server}}
# A name is looked up on PATH, then in ~/.local/bin (marge).
case "$lks" in
  */*) ;;
  *) lks=$(command -v "$lks" || echo "$HOME/.local/bin/$lks") ;;
esac
if [ ! -x "$lks" ]; then
  echo "skipped: no livekit-server (scripts/fetch-livekit-server.sh)"
  exit 0
fi
if command -v clang++-21 >/dev/null 2>&1; then export CC=clang-21 CXX=clang++-21; fi
out=test-output/voice-bundled
mkdir -p "$out"
work=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill -9 "$p" 2>/dev/null; done; rm -rf "$work"; }
trap cleanup EXIT
fails=0
summary=$out/summary.txt
: >"$summary"
check() { # NAME OK(0/1) [DETAIL]
  if [ "$2" = 0 ]; then echo "PASS $1" | tee -a "$summary"
  else echo "FAIL $1 ${3:-}" | tee -a "$summary"; fails=$((fails + 1)); fi
}

echo "building ocw-server and the voice test"
cargo build -q -p ocw-server || exit 1
cargo test -q -p ocw-voice --features livekit --test bundled_voice_server --no-run || exit 1
server=target/debug/ocw-server

free_ports() { python3 -I -c '
import socket
s=[socket.socket(socket.AF_INET, t) for t in (socket.SOCK_DGRAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_DGRAM)]
for x in s: x.bind(("0.0.0.0", 0))
print(" ".join(str(x.getsockname()[1]) for x in s))'; }
read -r game sig tcp udp < <(free_ports)
export XDG_DATA_HOME=$work/data   # the voice server's log and pid file
unset OCW_LIVEKIT_URL OCW_LIVEKIT_API_KEY OCW_LIVEKIT_API_SECRET

# 1. The game server starts its voice server.
"$server" --listen "127.0.0.1:$game" --bots 0 --voice-server "$lks" \
  --voice-ports "$sig,$tcp,$udp" >"$work/out" 2>"$work/err" &
gs=$!; pids+=("$gs")
for _ in $(seq 1 600); do grep -q "Clients connect with" "$work/out" && break; kill -0 "$gs" 2>/dev/null || break; sleep 0.1; done
# ocw-server logs to stdout.
grep -q "voice chat on (bundled LiveKit" "$work/out"; check "ocw-server --listen starts the bundled voice server" $? "$(tail -3 "$work/out")"
lk=$(pgrep -P "$gs" | head -n 1)
[ -n "$lk" ] && [ "$(ps -o comm= -p "$lk")" = livekit-server ]; check "livekit-server is the game server's child" $?
secret=$(tr '\0' '\n' <"/proc/$lk/environ" 2>/dev/null | sed -n 's/^LIVEKIT_KEYS=[^:]*: //p')
[ ${#secret} = 64 ]; check "the API secret is in livekit-server's environment" $?
! tr '\0' ' ' <"/proc/$lk/cmdline" | grep -qF "$secret"; check "the secret is not on livekit-server's command line" $?
! grep -qF "$secret" "$work/out" "$work/err" "$XDG_DATA_HOME"/ocw/voice/*.log; check "the secret is in no log" $?
[ -f "$XDG_DATA_HOME/ocw/voice/livekit-$sig.pid" ]; check "the pid file is written" $?
fp=$(sed -n 's/.*--server-cert \([0-9a-fA-F:]*\).*/\1/p' "$work/out" | head -n 1)

# 2. Two riders get grants and join the room.
OCW_VOICE_TEST_SERVER=127.0.0.1:$game OCW_VOICE_TEST_CERT=$fp OCW_VOICE_TEST_URL=ws://127.0.0.1:$sig \
  OCW_VOICE_TEST_HOLD_S=6 \
  cargo test -q -p ocw-voice --features livekit --test bundled_voice_server -- --nocapture >"$work/test" 2>&1 &
tp=$!; pids+=("$tp")
for _ in $(seq 1 600); do grep -q "both riders connected" "$work/test" && break; kill -0 "$tp" 2>/dev/null || break; sleep 0.1; done
grep -q "both riders connected" "$work/test"; check "two riders get grants naming the voice URL and connect to the room" $? "$(tail -5 "$work/test")"
# Who is in the room, asked from LiveKit's server API with this start's keys.
room=$(sed -n 's/^grants: .* for room \([^ ]*\) at .*/\1/p' "$work/test" | head -n 1)
key=$(tr '\0' '\n' <"/proc/$lk/environ" 2>/dev/null | sed -n 's/^LIVEKIT_KEYS=\([^:]*\): .*/\1/p')
members=$(OCW_K="$key" OCW_S="$secret" OCW_R="$room" OCW_P="$sig" python3 -I -c '
import base64, hashlib, hmac, json, os, time, urllib.request
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=").decode()
now = int(time.time())
h = b(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
p = b(json.dumps({"iss": os.environ["OCW_K"], "sub": "ocw-test", "nbf": now - 5, "exp": now + 60,
                  "video": {"room": os.environ["OCW_R"], "roomAdmin": True}}).encode())
t = h + "." + p + "." + b(hmac.new(os.environ["OCW_S"].encode(), (h + "." + p).encode(), hashlib.sha256).digest())
req = urllib.request.Request("http://127.0.0.1:%s/twirp/livekit.RoomService/ListParticipants" % os.environ["OCW_P"],
                             data=json.dumps({"room": os.environ["OCW_R"]}).encode(),
                             headers={"Authorization": "Bearer " + t, "Content-Type": "application/json"})
print(" ".join(sorted(x["identity"] for x in json.load(urllib.request.urlopen(req, timeout=5)).get("participants", []))))' 2>&1)
[ "$members" = "rider-1 rider-2" ]; check "LiveKit lists both riders in the session room ($members)" $?
wait "$tp"; r=$?
cp "$work/test" "$out/riders.log"
cat "$work/out" "$work/err" >"$out/ocw-server.log"
cp "$XDG_DATA_HOME/ocw/voice/livekit-$sig.log" "$out/livekit-server.log" 2>/dev/null
if grep -q "^KNOWN:" "$work/test"; then
  echo "KNOWN the Rust voice backend did not report the other rider (spec 7.1.8 Open); hearing not checked" | tee -a "$summary"
else
  grep -q "Bea hears Ada" "$work/test" && [ $r = 0 ]; check "one rider hears the other's tone" $? "$(tail -5 "$work/test")"
fi
grep -c "voice token issued" "$work/out" | grep -qx 2; check "the game server issued two voice tokens" $?

# 3. No orphan: SIGKILL the game server, livekit-server goes too.
kill -9 "$gs"; wait "$gs" 2>/dev/null
gone=1
for _ in $(seq 1 50); do
  st=$(ps -o stat= -p "$lk" 2>/dev/null | tr -d ' ')
  if [ -z "$st" ] || [ "${st#Z}" != "$st" ]; then gone=0; break; fi
  sleep 0.1
done
check "livekit-server ends when ocw-server is killed (SIGKILL)" $gone

# 4. --voice off, --livekit-url: no voice server of its own.
for mode in off external; do
  read -r game2 _ _ _ < <(free_ports)
  if [ $mode = off ]; then extra=(--voice off); else extra=(--livekit-url ws://127.0.0.1:9 --livekit-api-key devkey); fi
  OCW_LIVEKIT_API_SECRET=$([ $mode = external ] && echo "test-only-secret") \
    "$server" --listen "127.0.0.1:$game2" --bots 0 --voice-server "$lks" --voice-ports "$sig,$tcp,$udp" \
    "${extra[@]}" >"$work/out2" 2>"$work/err2" &
  p=$!; pids+=("$p")
  for _ in $(seq 1 300); do grep -q "Clients connect with" "$work/out2" && break; sleep 0.1; done
  sleep 0.5
  child=$(pgrep -P "$p" | head -n 1)
  if [ $mode = off ]; then want="voice chat off (--voice off)"; else want="voice chat on (external LiveKit)"; fi
  grep -qF "$want" "$work/out2" && [ -z "$child" ]; check "no voice server of its own with $([ $mode = off ] && echo "--voice off" || echo "--livekit-url")" $? "$(grep -i voice "$work/out2" | tail -2)"
  kill "$p" 2>/dev/null; wait "$p" 2>/dev/null
done

echo "failures: $fails" | tee -a "$summary"
[ "$fails" = 0 ]
