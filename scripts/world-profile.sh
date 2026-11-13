#!/usr/bin/env bash
# World build profile (spec §4.7, "Build profile"): builds world packages
# into a scratch directory with the world builder's stage profile, the
# same arguments as scripts/build-{liechtenstein,blackforest,tenerife}.sh
# (no routes, unsigned), and compares them with reference packages.
#
#   scripts/world-profile.sh [--data DIR] [--out DIR] [--wb BIN]
#                            [--ref DIR] [WORLD...]
#
#   WORLD    liechtenstein, bf-south, bf-middle, bf-north, tenerife
#            (default: liechtenstein bf-north tenerife)
#   --data   the source data (default: data; e.g. ~/src/OCW/data on a build
#            machine for the main checkout's data, read only)
#   --out    output directory (default: test-output/world-profile/LABEL)
#   --label  name of the run (default: the date and time)
#   --wb     world-builder binary (default: build target/release/world-builder)
#   --no-profile  leave out --profile (a world-builder from before v2.49,
#            to build a reference package with the old code)
#   --ref    directory with reference packages WORLD.ocwpkg (e.g. the --out
#            of an earlier run): each new package is compared byte for byte
#
# Writes OUT/WORLD.ocwpkg, OUT/WORLD.log (the builder's output with the stage
# table), OUT/WORLD.profile.json and OUT/measure.log (wall time and peak RSS
# per world, scripts/measure.py); prints the stage tables and the
# comparison (`identical` or `DIFFERENT`).
set -euo pipefail
# Decimal points in the numbers (a German locale prints 8,0).
export LC_ALL=C
cd "$(dirname "$0")/.."
data=data out="" wb="" ref="" label=$(date +%Y%m%d-%H%M%S) profile=1
worlds=()
while [ $# -gt 0 ]; do
  case "$1" in
    --data) data=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --label) label=$2; shift 2 ;;
    --wb) wb=$2; shift 2 ;;
    --ref) ref=$2; shift 2 ;;
    --no-profile) profile=0; shift ;;
    -h | --help) sed -n '2,/^[^#]/{/^#/p}' "$0"; exit 0 ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) worlds+=("$1"); shift ;;
  esac
done
[ ${#worlds[@]} -gt 0 ] || worlds=(liechtenstein bf-north tenerife)
out=${out:-test-output/world-profile/$label}
mkdir -p "$out"
if [ -z "$wb" ]; then
  cargo build -q --release -p world-builder
  wb=target/release/world-builder
fi

args_of() {
  local P=$data/pilot B=$data/blackforest T=$data/tenerife
  case "$1" in
    liechtenstein)
      echo --pbf "$P/liechtenstein.osm.pbf" --poly "$P/liechtenstein.poly" --hgt "$P/N47E009.hgt" \
        --dem "$P/swissalti3d" --dem-attribution swissalti3d --dem-attribution srtm --name Liechtenstein ;;
    bf-south | bf-middle | bf-north)
      local box
      case "$1" in
        bf-south) box=47.72,7.62,48.12,8.30 ;;
        bf-middle) box=48.12,7.70,48.45,8.40 ;;
        bf-north) box=48.45,7.85,48.80,8.45 ;;
      esac
      echo --pbf "$B/freiburg-regbez.osm.pbf" --pbf "$B/karlsruhe-regbez.osm.pbf" \
        --poly "$B/freiburg-regbez.poly" --poly "$B/karlsruhe-regbez.poly" \
        --hgt "$B/N47E007.hgt" --hgt "$B/N47E008.hgt" --hgt "$B/N48E007.hgt" --hgt "$B/N48E008.hgt" \
        --dem "$B/dgm1" --bbox "$box" --origin 47.92,7.96 --name "$1" \
        --dem-attribution lgl-dgm1 --dem-attribution srtm ;;
    tenerife)
      echo --pbf "$T/canary-islands.osm.pbf" --poly "$T/canary-islands.poly" \
        --hgt "$T/N28W017.hgt" --hgt "$T/N27W017.hgt" --dem "$T/mdt05" \
        --bbox 27.98,-16.94,28.61,-16.10 --origin 28.29,-16.52 --name Tenerife --tiles-near-roads \
        --dem-attribution ign-mdt05 --dem-attribution srtm ;;
    *) echo "unknown world $1" >&2; return 1 ;;
  esac
}

status=0
for w in "${worlds[@]}"; do
  read -r -a a <<<"$(args_of "$w")"
  pkg=$out/$w.ocwpkg
  prof=()
  [ "$profile" = 0 ] || prof=(--profile "$out/$w.profile.json")
  scripts/measure.py --label "$w" --out "$out/measure.log" -- \
    "$wb" package "${a[@]}" --out "$pkg" "${prof[@]}" >"$out/$w.log" 2>&1 || status=1
  echo "== $w: $(tail -n 1 "$out/measure.log")"
  sed -n '/^build profile/,$p' "$out/$w.log"
  if [ -n "$ref" ] && [ -f "$ref/$w.ocwpkg" ]; then
    if cmp -s "$ref/$w.ocwpkg" "$pkg"; then echo "$w: identical to $ref/$w.ocwpkg"
    else echo "$w: DIFFERENT from $ref/$w.ocwpkg"; status=1; fi
  fi
done
exit $status
