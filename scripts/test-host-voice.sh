#!/usr/bin/env bash
# Ride -> Host a ride with voice, end to end (spec §7.1.6 V2, §7.1.8
# *Built (V2)*; `make test-host-voice`, on marge with livekit-server in
# ~/.local/bin):
#
#   scripts/test-host-voice.sh [LIVEKIT_SERVER]
#
# 1. The desktop app runs headless (Xvfb, .claude/skills/run-ocw/drive.sh)
#    and hosts a ride (`host start`): it starts livekit-server through the
#    shared launcher (`ocw_server::hosting`), found on PATH, as a child of
#    the app; the secret is in its environment only. UPnP is switched off
#    (`--upnp off`): no real router is asked in a test.
# 2. Two headless riders (crates/ocw-voice/tests/bundled_voice_server.rs)
#    join the hosted ride, get voice grants naming the voice URL they reach
#    and join the room; LiveKit's server API lists both.
# 3. The app ends (the shot's end): livekit-server goes with it.
# Skipped (exit 0) without a livekit-server binary. Results:
# test-output/host-voice/summary.txt (and the shot, its log).
set -uo pipefail
cd "$(dirname "$0")/.."
source ~/.cargo/env 2>/dev/null || true

lks=${1:-${LIVEKIT_SERVER:-livekit-server}}
case "$lks" in
  */*) ;;
  *) lks=$(command -v "$lks" || echo "$HOME/.local/bin/$lks") ;;
esac
if [ ! -x "$lks" ]; then
  echo "skipped: no livekit-server (scripts/fetch-livekit-server.sh)"
  exit 0
fi
if command -v clang++-21 >/dev/null 2>&1; then export CC=clang-21 CXX=clang++-21; fi
out=test-output/host-voice
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

echo "building ocw-desktop and the voice test"
cargo build -q -p ocw-desktop || exit 1
cargo test -q -p ocw-voice --features livekit --test bundled_voice_server --no-run || exit 1

free_ports() { python3 -I -c '
import socket
s=[socket.socket(socket.AF_INET, t) for t in (socket.SOCK_DGRAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_DGRAM)]
for x in s: x.bind(("0.0.0.0", 0))
print(" ".join(str(x.getsockname()[1]) for x in s))'; }
read -r game sig tcp udp < <(free_ports)
export XDG_DATA_HOME=$work/data   # the app's data: host-tls, the voice log
export PATH="$(dirname "$lks"):$PATH"
export OCW_HOST_VOICE_PORTS="$sig,$tcp,$udp"
unset OCW_LIVEKIT_URL OCW_LIVEKIT_API_KEY OCW_LIVEKIT_API_SECRET

# 1. The app hosts; the shot keeps it running for 60 s.
OCW_TEST_OUTPUT=off OCW_SIZE=1280x720 .claude/skills/run-ocw/drive.sh shot "$out/hosting.jpg" 60 -- \
  --bots 0 --cmd "host start --port $game --bots 0 --upnp off" >"$work/drive" 2>&1 &
dp=$!; pids+=("$dp")
log=$out/hosting.log
for _ in $(seq 1 1800); do
  grep -q "certificate SHA-256" "$log" 2>/dev/null && break
  kill -0 "$dp" 2>/dev/null || break
  sleep 0.1
done
grep -q "voice chat on (bundled LiveKit" "$log" 2>/dev/null; check "Host a ride starts the bundled voice server" $? "$(grep -i voice "$log" 2>/dev/null | tail -3)"
# Our voice server: the livekit-server whose config has this run's port
# (another one may run on the machine, e.g. the kiosk's).
lk=""
for p in $(pgrep -x livekit-server); do
  if tr '\0' '\n' <"/proc/$p/environ" 2>/dev/null | grep -qx "port: $sig"; then lk=$p; break; fi
done
app=$([ -n "$lk" ] && ps -o ppid= -p "$lk" | tr -d ' ')
[ -n "$lk" ] && [ "$(ps -o comm= -p "$app" 2>/dev/null)" = ocw-desktop ]; check "livekit-server is the app's child" $? "$(ps -o comm= -p "$app" 2>/dev/null)"
secret=$(tr '\0' '\n' <"/proc/$lk/environ" 2>/dev/null | sed -n 's/^LIVEKIT_KEYS=[^:]*: //p')
[ ${#secret} = 64 ]; check "the API secret is in livekit-server's environment" $?
! tr '\0' ' ' <"/proc/$lk/cmdline" 2>/dev/null | grep -qF "$secret"; check "the secret is not on its command line" $?
grep -q "TCP $sig (voice signalling)" "$log"; check "the hosting status lists the voice ports" $? "$(grep -i ports "$log" | tail -2)"
grep -q "router: open them by hand" "$log"; check "the router switch is off for this run (--upnp off)" $?
fp=$(sed -n 's/.*certificate SHA-256 \([0-9a-fA-F:]*\).*/\1/p' "$log" | head -n 1)

# 2. Two riders get grants and join the room.
OCW_VOICE_TEST_SERVER=127.0.0.1:$game OCW_VOICE_TEST_CERT=$fp OCW_VOICE_TEST_URL=ws://127.0.0.1:$sig \
  OCW_VOICE_TEST_HOLD_S=6 \
  cargo test -q -p ocw-voice --features livekit --test bundled_voice_server -- --nocapture >"$work/test" 2>&1 &
tp=$!; pids+=("$tp")
for _ in $(seq 1 600); do grep -q "both riders connected" "$work/test" && break; kill -0 "$tp" 2>/dev/null || break; sleep 0.1; done
grep -q "both riders connected" "$work/test"; check "two riders get grants naming the voice URL and connect to the room" $? "$(tail -5 "$work/test")"
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
[ "$(echo "$members" | wc -w)" = 2 ] && [[ "$members" == rider-* ]]; check "LiveKit lists both riders in the hosted session's room ($members)" $?
wait "$tp"
cp "$work/test" "$out/riders.log"
grep -c "voice token issued" "$log" | grep -qx 2; check "the hosted server issued two voice tokens" $?

# 3. The app ends with the shot: the voice server goes with it.
wait "$dp"; cp "$work/drive" "$out/drive.log"
gone=1
for _ in $(seq 1 50); do
  st=$(ps -o stat= -p "$lk" 2>/dev/null | tr -d ' ')
  if [ -z "$st" ] || [ "${st#Z}" != "$st" ]; then gone=0; break; fi
  sleep 0.1
done
check "livekit-server ends with the app" $gone
cp "$XDG_DATA_HOME/ocw/voice/livekit-$sig.log" "$out/livekit-server.log" 2>/dev/null

echo "failures: $fails" | tee -a "$summary"
[ "$fails" = 0 ]
