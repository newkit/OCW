#!/usr/bin/env bash
# Multiplayer test over the real network (spec §11.4.1, `make test-multiplayer`):
# the game server (`ocw-server --listen`) and the GUI client in kiosk mode run
# on the server machine (default marge, its screen), headless bot clients
# (`ocw-bot`) on the other build machines join as remote riders; at the end
# `ocw-bot verdict` checks the server's statistics and every bot's report.
#
#   scripts/test-multiplayer.sh            (or: make test-multiplayer)
#
# Environment (all optional):
#   OCW_MP_SERVER    server machine from config/machines.toml (default marge)
#   OCW_MP_BOTS      bot machines; unreachable ones are skipped
#                    (default "marge macos Win11Builder leela")
#   OCW_MP_RIDERS    riders per bot machine (default 2)
#   OCW_MP_SECONDS   how long the server runs, s (default 180)
#   OCW_MP_PORT      UDP port (default 47800)
#   OCW_MP_KIOSK     1: switch the server machine's kiosk into the session
#                    and take screenshots (default); 0: bots only
#   OCW_MP_HOST      address the bots use for the server (default: its `host`
#                    in machines.toml)
#   OCW_MP_BUILD     0: skip the builds (binaries already there)
#   OCW_MP_CAMERA    a headless desktop client "Camera" on the server machine
#                    (weston, drive.sh, release build) that rides along and
#                    takes the screenshots and video: auto (default: when there
#                    is no kiosk, or its screen is locked and its build is
#                    too old to draw anyway), 1, 0
#   OCW_MP_VIDEO_S   length of the kiosk's and the camera's real-time 25 fps
#                    videos (app recorder, FPS counter on; default 15)
#
# The server address is read from config/machines.toml here and passed to the
# remote commands only; it is never printed, and the logs kept in test-output/
# have every IPv4 address and the host name replaced by <ip> / <server>.
# Nothing on the machines' firewalls is changed: a blocked UDP port shows up
# as a bot that gets no answer, and is reported as such.
set -uo pipefail
cd "$(dirname "$0")/.."

server_m=${OCW_MP_SERVER:-marge}
bot_ms=${OCW_MP_BOTS:-"marge macos Win11Builder leela"}
riders=${OCW_MP_RIDERS:-2}
run_s=${OCW_MP_SECONDS:-180}
port=${OCW_MP_PORT:-47800}
use_kiosk=${OCW_MP_KIOSK:-1}
build=${OCW_MP_BUILD:-1}
camera=${OCW_MP_CAMERA:-auto}
camera_name=Camera
kiosk_name=Kiosk
kiosk_dir=/var/tmp/ocw
# The kiosk rides at 200 W from its start (virtual trainer; `sim.power` is a
# dev command, refused on a remote server); the bots catch up and ride ahead
# of it. Screenshots and video frames are taken after app game time T1 (s).
t1=${OCW_MP_T1:-50}
# Length of the kiosk's and the camera's videos (s).
video_s=${OCW_MP_VIDEO_S:-15}
remote_mp=target/mp

stamp=$(date +%Y%m%d-%H%M%S)
sha=$(scripts/build-id.sh 2>/dev/null | tr ' ' '-' || echo unknown)
out=${OCW_MP_OUT:-test-output/multiplayer/$stamp}
mkdir -p "$out"
summary=$out/summary.txt
say() { echo "$*" | tee -a "$summary"; }

R() { scripts/remote.sh "$@"; }

host=${OCW_MP_HOST:-$(R "$server_m" field host 2>/dev/null)}
[ -n "$host" ] || { echo "no host for $server_m in config/machines.toml" >&2; exit 2; }
addr="$host:$port"
case "$host" in *:*) addr="[$host]:$port" ;; esac

