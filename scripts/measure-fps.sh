#!/usr/bin/env bash
# Measures the render rate of an ocw-desktop binary on a headless weston with
# the GPU renderer (a machine with a GPU, e.g. marge; spec §15, §11.4):
# no vsync, the app's own `render rate fps=…` log every 5 s, the first
# WARMUP seconds left out. Prints one line: label, mean and min fps, the
# frame time and the samples. Use it to compare two builds on the same
# scene, interleaved (A B A B) because the machine is shared.
#
#   scripts/measure-fps.sh [--app BIN] [--seconds N] [--warmup N]
#                          [--size WxH] [--label TEXT] -- ocw-desktop args...
#
# BIN defaults to target/release/ocw-desktop (not built here: build it first,
# or pass a saved binary, e.g. the previous commit's).
set -euo pipefail
cd "$(dirname "$0")/.."
app=target/release/ocw-desktop seconds=40 warmup=12 size=1280x720 label=""
while [ $# -gt 0 ]; do
  case "$1" in
    --app) app=$2; shift 2 ;;
    --seconds) seconds=$2; shift 2 ;;
    --warmup) warmup=$2; shift 2 ;;
    --size) size=$2; shift 2 ;;
    --label) label=$2; shift 2 ;;
    --) shift; break ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[ -x "$app" ] || { echo "$app missing: build it first" >&2; exit 1; }
[ -r /dev/dri/renderD128 ] || { echo "no GPU render node: the numbers would be software Vulkan" >&2; exit 1; }
log=$(mktemp)
runtime=$(mktemp -d); chmod 700 "$runtime"
socket="ocw-fps-$$"
XDG_RUNTIME_DIR=$runtime weston --backend=headless --renderer=gl --socket="$socket" \
  --width="${size%x*}" --height="${size#*x}" --idle-time=0 >/dev/null 2>&1 &
weston_pid=$!
trap 'kill $weston_pid 2>/dev/null || true; rm -rf "$runtime" "$log"' EXIT
for _ in $(seq 50); do [ -S "$runtime/$socket" ] && break; sleep 0.1; done
status=0
timeout $((seconds + 60)) env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
  WGPU_BACKEND=vulkan RUST_LOG=info,wgpu=warn,naga=warn \
  "$app" --virtual --window "$size" --no-vsync --log-fps 5 --record false \
  --at "$seconds=quit" "$@" >"$log" 2>&1 || status=$?
# Lines like "... render rate fps=87.3 ..." with the app's uptime in seconds.
sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -oE 'render rate fps="?[0-9.]+' | grep -oE '[0-9.]+$' >"$log.fps" || true
n=$(wc -l <"$log.fps")
skip=$((warmup / 5))
tail -n +$((skip + 1)) "$log.fps" | awk -v label="$label" -v n="$n" -v st="$status" '
  { s += $1; c += 1; if (min == "" || $1 < min) min = $1; v = v sprintf("%.0f ", $1) }
  END {
    if (c == 0) { printf "%s: no samples (%d lines, exit %d)\n", label, n, st; exit 1 }
    printf "%s: %.1f fps mean (%.2f ms), min %.1f, %d samples: %s\n", label, s / c, 1000 * c / s, min, c, v
  }'
rm -f "$log.fps"
