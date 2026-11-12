#!/usr/bin/env bash
# Builds the macOS app bundle of the desktop app and packs it (run on macOS):
#
#   scripts/build-macos-app.sh [OUT_DIR]     (default: target/macos)
#
# Writes OUT_DIR/Open Cycling World.app (stripped release binary,
# Info.plist, icon from artwork/logos), signs it ad hoc and packs it as
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-macos-<arch>.dmg   (drag to Applications)
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-macos-<arch>.zip
#
# Ad-hoc signed, not notarised: Gatekeeper asks before the first start
# (right-click → Open, or System Settings → Privacy & Security → Open Anyway).
# Set CODESIGN_IDENTITY to a "Developer ID Application: …" identity to sign
# with the hardened runtime instead (notarisation is a separate step).
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo "build-macos-app.sh runs on macOS" >&2; exit 1; }

out=${1:-target/macos}
name="Open Cycling World"
bundle_id=org.opencyclingworld.desktop
exe=ocw-desktop

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
# OCW_SHA: commit for copies without .git (scripts/remote.sh).
sha=${OCW_SHA:-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)}
arch=$(uname -m)
# Command Line Tools whose usr/include/c++/v1 is a near-empty leftover (seen
# after macOS upgrades) shadow the SDK's libc++ headers, so C++ builds such as
# the voice backend (webrtc-sys, cxx) fail with "'string' file not found".
# Point the compiler at the SDK's headers then.
if ! printf '#include <string>\n' | c++ -x c++ -std=c++17 -fsyntax-only - 2>/dev/null; then
  export CXXFLAGS="${CXXFLAGS:-} -nostdinc++ -isystem $(xcrun --show-sdk-path)/usr/include/c++/v1"
  echo "libc++ headers not found by c++; using the SDK's ($CXXFLAGS)"
fi
# OCW_FEATURES: extra cargo features, e.g. ocw-desktop/voice (release builds).
cargo build --release -p ocw-desktop ${OCW_FEATURES:+--features "$OCW_FEATURES"}
bin=target/release/$exe

# Minimum macOS version the binary was built for (LC_BUILD_VERSION minos).
minos=$(vtool -show-build "$bin" 2>/dev/null | awk '$1 == "minos" { print $2; exit }')
[ -n "$minos" ] || minos=$(otool -l "$bin" | awk '$1 == "minos" { print $2; exit }')
[ -n "$minos" ] || minos=11.0

mkdir -p "$out"
app="$out/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$bin" "$app/Contents/MacOS/$exe"
strip -x "$app/Contents/MacOS/$exe"

# Icon: iconset from the PNG renders (the 1024 px 512@2x is missing: no
# render of that size yet, and upscaling would only blur it).
iconset=$(mktemp -d)/ocw.iconset
mkdir -p "$iconset"
for s in 16 32 128 256 512; do
  cp "artwork/logos/ocw-$s.png" "$iconset/icon_${s}x${s}.png"
  [ "$s" = 512 ] || cp "artwork/logos/ocw-$((s * 2)).png" "$iconset/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o "$app/Contents/Resources/ocw.icns" "$iconset"
rm -rf "$(dirname "$iconset")"

cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundleExecutable</key><string>$exe</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version+$sha</string>
  <key>CFBundleIconFile</key><string>ocw</string>
  <key>LSMinimumSystemVersion</key><string>$minos</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.sports</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSBluetoothAlwaysUsageDescription</key>
  <string>Open Cycling World connects to your smart trainer, power meter, cadence sensor and heart rate monitor over Bluetooth.</string>
  <key>NSHumanReadableCopyright</key><string>Open Cycling World contributors, MIT OR Apache-2.0. Map data © OpenStreetMap contributors.</string>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist"

# Ad-hoc signature (or a real identity), then check it.
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp -s "$CODESIGN_IDENTITY" "$app"
else
  codesign --force -s - "$app"
fi
codesign --verify --deep --strict --verbose=2 "$app"
spctl -a -vv "$app" || echo "(Gatekeeper rejects ad-hoc signed apps: expected without a Developer ID and notarisation)"

base="$out/OpenCyclingWorld-$version-$sha-macos-$arch"
rm -f "$base.zip" "$base.dmg"
ditto -c -k --keepParent "$app" "$base.zip"
# DMG with the app and a link to /Applications to drag it onto.
stage=$(mktemp -d)
ditto "$app" "$stage/$name.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "$name" -srcfolder "$stage" -ov -format UDZO "$base.dmg" >/dev/null
rm -rf "$stage"
echo "App: $app (minimum macOS $minos)"
ls -lh "$base.dmg" "$base.zip"