# Everything shown or logged goes through this: the machines' host names
# (one per line in $redact_hosts; the bot machines are added once known),
# the peers in the server's connection log (IPv4 or IPv6) and every IPv4
# address but loopback are replaced (ssh errors name them too).
# Unbuffered, so a log can be watched.
redact_hosts=$host
redact_perl='BEGIN { $| = 1; @h = grep { length } split /\n/, $ENV{OCW_MP_H} }
  for my $x (@h) { s/\Q$x\E/<host>/g }
  s/(remote(?:\e\[[0-9;]*m)*=(?:\e\[[0-9;]*m)*)[^\s\e]+/$1<peer>/g;
  s/\b(?!127\.0\.0\.1\b)(\d{1,3}\.){3}\d{1,3}\b/<ip>/g'
redact_stream() { OCW_MP_H=$redact_hosts perl -pe "$redact_perl"; }

# --- 1. Machines --------------------------------------------------------
os_of() { R "$1" field os 2>/dev/null; }
up() { timeout 30 scripts/remote.sh "$1" ssh "echo up" 2>/dev/null | grep -q up; }
label_of() { case "$1" in linux) echo linux ;; macos) echo macos ;; windows) echo windows ;; *) echo "$1" ;; esac; }
prefix_of() { case "$1" in linux) echo lin ;; macos) echo mac ;; windows) echo win ;; *) echo bot ;; esac; }

say "OCW multiplayer test $stamp (commit $sha)"
up "$server_m" || { say "FAIL: server machine $server_m is not reachable"; exit 1; }
R "$server_m" enabled >/dev/null 2>&1 || { say "FAIL: server machine $server_m is disabled for building (build dashboard)"; exit 1; }
used=() skipped=() disabled=()
for m in $bot_ms; do
  # Machines disabled for building in the build dashboard get no bot build.
  if [ "$m" != "$server_m" ] && ! R "$m" enabled >/dev/null 2>&1; then disabled+=("$m"); continue; fi
  if [ "$m" = "$server_m" ] || up "$m"; then used+=("$m"); else skipped+=("$m"); fi
done
for m in $bot_ms; do
  h=$(R "$m" field host 2>/dev/null) && [ -n "$h" ] && redact_hosts+=$'\n'"$h"
done
say "server:       $server_m (UDP $port)"
say "bot machines: ${used[*]:-none}${skipped:+ (skipped, unreachable: ${skipped[*]})}${disabled:+ (skipped, disabled for building: ${disabled[*]})}"
[ ${#used[@]} -gt 0 ] || { say "FAIL: no bot machine reachable"; exit 1; }

# --- 2. Builds (parallel; remote.sh waits for free build slots) ----------
if [ "$build" != 0 ]; then
  say "building (release): ocw-server + ocw-bot on $server_m, ocw-bot on the bot machines"
  declare -A bpid
  for m in $(printf '%s\n' "$server_m" "${used[@]}" | awk '!seen[$0]++'); do
    cmd="cargo build --release -p ocw-bot"
    if [ "$m" = "$server_m" ]; then
      cmd="cargo build --release -p ocw-server -p ocw-bot"
      # The camera client runs the release build: a real-time video needs
      # 25 rendered frames a second, which the debug build does not reach.
      [ "$camera" != 0 ] && cmd="cargo build --release -p ocw-server -p ocw-bot -p ocw-desktop"
    fi
    (R "$m" run "$cmd" 2>&1 | redact_stream >"$out/build-$m.log") &
    bpid[$m]=$!
  done
  for m in "${!bpid[@]}"; do
    wait "${bpid[$m]}"
  done
  for m in "${!bpid[@]}"; do
    # remote.sh's exit status is lost in the pipe: look for cargo's errors.
    if grep -qE '^error(\[|:)' "$out/build-$m.log"; then
      say "FAIL: build on $m failed, see $out/build-$m.log"
      exit 1
    fi
  done
fi

# --- 3. The course: what the kiosk rides --------------------------------
route_args=() bot_route=() route_file=""
kiosk=0
if [ "$use_kiosk" = 1 ]; then
  kc=$(R "$server_m" ssh "pgrep -af 'ocw-kiosk.sh run' | grep -v pgrep | head -n 1" 2>/dev/null || true)
  if [ -z "$kc" ]; then
    say "note: no kiosk running on $server_m: bots only"
  else
    words=$(R "$server_m" ssh "grep -v '^[[:space:]]*#' $kiosk_dir/ocw-desktop.args 2>/dev/null | tr '\n' ' '" 2>/dev/null)
    words="$words ${kc#* run}"
    rf="" rt="" world="" graph=0
    set -- $words
    while [ $# -gt 0 ]; do
      case "$1" in
        --route-file) rf=${2:-}; shift ;;
        --route) rt=${2:-}; shift ;;
        --world) world=${2:-}; shift ;;
        --free-ride | --route-gpx) graph=1 ;;
      esac
      shift
    done
    if [ -n "$rf" ]; then
      route_file=$rf
      kiosk=1
    elif [ -n "$world" ] && { [ "$graph" = 1 ] || { [ -n "$rt" ] && [ "$rt" != loop ] && [ "$rt" != hills ]; }; }; then
      say "note: the kiosk rides a road graph (free ride / package route), which ocw-bot does not support: bots only"
    else
      route_args=(--test-route "${rt:-hills}")
      bot_route=(--route "${rt:-hills}")
      kiosk=1
    fi
  fi
