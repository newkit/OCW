#!/usr/bin/env bash
# Outside test of a hosted ocw-server (spec 11.5; the Fly.io server), run on
# a build machine that is NOT the server (marge):
#
#   OCW_REMOTE_HOST=IPV4 OCW_REMOTE_CERT=FINGERPRINT OCW_REMOTE_WEB='server=…&cert-hash=…' \
#   OCW_VOICE_KEYS_FILE=FILE OCW_VOICE_ROOM=ROOM OCW_ECHO_RIDER=ID \
#     scripts/test-remote-server.sh [SECONDS]
#
# 1. With OCW_REMOTE_BOTS=N: N ocw-bot riders join over QUIC and ride (only
#    for a server on a test route or route file; bots ride no road graph,
#    so they are refused by a world server).
# 2. Optional, only with OCW_VOICE_KEYS_FILE (prefer the keyless check of
#    5: two browser riders hear each other): the echo participant
#    (crates/ocw-voice/examples/echo_participant.rs) joins the server's voice room as its Echo rider over the public voice
#    path (ws://HOST:7880, media on UDP 7882 / TCP 7881). It needs this
#    start's LiveKit keys: OCW_VOICE_KEYS_FILE holds "KEY SECRET" on one
#    line (read from the server machine straight into the file, mode 600;
#    deleted here at the end; never printed).
# 3. The desktop app with the `voice` feature (Xvfb, drive.sh) joins with
#    its microphone on, rides a minute and takes a screenshot; it must
#    connect to the voice room, and the echo participant must hear it.
# 4. Only with the keys too: LiveKit's room listing (its API on TCP 7880)
#    shows the app's rider active with its microphone track.
# 5. With OCW_REMOTE_WEB: a browser rider (scripts/test-remote-web.py)
#    joins over WebTransport, streams the world from the local dev server,
#    connects to the voice room and hears the others. OCW_REMOTE_HTTPS=1
#    serves that page over https:// (as the public preview is): its voice
#    grant must then be wss:// (the server's --voice-public-url).
#    OCW_REMOTE_WEB2=1 adds a second browser rider: each plays a voice-like
#    tone from its fake microphone, so both must see the other in the room
#    and hear it speak, over the server's grants and the real SFU path
#    (no server keys needed). The desktop app's voice is judged from its
#    own log (connected to the room).
#
# The riders get different looks (riders.toml of their own config folder).
# Results: test-output/remote-server/<time>/ (summary.txt, logs, pictures).
set -u
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
host=${OCW_REMOTE_HOST:?OCW_REMOTE_HOST: the public IPv4 of the server}
cert=${OCW_REMOTE_CERT:?OCW_REMOTE_CERT: its QUIC certificate fingerprint}
seconds=${1:-75}
game_port=${OCW_REMOTE_PORT:-47800}
world=${OCW_REMOTE_WORLD:-$HOME/src/OCW/data/pilot/liechtenstein.ocwpkg}
stamp=$(date +%Y%m%d-%H%M%S)
out=test-output/remote-server/$stamp
mkdir -p "$out"
work=$(mktemp -d)
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
  sleep 0.5
  for p in "${pids[@]}"; do kill -9 "$p" 2>/dev/null; done
  [ -n "${OCW_VOICE_KEYS_FILE:-}" ] && rm -f "$OCW_VOICE_KEYS_FILE"
  rm -rf "$work"
}
trap cleanup EXIT
fails=0
summary=$out/summary.txt
: >"$summary"
check() { # NAME OK(0/1) [DETAIL]
  if [ "$2" = 0 ]; then echo "PASS $1" | tee -a "$summary"
  else echo "FAIL $1 ${3:-}" | tee -a "$summary"; fails=$((fails + 1)); fi
}
plain() { sed 's/\x1b\[[0-9;]*m//g' "$@"; }

