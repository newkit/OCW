#!/usr/bin/env bash
# The J5 comparison shots on marge (spec 4.8.8 Rule: the sides of a road are
# as smooth as the road): junction corners of the J4 packages (data/j4-build)
# and of the J5 packages (data/j5-build) from the same cameras, Wayland.
# Run it in a checkout of J4's code for the "before" pictures (WHICH=j4) and
# in this one for the "after" pictures (WHICH=j5); VIEWS is a file of lines
#   NAME PACKAGE X Y Z HEADING PITCH START_X,START_Z
# (PACKAGE: pilot/liechtenstein, tenerife/tenerife, blackforest/...).
# ONLY="name ..." shoots only those views.
#   WHICH=j5 VIEWS=scripts/j5-views.txt scripts/j5-shots.sh
set -u
cd "$(dirname "$0")/.."
D=$HOME/src/OCW/data
WHICH=${WHICH:-j5}
VIEWS=${VIEWS:-scripts/j5-views.txt}
OUT=${OUT:-test-output/j5-sides}
mkdir -p "$OUT"
export OCW_SHOT_TIME=${OCW_SHOT_TIME:-17:30}
while read -r name pkg x y z heading pitch start; do
  [ -z "${name:-}" ] && continue
  case $name in \#*) continue ;; esac
  if [ -n "${ONLY:-}" ] && [[ " $ONLY " != *" $name "* ]]; then continue; fi
  P=$D/$WHICH-build/$pkg.ocwpkg
  if scripts/junction-shots.sh "$OUT/$name-$WHICH.jpg" "$P" "$x" "$y" "$z" "$heading" "$pitch" "$start" \
      > "$OUT/.$name-$WHICH.log" 2>&1; then
    echo "ok $name-$WHICH"
  else
    echo "FAILED $name-$WHICH"; tail -5 "$OUT/.$name-$WHICH.log"
  fi
done < "$VIEWS"
ls -la "$OUT"