fi
# The camera client rides the same course (with the kiosk's world package
# for the scenery, if it has one).
cam_course=()
if [ -n "$route_file" ]; then
  cam_course=(--route-file "$route_file")
elif [ ${#bot_route[@]} -gt 0 ]; then
  cam_course=("${bot_route[@]}")
else
  cam_course=(--route loop)
fi
[ -n "${world:-}" ] && cam_course+=(--world "$world")
if [ -n "$route_file" ]; then
  rname=$(basename "$route_file")
  say "course:       route file $rname (the kiosk's)"
  R "$server_m" ssh "cat $(printf %q "$route_file")" >"$out/$rname" 2>/dev/null ||
    { say "FAIL: cannot read the kiosk's route file on $server_m"; exit 1; }
  route_args=(--route-file "$remote_mp/$rname")
  bot_route=(--route-file "$remote_mp/$rname")
  for m in $(printf '%s\n' "$server_m" "${used[@]}" | awk '!seen[$0]++'); do
    R "$m" put "$out/$rname" "$remote_mp" 2>&1 | redact_stream
  done
elif [ ${#route_args[@]} -eq 0 ]; then
  route_args=(--test-route loop)
  bot_route=(--route loop)
fi
[ -n "$route_file" ] || say "course:       test route ${route_args[1]}"

# --- 4. Server -----------------------------------------------------------
pids=()
cleanup_remote() {
  R "$server_m" ssh "pkill -f -- '--stats-file $remote_mp/stats.json'" >/dev/null 2>&1 || true
  R "$server_m" ssh "pkill -f -- '/tmp/ocw-mp-camera-$stamp/final.jpg'" >/dev/null 2>&1 || true
  for m in "${used[@]}"; do
    if [ "$(os_of "$m")" = windows ]; then
      R "$m" ssh "Stop-Process -Name ocw-bot -ErrorAction SilentlyContinue" >/dev/null 2>&1 || true
    else
      R "$m" ssh "pkill -f 'target/release/ocw-bot --server'" >/dev/null 2>&1 || true
    fi
  done
}
say "starting the server for $run_s s"
(OCW_REMOTE_SLOTS=0 R "$server_m" run "mkdir -p $remote_mp && rm -f $remote_mp/stats.json && target/release/ocw-server --listen 0.0.0.0:$port --bots 0 --seconds $run_s ${route_args[*]} --tls-dir $remote_mp/tls --stats-file $remote_mp/stats.json" 2>&1 | redact_stream >"$out/server.log") &
server_pid=$!
fp=""
for _ in $(seq 120); do
  fp=$(grep -oE -- '--server-cert [0-9a-f]{64}' "$out/server.log" 2>/dev/null | head -n 1 | awk '{print $2}')
  [ -n "$fp" ] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.5
done
if [ -z "$fp" ]; then
  say "FAIL: the server did not start (see $out/server.log)"
  grep -qi "address in use\|in use" "$out/server.log" && say "      UDP $port is in use on $server_m (another server running?)"
  exit 1
fi
say "server up, certificate pinned (sha256 ${fp:0:12}…)"

# --- 5. Kiosk into the session -------------------------------------------
shots=/var/tmp/ocw-mp/$stamp
kiosk_on=0
restore_kiosk() {
  [ "$kiosk_on" = 1 ] || return 0
  kiosk_on=0
  # Back to the saved arguments – only if nobody deployed meanwhile (the
  # file is still ours) – and restart the app (a new binary inode).
  local r
  r=$(R "$server_m" ssh "D=$kiosk_dir; S=$shots
    if cmp -s \$S/args.mp \$D/ocw-desktop.args; then
      if [ -f \$S/args.saved ]; then cp \$S/args.saved \$D/.args.tmp && chmod 644 \$D/.args.tmp && mv -f \$D/.args.tmp \$D/ocw-desktop.args
      else rm -f \$D/ocw-desktop.args; fi
      cp -p \$D/ocw-desktop \$D/.ocw-desktop.mp && mv -f \$D/.ocw-desktop.mp \$D/ocw-desktop
      echo restored
    else echo changed; fi" 2>&1 | redact_stream)
  case "$r" in
    *restored*) say "kiosk: back to its normal arguments (restarting)" ;;
    *changed*) say "kiosk: its arguments were replaced meanwhile (a deployment, which restarted it normally): left as they are" ;;
    *) say "WARNING: could not restore the kiosk: $r (check $kiosk_dir/ocw-desktop.args on $server_m)" ;;
  esac
}
on_exit() {
  restore_kiosk
  for p in "${pids[@]}" "$server_pid"; do kill "$p" 2>/dev/null || true; done
}
trap 'on_exit' EXIT
trap 'say "interrupted"; cleanup_remote; exit 130' INT TERM

if [ "$kiosk" = 1 ]; then
  at() { printf -- '--at "%s=%s"\n' "$1" "$2"; }
  # A kiosk build older than --video-dir / --no-vsync / --max-fps would not
  # start with them: then stills only (deploy first, `make deploy-test`).
  kiosk_new=0
  R "$server_m" ssh "$kiosk_dir/ocw-desktop --help 2>/dev/null | grep -q -- --max-fps" >/dev/null 2>&1 && kiosk_new=1
  [ "$kiosk_new" = 1 ] || say "note: the kiosk's build is older than this test (no --video-dir, --max-fps): stills only, no kiosk video, nothing while its screen is locked (make deploy-test first)"
  {
    echo "# multiplayer test $stamp (scripts/test-multiplayer.sh); restored afterwards"
    echo "--server 127.0.0.1:$port --server-cert $fp"
    echo "--name $kiosk_name --virtual --power 200 --record false"
    # The FPS counter (real render rate) in every picture, and a real-time
    # 25 fps video of $video_s s from the app's own recorder (console `video`).
    echo "--cmd \"fps on\""
    for d in 12 25 40; do at "$((t1 + d))" "screenshot $shots/still-$d.jpg"; done
    if [ "$kiosk_new" = 1 ]; then
      # No waiting for the display's refresh, 60 fps at most: the app
      # draws (and records) even while the screen is locked and off.
      echo "--no-vsync --max-fps 60 --video-dir $shots"
      at "$((t1 + 14))" "video start"
      at "$((t1 + 14 + video_s))" "video stop"
    fi
  } >"$out/kiosk-mp.args"
  R "$server_m" ssh "mkdir -p $shots && chmod 777 $shots && cat > $shots/kiosk-mp.args" <"$out/kiosk-mp.args" 2>&1 | redact_stream
  if R "$server_m" ssh "D=$kiosk_dir; S=$shots; set -e
    [ -f \$D/ocw-desktop.args ] && cp -p \$D/ocw-desktop.args \$S/args.saved
    { [ -f \$S/args.saved ] && cat \$S/args.saved; cat \$S/kiosk-mp.args; } > \$S/args.mp
    cp \$S/args.mp \$D/.args.tmp && chmod 644 \$D/.args.tmp && mv -f \$D/.args.tmp \$D/ocw-desktop.args
    cp -p \$D/ocw-desktop \$D/.ocw-desktop.mp && mv -f \$D/.ocw-desktop.mp \$D/ocw-desktop" 2>&1 | redact_stream; then
    kiosk_on=1
    say "kiosk: joins as \"$kiosk_name\" (restarting with --server 127.0.0.1:$port); screenshots from app time ${t1} s"
  else
    say "FAIL: could not switch the kiosk"
  fi
fi

# A locked desktop session turns the kiosk's screen off: the X server then
# paces a window that waits for the refresh (vsync) at one frame a second,
# so its game time (--at) crawls and no screenshot is taken. A kiosk build
# with --no-vsync --max-fps (above) draws at 60 fps anyway.
locked=no
if [ "$kiosk" = 1 ]; then
  locked=$(R "$server_m" ssh "s=\$(loginctl list-sessions --no-legend 2>/dev/null | awk '\$4==\"seat0\" {print \$1; exit}'); [ -n \"\$s\" ] && loginctl show-session \"\$s\" -p LockedHint --value" 2>/dev/null || true)
  if [ "$locked" = yes ] && [ "${kiosk_new:-0}" = 1 ]; then
    say "note: the kiosk's desktop session is locked: it draws anyway (--no-vsync --max-fps 60 for the test)"
  elif [ "$locked" = yes ]; then
    say "note: the kiosk's desktop session is locked: its old build draws about once a second, so it takes no screenshots (deploy first, or unlock the screen on $server_m)"
  fi
fi
use_camera=0
case "$camera" in
  1) use_camera=1 ;;
  auto) { [ "$kiosk" != 1 ] || { [ "$locked" = yes ] && [ "${kiosk_new:-0}" != 1 ]; }; } && use_camera=1 ;;
