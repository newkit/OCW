#!/usr/bin/env bash
# Headless pictures of the unicorn's flight cycles in OCW Studio's Models
# viewer (spec §14 Phase 16, 16B, E3; `studio cycle …`) through
# .claude/skills/run-ocw/drive.sh, on Wayland (headless weston) and X11
# (Xvfb), and a short video of the wing beat and of the landing (frames
# recorded by `studio cycle record`, one per frame of the window):
#
#   scripts/unicorn-cycle-shots.sh [OUT_DIR]      (default test-output/unicorn-cycles)
#
# Writes OUT_DIR/index.md with one row per picture.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
out=${1:-test-output/unicorn-cycles}
mkdir -p "$out"
out=$(cd "$out" && pwd)
export OCW_PACKAGE=ocw-studio OCW_SIZE=${OCW_SIZE:-1600x900} OCW_FPS=0
drive=.claude/skills/run-ocw/drive.sh
model=creatures/unicorn-winged
printf '# OCW Studio – the unicorn'"'"'s flight cycles (spec §14 Phase 16, 16B, E3)\n\n| File | What it shows | Arguments |\n|---|---|---|\n' >"$out/index.md"
row() { printf '| [%s](%s) | %s | `%s` |\n' "$1" "$1" "$2" "$3" >>"$out/index.md"; }
# Mid downstroke of the wing beat (wing phase 0.2 at 1.65 s) and the
# gallop with the legs stretched (gallop phase 0.25 at 2.45 s), paused.
wing=(--model "$model" --cmd "model show gizmos off" --cmd "model orbit 125 12" --cmd "studio cycle wing" --cmd "studio cycle at 1.65")
gallop=(--model "$model" --cmd "model show gizmos off" --cmd "model orbit 90 5" --cmd "studio cycle gallop" --cmd "studio cycle at 2.45")
for ws in wayland x11; do
  mode=shot-$ws
  [ "$ws" = x11 ] && mode=shot
  "$drive" "$mode" "$out/wing-beat-$ws.jpg" 12 -- "${wing[@]}"
  row "wing-beat-$ws.jpg" "The unicorn mid wing-beat (climbing: the downstroke, wing phase 0.2), $ws" "${wing[*]}"
  "$drive" "$mode" "$out/gallop-$ws.jpg" 12 -- "${gallop[@]}"
  row "gallop-$ws.jpg" "The unicorn mid gallop (slow flight: the legs stretched, gallop phase 0.25), $ws" "${gallop[*]}"
done
# Videos: 4 s of the wing beat and one loop of the landing at 25 fps.
for c in wing landing; do
  frames=$out/frames-$c
  rm -rf "$frames"
  args=(--model "$model" --cmd "model show gizmos off" --cmd "model orbit 125 12" --cmd "studio cycle $c" --cmd "studio cycle at $([ "$c" = wing ] && echo 1 || echo 0)" --cmd "studio cycle record $frames 4 25")
  "$drive" shot-wayland "$out/$c-video-end.jpg" 45 -- "${args[@]}"
  n=$(find "$frames" -name 'frame_*.png' | wc -l)
  echo "$c: $n frames"
  ffmpeg -loglevel error -y -framerate 25 -i "$frames/frame_%05d.png" -c:v libx264 -pix_fmt yuv420p -crf 20 "$out/$c-cycle.mp4"
  rm -rf "$frames"
  row "$c-cycle.mp4" "4 s of the $c cycle at 25 fps (frame-stepped, $n frames), Wayland" "${args[*]}"
  row "$c-video-end.jpg" "The window after the $c recording, Wayland" "${args[*]}"
done
echo "pictures in $out"
