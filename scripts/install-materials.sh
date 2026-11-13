#!/usr/bin/env bash
# Copies the material library (spec §4.6, Phase 8 T1) into a package or an
# install folder, fetching it first (scripts/fetch-materials.sh; cheap when
# it is up to date):
#
#   scripts/install-materials.sh DEST
#
# DEST gets the KTX2 textures, manifest.toml and CREDITS.md (not the
# fetch stamps). The app finds the library beside its binary (materials/),
# in ../share/ocw/materials (AppImage, Linux installs) or
# ../Resources/materials (macOS bundle).
#
# OCW_MATERIALS_OPTIONAL=1: without network the package is built without
# materials (the app then draws flat colours) instead of failing.
set -euo pipefail
cd "$(dirname "$0")/.."
dest=${1:?usage: install-materials.sh DEST}
scripts/fetch-materials.sh
if [ ! -f assets/materials/manifest.toml ]; then
  echo "warning: no material library to install into $dest" >&2
  exit 0
fi
mkdir -p "$dest"
for f in assets/materials/*.ktx2 assets/materials/manifest.toml assets/materials/CREDITS.md; do
  # Atomic per file (a running kiosk may read the folder).
  install -m 644 "$f" "$dest/.$(basename "$f").new"
  mv -f "$dest/.$(basename "$f").new" "$dest/$(basename "$f")"
done
echo "material library installed: $dest ($(du -sh "$dest" | cut -f1))"
