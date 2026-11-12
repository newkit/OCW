#!/usr/bin/env bash
# Builds an AppImage of the desktop app (Linux):
#
#   scripts/build-appimage.sh [OUT]     (default: target/OpenCyclingWorld-x86_64.AppImage)
#
# Assembles target/appimage/AppDir (binary, desktop entry, icons, .DirIcon,
# which file managers show for the AppImage) and packs it with
# appimagetool (https://github.com/AppImage/appimagetool) if it is on
# PATH or in ~/.local/bin (`make deps-appimage` installs it there);
# otherwise it stops after the AppDir and says how to pack it. Without
# FUSE 2 (libfuse2t64 / libfuse2) appimagetool runs extracted
# (APPIMAGE_EXTRACT_AND_RUN=1).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/OpenCyclingWorld-$(uname -m).AppImage}
# OCW_FEATURES: extra cargo features, e.g. ocw-desktop/voice (release builds).
cargo build --release -p ocw-desktop ${OCW_FEATURES:+--features "$OCW_FEATURES"}
app=target/appimage/AppDir
rm -rf "$app"
mkdir -p "$app/usr/bin" "$app/usr/share/applications"
install -m 755 target/release/ocw-desktop "$app/usr/bin/ocw-desktop"
install -m 644 packaging/linux/ocw-desktop.desktop "$app/ocw-desktop.desktop"
install -m 644 packaging/linux/ocw-desktop.desktop "$app/usr/share/applications/ocw-desktop.desktop"
for s in 16 32 48 64 128 256 512; do
  d="$app/usr/share/icons/hicolor/${s}x${s}/apps"
  mkdir -p "$d" && install -m 644 "artwork/logos/ocw-$s.png" "$d/ocw-desktop.png"
done
mkdir -p "$app/usr/share/icons/hicolor/scalable/apps"
install -m 644 artwork/logos/ocw.svg "$app/usr/share/icons/hicolor/scalable/apps/ocw-desktop.svg"
install -m 644 artwork/logos/ocw-256.png "$app/ocw-desktop.png"
ln -s ocw-desktop.png "$app/.DirIcon"
cat >"$app/AppRun" <<'RUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/ocw-desktop" "$@"
RUN
chmod 755 "$app/AppRun"
tool=$(command -v appimagetool || true)
if [ -z "$tool" ] && [ -x "$HOME/.local/bin/appimagetool" ]; then
  tool=$HOME/.local/bin/appimagetool
fi
if [ -n "$tool" ]; then
  # Without FUSE 2 an AppImage cannot mount itself; it then runs extracted.
  if ! { [ -e /dev/fuse ] && /sbin/ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; }; then
    export APPIMAGE_EXTRACT_AND_RUN=1
  fi
  ARCH=$(uname -m) "$tool" "$app" "$out"
  echo "AppImage: $out"
else
  echo "AppDir ready: $app"
  echo "appimagetool not found; install it ('make deps-appimage') and run this script again, or:"
  echo "  ARCH=$(uname -m) appimagetool $app $out"
fi
