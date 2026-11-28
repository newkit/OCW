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
  # Regression (2026-11-14): a new checkout has no manifest, so VALID is
  # empty; every file must be sent (NR == FNR sent none).
  : >"$s/valid"
  check "$(win_changed "$s/valid" "$s/hashes" | tr '\n' ' ')" "crates/a/lib.rs same.txt extra.txt " \
    "regression: windows: a new checkout (empty manifest) gets every file"
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

# Stale files (spec 11.2): sync_tree end to end for Linux and macOS against
# a local scratch "machine" – `ssh` is a script that runs the command with
# bash (also rsync's remote side). A file moved away here disappears there
# (with its emptied directory); untracked build outputs, git-ignored files
# and anything an earlier sync did not send stay; `..`, absolute paths,
# config/, target/ and paths through a symlinked directory are never
# deleted; the first sync deletes nothing.
for st_os in linux macos; do
  (
    set -e
    sf=""
    for f in snapshot_tree stale_report stale_script stale_paths sweep_candidates sync_tree; do
      sf+=$(awk -v f="$f" '$0 ~ "^" f "\\(\\) \\{" { p = 1; print; if ($0 ~ /\}[ \t]*$/) exit; next } p { print } p && /^}/ { exit }' scripts/remote.sh)$'\n'
    done
    sf+=$(awk '/^read -r -d .. stale_sh / { p = 1 } p { print } p && /^EOF/ { exit }' scripts/remote.sh)$'\n'
    sf+=$(grep -E '^(rsync_sync_opts|sweep_dirs|sweep_skip|sweep_max)=' scripts/remote.sh)$'\n'
    eval "$sf"
    check "$(stale_script | grep -c "'" || true)" 0 "stale ($st_os): the machine script has no single quote (sh -c '…')"
    check "$(stale_script | grep -c '@DIRS@\|@SKIP@' || true)" 0 "stale ($st_os): the sweep's directories and names are filled in"
    x=$(mktemp -d)
    trap 'rm -rf "$x"' EXIT
    printf '#!/usr/bin/env bash\nshift; exec bash -c "$*"\n' >"$x/fake-ssh"; chmod +x "$x/fake-ssh"
    os=$st_os SSH=("$x/fake-ssh") target=fake repo=$x/machine
    mkdir -p "$x/outside"; echo keep >"$x/outside/victim.txt"; echo keep >"$x/outside.txt"
    cd "$x"
    gq() { git -c user.name=t -c user.email=t@t "$@"; }
    gq init -q -b main src && cd src
    mkdir -p apps/desktop/src crates/ocw-ui/src
    echo "mod workshop;" >apps/desktop/src/main.rs; echo "pub fn w() {}" >apps/desktop/src/workshop.rs
    mkdir -p apps/desktop/src/workshop; echo "pub fn p() {}" >apps/desktop/src/workshop/parts.rs
    echo keep >keep.txt; printf '*.log\n' >.gitignore; echo local >notes.log
    gq add -A && gq commit -q -m one
    log=$x/log
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(grep -c 'first sync of this checkout' "$log")/$(cat "$repo/.ocw-sync/synced.txt" | wc -l)" "1/5" "stale ($st_os): the first sync notes it and records the files sent"
    check "$([ -e "$repo/notes.log" ] && echo sent || echo not)" not "stale ($st_os): git-ignored files are not sent"
    # The machine's own files: a build output, a git-ignored log, a file
    # no sync sent, a file in config/.
    mkdir -p "$repo/target/debug" "$repo/config"; echo bin >"$repo/target/debug/ocw"; echo log >"$repo/notes.log"
    echo mine >"$repo/remote-only.txt"; echo secret >"$repo/config/machines.toml"
    gq mv apps/desktop/src/workshop.rs apps/desktop/src/workshop/mod.rs
    gq mv apps/desktop/src/workshop crates/ocw-ui/src/workshop
    gq commit -q -m move
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(cd "$repo" && ls apps/desktop/src/workshop.rs apps/desktop/src/workshop/parts.rs 2>/dev/null | wc -l)/$(cat "$repo/crates/ocw-ui/src/workshop/mod.rs")" \
      "0/pub fn w() {}" "stale ($st_os): files moved away here are deleted there, the new ones arrive"
    check "$([ -d "$repo/apps/desktop/src/workshop" ] && echo left || echo gone)/$([ -f "$repo/apps/desktop/src/main.rs" ] && echo main)" \
      "gone/main" "stale ($st_os): the emptied directory is removed, its parent with files stays"
    check "$(grep -c 'deleted 2 stale file(s) there' "$log")" 1 "stale ($st_os): the deletion is reported"
    check "$(cd "$repo" && cat target/debug/ocw notes.log remote-only.txt config/machines.toml | tr '\n' ' ')" "bin log mine secret " \
      "stale ($st_os): build outputs, git-ignored and never-sent files and config/ stay"
    # A tampered list: only the safe path goes.
    ln -s "$x/outside" "$repo/link"
    echo x >"$repo/fake-old.txt"
    printf '%s\n' ../outside.txt "$x/outside.txt" link/victim.txt config/machines.toml target/debug/ocw fake-old.txt >>"$repo/.ocw-sync/synced.txt"
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(cat "$x/outside.txt" "$x/outside/victim.txt" "$repo/config/machines.toml" "$repo/target/debug/ocw" | tr '\n' ' ')$([ -e "$repo/fake-old.txt" ] && echo kept || echo deleted)" \
      "keep keep secret bin deleted" "stale ($st_os): .., absolute, symlinked-directory, config/ and target/ paths are never deleted"
    check "$(grep -c 'refused 3 stale path' "$log")/$(grep -c 'deleted 1 stale' "$log")" 1/1 "stale ($st_os): the refused paths are reported"
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(wc -c <"$log" | tr -d ' ')" 0 "stale ($st_os): nothing to delete, nothing said"
    # First sync of a checkout seeded from an older tree (v7.00; OCW-HEAD
    # and OCW-merge-ios, 2026-11-27/28: an old apps/desktop/src/workshop/
    # next to workshop.rs): the stale source files of the source
    # directories go, build outputs, data, caches, git-ignored files, the
    # top level and the directories the sync does not own stay.
    repo=$x/seeded
    mkdir -p "$repo/apps/desktop/src/workshop" "$repo/target/debug" "$repo/data/pilot" "$repo/test-output" "$repo/config" \
      "$repo/tools/x/__pycache__" "$repo/tools/x/node_modules/m" "$repo/crates/old/src" "$repo/artwork/old" "$repo/worlds/old" "$repo/crates/ocw-ui/src/workshop"
    echo "pub fn old() {}" >"$repo/apps/desktop/src/workshop/mod.rs"; echo "pub fn old() {}" >"$repo/crates/old/src/lib.rs"
    echo "pub fn w0() {}" >"$repo/crates/ocw-ui/src/workshop/mod.rs"
    echo bin >"$repo/target/debug/ocw"; echo pkg >"$repo/data/pilot/p.ocw"; echo shot >"$repo/test-output/a.txt"; echo s >"$repo/config/machines.toml"
    echo pyc >"$repo/tools/x/__pycache__/a.pyc"; echo js >"$repo/tools/x/node_modules/m/i.js"; echo log >"$repo/crates/old/build.log"
    echo top >"$repo/release.txt"; echo art >"$repo/artwork/old/a.glb"; echo w >"$repo/worlds/old/p.toml"
    ln -s "$x/outside" "$repo/scripts"
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(cd "$repo" && ls apps/desktop/src/workshop/mod.rs crates/old/src/lib.rs 2>/dev/null | wc -l)/$([ -d "$repo/apps/desktop/src/workshop" ] && echo left || echo gone)/$(cat "$repo/crates/ocw-ui/src/workshop/mod.rs" "$repo/apps/desktop/src/main.rs" | tr '\n' ' ')" \
      "0/gone/pub fn w() {} mod workshop; " "stale ($st_os): the first sync deletes the stale source files of an older tree, then the tree arrives"
    check "$(cd "$repo" && cat target/debug/ocw data/pilot/p.ocw test-output/a.txt config/machines.toml tools/x/__pycache__/a.pyc tools/x/node_modules/m/i.js crates/old/build.log release.txt artwork/old/a.glb worlds/old/p.toml "$x/outside/victim.txt" | tr '\n' ' ')" \
      "bin pkg shot s pyc js log top art w keep " "stale ($st_os): the first sync keeps target/, data/, test-output/, config/, caches, git-ignored files, the top level, artwork/, worlds/ and symlinked directories"
    check "$(grep -c 'first sync of this checkout' "$log")/$(grep -c 'deleted 2 file(s) there that are not in this tree (first sync: left from an older tree): apps/desktop/src/workshop/mod.rs crates/old/src/lib.rs$' "$log")" \
      "1/1" "stale ($st_os): the sweep is reported with its count and paths"
    mkdir -p "$repo/crates/old/src"; echo "pub fn old() {}" >"$repo/crates/old/src/lib.rs"
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(cat "$repo/crates/old/src/lib.rs")/$(wc -c <"$log" | tr -d ' ')" "pub fn old() {}/0" \
      "stale ($st_os): a later sync deletes only the previous list's files (a file no sync sent stays)"
    # Turned off, and refused when there are too many (a wrong checkout).
    rm -rf "$x/seeded"; mkdir -p "$repo/crates/old/src"; echo o >"$repo/crates/old/src/lib.rs"
    snapshot_tree; OCW_FIRST_SWEEP=0 sync_tree 2>"$log"; rm -rf "$snap_dir"
    check "$(cat "$repo/crates/old/src/lib.rs")" o "stale ($st_os): OCW_FIRST_SWEEP=0 sweeps nothing"
    rm -rf "$x/seeded"; mkdir -p "$repo/crates/old/src"; echo o >"$repo/crates/old/src/lib.rs"; echo o >"$repo/crates/old/src/b.rs"
    sweep_max=1
    snapshot_tree; sync_tree 2>"$log"; rm -rf "$snap_dir"
    sweep_max=2000
    check "$(cat "$repo/crates/old/src/lib.rs")/$(grep -c '2 files there are not in this tree – more than 1, so none deleted' "$log")" o/1 \
      "stale ($st_os): more than sweep_max candidates delete nothing"
    [ "$fail" = 0 ]
  ) || fail=1
