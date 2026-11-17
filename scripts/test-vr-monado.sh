#!/usr/bin/env bash
# The VR integration test against Monado's simulated headset (spec §14
# Phase 11, X1; 11D *Integration tests*): starts monado-service headless
# (null compositor, simulated HMD), runs `ocw-desktop --vr` from a
# `--features vr` build for a few seconds under Xvfb and checks that the
# session started, frames ran and nothing failed.
#
# Needs the OpenXR loader and Monado on the machine. The build machines
# have neither yet (2026-11-15); installing system packages is the user's
# call, so this script only says what is missing and exits 0 ("skipped"):
#
#   Debian / Ubuntu:  sudo apt install libopenxr-loader1 libopenxr1-monado monado-service monado-cli
#   Fedora:           sudo dnf install openxr-libs monado
#
# or Monado built from source into ~/.local (cmake, ninja, Eigen, Vulkan
# and X11 headers as build dependencies):
#
#   git clone https://gitlab.freedesktop.org/monado/monado.git && cd monado
#   cmake -G Ninja -B build -DCMAKE_INSTALL_PREFIX=$HOME/.local -DXRT_FEATURE_SERVICE=ON
#   ninja -C build install
#
# Usage: scripts/test-vr-monado.sh [SECONDS]   (run on a Linux build machine)
set -euo pipefail
cd "$(dirname "$0")/.."
seconds=${1:-8}
out=test-output/vr-monado
mkdir -p "$out"

runtime_json=""
for f in "${XR_RUNTIME_JSON:-}" "$HOME/.local/share/openxr/1/openxr_monado.json" \
  /usr/share/openxr/1/openxr_monado.json /usr/local/share/openxr/1/openxr_monado.json; do
  if [ -n "$f" ] && [ -f "$f" ]; then runtime_json=$f; break; fi
done
service=$(command -v monado-service || true)
[ -z "$service" ] && [ -x "$HOME/.local/bin/monado-service" ] && service=$HOME/.local/bin/monado-service
# ldconfig lives in /sbin on Debian, which is not on a user's PATH: look there
# too, and in the usual library folders.
ldconfig_bin=$(command -v ldconfig || command -v /sbin/ldconfig || command -v /usr/sbin/ldconfig || true)
loader=$( { [ -n "$ldconfig_bin" ] && "$ldconfig_bin" -p 2>/dev/null; ls /usr/lib/*/libopenxr_loader.so.1 /usr/lib64/libopenxr_loader.so.1 /usr/lib/libopenxr_loader.so.1 2>/dev/null; } | grep -c 'libopenxr_loader.so.1' || true)
if [ -z "$runtime_json" ] || [ -z "$service" ] || [ "$loader" = 0 ]; then
  echo "test-vr-monado: skipped – missing:" \
    "$([ "$loader" = 0 ] && echo 'the OpenXR loader (libopenxr-loader1)')" \
    "$([ -z "$service" ] && echo 'monado-service')" \
    "$([ -z "$runtime_json" ] && echo 'the Monado runtime (openxr_monado.json)')"
  echo "  see the top of $0 for the packages"
  exit 0
fi

source ~/.cargo/env 2>/dev/null || true
cargo build -p ocw-desktop --features vr

log=$out/monado.log
app_log=$out/ocw-desktop.log
# Headless: no compositor window, the simulated HMD (no hardware).
# monado-service watches its stdin with epoll, which fails on /dev/null (a
# background job over ssh): "epoll_ctl(stdin) failed", "Failed to init ipc
# main loop". Give it a pipe that stays open for the run.
sleep 3600 | XRT_COMPOSITOR_NULL=1 SIMULATED_ENABLE=1 XRT_COMPOSITOR_LOG=debug "$service" > "$log" 2>&1 &
svc=$!
trap 'kill $svc 2>/dev/null || true; pkill -P $$ sleep 2>/dev/null || true' EXIT
sleep 2
code=0
XR_RUNTIME_JSON=$runtime_json xvfb-run -a -s "-screen 0 1280x720x24" \
  target/debug/ocw-desktop --vr --virtual --no-audio --random-start false --record false \
  --screenshot "$out/mirror.jpg" --screenshot-delay "$seconds" \
  --at "$((seconds > 3 ? seconds - 2 : 1))=vr status" > "$app_log" 2>&1 || code=$?

fail=0
# Regression (v4.68, the exit crash on Windows): the app ends the session
# itself in its last frame and exits cleanly.
[ "$code" = 0 ] || { echo "ocw-desktop exited with $code (see $app_log)"; fail=1; }
grep -q "VR: session ended for the exit" "$app_log" || { echo "the session was not torn down at the exit"; fail=1; }
grep -q "VR: .*Monado" "$app_log" || { echo "no OpenXR session with Monado (see $app_log)"; fail=1; }
grep -q "VR is off" "$app_log" && { grep "VR is off" "$app_log"; fail=1; }
grep -qE "frame [0-9.]+ ms" "$app_log" || { echo "no frame statistics from vr status"; fail=1; }
[ -f "$out/mirror.jpg" ] || { echo "no mirror screenshot"; fail=1; }
grep -q "screenshot of the left eye" "$app_log" || { echo "the screenshot was not the left eye"; fail=1; }
if [ "$fail" = 0 ]; then
  echo "test-vr-monado: ok ($(grep -oE 'frame [0-9.]+ ms[^,]*' "$app_log" | tail -1))"
fi
exit $fail