esac
cam=/tmp/ocw-mp-camera-$stamp
cam_pid=""
if [ "$use_camera" = 1 ]; then
  {
    echo "--server 127.0.0.1:$port --server-cert $fp --name $camera_name --virtual --power 200 --record false"
    echo "${cam_course[*]}"
    echo "--video-dir $cam --no-audio"
    for d in 30 50 65; do echo "--at \"$d=screenshot $cam/still-$d.jpg\""; done
    # A real-time 25 fps video from the app's own recorder (console
    # `video`); drive.sh shows the FPS counter and logs the render rate.
    echo "--at \"32=video start\""
    echo "--at \"$((32 + video_s))=video stop\""
  } >"$out/camera.args"
  R "$server_m" ssh "rm -rf $cam && mkdir -p $cam && cat > $cam/camera.args" <"$out/camera.args" 2>&1 | redact_stream
  (OCW_REMOTE_SLOTS=0 R "$server_m" run "OCW_PROFILE=release OCW_ARGS_FILE=$cam/camera.args OCW_TEST_OUTPUT=off .claude/skills/run-ocw/drive.sh shot-wayland $cam/final.jpg $run_s" 2>&1 | redact_stream >"$out/camera.log") &
  cam_pid=$!
  say "camera:       headless client \"$camera_name\" on $server_m (weston) rides along and takes the pictures"
