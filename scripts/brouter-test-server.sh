#!/usr/bin/env bash
# Our own BRouter for the planner tests (spec 14 Phase 16, 16D; 16H Q22 (a)):
# its routing data (`rd5` segments) built with BRouter's own map tool from
# our OSM extract and elevation data, the server only running while a test
# needs it. Never downloads brouter.de's ready segment files.
#
#   scripts/brouter-test-server.sh fetch      BRouter (pinned GitHub release,
#                                             SHA-256 checked) and the two
#                                             map-creation profiles of its tag
#   scripts/brouter-test-server.sh build      the segments (cached: rebuilt
#                                             only when an input changes)
#   scripts/brouter-test-server.sh start|stop|status
#   scripts/brouter-test-server.sh run CMD... build, start, run CMD with
#                                             OCW_BROUTER_URL and
#                                             OCW_BROUTER_PACKAGE set, and
#                                             stop the server afterwards
#                                             (also when CMD fails or is
#                                             interrupted)
#   scripts/brouter-test-server.sh config     print the settings in use
#
# `make test-studio-planners` runs `run` with the studio's live planner test.
#
# The data build (what process_pbf_planet.sh of BRouter's tag does, on one
# extract):
#   1. ElevationRasterTileConverter: our SRTM 1" tiles (`N47E009.hgt`, the DEM
#      the world builder uses outside swissALTI3D) -> BRouter's 5 x 5 degree
#      elevation tiles (`srtm_38_03.bef`); the SRTM tile names come from the
#      .hgt files found, so other regions need no change here;
#   2. OsmFastCutter: the OSM extract -> node and way tiles (lookups.dat and
#      all / trekking / softaccess.brf of the same tag);
#   3. PosUnifier: positions unified, heights from step 1;
#   4. WayLinker: the `rd5` segment files.
# The result goes to CACHE/segments-KEY/ (KEY: a hash of the BRouter
# version and the inputs' SHA-256) with build.json (inputs, time, size);
# older builds are removed.
#
# Settings (machine-specific paths) come from a config file of KEY=VALUE
# lines, the first one found of: $OCW_BROUTER_CONFIG, config/brouter-test.conf
# (this checkout; config/ is never copied to build machines),
# ${XDG_CONFIG_HOME:-~/.config}/ocw/brouter-test.conf. The keys and their
# defaults are in config/brouter-test.example.conf; environment variables
# of the same names win over the file. A leading `~/` is the home folder.
#
# Licence: BRouter is MIT (its LICENSE is kept next to the jar in the cache);
# the segments are derived from OSM data (ODbL) and SRTM (public domain) and
# stay on the build machine.
set -euo pipefail
cd "$(dirname "$0")/.."

# --- Pinned BRouter (the version, the release asset's and the profiles'
# SHA-256 and the tag's commit change together; the asset's digest is the
# one GitHub lists for the release, measured again 2026-11-26).
version=1.7.10
commit=4d2639af77ea5ed9c30d3e400764eb6f9e8522da # tag v1.7.10
zip_sha=023fec3ba997758e8cd7ab9e1bae52e962af3f00b57683e3de86b84ffad01532
zip_url=https://github.com/abrensch/brouter/releases/download/v$version/brouter-$version.zip
raw=https://raw.githubusercontent.com/abrensch/brouter/$commit
# Map-creation profiles, not in the release zip: path and SHA-256.
profiles=(
  "misc/profiles2/all.brf 87d49d6a2850db277e2c86a81de5e2e6727e0498e47f10662ababa0a9b56f144"
  "misc/profiles2/softaccess.brf 0c04a5889d61bef88b62a8abd22e76555b2d3a2299e33811b2f7e15ac7587190"
)

# --- Settings.
expand() { case "$1" in "~/"*) echo "$HOME/${1#\~/}" ;; "~") echo "$HOME" ;; *) echo "$1" ;; esac; }
config_file=""
for c in "${OCW_BROUTER_CONFIG:-}" config/brouter-test.conf "${XDG_CONFIG_HOME:-$HOME/.config}/ocw/brouter-test.conf"; do
  if [ -n "$c" ] && [ -f "$c" ]; then config_file=$c; break; fi
done
if [ -n "${OCW_BROUTER_CONFIG:-}" ] && [ "$config_file" != "$OCW_BROUTER_CONFIG" ]; then
  echo "OCW_BROUTER_CONFIG: no such file" >&2; exit 2
