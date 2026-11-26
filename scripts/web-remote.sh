#!/usr/bin/env bash
# Builds the browser app on a build machine and fetches the page here
# (`make web-remote MACHINE=marge`; spec §14 Phase 10, §11.2): builds run on
# the build machines, never on the coordinator, so this is how a machine
# without a local build gets target/web/ for `make web-serve`.
#
#   scripts/web-remote.sh MACHINE           make deps-web && make web there, fetch the page
#   WORLDS=1 scripts/web-remote.sh MACHINE  also make web-worlds there, fetch the worlds and
#                                           the dev root's public key (target/web-keys/root.hex)
#
# WEB_GPU, WEB_PROFILE and WEB_WORLD_PKGS are passed on to the build.
#
# Fetched: only what the page needs – index.html, the js / wasm of both
# builds, the voice shim and LiveKit's SDK with its licence, the GPU choice
# (ocw-gpu.js), sizes.txt – and
# target/web/worlds/ when the machine has published worlds (rsync --delete:
# the local copy follows the machine's). The private signing keys stay on
# the build machine. Local worlds stay when the machine has none.
set -euo pipefail
cd "$(dirname "$0")/.."
machine=${1:?usage: scripts/web-remote.sh MACHINE  (WORLDS=1: also make web-worlds)}

cmd="make deps-web && make web"
[ "${WORLDS:-0}" = 1 ] && cmd="$cmd && make web-worlds"
env=""
for v in WEB_GPU WEB_PROFILE WEB_WORLD_PKGS; do
  if [ -n "${!v:-}" ]; then env+="export $v=$(printf %q "${!v}"); "; fi
done
echo "web-remote: building on $machine: $cmd"
scripts/remote.sh "$machine" run "${env}${cmd}"

# The page: only the files it loads (no cargo output, even from older builds
# whose `web` profile wrote into target/web/).
files=(index.html ocw-voice.js ocw-gpu.js ocw-log.js ocw-xr.js ocw-net-worker.js ocw_web.js ocw_web_bg.wasm ocw_web_webgl2.js ocw_web_webgl2_bg.wasm
       ocw_web_webxr.js ocw_web_webxr_bg.wasm
       livekit-client.umd.js livekit-client.LICENSE.txt sizes.txt)
filters=(--include=/web/)
for f in "${files[@]}"; do filters+=("--include=/web/$f"); done
filters+=(--exclude=*)
mkdir -p target/web
# --delete: a file of an earlier fetch that this build has not (e.g. one
# GPU build) goes; what the filters exclude (worlds/, other files) stays.
scripts/remote.sh "$machine" fetch target/web target --delete "${filters[@]}"
[ -f target/web/index.html ] || { echo "web-remote: no index.html fetched from $machine" >&2; exit 1; }

# The worlds and the key that signed them, together (another machine's
# worlds need its key).
if scripts/remote.sh "$machine" fetch target/web-keys/root.hex target/web-keys.fetch 2>/dev/null; then
  scripts/remote.sh "$machine" fetch target/web/worlds target/web --delete ||
    echo "web-remote: warning: $machine has a dev root but no target/web/worlds/" >&2
  mkdir -p target/web-keys
  mv target/web-keys.fetch/root.hex target/web-keys/root.hex
  echo "web-remote: worlds and target/web-keys/root.hex fetched"
else
  echo "web-remote: $machine has no published worlds (WORLDS=1 publishes them)"
fi
rm -rf target/web-keys.fetch

ls target/web
echo "web-remote: target/web/ is $machine's build; serve it: make web-serve"
