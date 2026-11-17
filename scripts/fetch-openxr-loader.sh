#!/usr/bin/env bash
# The Khronos OpenXR loader for the Windows release (spec §14 Phase 11,
# v4.38: "ship the official DLL"):
#
#   scripts/fetch-openxr-loader.sh [OUT_DIR]     (default target/openxr-vendor/out)
#
# Puts openxr_loader.dll (x64) and its licence, openxr_loader.LICENSE.txt,
# into OUT_DIR – beside ocw-desktop.exe in the Windows zip, where the VR
# build (`--features vr`) opens it at run time. The file is the official
# prebuilt loader of the Khronos release (openxr_loader_windows-1.1.65.zip
# of KhronosGroup/OpenXR-SDK-Source), checked against its pinned SHA-256s
# and cached in target/openxr-vendor/ (fetched once). It is not in the
# repository.
#
# Licence: Apache-2.0 (The Khronos Group Inc.); the licence text goes next
# to the DLL. Version and hashes change together, here only.
set -euo pipefail
cd "$(dirname "$0")/.."

version=1.1.65
# sha256 of the release asset openxr_loader_windows-1.1.65.zip
zip_sha256=859d7de35b7a968112db770b09ed8bddeed3cb16fd8ae070643a857676bea6fd
# sha256 of x64/bin/openxr_loader.dll inside it
dll_sha256=b0d0eed4e2b6aeb3ce529d82daf3daa5db104857d65c872884b4661142366ce2

out=${1:-target/openxr-vendor/out}
cache=target/openxr-vendor
zip=$cache/openxr_loader_windows-$version.zip
mkdir -p "$cache" "$out"
sum() { sha256sum "$1" | cut -d' ' -f1; }
if ! { [ -f "$zip" ] && [ "$(sum "$zip")" = "$zip_sha256" ]; }; then
  echo "fetching the OpenXR loader $version (KhronosGroup/OpenXR-SDK-Source release)"
  curl -sSfL --retry 3 -o "$zip.part" \
    "https://github.com/KhronosGroup/OpenXR-SDK-Source/releases/download/release-$version/openxr_loader_windows-$version.zip"
  if [ "$(sum "$zip.part")" != "$zip_sha256" ]; then
    echo "OpenXR loader $version: the download does not match its SHA-256" >&2
    rm -f "$zip.part"
    exit 1
  fi
  mv "$zip.part" "$zip"
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
unzip -q -o "$zip" x64/bin/openxr_loader.dll share/doc/openxr/LICENSE -d "$tmp"
if [ "$(sum "$tmp/x64/bin/openxr_loader.dll")" != "$dll_sha256" ]; then
  echo "OpenXR loader $version: openxr_loader.dll does not match its SHA-256" >&2
  exit 1
fi
cp "$tmp/x64/bin/openxr_loader.dll" "$out/openxr_loader.dll"
cp "$tmp/share/doc/openxr/LICENSE" "$out/openxr_loader.LICENSE.txt"
echo "OpenXR loader $version -> $out/openxr_loader.dll (+ openxr_loader.LICENSE.txt)"
