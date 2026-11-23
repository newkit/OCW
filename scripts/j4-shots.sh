#!/usr/bin/env bash
# The J4 (spec 4.8.8 Q7) comparison shots on marge: the three crease
# examples with live / J3 / J4, the terrain before (J3) / after (J4).
# ONLY="name ..." shoots only those views.
set -u
cd "$(dirname "$0")/.."
D=$HOME/src/OCW/data
LIVE=$D/pilot/liechtenstein.ocwpkg
J3=$D/j3-build/pilot/liechtenstein.ocwpkg
J4=$D/j4-build/pilot/liechtenstein.ocwpkg
OUT=test-output/j4-q7
mkdir -p $OUT
shot() { # name pkgs x y z heading pitch start
  local name=$1 pkgs=$2; shift 2
  if [ -n "${ONLY:-}" ] && [[ " $ONLY " != *" $name "* ]]; then return; fi
  for pk in $pkgs; do
    case $pk in live) P=$LIVE;; j3) P=$J3;; j4) P=$J4;; esac
    if scripts/junction-shots.sh $OUT/$name-$pk.jpg $P "$@" > $OUT/.$name-$pk.log 2>&1; then echo "ok $name-$pk"; else echo "FAILED $name-$pk"; tail -5 $OUT/.$name-$pk.log; fi
  done
}
export OCW_SHOT_TIME=17:30
shot a-side "live j3 j4" -1437.7 554.38 5970.6 2.1 -6 -1440,5868
shot a-approach "live j3 j4" -1445.0 554.87 5965.3 67.7 -4 -1440,5868
shot a-rider "live j3 j4" -1449.3 554.27 5958.1 92.1 -3 -1446,6048
shot b-side "live j3 j4" -623.4 1162.06 3355.7 244.7 -5 -670,3289
shot b-rider "live j3 j4" -625.8 1162.57 3383.4 334.7 -3 -593,3452
shot c-side "live j3 j4" -2755.4 492.79 -5881.6 95.4 -6 -2799,-5960
shot c-rider "live j3 j4" -2738.7 493.97 -5899.1 185.4 -4 -2680,-5826
shot c-up "live j3 j4" -2742.3 491.42 -5861.3 5.4 3 -2680,-5826
export OCW_SHOT_TIME=19:45
shot terrain-edge "j3 j4" -3310 590 -6240 0 -18 -3310,-6310
export OCW_SHOT_TIME=07:15
shot terrain-meadow "j3 j4" -3310 562 -6240 10 -9 -3310,-6310
ls -la $OUT
