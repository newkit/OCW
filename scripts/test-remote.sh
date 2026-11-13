#!/usr/bin/env bash
# Tests pieces of scripts/remote.sh that run on the build machines, locally
# (no machine needed): the first-come-first-served rule of the build slots
# (`ocw_older` against fake task markers), remote-gc.sh's choice of the
# checkouts to clean, the tree copy (snapshot at invocation, content-based
# sync without source mtimes, the slot/sync handshake, the Windows manifest
# diff) and the build log's cargo numbers (scripts/build_profile.py, also its own unit tests).
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

# remote.sh's tree copy (v2.63): the snapshot taken at invocation, the
# content-based rsync without source mtimes, the slot/sync handshake and the
# Windows manifest diff – run locally against scratch directories.
fns=""
for f in snapshot_tree win_changed with_sync sync_watch profile_hash profile_check janitor_log json_str; do
  fns+=$(awk -v f="$f" '$0 ~ "^" f "\\(\\) \\{" { p = 1; print; if ($0 ~ /\}[ \t]*$/) exit; next } p { print } p && /^}/ { exit }' scripts/remote.sh)$'\n'
done
fns+=$(grep '^rsync_sync_opts=' scripts/remote.sh)$'\n'
s=$(mktemp -d)
(
  set -e
  eval "$fns"
  cd "$s"
  gq() { git -c user.name=t -c user.email=t@t "$@"; }
  gq init -q -b main src && cd src
  mkdir -p crates/a config
  echo "new code" >crates/a/lib.rs; echo same >same.txt; echo secret >config/machines.toml; echo gone >gone.txt
  printf 'config/\n' >.gitignore
  gq add -A && gq commit -q -m one
  echo untracked >extra.txt; rm gone.txt
  snapshot_tree
  check "$(LC_ALL=C sort "$snap_dir/list" | tr '\n' ' ')" ".gitignore crates/a/lib.rs extra.txt gone.txt same.txt " "snapshot: tracked + untracked, config/ left out"
  check "$(cat "$snap_dir/tree/crates/a/lib.rs")" "new code" "snapshot: content copied"
  echo "edited after the invocation" >crates/a/lib.rs
  check "$(cat "$snap_dir/tree/crates/a/lib.rs")" "new code" "snapshot: later edits (a merge) do not reach it"
  check "$([ -e "$snap_dir/tree/gone.txt" ] && echo yes || echo no)" no "snapshot: deleted files only in the list"
  # The machine's checkout, built at "build time" B after the newer file.
  dst=$s/dst; mkdir -p "$dst/crates/a"
  echo "newer code" >"$dst/crates/a/lib.rs"; echo same >"$dst/same.txt"; echo x >"$dst/gone.txt"
  touch -d '2024-01-02 00:00' "$dst/crates/a/lib.rs" "$dst/same.txt"
  # The snapshot's lib.rs is older content with an older mtime (a revert).
  touch -d '2023-06-01 00:00' "$snap_dir/tree/crates/a/lib.rs"
  (cd "$snap_dir/tree" && rsync "${rsync_sync_opts[@]}" --delete-missing-args --files-from="$snap_dir/list" ./ "$dst/")
  check "$(cat "$dst/crates/a/lib.rs")" "new code" "sync: the snapshot's content arrives"
  check "$([ "$(stat -c %Y "$dst/crates/a/lib.rs")" -gt "$(date -d '2024-01-02' +%s)" ] && echo fresh || echo stale)" fresh \
    "sync: a changed file gets the copy's time, not its older source mtime (cargo rebuilds it)"
  check "$(stat -c %Y "$dst/same.txt")" "$(date -d '2024-01-02 00:00' +%s)" "sync: unchanged files are not touched (no rebuild)"
  check "$([ -e "$dst/gone.txt" ] && echo yes || echo no)" no "sync: files deleted here are deleted there"
  check "$([ -e "$dst/config" ] && echo yes || echo no)" no "sync: config/ is never sent"
  # The dev profile check: the hash follows [profile.dev*] / [profile.test]
  # only; a change removes the checkout's target/debug once (here a scratch
  # "machine": SSH=(bash) and target=-c run the remote command locally).
  printf '[package]\nname = "x"\n[profile.dev]\ndebug = 1\n[profile.release]\nlto = true\n' >"$snap_dir/tree/Cargo.toml"
  h1=$(profile_hash)
  sed -i 's/lto = true/lto = false/' "$snap_dir/tree/Cargo.toml"
  check "$(profile_hash)" "$h1" "profile: a release-profile change does not count"
  os=linux SSH=(bash) target=-c repo=$s/machine checkout_dir=OCW-test-profile-$$ machine=testbox log_root=$s/log
  mkdir -p "$repo/target/debug" "$repo/target/release"; echo x >"$repo/target/debug/a"
  profile_check 1 2>/dev/null
  check "$(cat "$repo/.ocw-profile-hash")/$([ -d "$repo/target/debug" ] && echo kept)" "$h1/kept" "profile: the first run only records the hash"
  sed -i 's/debug = 1/debug = 2/' "$snap_dir/tree/Cargo.toml"
  check "$([ "$(profile_hash)" != "$h1" ] && echo changed)" changed "profile: a dev-profile change changes the hash"
  profile_check 1 2>/dev/null
  check "$([ -d "$repo/target/debug" ] && echo kept || echo removed)/$([ -d "$repo/target/release" ] && echo release-kept)" \
    "removed/release-kept" "profile: changed dev profile removes target/debug only"
  check "$(grep -c '"kind":"profile"' "$s"/log/*/janitor.jsonl)" 1 "profile: the deletion is logged to janitor.jsonl"
  # Regression (Win11Builder, 2026-11-13): the remote files were touched
  # into the future to force a rebuild, a build ran, then newer content came
  # with an OLDER local mtime (merge time). The changed file must end up
  # newer than that build, whatever the remote mtime was before.
  echo "content before the merge" >"$dst/crates/a/lib.rs"
  touch -d '+1 day' "$dst/crates/a/lib.rs" "$dst/same.txt"
  echo dep-info >"$dst/build.d"; touch -d '-5 seconds' "$dst/build.d"   # the build after the touch
  same_before=$(stat -c %Y "$dst/same.txt")
  echo "merged content: the new field" >"$snap_dir/tree/crates/a/lib.rs"
  touch -d '2022-03-01 00:00' "$snap_dir/tree/crates/a/lib.rs"
  (cd "$snap_dir/tree" && rsync "${rsync_sync_opts[@]}" --delete-missing-args --files-from="$snap_dir/list" ./ "$dst/")
  check "$(cat "$dst/crates/a/lib.rs")" "merged content: the new field" "regression: future-touched remote file gets the new content"
  check "$([ "$dst/crates/a/lib.rs" -nt "$dst/build.d" ] && echo rebuild || echo stale)" rebuild \
    "regression: changed file newer than the last build despite an older local mtime (cargo rebuilds)"
  check "$([ "$(stat -c %Y "$dst/crates/a/lib.rs")" -le "$(date +%s)" ] && echo now || echo future)" now \
    "regression: changed file gets the time of the copy, not the future one"
  check "$(stat -c %Y "$dst/same.txt")" "$same_before" "regression: unchanged files keep their mtime (no rebuild)"
  # Windows: the manifest diff.
  printf 'aaa\tcrates/a/lib.rs\nbbb\tsame.txt\n' >"$s/valid"
  printf 'ccc\tcrates/a/lib.rs\nbbb\tsame.txt\nddd\textra.txt\n' >"$s/hashes"
  check "$(win_changed "$s/valid" "$s/hashes" | tr '\n' ' ')" "crates/a/lib.rs extra.txt " "windows: only changed and new files are sent"
  # A file touched on the machine no longer matches its manifest entry
  # (size and mtime), so the machine leaves it out of VALID: it is sent
  # again and unpacked with tar -m (mtime = now), never trusted by mtime.
  printf 'aaa\tcrates/a/lib.rs\n' >"$s/valid"
  check "$(win_changed "$s/valid" "$s/hashes" | tr '\n' ' ')" "crates/a/lib.rs same.txt extra.txt " \
    "windows: a file touched there is re-sent with the time of the copy"
  # Windows extraction, simulated with GNU tar (same -m): the changed file
  # (older local mtime) unpacks with the time of the copy, newer than the
  # build; files not sent keep their time.
  w=$s/win; mkdir -p "$w/crates/a"; echo old >"$w/crates/a/lib.rs"; echo same >"$w/same.txt"
  touch -d '+1 day' "$w/crates/a/lib.rs"; touch -d '2024-01-02 00:00' "$w/same.txt"
  echo d >"$w/build.d"; touch -d '-5 seconds' "$w/build.d"
  (cd "$snap_dir/tree" && tar -cf - crates/a/lib.rs) | tar -xmf - -C "$w"
  check "$(cat "$w/crates/a/lib.rs")/$([ "$w/crates/a/lib.rs" -nt "$w/build.d" ] && echo rebuild)/$([ "$(stat -c %Y "$w/crates/a/lib.rs")" -le "$(date +%s)" ] && echo now)" \
    "merged content: the new field/rebuild/now" "windows (simulated tar -m): changed file newer than the build, not in the future"
  check "$(stat -c %Y "$w/same.txt")" "$(date -d '2024-01-02 00:00' +%s)" "windows (simulated): unsent files untouched"
  # The handshake: the "remote" prints @@ocw-sync once it has its slot,
  # the sync runs then (not before), the remote gets "go" and runs on.
  sync_tree() { echo "sync at $(cat "$s/clock")" >>"$s/synced"; }
  profile_check() { :; }
  echo before-slot >"$s/clock"
  out=$(with_sync bash -c "echo waiting; echo after-slot >'$s/clock'; echo '@@ocw-sync 42'; read -r g; echo \"got \$g\"; read -r more || echo eof; printf tail")
  check "$(printf '%s' "$out" | tr '\n' '|')" "waiting|got go|eof|tail" "handshake: output passes through, stdin gets go, then EOF"
  check "$(cat "$s/synced")" "sync at after-slot" "handshake: the tree is synced once, after the slot is taken"
  check "$([ -s "$snap_dir/sync_s" ] && echo yes || echo no)" yes "handshake: sync time recorded for the build log"
  rm -f "$s/synced"
  out=$(with_sync bash -c 'echo no-handshake; exit 3') && rc=0 || rc=$?
  check "$out/$rc/$([ -e "$s/synced" ] && echo synced || echo none)" "no-handshake/3/none" "handshake: exit code kept, no sync when not asked"
  sync_tree() { return 1; }
  out=$(with_sync bash -c "echo '@@ocw-sync 1'; read -r g; echo \"got \$g\"")
  check "$out" "got fail" "handshake: a failed sync answers fail"
  rm -rf "$snap_dir"
  [ "$fail" = 0 ]
) || fail=1
rm -rf "$s"

