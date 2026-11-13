#!/usr/bin/env bash
# Fetches the material library (spec §4.6, Phase 8 T1): downloads the sources
# pinned in assets/materials.toml (URL + SHA-256) once into a cache, checks
# every hash, converts them to KTX2 and writes the git-ignored
# assets/materials/ (textures, manifest copy, CREDITS.md). Materials already
# converted from the same pins are skipped, so every build path can run it.
#
#   scripts/fetch-materials.sh [--out DIR] [--only meadow,asphalt] [--cache DIR]
#
# Cache: $OCW_MATERIAL_CACHE, else ~/.cache/ocw/material-sources (Windows:
# %LOCALAPPDATA%\ocw\material-sources). Windows without bash:
#   cargo run --release -p ocw-assets -- materials fetch
# OCW_MATERIALS_OPTIONAL=1: a failed fetch (no network) only warns; the app
# then draws the flat land-use colours.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
if ! cargo run -q --release -p ocw-assets -- materials fetch "$@"; then
  if [ "${OCW_MATERIALS_OPTIONAL:-0}" = 1 ]; then
    echo "warning: material library not fetched; the app draws flat colours" >&2
    exit 0
  fi
  exit 1
fi