fi
declare -A conf=()
if [ -n "$config_file" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%%#*}
    line=$(echo "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    [ -z "$line" ] && continue
    case "$line" in
      BROUTER_[A-Z]*=*) conf[${line%%=*}]=${line#*=} ;;
      *) echo "$config_file: not a BROUTER_KEY=VALUE line: $line" >&2; exit 2 ;;
    esac
  done <"$config_file"
fi
setting() { # KEY DEFAULT: the environment, else the file, else DEFAULT
  local v=${!1:-}
  [ -z "$v" ] && v=${conf[$1]:-}
  [ -z "$v" ] && v=$2
  expand "$v"
}
data=$(setting BROUTER_DATA data/pilot)
pbf=$(setting BROUTER_PBF "$data/liechtenstein.osm.pbf")
dem=$(setting BROUTER_DEM "$data")
package=$(setting BROUTER_PACKAGE "$data/liechtenstein.ocwpkg")
cache=$(setting BROUTER_CACHE "${XDG_CACHE_HOME:-$HOME/.cache}/ocw/brouter-test")
port=$(setting BROUTER_PORT 17777)
bind=$(setting BROUTER_BIND 127.0.0.1)
java=$(setting BROUTER_JAVA java)
xmx=$(setting BROUTER_XMX 3g)

vendor=$cache/vendor
home=$cache/brouter-$version
jar=$home/brouter-$version-all.jar
cp_all="$jar:$home/lib/*"
pidfile=$cache/server-$port.pid
logfile=$cache/server-$port.log

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
say() { echo "brouter-test: $*" >&2; }

fetch_one() { # URL FILE SHA256
  local url=$1 file=$2 want=$3
  if [ -f "$file" ] && [ "$(sha256 "$file")" = "$want" ]; then return 0; fi
  say "fetching $(basename "$file")"
  curl -sSfL --retry 3 -o "$file.part" "$url"
  if [ "$(sha256 "$file.part")" != "$want" ]; then
    rm -f "$file.part"
    say "$(basename "$file"): the download does not match its SHA-256"; return 1
  fi
  mv "$file.part" "$file"
}

do_fetch() {
  mkdir -p "$vendor"
  fetch_one "$zip_url" "$vendor/brouter-$version.zip" "$zip_sha"
  for p in "${profiles[@]}"; do
    set -- $p
    fetch_one "$raw/$1" "$vendor/$(basename "$1")" "$2"
  done
  if [ ! -f "$jar" ] || [ ! -f "$home/.ok" ]; then
    local tmp
    tmp=$(mktemp -d "$cache/unpack.XXXXXX")
    if command -v unzip >/dev/null 2>&1; then unzip -q "$vendor/brouter-$version.zip" -d "$tmp"
    else python3 -I -m zipfile -e "$vendor/brouter-$version.zip" "$tmp"; fi
    rm -rf "$home"
    mv "$tmp/brouter-$version" "$home"
    rmdir "$tmp"
    mkdir -p "$home/customprofiles"
    # The licence of the tag, next to the jar.
    fetch_licence
    touch "$home/.ok"
  fi
  command -v "$java" >/dev/null || { say "no Java ($java; BROUTER_JAVA): BRouter needs Java 11 or newer"; return 1; }
}

fetch_licence() {
  [ -f "$home/LICENSE" ] && return 0
  curl -sSfL --retry 3 -o "$home/LICENSE" "$raw/LICENSE" || say "warning: BRouter's LICENSE not fetched"
}

# The SRTM tiles BRouter wants for our .hgt files (srtm_XX_YY, 5 x 5 degrees).
srtm_tiles() {
  local f n lat lon latb lonb
  for f in "$dem"/*.hgt; do
    [ -f "$f" ] || continue
    n=$(basename "$f" .hgt)
    [[ $n =~ ^([NS])([0-9]{2})([EW])([0-9]{3})$ ]] || continue
    lat=$((10#${BASH_REMATCH[2]})); [ "${BASH_REMATCH[1]}" = S ] && lat=$((-lat))
    lon=$((10#${BASH_REMATCH[4]})); [ "${BASH_REMATCH[3]}" = W ] && lon=$((-lon))
    latb=$(( (lat + 90) / 5 * 5 - 90 )); lonb=$(( (lon + 180) / 5 * 5 - 180 ))
    printf 'srtm_%02d_%02d\n' $(( (lonb + 180) / 5 + 1 )) $(( (60 - latb) / 5 ))
  done | sort -u
}

# The arc seconds of the first .hgt (1" tiles have 3601 rows).
hgt_arcsec() {
  local f
  for f in "$dem"/*.hgt; do
    [ -f "$f" ] || continue
    if [ "$(stat -c %s "$f")" = $((3601 * 3601 * 2)) ]; then echo 1; else echo 3; fi
    return
  done
}

inputs_key() {
  {
    echo "brouter $version $zip_sha"
    for p in "${profiles[@]}"; do echo "$p"; done
    echo "pbf $(sha256 "$pbf")"
    local f
    for f in "$dem"/*.hgt; do [ -f "$f" ] && echo "hgt $(basename "$f") $(sha256 "$f")"; done
    echo "recipe 1"
  } | sha256sum | cut -c1-16
}

segments_dir() { echo "$cache/segments-$(inputs_key)"; }

do_build() {
  do_fetch
  [ -f "$pbf" ] || { say "no OSM extract (BROUTER_PBF; see config/brouter-test.example.conf)"; return 1; }
  local tiles key out
  tiles=$(srtm_tiles)
  [ -n "$tiles" ] || { say "no SRTM .hgt tiles in the DEM folder (BROUTER_DEM)"; return 1; }
  key=$(inputs_key)
  out=$cache/segments-$key
  if [ -f "$out/build.json" ] && compgen -G "$out/*.rd5" >/dev/null; then
    say "segments up to date ($(du -sh "$out" | cut -f1), key $key)"
    return 0
  fi
  local work t0 t1 arcsec
  # A failed build's folder (with its logs) stays until the next build.
  find "$cache" -maxdepth 1 -name 'build.*' -exec rm -rf {} +
  work=$(mktemp -d "$cache/build.XXXXXX")
  t0=$(date +%s)
  arcsec=$(hgt_arcsec)
  local p2=$home/profiles2
  mkdir -p "$work"/{bef,nodetiles,waytiles,waytiles55,nodes55,unodes55,segments}
  for t in $tiles; do
    say "elevation: $t from the ${arcsec}\" .hgt tiles"
    (cd "$work" && "$java" -Xmx"$xmx" -cp "$cp_all" btools.mapcreator.ElevationRasterTileConverter \
      "$t" "$(realpath "$dem")" bef "$arcsec") >"$work/elevation.log" 2>&1 \
      || { cat "$work/elevation.log" >&2; return 1; }
  done
  say "cutting the OSM extract"
  (cd "$work" && "$java" -Xmx"$xmx" -cp "$cp_all" -Ddeletetmpfiles=true -DuseDenseMaps=true \
    btools.mapcreator.OsmFastCutter "$p2/lookups.dat" nodetiles waytiles nodes55 waytiles55 \
    bordernids.dat relations.dat restrictions.dat "$vendor/all.brf" "$p2/trekking.brf" \
    "$vendor/softaccess.brf" "$(realpath "$pbf")") >"$work/cut.log" 2>&1 \
    || { tail -40 "$work/cut.log" >&2; return 1; }
  say "unifying positions, heights"
  (cd "$work" && "$java" -Xmx"$xmx" -cp "$cp_all" -Ddeletetmpfiles=true -DuseDenseMaps=true \
    btools.mapcreator.PosUnifier nodes55 unodes55 bordernids.dat bordernodes.dat bef) \
    >"$work/unify.log" 2>&1 || { tail -40 "$work/unify.log" >&2; return 1; }
  say "linking ways into segments"
  (cd "$work" && "$java" -Xmx"$xmx" -cp "$cp_all" -DuseDenseMaps=true -DskipEncodingCheck=true \
    btools.mapcreator.WayLinker unodes55 waytiles55 bordernodes.dat restrictions.dat \
    "$p2/lookups.dat" "$vendor/all.brf" segments rd5) >"$work/link.log" 2>&1 \
    || { tail -40 "$work/link.log" >&2; return 1; }
  compgen -G "$work/segments/*.rd5" >/dev/null || { say "the map tool wrote no .rd5"; return 1; }
  t1=$(date +%s)
  rm -rf "$out.part"
  mv "$work/segments" "$out.part"
  local rd5 bytes
  rd5=$(cd "$out.part" && ls -- *.rd5 | tr '\n' ' ')
  bytes=$(cat "$out.part"/*.rd5 | wc -c)
  cat >"$out.part/build.json" <<EOF
{
  "brouter": "$version",
  "commit": "$commit",
  "pbf": "$(basename "$pbf")",
  "pbf_sha256": "$(sha256 "$pbf")",
  "srtm_tiles": "$(echo $tiles)",
  "hgt_arcsec": $arcsec,
  "rd5": "${rd5% }",
  "rd5_bytes": $bytes,
  "seconds": $((t1 - t0)),
  "built": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
  # Only the newest build stays.
  find "$cache" -maxdepth 1 -name 'segments-*' ! -name "segments-$key.part" -exec rm -rf {} +
  mv "$out.part" "$out"
  rm -rf "$work"
  say "segments built in $((t1 - t0)) s: ${rd5% } ($bytes bytes, key $key)"
}

server_pid() {
  [ -f "$pidfile" ] || return 1
  local pid
  pid=$(cat "$pidfile")
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && grep -q RouteServer "/proc/$pid/cmdline" 2>/dev/null \
    && echo "$pid"
}

url() { echo "http://$bind:$port"; }

# A route query near the data (Vaduz – Triesen): any HTTP answer means the
# server is up.
probe() { curl -s -o /dev/null -m 5 -w '%{http_code}' "$(url)/brouter?lonlats=9.5209,47.1410%7C9.5280,47.1080&profile=trekking&alternativeidx=0&format=geojson" 2>/dev/null; }

do_start() {
  local seg pid
  seg=$(segments_dir)
  compgen -G "$seg/*.rd5" >/dev/null || { say "no segments: run \`$0 build\` first"; return 1; }
  if pid=$(server_pid); then say "already running (pid $pid) at $(url)"; return 0; fi
  if [ "$(probe)" != "000" ]; then say "port $port is taken by another program (BROUTER_PORT)"; return 1; fi
  mkdir -p "$cache"
  # setsid: the server leaves the caller's process group, and `stop` ends it.
  setsid "$java" -Xmx256M -Xms64M -DmaxRunningTime=60 -DuseRFCMimeType=false -cp "$jar" \
    btools.server.RouteServer "$seg" "$home/profiles2" "$home/customprofiles" "$port" 2 "$bind" \
    >"$logfile" 2>&1 </dev/null &
  echo $! >"$pidfile"
  local i
  for i in $(seq 1 120); do
    if [ "$(probe)" != 000 ]; then say "started (pid $(cat "$pidfile")) at $(url)"; return 0; fi
    if ! server_pid >/dev/null; then tail -20 "$logfile" >&2; say "the server ended at once"; rm -f "$pidfile"; return 1; fi
    sleep 0.5
  done
  say "the server did not answer within 60 s"; tail -20 "$logfile" >&2
  do_stop; return 1
}

do_stop() {
  local pid i
  if pid=$(server_pid); then
    kill "$pid" 2>/dev/null || true
    for i in $(seq 1 40); do kill -0 "$pid" 2>/dev/null || break; sleep 0.25; done
    kill -9 "$pid" 2>/dev/null || true
    say "stopped (pid $pid)"
  fi
  rm -f "$pidfile"
}

do_config() {
  echo "config: ${config_file:-(none; defaults)}"
  echo "BROUTER_DATA=$data"; echo "BROUTER_PBF=$pbf"; echo "BROUTER_DEM=$dem"
  echo "BROUTER_PACKAGE=$package"; echo "BROUTER_CACHE=$cache"
  echo "BROUTER_PORT=$port"; echo "BROUTER_BIND=$bind"; echo "BROUTER_JAVA=$java"; echo "BROUTER_XMX=$xmx"
  echo "brouter $version (commit $commit)"
}

cmd=${1:-}
[ $# -gt 0 ] && shift
mkdir -p "$cache"
case "$cmd" in
  fetch) do_fetch ;;
  build) do_build ;;
  start) do_start ;;
  stop) do_stop ;;
  status)
    if pid=$(server_pid); then echo "running (pid $pid) at $(url)"; else echo "stopped"; exit 1; fi ;;
  config) do_config ;;
  run)
    [ $# -gt 0 ] || { echo "usage: $0 run CMD..." >&2; exit 2; }
    # One run at a time per cache (the port is shared).
    exec 9>"$cache/run.lock"
    flock 9
    do_build
    trap do_stop EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    do_start
    seg=$(segments_dir)
    say "data: $(tr -d '\n' <"$seg/build.json" | sed 's/  */ /g')"
    rc=0
    OCW_BROUTER_URL=$(url) OCW_BROUTER_PACKAGE=$package "$@" || rc=$?
    exit $rc ;;
  *) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
