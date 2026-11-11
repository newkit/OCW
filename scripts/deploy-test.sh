#!/usr/bin/env bash
# Builds the desktop app (release) on a test machine and deploys it to
# /var/tmp/ocw there, where ocw-kiosk.sh picks it up and restarts the app.
#
#   scripts/deploy-test.sh MACHINE      (machine from config/machines.toml)
#
# The binary is replaced atomically (copy, then rename), so a running kiosk
# never starts a half-written file.
set -euo pipefail
cd "$(dirname "$0")/.."
machine=${1:?usage: deploy-test.sh MACHINE}
version="$(git rev-parse --short HEAD)$( [ -n "$(git status --porcelain --untracked-files=no)" ] && echo -dirty) $(date '+%F %T')"
scripts/remote.sh "$machine" run "
  set -e
  cargo build -q --release -p ocw-desktop
  D=/var/tmp/ocw
  mkdir -p \$D && chmod 755 \$D
  install -m 755 scripts/ocw-kiosk.sh \$D/ocw-kiosk.sh
  [ -f \$D/devices.toml ] || { [ -f /tmp/ocw/kickr/devices.toml ] && install -m 644 /tmp/ocw/kickr/devices.toml \$D/devices.toml; } || true
  echo '$version' > \$D/VERSION.new && chmod 644 \$D/VERSION.new && mv -f \$D/VERSION.new \$D/VERSION
  for r in data/pilot/*.route data/pilot/*.ocwpkg; do [ -f \"\$r\" ] && install -m 644 \"\$r\" \$D/; done
  install -m 755 target/release/ocw-desktop \$D/.ocw-desktop.new
  mv -f \$D/.ocw-desktop.new \$D/ocw-desktop
  echo \"deployed \$(cat \$D/VERSION) to \$D\"
"
