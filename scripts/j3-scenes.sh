#!/usr/bin/env bash
# Before / after scenes of 4.8.8 J3 (with Q6 (d)) on a build machine: the
# junction-* scenes on the J2b packages with a build of main (before,
# OCW_BEFORE_APP, default the deployed /var/tmp/ocw/ocw-desktop) and on the
# J3 packages with this checkout's release build (after), on Wayland and
# X11, into test-output/junctions-j3/{before,after}-{wayland,x11}/.
# Usage: scripts/j3-scenes.sh SCENE,...   (names without -rider / -drone)
set -euo pipefail
D=${OCW_DATA:-$HOME/src/OCW/data}
BEFORE_APP=${OCW_BEFORE_APP:-/var/tmp/ocw/ocw-desktop}
OUT=test-output/junctions-j3
only=""
for s in ${1//,/ }; do only="$only,junction-$s-rider,junction-$s-drone"; done
only=${only#,}
worlds() {
  local dir=$1
  echo --world "liechtenstein=$dir/pilot/liechtenstein.ocwpkg" \
    --world "blackforest-north=$dir/blackforest/blackforest-north.ocwpkg" \
    --world "tenerife=$dir/tenerife/tenerife.ocwpkg"
}
cargo build --release -p ocw-desktop 2>&1 | grep -E "^(error|warning)" -A6 | head -20 || true
for ws in wayland x11; do
  flag=""; [ "$ws" = x11 ] && flag=--x11
  # shellcheck disable=SC2046
  OCW_APP=$BEFORE_APP scripts/render-scenes.sh $flag --only "$only" $(worlds "$D/j2b-build") --out "$OUT/before-$ws" || true
  # shellcheck disable=SC2046
  OCW_PROFILE=release scripts/render-scenes.sh $flag --only "$only" $(worlds "$D/j3-build") --out "$OUT/after-$ws" || true
done
ls -R "$OUT" | head -80
