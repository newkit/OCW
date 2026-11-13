#!/usr/bin/env bash
# Builds the desktop app (release) on a test machine and deploys it to
# /var/tmp/ocw there, where ocw-kiosk.sh picks it up and restarts the app.
#
#   scripts/deploy-test.sh MACHINE      (machine from config/machines.toml)
#
# If the pilot data is there (scripts/fetch-pilot.sh), the pilot world
# package (terrain, roads, land use, buildings) is rebuilt from this
# checkout's world-builder and installed too, and ocw-desktop.args beside
# the binary makes the app load it (`--world`, see config/README.md).
#
# Environment:
#   OCW_DEPLOY_DIR   install directory on the machine (default /var/tmp/ocw)
#   OCW_DEPLOY_WORLD 0: don't rebuild the world package
#   OCW_KIOSK_ARGS   content of ocw-desktop.args (default: scripts/kiosk.args
#                    with @DIR@ replaced by the install directory, if the
#                    world package exists; "" writes an empty file)
#   OCW_DEPLOY_WORLDS further world packages to install beside the binary
#                    (paths or globs on the machine, space-separated, each
#                    with its .manifest.json if there is one), so the kiosk's
#                    World -> Change world can switch to them (spec §3.2.1).
#                    Default: the Black Forest packages of this checkout and
#                    of the machine's other checkouts
#                    (data/blackforest*/*.ocwpkg under ~/src/OCW*); "" none.
#                    The newest file of each name is installed, unchanged
#                    ones are not copied again. The default world
#                    (ocw-desktop.args) stays as it is.
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
extra_worlds=${OCW_DEPLOY_WORLDS-'data/blackforest*/*.ocwpkg $HOME/src/OCW*/data/blackforest*/*.ocwpkg'}
args_set=${OCW_KIOSK_ARGS+set}
args_b64=$(printf '%s' "${OCW_KIOSK_ARGS-}" | base64 -w0)
scripts/remote.sh "$machine" run "
  set -e
  D=$(printf '%q' "$dir")
  put() { install -m \$1 \"\$2\" \"\$D/.\$3.new\" && mv -f \"\$D/.\$3.new\" \"\$D/\$3\"; }
  # The same, keeping the file's time (the newest world package wins).
  putp() { install -p -m \$1 \"\$2\" \"\$D/.\$3.new\" && mv -f \"\$D/.\$3.new\" \"\$D/\$3\"; }
  # Voice chat (spec §7) where clang 21 is installed (its bindings need it).
  if command -v clang++-21 >/dev/null; then
    CC=clang-21 CXX=clang++-21 cargo build -q --release -p ocw-desktop --features voice
  else
    cargo build -q --release -p ocw-desktop
  fi
  if [ '$world' != 0 ] && [ -f data/pilot/liechtenstein.osm.pbf ] && [ -f data/pilot/liechtenstein.poly ] && [ -f data/pilot/N47E009.hgt ]; then
    echo 'building the pilot world package'
    cargo run -q --release -p world-builder -- package
  fi
  mkdir -p \"\$D\" && chmod 755 \"\$D\"
  put 755 scripts/ocw-kiosk.sh ocw-kiosk.sh
  [ -f \"\$D/devices.toml\" ] || { [ -f /tmp/ocw/kickr/devices.toml ] && install -m 644 /tmp/ocw/kickr/devices.toml \"\$D/devices.toml\"; } || true
  echo '$version' > \"\$D/VERSION.new\" && chmod 644 \"\$D/VERSION.new\" && mv -f \"\$D/VERSION.new\" \"\$D/VERSION\"
  for r in data/pilot/*.route data/pilot/*.ocwpkg data/pilot/*.ocwpkg.manifest.json; do [ -f \"\$r\" ] && put 644 \"\$r\" \"\$(basename \"\$r\")\"; done
  # Further worlds for World -> Change world (spec 3.2.1).
  for r in $extra_worlds; do
    [ -f \"\$r\" ] || continue
    b=\$(basename \"\$r\")
    if [ -f \"\$D/\$b\" ] && [ \"\$D/\$b\" -nt \"\$r\" ]; then continue; fi
    cmp -s \"\$r\" \"\$D/\$b\" || { echo \"installing world \$b from \$r\"; putp 644 \"\$r\" \"\$b\"; }
    if [ -f \"\$r.manifest.json\" ]; then
      cmp -s \"\$r.manifest.json\" \"\$D/\$b.manifest.json\" || putp 644 \"\$r.manifest.json\" \"\$b.manifest.json\"
    fi
  done
  ls \"\$D\"/*.ocwpkg 2>/dev/null | sed 's|^|  world: |' || true
  if [ '$args_set' = set ]; then
    echo '$args_b64' | base64 -d > \"\$D/.args.tmp\"
  elif [ -f \"\$D/liechtenstein.ocwpkg\" ]; then
    sed \"s|@DIR@|\$D|g\" scripts/kiosk.args > \"\$D/.args.tmp\"
  else
    rm -f \"\$D/ocw-desktop.args\"
  fi
  if [ -f \"\$D/.args.tmp\" ]; then
    chmod 644 \"\$D/.args.tmp\" && mv -f \"\$D/.args.tmp\" \"\$D/ocw-desktop.args\"
    echo \"default arguments (\$D/ocw-desktop.args):\"; grep -v '^#' \"\$D/ocw-desktop.args\" | sed 's/^/  /'
  fi
  put 755 target/release/ocw-desktop ocw-desktop
  echo \"deployed \$(cat \"\$D/VERSION\") to \$D\"
"
