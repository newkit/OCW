#!/usr/bin/env bash
# Collects screenshots and videos from agent worktrees (or any other
# checkouts) into this checkout's test-output/ archive (see the run-ocw
# skill's drive.sh): the build history in test-output/index.md.
#
#   scripts/collect-test-output.sh [CHECKOUT...]
#       (default: every .claude/worktrees/*)
#
# - Archived runs (test-output/YYYY-MM-DD/*) are copied as they are and
#   their index rows added (rows already present are skipped).
# - Other files there (copied by hand: *.png, *.mp4, *.jpg, *.gif, *.webm,
#   also in subfolders) go to test-output/YYYY-MM-DD/<checkout>/ with one
#   index row per folder (date and time from the newest file, commit = the
#   checkout's HEAD, mode "collected").
# Existing files are never overwritten, so running it again is harmless.
set -euo pipefail
cd "$(dirname "$0")/.."
root=${OCW_TEST_OUTPUT:-$PWD/test-output}
index=$root/index.md
mkdir -p "$root"
if [ ! -f "$index" ]; then
  printf '# OCW test output\n\n| Date | Time | Commit | Mode | Window system | App arguments | Files |\n|---|---|---|---|---|---|---|\n' >"$index"
fi

if [ $# -eq 0 ]; then
  set -- .claude/worktrees/*
fi

added_rows=0 copied=0
for co in "$@"; do
  src=$co/test-output
  [ -d "$src" ] || continue
  [ "$(cd "$src" && pwd -P)" = "$(cd "$root" && pwd -P)" ] && continue
  name=$(basename "$co")
  commit=$(git -C "$co" rev-parse --short HEAD 2>/dev/null || echo unknown)

  # 1. Archived runs and their index rows.
  for d in "$src"/[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]; do
    [ -d "$d" ] || continue
    mkdir -p "$root/$(basename "$d")"
    for f in "$d"/*; do
      [ -f "$f" ] || continue
      t="$root/$(basename "$d")/$(basename "$f")"
      [ -e "$t" ] || { cp -p "$f" "$t"; copied=$((copied + 1)); }
    done
  done
  if [ -f "$src/index.md" ]; then
    while IFS= read -r row; do
      case "$row" in "| 20"*) ;; *) continue ;; esac
      grep -qxF -- "$row" "$index" || { printf '%s\n' "$row" >>"$index"; added_rows=$((added_rows + 1)); }
    done <"$src/index.md"
  fi

  # Archived files whose index row stayed on the build machine (fetched
  # without index.md): one row per run (same HHMMSS-commit-mode stem).
  for d in "$src"/[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]; do
    [ -d "$d" ] || continue
    day=$(basename "$d")
    declare -A runs=()
    for f in "$d"/*; do
      b=$(basename "$f")
      grep -qF -- "($day/$b)" "$index" && continue
      if [[ $b =~ ^([0-9]{6})-(.+)-(spike-video|spike|shot-wayland|video-wayland|shot|video)-(.+)$ ]]; then
        stem="${BASH_REMATCH[1]}-${BASH_REMATCH[2]}-${BASH_REMATCH[3]}"
        runs[$stem]+="[${BASH_REMATCH[4]}]($day/$b) "
      fi
    done
    for stem in "${!runs[@]}"; do
      [[ $stem =~ ^([0-9]{2})([0-9]{2})([0-9]{2})-(.+)-(spike-video|spike|shot-wayland|video-wayland|shot|video)$ ]] || continue
      ws=x11; case "${BASH_REMATCH[5]}" in *wayland | spike*) ws=wayland ;; esac
      printf '| %s | %s:%s:%s | %s | %s | %s | `-` (from %s) | %s |\n' "$day" "${BASH_REMATCH[1]}" \
        "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}" "${BASH_REMATCH[4]}" "${BASH_REMATCH[5]}" "$ws" "$name" \
        "${runs[$stem]% }" >>"$index"
      added_rows=$((added_rows + 1))
    done
    unset runs
  done

  # 2. Loose media files, grouped by folder.
  while IFS= read -r dir; do
    files=()
    while IFS= read -r f; do files+=("$f"); done < <(
      find "$dir" -maxdepth 1 -type f \( -iname '*.png' -o -iname '*.mp4' -o -iname '*.jpg' \
        -o -iname '*.gif' -o -iname '*.webm' -o -iname '*.log' \) | sort)
    media=0
    for f in ${files[@]+"${files[@]}"}; do case "$f" in *.log) ;; *) media=1 ;; esac; done
    [ "$media" = 1 ] || continue
    newest=$(ls -t "${files[@]}" | head -1)
    day=$(date -r "$newest" +%F); time=$(date -r "$newest" +%T)
    rel=${dir#"$src"}; rel=${rel#/}
    sub="$day/$name${rel:+/$rel}"
    mkdir -p "$root/$sub"
    links=""
    for f in "${files[@]}"; do
      b=$(basename "$f")
      [ -e "$root/$sub/$b" ] || { cp -p "$f" "$root/$sub/$b"; copied=$((copied + 1)); }
      links+="[$b]($sub/$b) "
    done
    row="| $day | $time | $commit | collected | - | \`${rel:-test-output}\` from $name | ${links% } |"
    grep -qF -- "($sub/" "$index" || { printf '%s\n' "$row" >>"$index"; added_rows=$((added_rows + 1)); }
  done < <(find "$src" -type d ! -path "$src/[0-9][0-9][0-9][0-9]-*" | sort)
done
# Rows in time order (date, then time); the header stays on top.
{
  grep -v '^| 20' "$index"
  grep '^| 20' "$index" | sort -t'|' -k2,2 -k3,3 -s
} >"$index.tmp" && mv "$index.tmp" "$index"
echo "collected: $copied files, $added_rows index rows -> $index"
