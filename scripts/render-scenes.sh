#!/usr/bin/env bash
# Scene-rendering harness: renders the scenes of scripts/scenes.toml
# (screenshots and short videos) headlessly with the run-ocw skill's drive.sh.
#
#   scripts/render-scenes.sh --list                 list the scenes
#   scripts/render-scenes.sh [--only NAME,GLOB] [--world PKG] [--x11] [--out DIR]
#                                                   render (default: Wayland, all
#                                                   scenes) into test-output/scenes/<time>/
#   scripts/render-scenes.sh --remote marge [--only ...]
#                                                   the same on a build machine
#                                                   (scripts/remote.sh), then fetch
#   scripts/render-scenes.sh --pick test-output/scenes/<time> [--only ...]
#                                                   copy into test-output/build-history
#                                                   (shrunk) with index.md rows
#
# The world package (scenes with world = true) comes from --world, $OCW_WORLD,
# data/pilot/liechtenstein.ocwpkg, the main checkout's data/pilot/ (git
# worktrees) or the kiosk install /var/tmp/ocw/; without one these scenes are
# skipped. Details: README.md, "Scene harness"; the logic is render_scenes.py.
set -euo pipefail
cd "$(dirname "$0")/.."
# tomllib needs Python 3.11+; a pyenv shim may point at an older one.
for py in python3 /usr/bin/python3 python3.13 python3.12 python3.11; do
  if command -v "$py" >/dev/null && "$py" -c 'import tomllib' 2>/dev/null; then
    exec "$py" scripts/render_scenes.py "$@"
  fi
done
echo "render-scenes.sh needs Python 3.11+ (tomllib)" >&2
exit 1
