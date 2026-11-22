#!/usr/bin/env bash
# Screenshot of the "nobody hears you" notice (spec 7.1.7 Q22 (b)) on
# Wayland: ocw-server with its bundled livekit-server, one ocw-bot rider (so
# the app's voice joins), the desktop app (voice build) with My voice:
# Selected and nobody selected, turned on by the console – the HUD shows
# the notice. On marge (livekit-server in ~/.local/bin). Output:
# test-output/voice-notice/.
set -u
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
lks=$(command -v livekit-server || echo "$HOME/.local/bin/livekit-server")
[ -x "$lks" ] || { echo "no livekit-server"; exit 1; }
if command -v clang++-21 >/dev/null 2>&1; then export CC=clang-21 CXX=clang++-21; fi
out=test-output/voice-notice
mkdir -p "$out"
work=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; sleep 0.5
            for p in "${pids[@]}"; do kill -9 "$p" 2>/dev/null; done; rm -rf "$work"; }
trap cleanup EXIT
cargo build -q -p ocw-server -p ocw-bot || exit 1
cargo build -q -p ocw-desktop --features voice || exit 1
read -r game sig tcp udp < <(python3 -I -c '
import socket
s=[socket.socket(socket.AF_INET, t) for t in (socket.SOCK_DGRAM, socket.SOCK_STREAM, socket.SOCK_STREAM, socket.SOCK_DGRAM)]
for x in s: x.bind(("0.0.0.0", 0))
print(" ".join(str(x.getsockname()[1]) for x in s))')
export XDG_DATA_HOME=$work/data
unset OCW_LIVEKIT_URL OCW_LIVEKIT_API_KEY OCW_LIVEKIT_API_SECRET
target/debug/ocw-server --listen "127.0.0.1:$game" --bots 0 --test-route hills \
  --voice-server "$lks" --voice-ports "$sig,$tcp,$udp" >"$work/server" 2>&1 &
gs=$!; pids+=("$gs")
for _ in $(seq 1 600); do grep -q "Clients connect with" "$work/server" && break; kill -0 "$gs" 2>/dev/null || break; sleep 0.1; done
fp=$(sed -n 's/.*--server-cert \([0-9a-fA-F:]*\).*/\1/p' "$work/server" | head -n 1)
target/debug/ocw-bot --server "127.0.0.1:$game" --server-cert "$fp" --riders 1 --seconds 120 \
  --route hills --name-prefix "Dora Descender" >"$work/bot" 2>&1 &
pids+=("$!")
OCW_APP=target/debug/ocw-desktop OCW_TEST_OUTPUT=off .claude/skills/run-ocw/drive.sh shot-wayland "$out/${1:-nobody-hears-you}.jpg" "${2:-30}" -- \
  --server "127.0.0.1:$game" --server-cert "$fp" --route hills --name "Cleo Climber" \
  --settings-file "$work/settings.toml" --cmd "set voice.first_call_notice off" \
  --at "${3:-25}=set voice.my_voice selected" --at "${3:-25}=voice.mute-mic off" --at "$(( ${3:-25} + 1 ))=voice.status"
cp "$work/server" "$out/ocw-server.log"
grep -E "voice|console" "$out/${1:-nobody-hears-you}.log" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | tail -15
