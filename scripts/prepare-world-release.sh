#!/usr/bin/env bash
# Prepares the release zips of world packages (spec §4.7, §11 "Releases on
# GitHub Releases with the world package"), without uploading anything:
#
#   scripts/prepare-world-release.sh OUT_DIR PKG...
#
# For every package PKG (e.g. data/blackforest/blackforest-north.ocwpkg):
#   OUT_DIR/ocw-world-<stem>-v<N>.zip   the package, its manifest, the route
#       files beside it (the ones named in its routes), LICENSE-DATA.md (every
#       source with its attribution, from the manifest) and README.md;
# and per source folder (data/pilot, data/blackforest) once:
#   OUT_DIR/<region>-osm-source.zip     the OSM extract(s) and .poly files
#       the packages were built from, with SOURCES.md: the ODbL offer of the
#       Derivative Database (road graph) in the packages.
# N is the manifest's version. Needs python3 and zip; run on the build
# machine after scripts/build-*.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: prepare-world-release.sh OUT_DIR PKG...}; shift
[ $# -gt 0 ] || { echo "no packages" >&2; exit 2; }
mkdir -p "$out"
out=$(cd "$out" && pwd)
cargo build -q --release -p world-builder
WB=$PWD/target/release/world-builder
declare -A done_src=()
for pkg in "$@"; do
  dir=$(dirname "$pkg"); stem=$(basename "$pkg" .ocwpkg)
  manifest=$pkg.manifest.json
  [ -f "$manifest" ] || { echo "$manifest missing" >&2; exit 1; }
  region=$(basename "$dir"); [ "$region" = pilot ] && region=liechtenstein
  # id, version, name, attributions from the signed part of the manifest.
  eval "$(python3 -I - "$manifest" <<'PY'
import json, shlex, sys
m = json.load(open(sys.argv[1]))
s = m["signed"]
if isinstance(s, str):
    s = json.loads(s)
print(f"id={shlex.quote(s['id'])} version={s['version']} name={shlex.quote(s['name'])}")
print("bounds=" + shlex.quote(", ".join(f"{k} {v}" for k, v in (s.get("bounds") or {}).items())))
PY
)"
  name_v=ocw-world-$stem-v$version
  stage=$out/$name_v
  rm -rf "$stage"; mkdir -p "$stage"
  cp "$pkg" "$manifest" "$stage/"
  routes=$($WB routes --world "$pkg")
  # Route files beside the package that belong to it (see build-*.sh).
  for f in "$dir"/*.route; do
    [ -e "$f" ] || continue
    rname=$(head -3 "$f" | sed -n 's/^name //p')
    if grep -qF -- "$rname:" <<<"$routes"; then cp "$f" "$stage/"; fi
  done
  python3 -I - "$manifest" "$stage/LICENSE-DATA.md" "$name ($id, version $version)" "$bounds" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
s = m["signed"] if not isinstance(m["signed"], str) else json.loads(m["signed"])
lines = [f"# {sys.argv[3]} – data licence", "",
         f"Region box: {sys.argv[4]}. Built by `{s.get('generator', 'world-builder')}` (Open Cycling World).", "",
         "Sources and their attribution (also shown on the app's credits screen):", ""]
for a in s.get("attribution", []):
    url = f" ({a['url']})" if a.get("url") else ""
    lines.append(f"- **{a['source']}** – {a['licence']}{url}: {a['notice']}")
lines += ["",
          "- **OpenStreetMap / ODbL:** the road graph (section `GRPH`, with the routes `ROUT`,",
          "  land use `LUSE`, buildings `BLDG` and region `REGN`) and the route files are a",
          "  Derivative Database under the Open Database License 1.0",
          "  (https://opendatacommons.org/licenses/odbl/1-0/) and are made available under it.",
          "  The OSM extract(s) they were built from are offered in this release",
          "  (`*-osm-source.zip`). Terrain, meshes and rendered views are Produced Works and",
          "  carry \"© OpenStreetMap contributors\".",
          "- **Elevation models:** the terrain and road heights were derived from the models",
          "  above (averaged, smoothed along the roads and adapted to them), i.e. modified.",
          "",
          "The Open Cycling World code that reads this package is dual MIT / Apache-2.0;",
          "those licences do not apply to this data.", ""]
open(sys.argv[2], "w").write("\n".join(lines))
PY
  {
    echo "# Open Cycling World – $name (version $version)"
    echo
    echo "Start the app with the package (keep the manifest next to it):"
    echo
    echo "    ocw-desktop --world path/to/$stem.ocwpkg                 # rides the first route"
    echo "    ocw-desktop --world path/to/$stem.ocwpkg --free-ride     # ride anywhere"
    echo
    echo "Packages of neighbouring regions in the same folder are joined at the"
    echo "border (free ride across, spec §2.2.2). Routes in the package:"
    echo
    sed 's/^/    /' <<<"$routes"
    echo
    echo "The package is not signed (development build); the app loads it from a"
    echo "local path with a warning. Licence: LICENSE-DATA.md."
  } > "$stage/README.md"
  (cd "$out" && rm -f "$name_v.zip" && zip -q -r "$name_v.zip" "$name_v")
  echo "$out/$name_v.zip ($(du -h "$out/$name_v.zip" | cut -f1))"
  # The OSM source of this folder, once.
  if [ -z "${done_src[$dir]:-}" ]; then
    done_src[$dir]=1
    src=$region-osm-source
    rm -rf "${out:?}/$src"; mkdir -p "$out/$src"
    cp "$dir"/*.osm.pbf "$dir"/*.poly "$out/$src/"
    cp "$dir/SOURCES.md" "$out/$src/SOURCES.md"
    (cd "$out" && rm -f "$src.zip" && zip -q -r "$src.zip" "$src")
    echo "$out/$src.zip ($(du -h "$out/$src.zip" | cut -f1))"
  fi
done
