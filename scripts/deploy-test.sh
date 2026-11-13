#!/usr/bin/env bash
# Builds the desktop app (release) on a test machine and deploys it to
# /var/tmp/ocw there, where ocw-kiosk.sh picks it up and restarts the app.
#
#   scripts/deploy-test.sh MACHINE      (machine from config/machines.toml)
#
# The world packages built on the machine are installed too, all side by
# side in the install directory with their manifests and route files (the
# app joins neighbouring packages of one folder, and its world list shows
# them): data/pilot/liechtenstein.ocwpkg (scripts/build-liechtenstein.sh;
# built here if it is missing and the pilot data is there) and the Black
# Forest packages data/blackforest/blackforest*.ocwpkg
# (scripts/build-blackforest.sh, never built by a deploy: 10 minutes). The
# data comes from the deploying checkout's data/ or OCW_WORLD_DATA.
# ocw-desktop.args beside the binary makes the app load the kiosk's world
# (`--world`, `--route`, see config/README.md).
#
# Environment:
#   OCW_DEPLOY_DIR   install directory on the machine (default /var/tmp/ocw)
#   OCW_DEPLOY_WORLD 0: no world packages; rebuild: build the Liechtenstein
#                    package again before installing (default: install what
#                    is built)
#   OCW_WORLD_DATA   folder with pilot/ and blackforest/ on the machine
#                    (default: data/ of the deploying checkout)
#   OCW_KIOSK_WORLD  the kiosk's world: liechtenstein (default) or
#                    blackforest (the north package if it is installed, else
#                    the south one)
#   OCW_KIOSK_ROUTE  its route (default: Liechtenstein: the package's first
#                    route; Black Forest north: "Baden-Baden - Mummelsee",
#                    south: "Freiburg - Schauinsland")
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
dir=${OCW_DEPLOY_DIR:-/var/tmp/ocw}
world=${OCW_DEPLOY_WORLD:-1}
data=${OCW_WORLD_DATA:-data}
kiosk_world=${OCW_KIOSK_WORLD:-liechtenstein}
case "$kiosk_world" in liechtenstein|blackforest) ;; *) echo "OCW_KIOSK_WORLD: liechtenstein or blackforest" >&2; exit 2 ;; esac
route_b64=$(printf '%s' "${OCW_KIOSK_ROUTE-}" | base64 -w0)
args_set=${OCW_KIOSK_ARGS+set}
args_b64=$(printf '%s' "${OCW_KIOSK_ARGS-}" | base64 -w0)
scripts/remote.sh "$machine" run "
  set -e
  D=$(printf '%q' "$dir")
  put() { install -m \$1 \"\$2\" \"\$D/.\$3.new\" && mv -f \"\$D/.\$3.new\" \"\$D/\$3\"; }
  # Voice chat (spec §7) where clang 21 is installed (its bindings need it).
  if command -v clang++-21 >/dev/null; then
    CC=clang-21 CXX=clang++-21 cargo build -q --release -p ocw-desktop --features voice
  else
    cargo build -q --release -p ocw-desktop
  fi
  W=$(printf '%q' "$data")
  if [ '$world' != 0 ] && { [ '$world' = rebuild ] || [ ! -f \"\$W/pilot/liechtenstein.ocwpkg\" ]; } && [ -f \"\$W/pilot/liechtenstein.osm.pbf\" ]; then
    echo 'building the pilot world package'
    if [ \"\$W\" = data ]; then scripts/build-liechtenstein.sh > /dev/null; else echo \"(not in \$W: build it there)\"; fi
  fi
  mkdir -p \"\$D\" && chmod 755 \"\$D\"
  put 755 scripts/ocw-kiosk.sh ocw-kiosk.sh
  [ -f \"\$D/devices.toml\" ] || { [ -f /tmp/ocw/kickr/devices.toml ] && install -m 644 /tmp/ocw/kickr/devices.toml \"\$D/devices.toml\"; } || true
  echo '$version' > \"\$D/VERSION.new\" && chmod 644 \"\$D/VERSION.new\" && mv -f \"\$D/VERSION.new\" \"\$D/VERSION\"
  if [ '$world' != 0 ]; then
    for r in \"\$W\"/pilot/*.route \"\$W\"/pilot/*.ocwpkg \"\$W\"/pilot/*.ocwpkg.manifest.json \\
             \"\$W\"/blackforest/*.route \"\$W\"/blackforest/blackforest*.ocwpkg \"\$W\"/blackforest/blackforest*.ocwpkg.manifest.json; do
      [ -f \"\$r\" ] && put 644 \"\$r\" \"\$(basename \"\$r\")\"
    done
  fi
  route=\$(echo '$route_b64' | base64 -d)
  if [ '$args_set' = set ]; then
    echo '$args_b64' | base64 -d > \"\$D/.args.tmp\"
  elif [ '$kiosk_world' = blackforest ] && [ -f \"\$D/blackforest-north.ocwpkg\" ]; then
    printf '%s\\n' '# Black Forest north (OCW_KIOSK_WORLD=blackforest)' \"--world \$D/blackforest-north.ocwpkg\" \"--route \\\"\${route:-Baden-Baden - Mummelsee}\\\"\" > \"\$D/.args.tmp\"
  elif [ '$kiosk_world' = blackforest ] && [ -f \"\$D/blackforest.ocwpkg\" ]; then
    printf '%s\\n' '# Black Forest south (OCW_KIOSK_WORLD=blackforest)' \"--world \$D/blackforest.ocwpkg\" \"--route \\\"\${route:-Freiburg - Schauinsland}\\\"\" > \"\$D/.args.tmp\"
  elif [ -f \"\$D/liechtenstein.ocwpkg\" ]; then
    sed \"s|@DIR@|\$D|g\" scripts/kiosk.args > \"\$D/.args.tmp\"
    [ -z \"\$route\" ] || printf '%s\\n' \"--route \\\"\$route\\\"\" >> \"\$D/.args.tmp\"
  else
    rm -f \"\$D/ocw-desktop.args\"
  fi
  # Show mode (scripts/kiosk-show.args: virtual trainer at 500 W) for every world.
  [ '$args_set' = set ] || { [ -f \"\$D/.args.tmp\" ] && grep -v '^#' scripts/kiosk-show.args >> \"\$D/.args.tmp\"; } || true
  if [ -f \"\$D/.args.tmp\" ]; then
    chmod 644 \"\$D/.args.tmp\" && mv -f \"\$D/.args.tmp\" \"\$D/ocw-desktop.args\"
    echo \"default arguments (\$D/ocw-desktop.args):\"; grep -v '^#' \"\$D/ocw-desktop.args\" | sed 's/^/  /'
  fi
  put 755 target/release/ocw-desktop ocw-desktop
  echo \"deployed \$(cat \"\$D/VERSION\") to \$D\"
"
