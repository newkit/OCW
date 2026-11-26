#!/usr/bin/env bash
# Builds the desktop app (release) on a test machine and deploys it to
# /var/tmp/ocw there, where ocw-kiosk.sh picks it up and restarts the app.
#
#   scripts/deploy-test.sh MACHINE      (machine from config/machines.toml)
#
# A Windows machine (os = "windows", e.g. Weela with the Meta Quest) gets
# the VR kiosk instead: scripts/deploy-test-windows.sh (a `--features vr`
# build, the OpenXR loader, the materials and the kiosk's world into
# %LOCALAPPDATA%\OCW\kiosk for scripts/ocw-kiosk.ps1; spec §14 Phase 11, 11I).
#
# The world packages built on the machine are installed too, all side by
# side in the install directory with their manifests and route files (the
# app joins neighbouring packages of one folder, and its world list shows
# them): data/pilot/liechtenstein.ocwpkg (scripts/build-liechtenstein.sh;
# built here if it is missing and the pilot data is there) and the Black
# Forest packages data/blackforest/blackforest*.ocwpkg
# (scripts/build-blackforest.sh, never built by a deploy: 10 minutes) and
# Tenerife, data/tenerife/tenerife.ocwpkg (scripts/build-tenerife.sh, not
# built by a deploy either). The data comes from the deploying checkout's
# data/ or OCW_WORLD_DATA.
# ocw-desktop.args beside the binary makes the app load the kiosk's world
# (`--world`, `--route`, see config/README.md).
#
# The material library (scripts/fetch-materials.sh, spec §4.6) goes to
# /var/tmp/ocw/materials beside the binary, LiveKit's voice server
# (scripts/fetch-livekit-server.sh, spec §7.1.8) to /var/tmp/ocw/livekit-server.
#
# Environment:
#   OCW_DEPLOY_DIR   install directory on the machine (default /var/tmp/ocw)
#   OCW_DEPLOY_WORLD 0: no world packages; rebuild: build the Liechtenstein
#                    package again before installing (default: install what
#                    is built)
#   OCW_WORLD_DATA   folder with pilot/, blackforest/ and tenerife/ on the machine
#                    (default: data/ of the deploying checkout)
#   OCW_KIOSK_WORLD  the kiosk's world: liechtenstein, blackforest (the
#                    north package if it is installed, else the south one) or
#                    tenerife. Default: keep the world the kiosk has now (from
#                    its ocw-desktop.args), else liechtenstein.
#   OCW_KIOSK_ROUTE  its route (default: Liechtenstein: the package's first
#                    route; Black Forest north: "Baden-Baden - Mummelsee",
#                    south: "Freiburg - Schauinsland"; Tenerife: "Puerto de la
#                    Cruz - Teide")
#   OCW_KIOSK_ARGS   content of ocw-desktop.args (overrides the two above;
#                    "" writes an empty file)
#
# Every file is replaced atomically (copy, then rename), the binary last,
# so a running kiosk restarts once, with everything in place, and never
# starts a half-written file.
set -euo pipefail
cd "$(dirname "$0")/.."
machine=${1:?usage: deploy-test.sh MACHINE}
# Same commit id as the build id in the HUD (remote.sh passes it to the build).
version="$(scripts/build-id.sh) $(date '+%F %T')"
# The deploy log for the art project (OCW-ART's ART-PROJECT.md §2.7): one JSON line
# per deploy in the main checkout's test-output/build-log/DAY/deploys.jsonl
# (start, end, machine, commit, exit). OCW_BUILD_LOG=0 turns it off.
deploy_start=$(date --iso-8601=seconds)
deploy_log() {
  local rc=$? common root
  [ "${OCW_BUILD_LOG:-1}" = 0 ] && return 0
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 0
  root="$(dirname "$common")/test-output/build-log/$(date +%Y-%m-%d)"
  mkdir -p "$root" 2>/dev/null || return 0
  printf '{"start":"%s","end":"%s","machine":"%s","commit":"%s","exit":%s}\n' "$deploy_start" \
    "$(date --iso-8601=seconds)" "$machine" "$(scripts/build-id.sh 2>/dev/null)" "$rc" >>"$root/deploys.jsonl" || true
}
trap deploy_log EXIT
if [ "$(scripts/remote.sh "$machine" field os 2>/dev/null)" = windows ]; then
  scripts/deploy-test-windows.sh "$machine"
  exit 0
