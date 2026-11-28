#!/usr/bin/env bash
# Screenshots of the voice status panel (spec 7.1.13) in the desktop app,
# headless (drive.sh shot, Xvfb, 1280x720): the made-up states of
# `dev.voice_hud` on the Hills test route with bots, a different rider, kit
# and bike per picture, `hud.voice full`. The last pictures show the panel under
# several top-right lines (the FPS counter, the Strava badge) and folded to
# icons (`compact`). Each run also asks `dev.voice_hud` for the panel's
# place: "clear under N line(s)" must be in the log, "LAYOUT" never
# (drive.sh turns the FPS counter on: two lines above the panel).
#
#   scripts/shot-voice-hud.sh [OUT_DIR]      (default test-output/voice-hud)
#
# Each picture is taken after OCW_VOICE_SHOT_S seconds (40: a loaded build
# machine stalls at the start, and the timed commands follow the game time).
#
# Exits 0 when every picture was taken and every layout check holds.
set -u
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
out=${1:-test-output/voice-hud}
mkdir -p "$out"
cargo build -q -p ocw-desktop || exit 1
export OCW_APP=target/debug/ocw-desktop OCW_SIZE=${OCW_SIZE:-1280x720}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0

# NAME STATE RIDER [extra --cmd / --at args...]
# RIDER: "Name|weight kg|height cm|bike|frame|jersey|helmet|kit" - another
# rider, kit and bike per picture (made-up names).
shot() {
  local name=$1 state=$2 rider=$3
  shift 3
  local file="$out/desktop-$name.jpg"
  local rn kg cm bike frame jersey helmet kit
  IFS='|' read -r rn kg cm bike frame jersey helmet kit <<<"$rider"
  mkdir -p "$work/$name"
  printf 'active = "%s"\n[[rider]]\nname = "%s"\nweight_kg = %s.0\nheight_cm = %s.0\nbike = "%s"\nframe_color = "%s"\njersey_color = "%s"\nhelmet_color = "%s"\nkit = "%s"\n' \
    "$rn" "$rn" "$kg" "$cm" "$bike" "$frame" "$jersey" "$helmet" "$kit" >"$work/$name/riders.toml"
  OCW_CONFIG_DIR="$work/$name" .claude/skills/run-ocw/drive.sh shot "$file" "${OCW_VOICE_SHOT_S:-40}" -- \
    --route hills --bots 4 --power 240 \
    --cmd "weather.transition instant" --cmd "weather clear" --cmd "time 2026-06-21 17:10" \
    --cmd "hud.voice full" --cmd "dev.voice_hud $state" "$@" \
    --at "12=dev.voice_hud" >"$work/$name.out" 2>&1
  local log="${file%.jpg}.log"
  local line
  line=$(grep -a "voice HUD" "$log" 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' | grep -av "shows:" | tail -n 1)
  if [ ! -s "$file" ]; then
    echo "FAIL $name: no picture"; tail -n 5 "$work/$name.out"; fail=1
  elif [ "$state" = none ]; then
    case "$line" in *"not shown"*) echo "ok   $name: $line" ;; *) echo "FAIL $name: $line"; fail=1 ;; esac
  else
    case "$line" in
      *LAYOUT*|"") echo "FAIL $name: $line"; fail=1 ;;
      *"clear under"*) echo "ok   $name: $line" ;;
      *) echo "FAIL $name: $line"; fail=1 ;;
    esac
  fi
}

shot no-voice     none         "Ada Rider|64|170|road|#1565C0|#FDD835|#212121|band"
shot not-in-call  idle         "Bea Climber|58|166|gravel|#2E7D32|#FAFAFA|#E53935|sash"
shot mic-off      mic-off      "Cleo Pedal|82|188|road|#C62828|#263238|#FAFAFA|chevrons"
shot speaking     speaking     "Dana Rouleur|75|181|gravel|#6A1B9A|#FF7043|#FDD835|stripes"
shot others       others       "Eli Domestique|69|176|road|#F9A825|#1565C0|#FAFAFA|sash"
shot hear-nobody  nobody       "Fay Puncheur|55|162|road|#00838F|#E53935|#212121|band"
shot failed       failed       "Gus Grimpeur|90|192|gravel|#37474F|#8BC34A|#FF7043|chevrons"
shot reconnecting reconnecting "Hana Sprinter|61|168|road|#AD1457|#FAFAFA|#1565C0|stripes"
# Under several lines: the time stamp, the Strava badge, the FPS counter.
shot lines-several others      "Ivo Baroudeur|78|184|gravel|#EF6C00|#212121|#8BC34A|band" \
  --at "5=strava.record start"
# Folded to icons: compact, nothing changes for a few seconds.
shot compact-icons mic-off     "Jule Rider|66|174|road|#283593|#FDD835|#E53935|sash" \
  --at "1=hud.voice compact"
exit $fail
