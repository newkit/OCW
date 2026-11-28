#!/usr/bin/env bash
# Rebuilds the live world packages for the world-building rule of 2026-11-28
# – no tree or plant on a road surface (spec 4.6.1 *Rule: no plant on a
# road*, `ocw_world::road_clear`) – into data/plants-build/ on a build
# machine, with the step that changed: `world-builder vegetation` (the
# fields, plantations and mapped trees of section VEGE, now without the
# mapped trees on roads; the tree cells and planted trees of OBST renewed
# from the placement, which keeps off every road surface). Every other
# section as it was. Then `world-builder plants-check` (fails on any plant
# on a road) and the size and SHA-256 next to the live package's. The live
# packages are only read; the route files and the recipe are copied beside.
# Usage: scripts/plants-build.sh [PKG...]  (default: the four besides Andorra,
#   which scripts/build-andorra.sh rebuilds whole)
#   OCW_PLANTS_OUT=DIR  another output folder; OCW_SIGN_KEY  sign the manifest
set -euo pipefail
export LC_ALL=C
D=${OCW_DATA:-$HOME/src/OCW/data}
OUT=${OCW_PLANTS_OUT:-data/plants-build}
WB=${WB:-target/release/world-builder}
[ -n "${WB_PREBUILT:-}" ] || cargo build -q --release -p world-builder
pkgs=("$@")
[ ${#pkgs[@]} -gt 0 ] || pkgs=(pilot/liechtenstein tenerife/tenerife blackforest/blackforest-north blackforest/blackforest-middle blackforest/blackforest)
for p in "${pkgs[@]}"; do
  dir=${p%/*}
  mkdir -p "$OUT/$dir"
  cp -p "$D/$dir"/*.route "$OUT/$dir/" 2>/dev/null || true
  echo "== $p"
  start=$(date +%s)
  pbf=()
  for f in "$D/$dir"/*.osm.pbf; do [ -e "$f" ] && pbf+=(--pbf "$f"); done
  [ ${#pbf[@]} -gt 0 ] || { echo "no OSM extract in $D/$dir" >&2; exit 1; }
  # With the measured crop map where the build had one (build-*.sh).
  [ -f "$D/$dir/crops/index.json" ] && pbf+=(--crops "$D/$dir/crops")
  cp -p "$D/$p.ocwpkg" "$OUT/$p.ocwpkg"
  cp -p "$D/$p.ocwpkg.manifest.json" "$OUT/$p.ocwpkg.manifest.json" 2>/dev/null || true
  [ -f "$D/$p.recipe.toml" ] && cp -p "$D/$p.recipe.toml" "$OUT/$p.recipe.toml"
  "$WB" vegetation --world "$OUT/$p.ocwpkg" "${pbf[@]}" > "$OUT/$p.vegetation.log" 2>&1
  grep -E "mapped trees|obstacles|trees" "$OUT/$p.vegetation.log" | tail -3 || tail -3 "$OUT/$p.vegetation.log"
  if [ -n "${OCW_SIGN_KEY:-}" ]; then
    "$WB" sign --key "$OCW_SIGN_KEY" "$OUT/$p.ocwpkg"
  fi
  # No plant on a road: fails the build with the first cases.
  "$WB" plants-check --world "$OUT/$p.ocwpkg" --list 10 | tee "$OUT/$p.plants-check.txt"
  echo "build $(( $(date +%s) - start )) s"
  ls -l "$D/$p.ocwpkg" "$OUT/$p.ocwpkg" | awk '{print $5, $9}'
  sha256sum "$D/$p.ocwpkg" "$OUT/$p.ocwpkg"
done