fi

# --- 6. Bots ------------------------------------------------------------
follow=()
if [ "$use_camera" = 1 ]; then
  follow=(--follow "$camera_name")
elif [ "$kiosk" = 1 ]; then
  follow=(--follow "$kiosk_name")
fi
j=0
logs=()
for m in "${used[@]}"; do
  os=$(os_of "$m")
  label=$(label_of "$os")
  host_for=$addr
  bin=target/release/ocw-bot
  [ "$os" = windows ] && bin='.\target\release\ocw-bot.exe'
  # macOS stops the monotonic clock and the network while it idles to
  # sleep: keep it awake for the run (no setting is changed).
  [ "$os" = macos ] && bin="caffeinate -i -s $bin"

  args="--server $host_for --server-cert $fp --riders $riders --seconds $((run_s + 60)) --label $label --name-prefix $(prefix_of "$os") --index $((j * riders)) ${bot_route[*]} ${follow[*]}"
  log=$out/bot-$m.log
  logs+=("$log")
  (OCW_REMOTE_SLOTS=0 R "$m" run "$bin $args" 2>&1 | redact_stream >"$log") &
  pids+=($!)
  say "bots:         $riders on $m ($label)"
  j=$((j + 1))
done

wait "$server_pid"
for p in "${pids[@]}" $cam_pid; do wait "$p"; done
pids=()
say "server and bots finished"
restore_kiosk