key='' secret=''
if [ -n "${OCW_VOICE_KEYS_FILE:-}" ] && [ -s "$OCW_VOICE_KEYS_FILE" ]; then
  read -r key secret <"$OCW_VOICE_KEYS_FILE"
fi

if command -v clang++-21 >/dev/null 2>&1; then export CC=clang-21 CXX=clang++-21; fi
echo "building ocw-bot, the echo participant and ocw-desktop (voice)"
cargo build -q --release -p ocw-bot || exit 1
cargo build -q -p ocw-voice --features livekit --example echo_participant || exit 1
cargo build -q -p ocw-desktop --features voice || exit 1
echo "server $host, $seconds s" | tee -a "$summary"

# 1. Bot riders over QUIC (only for a server on a test route).
if [ "${OCW_REMOTE_BOTS:-0}" -gt 0 ]; then
  target/release/ocw-bot --server "$host:$game_port" --server-cert "$cert" --riders "$OCW_REMOTE_BOTS" \
    --seconds "$((seconds + 40))" --name-prefix "Ada Rider" --power 210 --voice-nearby \
    --follow "Cleo Climber" --report "$work/bots.json" >"$work/bots" 2>&1 &
  pids+=("$!")
fi

# 2. The echo participant as the Echo rider.
if [ -n "$key" ] && [ -n "${OCW_VOICE_ROOM:-}" ] && [ -n "${OCW_ECHO_RIDER:-}" ]; then
  OCW_LIVEKIT_URL="ws://$host:7880" OCW_LIVEKIT_API_KEY="$key" OCW_LIVEKIT_API_SECRET="$secret" \
    target/debug/examples/echo_participant "$OCW_VOICE_ROOM" "$OCW_ECHO_RIDER" 300 >"$work/echo" 2>&1 &
  pids+=("$!")
fi

# 3. The desktop app, its own look (a road bike, orange kit).
cfg=$work/desktop-config
mkdir -p "$cfg"
cat >"$cfg/riders.toml" <<'EOF'
active = "Cleo Climber"

[[rider]]
name = "Cleo Climber"
weight_kg = 64.0
ftp_w = 250
bike = "road"
frame_color = "#E65100"
jersey_color = "#FB8C00"
helmet_color = "#212121"
kit = "sash"
EOF
(OCW_CONFIG_DIR=$cfg OCW_TEST_OUTPUT=off OCW_SIZE=1600x900 \
   OCW_APP=target/debug/ocw-desktop .claude/skills/run-ocw/drive.sh shot "$out/desktop.jpg" "$seconds" -- \
   --server "$host:$game_port" --server-cert "$cert" --world "$world" --name "Cleo Climber" \
   --settings-file "$work/settings.toml" --cmd "set voice.my_voice everybody" --cmd "voice.mute-mic off" \
   --cmd "character --body a --age 30s --skin 2 --hair 2 --hair-colour blond" \
   --cmd "voice.status" >"$work/drive" 2>&1) &
dr=$!; pids+=("$dr")

# 5. The browser rider at the same time.
if [ -n "${OCW_REMOTE_WEB:-}" ]; then
  python3 scripts/test-remote-web.py --query "$OCW_REMOTE_WEB" --out "$out" \
    --seconds "$((seconds - 20))" --name "Dora Descender" ${OCW_REMOTE_HTTPS:+--https} >"$work/web" 2>&1 &
  web=$!; pids+=("$web")
fi
if [ -n "${OCW_REMOTE_WEB:-}" ] && [ -n "${OCW_REMOTE_WEB2:-}" ]; then
  sleep 5
  python3 scripts/test-remote-web.py --query "$OCW_REMOTE_WEB" --out "$out" \
    --seconds "$((seconds - 25))" --name "Erin Sprinter" ${OCW_REMOTE_HTTPS:+--https} >"$work/web2" 2>&1 &
  web2=$!; pids+=("$web2")
fi