# The Windows run wrapper as remote.sh assembles it (no Windows needed):
# placeholders filled, braces balanced, the sync step chosen.
(
  eval "$(awk '/^psq\(\) \{/' scripts/remote.sh)"
  for f in short_cmd run_windows; do
    eval "$(awk -v f="$f" '$0 ~ "^" f "\\(\\) \\{" { p = 1 } p { print } p && /^}/ { exit }' scripts/remote.sh)"
  done
  eval "$(awk '/^read -r -d .. marker_ps/ { p = 1 } p { print } p && /^EOF/ { exit }' scripts/remote.sh)"
  SSH=(printf '%s\n') target="" repo='B:\src\OCW-x' checkout_dir=OCW-x branch=b slots=1 jobs_env="" build_id=id
  for mode in default handshake; do
    if [ $mode = handshake ]; then win_sync_ps="[Console]::Out.WriteLine('@@ocw-sync ' + \$PID); \$ocwGo = [Console]::In.ReadLine();"; else win_sync_ps="\$ocwGo = 'go';"; fi
    ps=$(run_windows cargo test)
    opens=$(printf '%s' "$ps" | tr -cd '{' | wc -c); closes=$(printf '%s' "$ps" | tr -cd '}' | wc -c)
    check "$(printf '%s' "$ps" | grep -c '@[A-Z][A-Z]*@')/$opens" "0/$closes" "windows wrapper ($mode): placeholders filled, braces balanced"
  done
  check "$(printf '%s' "$ps" | grep -c "ReadLine")" 1 "windows wrapper: the handshake only when asked for"
  [ "$fail" = 0 ]
) || fail=1

# Machines enabled/disabled and build slots from the dashboard (v2.66):
# config/build-machines.toml against a scratch config (OCW_REMOTE_CONFIG;
# the hosts are .invalid, so nothing reaches a machine).
(
  c=$(mktemp -d)
  trap 'rm -rf "$c"' EXIT
  cat >"$c/machines.toml" <<'EOF'
[defaults]
connect_timeout_s = 2

[[machine]]
name = "testbox"
os = "linux"
host = "testbox.invalid"
slots = 3

[[machine]]
name = "offbox"
os = "linux"
host = "offbox.invalid"

[[machine]]
name = "macbox"
os = "macos"
host = "macbox.invalid"

[[machine]]
name = "plainbox"
os = "linux"
host = "plainbox.invalid"
slots = 3
EOF
  R() { OCW_BUILD_LOG=0 OCW_REMOTE_CONFIG=$c scripts/remote.sh "$@"; }
  check "$(R testbox slots)" 3 "slots: machines.toml without an override file"
  cat >"$c/build-machines.toml" <<'EOF'
# written by the dashboard
[[machine]]
name = "testbox"
enabled = true
slots = 5

[[machine]]
name = "offbox"
enabled = false
slots = 2
reason = "kiosk only \u0023 \"user\""

[[machine]]
name = "macbox"
enabled = true
EOF
  check "$(R testbox slots)" 5 "slots: the dashboard's count wins over machines.toml"
  check "$(OCW_REMOTE_SLOTS=1 R testbox slots)" 1 "slots: OCW_REMOTE_SLOTS wins over the dashboard"
  check "$(OCW_REMOTE_SLOTS=0 R testbox slots)" 0 "slots: OCW_REMOTE_SLOTS=0 (no limit) still works"
  check "$(R macbox slots)" 1 "slots: an entry without slots keeps the OS default (macos 1)"
  check "$(R plainbox slots)" 3 "slots: a machine without an entry keeps machines.toml"
  check "$(R testbox enabled >/dev/null 2>&1; echo $?)" 0 "enabled: exit 0"
  out=$(R offbox enabled 2>&1) && rc=0 || rc=$?
  check "$rc/$out" '3/offbox is disabled for building (kiosk only # "user")' "enabled: a disabled machine exits 3 with its reason"
  t0=$(date +%s)
  out=$(R offbox run 'echo should-not-run' 2>&1) && rc=0 || rc=$?
  check "$rc" 3 "run: refused on a disabled machine (exit 3)"
  check "$(printf '%s' "$out" | grep -c 'offbox is disabled for building (kiosk only # "user") – enable it in the build dashboard')" 1 \
    "run: the message names the machine, the reason and the dashboard"
  check "$(printf '%s' "$out" | grep -c should-not-run)/$(( $(date +%s) - t0 < 5 ))" 0/1 "run: refused at once, nothing run"
  out=$(R offbox sync 2>&1) && rc=0 || rc=$?
  check "$rc" 3 "sync: refused on a disabled machine (exit 3)"
  out=$(OCW_REMOTE_DEPLOY=1 R offbox sync 2>&1) && rc=0 || rc=$?
  check "$([ "$rc" != 3 ] && echo passed)/$(printf '%s' "$out" | grep -c 'allowed for a deploy')" passed/1 \
    "deploy (OCW_REMOTE_DEPLOY=1): a disabled machine is let through (then ssh fails on the .invalid host)"
  check "$(R offbox field os)" linux "field: allowed on a disabled machine"
  check "$(grep -c 'OCW_REMOTE_DEPLOY=1 scripts/remote.sh' scripts/deploy-test.sh)" 1 "deploy-test.sh sets OCW_REMOTE_DEPLOY=1"
  [ "$fail" = 0 ]
) || fail=1

# A lowered slot count: a run that still holds slot 3 after N went from 3
# to 2 keeps it; new runs only try slots 1..2 (the flock functions of
# remote.sh, their lock files moved into a scratch directory).
if command -v flock >/dev/null; then
  (
    sl=$(mktemp -d)
    hold() { (exec 9>"$sl/ocw-build-slot-$1.lock"; flock -n 9 && : >"$sl/held-$1"; exec sleep 30) & }
    hold 3; h3=$!; hold 1; h1=$!
    trap 'kill $h1 $h3 ${h2:-} 2>/dev/null; rm -rf "$sl"' EXIT
    for _ in $(seq 50); do [ -e "$sl/held-1" ] && [ -e "$sl/held-3" ] && break; sleep 0.1; done
    eval "$(awk '/^read -r -d .. slot_take_flock/ { f = 1; next } f && /^EOF/ { exit } f' scripts/remote.sh | sed "s|/tmp/ocw-build-slot-|$sl/ocw-build-slot-|g")"
    first() { (for s in $1; do ocw_take "$s" && { echo "$s"; exit 0; }; done; echo none); }
    check "$(first "1 2")" 2 "lowered slots: a new run takes the free slot 2 of 1..2"
    hold 2; h2=$!
    for _ in $(seq 50); do [ -e "$sl/held-2" ] && break; sleep 0.1; done
    check "$(first "1 2")" none "lowered slots: all of 1..2 taken, the run waits (slot 3 is not used)"
    kill $h3 2>/dev/null; wait $h3 2>/dev/null || true
    check "$(first "1 2")" none "lowered slots: the old slot-3 run ends normally, nothing changes for 1..2"
    kill $h1 2>/dev/null; wait $h1 2>/dev/null || true
    check "$(first "1 2")" 1 "lowered slots: a freed slot 1 is taken"
    [ "$fail" = 0 ]
  ) || fail=1
fi

python3 scripts/test_build_profile.py >/dev/null 2>&1 && echo "ok   build_profile.py unit tests" ||
  { echo "FAIL build_profile.py unit tests (python3 scripts/test_build_profile.py)"; fail=1; }
exit $fail
