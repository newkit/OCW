#!/usr/bin/env bash
# The art project's hourly collector (art/ART-PROJECT.md §5.1), run by the
# systemd user timer ocw-art-collect.timer on the dev machine (make
# art-install-timer) or by hand:
#
#   scripts/art-collect.sh [--sheets|--no-sheets] [DAY...]
#
# Default: yesterday and today (the last hour before midnight is not lost).
# Per day `art/diary.py gather` + `journal` (idempotent: sources are
# deduplicated by SHA-256, the build log is updated in place), then the mask
# check of the tracked text files (`check-masked`), and in the 23:xx run the
# contact sheets of today (`sheet`; --sheets / --no-sheets force it). No
# render, no publish: the coordinator does those in a session.
#
# Runs never overlap: the script holds an flock on art/build-history/.lock
# (the lock every art/diary.py command takes; OCW_ART_LOCK_HELD=1 tells the
# diary.py it starts that the lock is held). It waits OCW_ART_LOCK_WAIT
# seconds (default 2700) for a manual run to finish, else gives up with
# status "locked" and exit 75.
#
# Status: test-output/build-log/DAY/art.json (the latest run: start, end,
# status, new files, media count, mask check, errors) and art-runs.jsonl
# (every run); the output of every run is appended to art.log there. The
# build dashboard shows the latest status. Exit 0 = ok, 1 = failed,
# 75 = locked by another run.
set -uo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo" || exit 1

# Python 3.11+ (tomllib): the system one, not the pyenv default (3.8).
py=${OCW_ART_PYTHON:-}
if [ -z "$py" ]; then
  for c in /usr/bin/python3.13 /usr/bin/python3.12 /usr/bin/python3.11 /usr/bin/python3; do
    if [ -x "$c" ] && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null; then
      py=$c
      break
    fi
  done
fi
if [ -z "$py" ]; then
  echo "art-collect: no Python 3.11+ found (set OCW_ART_PYTHON)" >&2
  exit 1
fi

hist=${OCW_ART_HISTORY:-$repo/art/build-history}
outdir=${OCW_TEST_OUTPUT:-$repo/test-output}
day=$(date +%F)
status_dir=$outdir/build-log/$day
mkdir -p "$status_dir" "$hist"
logf=$status_dir/art.log
export OCW_ART_LOG=art.log

exec 9>>"$hist/.lock"
wait_s=${OCW_ART_LOCK_WAIT:-2700}
start=$(date +%FT%T%:z)
if ! flock -w "$wait_s" 9; then
  holder=$(head -c 200 "$hist/.lock" 2>/dev/null | tr -d '"\\\n')
  line=$(printf '{"start": "%s", "end": "%s", "status": "locked", "run_day": "%s", "new_files": 0, "media": null, "mask_check": null, "errors": ["build-history lock held for %ss by %s"], "log": "art.log"}' \
    "$start" "$(date +%FT%T%:z)" "$day" "$wait_s" "${holder:-another run}")
  printf '%s\n' "$line" > "$status_dir/art.json"
  printf '%s\n' "$line" >> "$status_dir/art-runs.jsonl"
  echo "art-collect: lock held by ${holder:-another run}; gave up after ${wait_s}s" | tee -a "$logf" >&2
  exit 75
fi
printf 'pid %s: scripts/art-collect.sh %s\n' "$$" "$*" > "$hist/.lock"
export OCW_ART_LOCK_HELD=1

{
  echo "=== $start art-collect $* (python: $("$py" -V 2>&1))"
  "$py" art/diary.py collect "$@" 2>&1
  rc=$?
  echo "=== $(date +%FT%T%:z) exit $rc"
  exit $rc
} | tee -a "$logf"
rc=${PIPESTATUS[0]}
: > "$hist/.lock"
exit "$rc"
