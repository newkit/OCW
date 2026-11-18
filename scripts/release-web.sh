#!/usr/bin/env bash
# Builds the browser app's release zip of a GitHub release (spec §11.3,
# §14 Phase 10) on the Linux build machine (marge):
#
#   scripts/release-web.sh [OUT_DIR]     (default: ~/ocw-release-<version>)
#
# run through `scripts/remote.sh marge run 'scripts/release-web.sh'` from a
# checkout of the "Release X.Y.Z" commit. `make web` (the WebGPU and the
# WebGL 2 build) and `make web-worlds` (WEB_WORLD_PKGS, signed by the
# checkout's development root in target/web-keys/), then
#   OUT_DIR/OpenCyclingWorld-<version>-<sha>-web.zip
# with the page, worlds/, serve.py (scripts/web-serve.py), the licences and a
# README.md (how to serve it, the trust-root of the included worlds).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
sha=${OCW_SHA:-${OCW_BUILD_ID:-$(git rev-parse --short HEAD 2>/dev/null || echo nogit)}}
case $sha in *-dirty) echo "refusing a release from a dirty tree ($sha)" >&2; exit 1 ;; esac
out=${1:-$HOME/ocw-release-$version}
mkdir -p "$out"
out=$(cd "$out" && pwd)
base=OpenCyclingWorld-$version-$sha-web

rm -rf target/web/worlds
make web
make web-worlds
root=$(cat target/web-keys/root.hex)

dir=$out/$base
rm -rf "$dir"
mkdir -p "$dir"
cp -r target/web/. "$dir/"
install -m 755 scripts/web-serve.py "$dir/serve.py"
install -m 644 LICENSE-MIT LICENSE-APACHE "$dir/"
worlds=$(cd "$dir/worlds" && find . -name latest -printf '%h\n' | sed 's|^\./||' | sort)
{
  cat <<EOF
# Open Cycling World $version – browser app (development build)

Built from commit \`$sha\`. The same game as the desktop app in the browser:
two builds of it, chosen by the page – \`ocw_web_bg.wasm\` (WebGPU: Chrome,
Edge, recent Firefox) and \`ocw_web_webgl2_bg.wasm\` (WebGL 2: Safari, iPhone
and iPad, browsers without a WebGPU adapter; \`?gpu=webgl2\` forces it).

## Serve it

The folder is a static site. Serve it with any HTTP server that sends
\`.wasm\` files as \`application/wasm\` and answers range requests (the
worlds are streamed tile by tile). The included server does both (Python 3,
no packages needed):

    python3 serve.py --dir . --port 8080

and open <http://localhost:8080/>. WebGPU, Web Bluetooth (trainers) and the
microphone need a secure context: **https://** or **http://localhost** /
**http://127.0.0.1**. From another device on the LAN
(\`python3 serve.py --dir . --bind 0.0.0.0\`) the page works only behind
HTTPS. In a source checkout: \`make web-serve\`.

Without options the page rides the built-in test route (Hills). Options go
in the query string, e.g. \`?route=loop&bots=3&power=250&name=Ada\`.
Bluetooth trainers: Chrome or Edge (the panel bottom right); without one the
ride uses virtual power.

## Worlds

Included in \`worlds/\` (the \`world-builder publish\` layout):

EOF
  for w in $worlds; do echo "- \`$w\`"; done
  cat <<EOF

They are signed by a development key of this build, which the page trusts
only on localhost when the URL names its root:

EOF
  for w in $worlds; do
    echo "    http://localhost:8080/?world=${w%%/*}&trust-root=$root"
  done
  cat <<EOF

Add \`&route=ROUTE\` for a route of the package (e.g. \`vaduz-malbun\`) or
\`&free=1\` for a free ride.

**More worlds:** unpack a world zip of the release and publish its package
into \`worlds/\` with \`world-builder\` (in the Linux and Windows downloads):

    world-builder publish path/to/<world>.ocwpkg --out worlds

The release's world packages are not signed; on localhost the page loads
them with \`&allow-unsigned=1\`, e.g.
\`http://localhost:8080/?world=blackforest-north&allow-unsigned=1\`.

## Licences

Code and artwork: MIT / Apache-2.0 (\`LICENSE-MIT\`, \`LICENSE-APACHE\`);
\`livekit-client.umd.js\`: Apache-2.0 (\`livekit-client.LICENSE.txt\`). Map
data: © OpenStreetMap contributors, ODbL 1.0 – the worlds' sources and their
attribution are in each package's manifest and on the app's credits screen;
the OSM extracts are attached to the GitHub release.
EOF
} >"$dir/README.md"
rm -f "$out/$base.zip"
(cd "$out" && zip -qr -9 "$base.zip" "$base")
(cd "$out" && sha256sum "$base.zip" && unzip -l "$base.zip" | tail -1)
echo "trust-root: $root"