# --- 7. Results -----------------------------------------------------------
# "1280x720, 25 fps, 15.0 s" of a video (ffprobe on this machine, if any).
video_info() {
  command -v ffprobe >/dev/null || { echo "?"; return; }
  ffprobe -v error -select_streams v:0 -show_entries stream=width,height,avg_frame_rate:format=duration \
    -of default=nw=1 "$1" 2>/dev/null | awk -F= '
      $1 == "width" { w = $2 } $1 == "height" { h = $2 } $1 == "duration" { d = $2 }
      $1 == "avg_frame_rate" { split($2, r, "/"); if (r[2] > 0) f = r[1] / r[2] }
      END { printf "%sx%s, %.0f fps, %.1f s", w, h, f, d }'
}
R "$server_m" fetch "$remote_mp/stats.json" "$out" >/dev/null 2>&1 || say "note: no stats.json from the server"
R "$server_m" put "${logs[@]}" "$remote_mp/bots" >/dev/null 2>&1
kiosk_opt=()
[ "$kiosk" = 1 ] && kiosk_opt+=(--client "$kiosk_name")
[ "$use_camera" = 1 ] && kiosk_opt+=(--client "$camera_name")
remote_logs=$(for l in "${logs[@]}"; do printf '%s/bots/%s ' "$remote_mp" "$(basename "$l")"; done)
OCW_REMOTE_SLOTS=0 R "$server_m" run "target/release/ocw-bot verdict --server-stats $remote_mp/stats.json --processes ${#logs[@]} --riders-per-process $riders ${kiosk_opt[*]} $remote_logs" 2>&1 |
  redact_stream | tee "$out/verdict.txt"
result=$(grep -oE 'RESULT: (PASS|FAIL)' "$out/verdict.txt" | tail -n 1)
[ -n "$result" ] || result="RESULT: FAIL"

# Firewall / reachability hints from the bots.
for l in "${logs[@]}"; do
  if grep -q "no QUIC answer" "$l"; then
    say "hint: $(basename "$l" .log | sed 's/^bot-//') got no answer on UDP $port from $server_m: a firewall on the way (or on $server_m) probably blocks it. Nothing was changed; open UDP $port inbound on $server_m to fix it."
  fi
done

# Kiosk screenshots and a short video from the frames (made on the server
# machine, which has ffmpeg).
if [ "$kiosk" = 1 ]; then
  # The recorder names its file by date: the one MP4 there is the video.
  R "$server_m" ssh "cd $shots 2>/dev/null && v=\$(ls -t *.mp4 2>/dev/null | head -n 1) && [ -n \"\$v\" ] && mv -f \"\$v\" kiosk.mp4" >/dev/null 2>&1 || true
  mkdir -p "$out/kiosk"
  R "$server_m" ssh "cd $shots && tar -cf - still-*.jpg kiosk.mp4 2>/dev/null" 2>/dev/null | tar -xf - -C "$out/kiosk" 2>/dev/null || true
  say "kiosk images: $(ls "$out/kiosk" | grep -c jpg) stills$([ -f "$out/kiosk/kiosk.mp4" ] && echo ", kiosk.mp4 ($(video_info "$out/kiosk/kiosk.mp4"))")"
  R "$server_m" ssh "rm -rf $shots" >/dev/null 2>&1 || true