fi
dir=${OCW_DEPLOY_DIR:-/var/tmp/ocw}
world=${OCW_DEPLOY_WORLD:-1}
data=${OCW_WORLD_DATA:-data}
kiosk_world=${OCW_KIOSK_WORLD:-keep}
case "$kiosk_world" in liechtenstein|blackforest|tenerife|keep) ;; *) echo "OCW_KIOSK_WORLD: liechtenstein, blackforest or tenerife" >&2; exit 2 ;; esac
route_b64=$(printf '%s' "${OCW_KIOSK_ROUTE-}" | base64 -w0)
args_set=${OCW_KIOSK_ARGS+set}
args_b64=$(printf '%s' "${OCW_KIOSK_ARGS-}" | base64 -w0)
# A deploy is allowed on a machine disabled for building in the build
# dashboard (its kiosk still gets new builds): OCW_REMOTE_DEPLOY=1.
OCW_REMOTE_DEPLOY=1 scripts/remote.sh "$machine" run "
  set -e
  D=$(printf '%q' "$dir")
  put() { install -m \$1 \"\$2\" \"\$D/.\$3.new\" && mv -f \"\$D/.\$3.new\" \"\$D/\$3\"; }
  # Voice chat (spec §7) where clang 21 is installed (its bindings need it).
  if command -v clang++-21 >/dev/null; then
    CC=clang-21 CXX=clang++-21 cargo build -q --release -p ocw-desktop --features voice
  else
    cargo build -q --release -p ocw-desktop
  fi
  # The kiosk's game server (ocw-kiosk.sh starts it; user 2026-11-15).
  cargo build -q --release -p ocw-server
  W=$(printf '%q' "$data")
  if [ '$world' != 0 ] && { [ '$world' = rebuild ] || [ ! -f \"\$W/pilot/liechtenstein.ocwpkg\" ]; } && [ -f \"\$W/pilot/liechtenstein.osm.pbf\" ]; then
    echo 'building the pilot world package'
    if [ \"\$W\" = data ]; then scripts/build-liechtenstein.sh > /dev/null; else echo \"(not in \$W: build it there)\"; fi
  fi
  mkdir -p \"\$D\" && chmod 755 \"\$D\"
  put 755 scripts/ocw-kiosk.sh ocw-kiosk.sh
  [ -f \"\$D/devices.toml\" ] || { [ -f /tmp/ocw/kickr/devices.toml ] && install -m 644 /tmp/ocw/kickr/devices.toml \"\$D/devices.toml\"; } || true
  # The local config folder (~/.config/ocw, user 2026-11-17): the remembered
  # devices live there (not under /tmp, which a reboot empties; the app moves
  # devices.toml into ocw.toml [devices], spec 3.2.4), and an example
  # secrets.toml is put there once (values commented out = Strava off).
  C=\${XDG_CONFIG_HOME:-\$HOME/.config}/ocw
  mkdir -p \"\$C\"
  if [ ! -f \"\$C/devices.toml\" ] && ! grep -qs '^\\[\\[devices\\.device\\]\\]' \"\$C/ocw.toml\"; then
    for f in /tmp/ocw/kickr/devices.toml \"\$D/devices.toml\"; do
      [ -f \"\$f\" ] && install -m 600 \"\$f\" \"\$C/devices.toml\" && echo \"devices: \$C/devices.toml\" && break
    done || true
  fi
  [ -f \"\$C/secrets.toml\" ] || [ -f \"\$C/strava.toml\" ] || { install -m 600 packaging/secrets.example.toml \"\$C/secrets.toml\" && echo \"example secrets (Strava): \$C/secrets.toml\"; }
  [ -f \"\$D/kiosk-local.args\" ] && [ -f \"\$C/devices.toml\" ] && \\
    sed -i \"s#^--devices-file /tmp/ocw/kickr/devices.toml\\\$#--devices-file \$C/devices.toml#\" \"\$D/kiosk-local.args\" || true
  echo '$version' > \"\$D/VERSION.new\" && chmod 644 \"\$D/VERSION.new\" && mv -f \"\$D/VERSION.new\" \"\$D/VERSION\"
  if [ '$world' != 0 ]; then
    for r in \"\$W\"/pilot/*.route \"\$W\"/pilot/*.ocwpkg \"\$W\"/pilot/*.ocwpkg.manifest.json \\
             \"\$W\"/blackforest/*.route \"\$W\"/blackforest/blackforest*.ocwpkg \"\$W\"/blackforest/blackforest*.ocwpkg.manifest.json \\
             \"\$W\"/tenerife/*.route \"\$W\"/tenerife/tenerife.ocwpkg \"\$W\"/tenerife/tenerife.ocwpkg.manifest.json; do
      [ -f \"\$r\" ] && put 644 \"\$r\" \"\$(basename \"\$r\")\"
    done
  fi
  route=\$(echo '$route_b64' | base64 -d)
  kw='$kiosk_world'
  if [ \"\$kw\" = keep ]; then
    if grep -q '^--world .*blackforest' \"\$D/ocw-desktop.args\" 2>/dev/null; then kw=blackforest
    elif grep -q '^--world .*tenerife' \"\$D/ocw-desktop.args\" 2>/dev/null; then kw=tenerife; else kw=liechtenstein; fi
  fi
  if [ '$args_set' = set ]; then
    echo '$args_b64' | base64 -d > \"\$D/.args.tmp\"
  elif [ \"\$kw\" = blackforest ] && [ -f \"\$D/blackforest-north.ocwpkg\" ]; then
    printf '%s\\n' '# Black Forest north (OCW_KIOSK_WORLD=blackforest)' \"--world \$D/blackforest-north.ocwpkg\" \"--route \\\"\${route:-Baden-Baden - Mummelsee}\\\"\" > \"\$D/.args.tmp\"
  elif [ \"\$kw\" = tenerife ] && [ -f \"\$D/tenerife.ocwpkg\" ]; then
    printf '%s\\n' '# Tenerife (OCW_KIOSK_WORLD=tenerife)' \"--world \$D/tenerife.ocwpkg\" \"--route \\\"\${route:-Puerto de la Cruz - Teide}\\\"\" > \"\$D/.args.tmp\"
  elif [ \"\$kw\" = blackforest ] && [ -f \"\$D/blackforest.ocwpkg\" ]; then
    printf '%s\\n' '# Black Forest south (OCW_KIOSK_WORLD=blackforest)' \"--world \$D/blackforest.ocwpkg\" \"--route \\\"\${route:-Freiburg - Schauinsland}\\\"\" > \"\$D/.args.tmp\"
  elif [ -f \"\$D/liechtenstein.ocwpkg\" ]; then
    sed \"s|@DIR@|\$D|g\" scripts/kiosk.args > \"\$D/.args.tmp\"
    [ -z \"\$route\" ] || printf '%s\\n' \"--route \\\"\$route\\\"\" >> \"\$D/.args.tmp\"
  else
    rm -f \"\$D/ocw-desktop.args\"
  fi
  # Show mode (scripts/kiosk-show.args: virtual trainer at 500 W) for every world.
  # Machine-local additions (kept across deploys): \$D/kiosk-local.args, e.g. the
  # trainer at marge (--devices-file …). With a devices file there the show
  # mode's virtual trainer (--virtual, --power) is left out, so the real
  # trainer rides.
  if [ '$args_set' != set ] && [ -f \"\$D/.args.tmp\" ]; then
    if grep -q '^--devices-file' \"\$D/kiosk-local.args\" 2>/dev/null; then
      grep -v '^#' scripts/kiosk-show.args | grep -vE '^--(virtual|power)' >> \"\$D/.args.tmp\"
    else
      grep -v '^#' scripts/kiosk-show.args >> \"\$D/.args.tmp\"
    fi
    [ -f \"\$D/kiosk-local.args\" ] && grep -v '^#' \"\$D/kiosk-local.args\" >> \"\$D/.args.tmp\"
  fi
  true
  if [ -f \"\$D/.args.tmp\" ]; then
    chmod 644 \"\$D/.args.tmp\" && mv -f \"\$D/.args.tmp\" \"\$D/ocw-desktop.args\"
    echo \"default arguments (\$D/ocw-desktop.args):\"; grep -v '^#' \"\$D/ocw-desktop.args\" | sed 's/^/  /'
  fi
  # Material library (spec §4.6) beside the binary, before it (the kiosk
  # restarts on the binary).
  OCW_MATERIALS_OPTIONAL=1 scripts/install-materials.sh \"\$D/materials\"
  # The voice server next to ocw-server (spec §7.1.8): the kiosk's game
  # server finds it there and starts it (pinned, SHA-256 checked, fetched
  # once into target/voice-vendor/). Without it the kiosk rides without voice.
  if scripts/fetch-livekit-server.sh '' target/voice-server/kiosk > /dev/null; then
    put 755 target/voice-server/kiosk/livekit-server livekit-server
    put 644 target/voice-server/kiosk/livekit-server.LICENSE.txt livekit-server.LICENSE.txt
  else
    echo 'livekit-server not fetched: the kiosk game server runs without voice'
  fi
  put 755 target/release/ocw-server ocw-server
  put 755 target/release/ocw-desktop ocw-desktop
  echo \"deployed \$(cat \"\$D/VERSION\") to \$D\"
"
