#!/usr/bin/env bash
# Builds an AppImage of the desktop app (Linux):
#
#   scripts/build-appimage.sh [OUT]     (default: target/OpenCyclingWorld-x86_64.AppImage)
#
# Assembles target/appimage/AppDir (binary, desktop entry, icons, .DirIcon,
# which file managers show for the AppImage) and packs it with
# appimagetool (https://github.com/AppImage/appimagetool) if it is on
# PATH; otherwise it stops after the AppDir and says how to pack it.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/OpenCyclingWorld-$(uname -m).AppImage}
cargo build --release -p ocw-desktop
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
if command -v appimagetool >/dev/null; then
  ARCH=$(uname -m) appimagetool "$app" "$out"
  echo "AppImage: $out"
else
  echo "AppDir ready: $app"
  echo "appimagetool not found; install it (https://github.com/AppImage/appimagetool) and run:"
  echo "  ARCH=$(uname -m) appimagetool $app $out"
fi