fi

if [ "$use_camera" = 1 ]; then
  R "$server_m" ssh "cd $cam 2>/dev/null && v=\$(ls -t *.mp4 2>/dev/null | head -n 1) && [ -n \"\$v\" ] && mv -f \"\$v\" camera.mp4" >/dev/null 2>&1 || true
  mkdir -p "$out/camera"
  R "$server_m" ssh "cd $cam && tar -cf - still-*.jpg final.jpg final.log camera.mp4 2>/dev/null" 2>/dev/null | tar -xf - -C "$out/camera" 2>/dev/null || true
  if [ -f "$out/camera/final.log" ]; then
    sed 's/\x1b\[[0-9;]*m//g' "$out/camera/final.log" | redact_stream >"$out/camera/app.log"
    rm -f "$out/camera/final.log"
  fi
  say "camera images: $(ls "$out/camera" 2>/dev/null | grep -c jpg) stills$([ -f "$out/camera/camera.mp4" ] && echo ", camera.mp4 ($(video_info "$out/camera/camera.mp4"))")"
  # The camera's real render rate (wall clock) while it rode in the session.
  if [ -f "$out/camera/app.log" ]; then
    rate=$(grep 'render rate' "$out/camera/app.log" | grep -oE 'fps="?[0-9.]+' | grep -oE '[0-9.]+$' |
      awk '{ s += $1; n++; if (min == "" || $1 < min) min = $1 } END { if (n) printf "mean %.1f fps, lowest %.1f fps (%d samples of 5 s)", s / n, min, n }')
    [ -n "$rate" ] && say "camera render rate: $rate"
    frames=$(grep 'video frames' "$out/camera/app.log" | tail -n 1 | grep -oE '(unique|repeated|dropped)=[0-9]+' | tr '\n' ' ')
    [ -n "$frames" ] && say "camera video frames: $frames"
  fi
  R "$server_m" ssh "rm -rf $cam" >/dev/null 2>&1 || true
fi

# Archive like the run-ocw skill's drive.sh: test-output/YYYY-MM-DD/ + index row.
root=${OCW_TEST_OUTPUT:-$PWD/test-output}
day=$(date +%Y-%m-%d)
stem="$(date +%H%M%S)-$sha-multiplayer"
mkdir -p "$root/$day"
links=""
for f in "$out"/kiosk/still-*.jpg "$out"/kiosk/kiosk.mp4 "$out"/camera/still-*.jpg "$out"/camera/final.jpg \
  "$out"/camera/camera.mp4 "$out/verdict.txt" "$out/summary.txt"; do
  [ -f "$f" ] || continue
  name=$(basename "$(dirname "$f")")-$(basename "$f")
  case "$f" in "$out"/verdict.txt | "$out"/summary.txt) name=$(basename "$f") ;; esac
  cp "$f" "$root/$day/$stem-$name"
  links+="[$name]($day/$stem-$name) "
done
if [ ! -f "$root/index.md" ]; then
  printf '# OCW test output\n\n| Date | Time | Commit | Mode | Window system | App arguments | Files |\n|---|---|---|---|---|---|---|\n' >"$root/index.md"
fi
printf '| %s | %s | %s | %s | %s | `%s` | %s|\n' "$day" "$(date +%H:%M:%S)" "$sha" multiplayer \
  "kiosk ($server_m)" "bots: ${used[*]} × $riders; $result" "$links" >>"$root/index.md"
say "logs:         $out/"
say "archived:     $root/$day/$stem-*"
say "$result"
[ "$result" = "RESULT: PASS" ]