done

# The Windows sync end to end (v2.94) against a simulated machine: `ssh`
# is a bash function that plays the PowerShell side – the manifest check
# (header naming the checkout, file exists with the manifest's size and
# mtime) and, after GNU tar -xm, the manifest write (only when tar
# succeeded, missing files reported) – in a scratch directory. It checks
# that the encoded scripts it stands in for are remote.sh's. Regression
# (Win11Builder, 2026-11-14): a new agent checkout got "sending 0 changed
# of 397 files" and stayed empty (only .ocw-sync, .ocw-last-run), because
# win_changed's `NR == FNR` read the hashes as the valid list when the
# machine's list was empty. Manual live test: see the end of this block.
(
  set -e
  wf=""
  for f in psq ps_encoded snapshot_tree win_changed win_send stale_paths stale_report sweep_candidates win_list_script sync_tree_windows; do
    wf+=$(awk -v f="$f" '$0 ~ "^" f "\\(\\) \\{" { p = 1; print; if ($0 ~ /\}[ \t]*$/) exit; next } p { print } p && /^}/ { exit }' scripts/remote.sh)$'\n'
  done
  for v in win_manifest_ps win_manifest_write_ps win_remove_ps win_list_ps; do
    wf+=$(awk -v v="$v" '$0 ~ "^read -r -d .. " v " " { p = 1 } p { print } p && /^EOF/ { exit }' scripts/remote.sh)$'\n'
  done
  wf+=$(grep -E '^(sweep_dirs|sweep_skip|sweep_max)=' scripts/remote.sh)$'\n'
  eval "$wf"
  x=$(mktemp -d)
  trap 'rm -rf "$x"' EXIT
  repo='B:\src\OCW-test' root=$x/machine target=win SSH=(fake_ssh) machine=testwin
  decode() { printf '%s' "${1##*-EncodedCommand }" | sed 's/[^A-Za-z0-9+/=].*//' | base64 -d | iconv -f utf-16le -t utf-8; }
  fstat() { stat -c '%s	%Y' "$1"; }
  fake_ssh() {
    shift
    local c=$1 l sha size mt p
    case "$c" in
      New-Item*) mkdir -p "$root" ;;
      Set-Location*) ;;
      tar\ -xmf*)
        [ "$(decode "$c")" = "${win_manifest_write_ps//@REPO@/"$(psq "$repo")"}" ] || { echo "fake: not the write script" >&2; return 9; }
        if [ -e "$x/tar-fail" ]; then cat >/dev/null; rm -f "$root/.ocw-sync/incoming.tsv"; echo '@@ocw-sync tar failed: exit 1'; return 1; fi
        tar -xmf - -C "$root"
        if [ -e "$x/drop" ]; then rm -f "$root/$(cat "$x/drop")"; [ -e "$x/drop-always" ] || rm -f "$x/drop"; fi
        local miss=0 hdr
        hdr=$(printf '# ocw-sync v2\t%s' "$repo")
        { printf '%s\n' "$hdr"
          while IFS=$'\t' read -r sha p; do
            if [ -e "$root/$p" ]; then printf '%s\t%s\t%s\t%s\n' "$sha" "$(fstat "$root/$p" | cut -f1)" "$(fstat "$root/$p" | cut -f2)" "$p"; else miss=$((miss + 1)); fi
          done <"$root/.ocw-sync/incoming.tsv"
          [ $miss = 0 ] || echo "@@missing $miss" >"$x/missing"
        } >"$root/.ocw-sync/manifest.tsv.new"
        mv "$root/.ocw-sync/manifest.tsv.new" "$root/.ocw-sync/manifest.tsv"; rm "$root/.ocw-sync/incoming.tsv"
        if [ -e "$x/missing" ]; then printf '@@ocw-sync missing %s: ...\r\n' "$(cut -d' ' -f2 "$x/missing")"; rm "$x/missing"; fi
        ;;
      powershell*)
        local dc ns paths ps n=0 bad=0 i=0 names="" d
        dc=$(decode "$c")
        if [ "$dc" = "$(win_list_script)" ]; then
          # The first sync's listing: files under the sweep directories,
          # skipped names left out.
          for d in $sweep_dirs; do
            [ -d "$root/$d" ] && [ ! -L "$root/$d" ] || continue
            (cd "$root" && find "$d" \( -name target -o -name node_modules -o -name __pycache__ \) -prune -o -type f -print) | while IFS= read -r p; do printf '@@found\t%s\r\n' "$p"; done
          done
          return 0
        fi
        if [ "$dc" != "${win_manifest_ps//@REPO@/"$(psq "$repo")"}" ]; then
          # The remove script: its paths and stale count, checked against
          # remote.sh's template, then played (files only, inside root).
          ns=$(printf '%s\n' "$dc" | sed -n '1s/.*\$ns = \([0-9]*\);.*/\1/p')
          paths=$(printf '%s\n' "$dc" | awk '/^'"'"'@ -split/ { p = 0 } p { print } /^\$ps = @'"'"'$/ { p = 1 }')
          ps=${win_remove_ps//@REPO@/"$(psq "$repo")"}; ps=${ps//@STALE@/$ns}
          [ -n "$ns" ] && [ "$dc" = "${ps//@PATHS@/$paths}" ] || { echo "fake: not the manifest or remove script" >&2; return 9; }
          echo "$paths" >>"$x/removed-paths"
          while IFS= read -r p; do
            [ -n "$p" ] || continue; i=$((i + 1))
            case "$p" in /*|*..*) bad=$((bad + 1)); continue ;; esac
            [ -f "$root/$p" ] || continue
            rm -f "$root/$p"
            if [ $i -le "$ns" ]; then n=$((n + 1)); names="$names $p"; fi
            d=$(dirname "$p"); while [ "$d" != . ] && rmdir "$root/$d" 2>/dev/null; do d=$(dirname "$d"); done
          done <<<"$paths"
          printf '@@ocw-sync removed %s refused %s:%s\r\n' "$n" "$bad" "$names"
          return 0
        fi
        local m=$root/.ocw-sync/manifest.tsv
        [ -f "$m" ] || return 0
        [ "$(head -n 1 "$m")" = "$(printf '# ocw-sync v2\t%s' "$repo")" ] || return 0
        tail -n +2 "$m" | while IFS=$'\t' read -r sha size mt p; do
          if [ -e "$root/$p" ] && [ "$(fstat "$root/$p")" = "$size	$mt" ]; then printf '%s\t%s\r\n' "$sha" "$p"; fi
        done
        printf '@@prev-list\r\n'
        tail -n +2 "$m" | cut -f4 | while IFS= read -r p; do printf '@@prev\t%s\r\n' "$p"; done
        ;;
      *) echo "fake: unexpected command $c" >&2; return 9 ;;
    esac
  }
  cd "$x"
  git -c user.name=t -c user.email=t@t init -q -b main src && cd src
  mkdir -p crates/a
  echo '[workspace]' >Cargo.toml; echo '[toolchain]' >rust-toolchain.toml; echo code >crates/a/lib.rs; echo z >zz.txt
  git add -A
  snapshot_tree
  sync() { rm -rf "$snap_dir/tree/.ocw-sync"; sync_tree_windows 2>"$x/log"; }
  sent() { grep -o 'sending [0-9]* changed of [0-9]* files' "$x/log"; }
  check "$(win_changed /dev/null <(printf 'a\tx\nb\ty\n') | tr '\n' ' ')" "x y " "windows regression: an empty valid list sends every file"
  sync
  check "$(sent)/$(cd "$root" && ls Cargo.toml rust-toolchain.toml crates/a/lib.rs zz.txt 2>/dev/null | wc -l)" "sending 4 changed of 4 files/4" "windows: a new checkout gets the whole tree"
  check "$(head -n 1 "$root/.ocw-sync/manifest.tsv" | cut -f2)/$(($(wc -l <"$root/.ocw-sync/manifest.tsv") - 1))/$([ -e "$root/.ocw-sync/incoming.tsv" ] && echo left || echo gone)" \
    'B:\src\OCW-test/4/gone' "windows: the manifest names its checkout and lists every file"
  sync
  check "$(sent)" "sending 0 changed of 4 files" "windows: a second sync sends nothing"
  rm "$root/crates/a/lib.rs" "$root/Cargo.toml"
  sync
  check "$(sent)/$(cat "$root/crates/a/lib.rs")/$([ -e "$root/Cargo.toml" ] && echo there)" "sending 2 changed of 4 files/code/there" "windows: files deleted there are re-sent"
  # Stale state: a manifest of another checkout (a copied directory) or of
  # the format before v2.94 is not trusted, even with the files there.
  for stale in other headerless empty; do
    case $stale in
      other) sed -i '1s/.*/# ocw-sync v2\tB:\\src\\OCW-other/' "$root/.ocw-sync/manifest.tsv" ;;
      headerless) sed -i '1d' "$root/.ocw-sync/manifest.tsv" ;;
      empty) rm -rf "$root"; mkdir -p "$root/.ocw-sync"; : >"$root/.ocw-sync/manifest.tsv"; : >"$root/.ocw-last-run" ;;
    esac
    sync
    check "$(sent)/$(cd "$root" && ls Cargo.toml rust-toolchain.toml 2>/dev/null | wc -l)" "sending 4 changed of 4 files/2" "windows: a stale .ocw-sync ($stale manifest) means a full send"
  done
  cp "$root/.ocw-sync/manifest.tsv" "$x/manifest.before"
  echo changed >"$snap_dir/tree/zz.txt"
  : >"$x/tar-fail"
  sync && rc=0 || rc=$?
  rm "$x/tar-fail"
  check "$rc/$(cmp -s "$x/manifest.before" "$root/.ocw-sync/manifest.tsv" && echo kept)/$([ -e "$root/.ocw-sync/incoming.tsv" ] && echo left || echo none)/$(grep -c 'manifest not updated' "$x/log")" \
    "1/kept/none/1" "windows: a failed extraction fails the sync and leaves the manifest as it was"
  echo Cargo.toml >"$x/drop"
  rm "$root/rust-toolchain.toml"
  sync && rc=0 || rc=$?
  check "$rc/$(grep -c 'sending the whole tree' "$x/log")/$(cd "$root" && ls Cargo.toml rust-toolchain.toml 2>/dev/null | wc -l)/$(cat "$root/zz.txt")" "0/1/2/changed" \
    "windows safety net: Cargo.toml missing after the sync sends the whole tree once"
  echo Cargo.toml >"$x/drop"; : >"$x/drop-always"
  sync && rc=0 || rc=$?
  check "$rc/$(grep -c 'still missing 1' "$x/log")" "1/1" "windows safety net: still missing after the full send fails the sync"
  rm -f "$x/drop" "$x/drop-always"
  # Stale files on Windows: the previous manifest's files that are gone
  # here are deleted there (emptied directories too); the machine's own
  # files stay; tampered manifest paths are refused before anything is sent.
  rm -rf "$snap_dir"; snapshot_tree; sync
  mkdir -p "$root/target/debug" "$root/config"; echo bin >"$root/target/debug/ocw.exe"; echo mine >"$root/remote-only.txt"; echo s >"$root/config/machines.toml"
  git mv crates/a/lib.rs crates/b.rs
  rm -rf "$snap_dir"; snapshot_tree; : >"$x/removed-paths"; sync
  check "$([ -e "$root/crates/a" ] && echo left || echo gone)/$(cat "$root/crates/b.rs")/$(grep -c 'deleted 1 stale file(s) there (removed or moved away here): crates/a/lib.rs' "$x/log")" \
    "gone/code/1" "windows stale: a moved file is deleted there with its emptied directory"
  check "$(cat "$root/target/debug/ocw.exe" "$root/remote-only.txt" "$root/config/machines.toml" | tr '\n' ' ')" "bin mine s " \
    "windows stale: build outputs, never-sent files and config/ stay"
  printf 'x\t1\t1\t%s\n' ../outside.txt 'C:/Windows/x.txt' 'crates\..\..\y' config/machines.toml target/debug/ocw.exe >>"$root/.ocw-sync/manifest.tsv"
  : >"$x/removed-paths"; sync
  check "$(grep -c 'refused 3 stale path' "$x/log")/$(grep -cE 'outside|Windows|config|target|y$' "$x/removed-paths")/$(cat "$root/config/machines.toml" "$root/target/debug/ocw.exe" | tr '\n' ' ')" \
    "1/0/s bin " "windows stale: .., drive-letter, backslash, config/ and target/ paths are never sent to the machine"
  check "$(grep -c 'first sync of this checkout' "$x/log")" 0 "windows stale: a checkout with a manifest is no first sync"
  rm -rf "$root"; sync
  check "$(grep -c 'first sync of this checkout' "$x/log")/$(grep -c 'deleted' "$x/log")" 1/0 "windows stale: a new checkout deletes nothing and says so"
  # A checkout seeded from an older tree (v7.00): its first sync deletes the
  # stale source files, target/, data/, caches and the top level stay.
  rm -rf "$root"; mkdir -p "$root/apps/desktop/src/workshop" "$root/target/debug" "$root/data/pilot" "$root/crates/a" "$root/tools/x/__pycache__"
  echo old >"$root/apps/desktop/src/workshop/mod.rs"; echo old >"$root/crates/a/old.rs"; echo bin >"$root/target/debug/ocw.exe"; echo pkg >"$root/data/pilot/p.ocw"
  echo pyc >"$root/tools/x/__pycache__/a.pyc"; echo top >"$root/release.txt"
  sync
  check "$(cd "$root" && ls apps/desktop/src/workshop/mod.rs crates/a/old.rs 2>/dev/null | wc -l)/$([ -d "$root/apps" ] && echo left || echo gone)/$(cat "$root/crates/b.rs" "$root/target/debug/ocw.exe" "$root/data/pilot/p.ocw" "$root/tools/x/__pycache__/a.pyc" "$root/release.txt" | tr '\n' ' ')" \
    "0/gone/code bin pkg pyc top " "windows stale: the first sync deletes an older tree's stale source files, target/, data/, caches and the top level stay"
  check "$(grep -c 'deleted 2 file(s) there that are not in this tree (first sync: left from an older tree): apps/desktop/src/workshop/mod.rs crates/a/old.rs$' "$x/log")" 1 \
    "windows stale: the sweep is reported with its count and paths"
  mkdir -p "$root/crates/a"; echo old >"$root/crates/a/old.rs"; sync
  check "$(cat "$root/crates/a/old.rs")/$(grep -c 'first sync' "$x/log")" old/0 "windows stale: a later sync sweeps nothing"
  rm -rf "$snap_dir"
  [ "$fail" = 0 ]
) || fail=1
# Manual live test (Win11Builder): from a branch with a fresh checkout name,
#   OCW_REMOTE_DIR='B:\src\OCW-synctest' scripts/remote.sh Win11Builder run 'Get-ChildItem -Recurse -File | Measure-Object | % Count'
# (first run: "sending N changed of N files", the count matches), again
# ("sending 0 changed"), then delete Cargo.toml and a source file there with
# `scripts/remote.sh Win11Builder ssh` and run once more (both re-sent);
# finally `cargo check -p ocw-core` there and remove the directory.

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
python3 scripts/test_web_scripts.py >/dev/null 2>&1 && echo "ok   web-serve.py, web_browser.py, test-web-readme.py unit tests" ||
  { echo "FAIL web script unit tests (python3 scripts/test_web_scripts.py)"; fail=1; }
exit $fail