connected=1
for _ in $(seq 1 1800); do
  plain "$out/desktop.log" 2>/dev/null | grep -q "voice: connected to" && { connected=0; break; }
  kill -0 "$dr" 2>/dev/null || break; sleep 0.1
done
check "the desktop app joins the game and connects to the voice room" $connected \
  "$(plain "$out/desktop.log" 2>/dev/null | grep -iE "voice|reject|connect" | tail -4)"
room=$(plain "$out/desktop.log" 2>/dev/null | sed -n 's/.*voice: connected to \([^ ]*\).*/\1/p' | head -n 1)
echo "voice room: $room" >>"$summary"
if [ -n "$key" ]; then
  heard=1
  for _ in $(seq 1 600); do grep -q "hearing Cleo Climber (rider-" "$work/echo" 2>/dev/null && { heard=0; break; }; sleep 0.1; done
  check "the echo participant hears the desktop rider over the public voice path" $heard "$(tail -3 "$work/echo" 2>/dev/null)"
  # 4. LiveKit's own view of the room.
  OCW_K="$key" OCW_S="$secret" OCW_H="$host" OCW_ROOM="$room" python3 -I -c '
import base64, hashlib, hmac, json, os, time, urllib.request
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=").decode()
now = int(time.time())
h = b(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
p = b(json.dumps({"iss": os.environ["OCW_K"], "sub": "ocw-test", "nbf": now - 5, "exp": now + 60,
                  "video": {"roomList": True, "roomAdmin": True, "room": os.environ["OCW_ROOM"]}}).encode())
t = h + "." + p + "." + b(hmac.new(os.environ["OCW_S"].encode(), (h + "." + p).encode(), hashlib.sha256).digest())
req = urllib.request.Request("http://%s:7880/twirp/livekit.RoomService/ListParticipants" % os.environ["OCW_H"],
                             data=json.dumps({"room": os.environ["OCW_ROOM"]}).encode(),
                             headers={"Authorization": "Bearer " + t, "Content-Type": "application/json"})
ps = json.load(urllib.request.urlopen(req, timeout=10)).get("participants", [])
for x in ps:
    print(x.get("identity"), repr(x.get("name")), x.get("state", "JOINING"),
          [(t.get("source"), "muted" if t.get("muted") else "on") for t in x.get("tracks", [])])
app = [x for x in ps if x.get("name") == "Cleo Climber"]
ok = app and app[0].get("state") == "ACTIVE" and any(t.get("source") == "MICROPHONE" for t in app[0].get("tracks", []))
raise SystemExit(0 if ok else 1)' >"$work/listing" 2>&1
  check "LiveKit lists the desktop rider, active, with its microphone track" $? "$(cat "$work/listing")"
  cat "$work/listing" >>"$summary"
fi

wait "$dr" 2>/dev/null
[ -f "$out/desktop.jpg" ]; check "the desktop app rode $seconds s and took its screenshot" $?
if [ -n "${OCW_REMOTE_WEB:-}" ]; then
  wait "$web"; check "the browser rider joined, rode and heard the voice room (test-remote-web.py)" $? "$(tail -5 "$work/web")"
fi
if [ -n "${web2:-}" ]; then
  wait "$web2"; check "the second browser rider joined, rode and heard the voice room" $? "$(tail -5 "$work/web2")"
fi
if [ "${OCW_REMOTE_BOTS:-0}" -gt 0 ]; then
  [ -s "$work/bots.json" ] && ! grep -qiE "rejected|FAIL" "$work/bots"; check "the bot riders joined over QUIC and rode" $? "$(tail -3 "$work/bots")"
fi
for f in bots echo web web2 drive listing; do [ -f "$work/$f" ] && cp "$work/$f" "$out/$f.log"; done
if [ -n "$secret" ]; then
  ! grep -rqF "$secret" "$out"; check "the voice secret is in no log" $?
fi
echo "failures: $fails" | tee -a "$summary"
[ "$fails" = 0 ]
