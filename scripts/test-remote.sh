#!/usr/bin/env bash
# Tests pieces of scripts/remote.sh that run on the build machines, locally
# (no machine needed): the first-come-first-served rule of the build slots
# (`ocw_older` against fake task markers), remote-gc.sh's choice of the
# checkouts to clean, and the build log's cargo numbers (scripts/build_profile.py, also its own unit tests).
set -euo pipefail
cd "$(dirname "$0")/.."
fail=0
check() { if [ "$1" = "$2" ]; then echo "ok   $3"; else echo "FAIL $3 (got $1, want $2)"; fail=1; fi; }

# The remote shell function, as remote.sh joins it into one line.
fn=$(awk '/^read -r -d .. slot_fifo/ { f = 1; next } f && /^EOF/ { exit } f' scripts/remote.sh | tr '\n' ' ')
[ -n "$fn" ] || { echo "FAIL: slot_fifo not found in remote.sh"; exit 1; }
ocw_d=$(mktemp -d)
sleep 60 & live=$!
trap 'kill $live 2>/dev/null || true; rm -rf "$ocw_d"' EXIT
ocw_t0=1000
ocw_f=$ocw_d/$$-1000.json
eval "$fn"
marker() { # pid state start
  printf '{"v":1,"pid":%s,"state":"%s","slot":null,"start":%s,"since":1}\n' "$1" "$2" "$3" >"$ocw_d/$1-$3.json"
}
older() { if ocw_older; then echo yes; else echo no; fi; }
marker $$ waiting 1000
check "$(older)" no "alone: take a free slot"
marker "$live" running 900
check "$(older)" no "an older running task does not count"
marker "$live" waiting 1100
check "$(older)" no "a younger waiting run goes after this one"
marker 999999 waiting 900
check "$(older)" no "an older waiting marker of a dead process is ignored"
marker "$live" waiting 950
check "$(older)" yes "an older waiting run goes first"
rm -f "$ocw_d"/*.json
marker $$ waiting 1000
if [ "$live" -lt $$ ]; then want=yes; else want=no; fi
marker "$live" waiting 1000
check "$(older)" $want "same second: the lower PID goes first"

# remote-gc.sh: which agent checkouts lose their target/ (a scratch repo).
vfn=$(awk '/^verdict\(\) \{/ { f = 1 } f { print } f && /^}/ { exit }' scripts/remote-gc.sh)
g=$(mktemp -d)
(
  cd "$g"
  gq() { git -c user.name=t -c user.email=t@t "$@"; }
  gq init -q -b main repo && cd repo && gq commit -q --allow-empty -m one
  gq branch worktree-agent-merged
  gq branch worktree-agent-live && gq worktree add -q ../live worktree-agent-live
  gq checkout -q -b worktree-agent-open && gq commit -q --allow-empty -m two
  gq checkout -q main
  worktrees=$(git worktree list --porcelain)
  eval "$vfn"
  check "$(verdict worktree-agent-merged)" "merged into main, worktree removed" "gc: merged branch without worktree"
  check "$(verdict worktree-agent-live)" "" "gc: a live worktree is kept (new branches look merged)"
  check "$(verdict worktree-agent-open)" "" "gc: unmerged branch kept"
  check "$(verdict worktree-agent-gone)" "branch gone" "gc: deleted branch"
  check "$(verdict bisect-123)" "" "gc: other checkouts kept"
  [ "$fail" = 0 ]
) || fail=1
rm -rf "$g"

python3 scripts/test_build_profile.py >/dev/null 2>&1 && echo "ok   build_profile.py unit tests" ||
  { echo "FAIL build_profile.py unit tests (python3 scripts/test_build_profile.py)"; fail=1; }
exit $fail
