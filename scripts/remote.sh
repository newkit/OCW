#!/usr/bin/env bash
# Build and test on a remote machine from config/machines.toml (see config/README.md).
#
#   scripts/remote.sh MACHINE sync           copy the working tree (tracked and
#                                            untracked, git-ignored files excluded)
#   scripts/remote.sh MACHINE run CMD...     sync, then run CMD in the checkout
#                                            (with ~/.cargo/env sourced); copies
#                                            Cargo.lock back afterwards
#   scripts/remote.sh MACHINE ssh [CMD...]   plain ssh (no sync)
#   scripts/remote.sh MACHINE fetch REMOTE_PATH LOCAL_DIR
#                                            copy files back (e.g. test-output/)
#   scripts/remote.sh MACHINE put LOCAL_FILE... REMOTE_DIR
#                                            copy files (also git-ignored ones,
#                                            e.g. logs, data) into REMOTE_DIR of
#                                            the checkout (no sync)
#   scripts/remote.sh MACHINE field KEY      print the machine's KEY from
#                                            machines.toml (os, slots, …; for
#                                            scripts: never log `host`)
#   scripts/remote.sh MACHINE enabled        exit 0 if the machine is enabled
#                                            for building, else print why not
#                                            and exit 3 (no ssh)
#   scripts/remote.sh MACHINE slots          print its effective build slots
#
# Machines are enabled/disabled for building and their build slots set in
# the build dashboard (v2.66, spec §11.2): config/build-machines.toml
# (git-ignored, written by tools/build-dashboard/build_machines.py; same
# [[machine]] format as machines.toml, which stays untouched). `run` and
# `sync` refuse a machine with `enabled = false` (exit 3) before anything is
# copied; ssh, field, fetch and put stay allowed, and so do deploys:
# scripts/deploy-test.sh sets OCW_REMOTE_DEPLOY=1 (kiosks on disabled
# machines still get new builds). Its `slots` wins over machines.toml.
#
# On a branch other than main the remote checkout is REPO_PATH-BRANCH.
#
# Files are copied with rsync; config/ (machine config, keys) is never sent.
# The tree is taken as it is when remote.sh is called (a snapshot), and a
# run copies that snapshot once it has its build slot; files are compared by
# content, and changed ones get the time of the copy (see snapshot_tree,
# rsync_sync_opts, with_sync).
#
# Each `run` leaves a task marker on the machine while it runs (read by
# tools/build-dashboard): a JSON file PID-START.json in /tmp/ocw-tasks/
# (Windows: %TEMP%\ocw-tasks\) with the checkout, branch, command (shortened),
# start time, the remote shell's PID, its state (waiting for a build slot,
# running in slot N) and since when, and the machine's slot count. The run removes it when it ends; markers whose PID is gone
# (killed runs) are ignored by the dashboard and swept by the next run.
set -euo pipefail
cd "$(dirname "$0")/.."
snap_dir=""
trap 'if [ -n "$snap_dir" ]; then rm -rf "$snap_dir"; fi' EXIT

machine=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch|put|field|enabled|slots ...}; shift
action=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch|put|field|enabled|slots ...}; shift

# config/ is git-ignored, so another worktree uses the main checkout's copy.
# OCW_REMOTE_CONFIG names another config directory (scripts/test-remote.sh).
config_dir=${OCW_REMOTE_CONFIG:-config}
if [ ! -f "$config_dir/machines.toml" ] && [ -z "${OCW_REMOTE_CONFIG:-}" ]; then
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
  [ -n "$common" ] && config_dir="$(dirname "$common")/config"
fi
conf=$config_dir/machines.toml
[ -f "$conf" ] || { echo "$conf missing (copy config/machines.example.toml)" >&2; exit 1; }
# Absolute, because the sync runs from inside the snapshot directory.
config_dir=$(cd "$config_dir" && pwd)
conf=$config_dir/machines.toml

# Minimal TOML lookup: key of the [[machine]] block with name = MACHINE,
# falling back to [defaults]. `field KEY [FILE]` (default machines.toml).
field() {
  awk -v want="$machine" -v key="$1" '
    /^\[\[machine\]\]/ { inm=1; ind=0; cur=""; next }
    /^\[defaults\]/   { ind=1; inm=0; next }
    /^\[/             { inm=0; ind=0 }
    {
      line=$0; sub(/#.*/, "", line)
      if (match(line, /^[ \t]*[a-z_]+[ \t]*=/)) {
        k=line; sub(/[ \t]*=.*/, "", k); gsub(/[ \t]/, "", k)
        v=line; sub(/^[^=]*=[ \t]*/, "", v); gsub(/^"|"[ \t]*$/, "", v)
        if (inm && k=="name") cur=v
        if (inm && k==key) vals[cur]=v
        if (ind && k==key) def=v
      }
    }
    END { if (want in vals) print vals[want]; else print def }' "${2:-$conf}"
}

# The dashboard's settings (see the top): `ofield KEY` from
# build-machines.toml, empty when it or the key is missing; `ounesc`
# undoes its string escapes (\u0023 for #, \" and \\).
over=$config_dir/build-machines.toml
ofield() { if [ -f "$over" ]; then field "$1" "$over"; fi; }
ounesc() { local s=$1 b='\' q='"' h='#'; s=${s//"${b}u0023"/"$h"}; s=${s//"$b$q"/"$q"}; s=${s//"$b$b"/"$b"}; printf '%s' "$s"; }
build_enabled=$(ofield enabled)
build_reason=$(ounesc "$(ofield reason)")
# `run` and `sync` on a disabled machine: refused (exit 3), unless a deploy.
refuse_disabled() {
  [ "$build_enabled" = false ] || return 0
  local why=""
  [ -n "$build_reason" ] && why=" ($build_reason)"
  if [ "${OCW_REMOTE_DEPLOY:-0}" = 1 ]; then
    echo "remote.sh: $machine is disabled for building$why; allowed for a deploy (OCW_REMOTE_DEPLOY=1)" >&2
    return 0
  fi
  echo "remote.sh: $machine is disabled for building$why – enable it in the build dashboard" >&2
  exit 3
}

host=$(field host); user=$(field user); port=$(field port)
key=$(field identity_file); ssh_dir=$(field ssh_dir); repo=$(field repo_path)
main_repo=$repo
timeout_s=$(field connect_timeout_s); os=$(field os)
[ -n "$host" ] || { echo "machine '$machine' not found in $conf" >&2; exit 1; }
case "$key" in /*) ;; *) key="$config_dir/${ssh_dir:-ssh}/$key" ;; esac
# Branches other than main get their own checkout on the machine, so parallel
# work (e.g. a running benchmark) is not disturbed. OCW_REMOTE_DIR overrides.
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo main)
if [ -n "${OCW_REMOTE_DIR:-}" ]; then
  repo=$OCW_REMOTE_DIR
elif [ "$branch" != "main" ]; then
  repo="$repo-${branch//\//-}"
fi
# Branch checkouts build from scratch (own target/); several agents doing
# that at once overload the machine, so they get fewer parallel jobs.
# OCW_REMOTE_JOBS overrides on any branch (empty = cargo's default, all cores). A machine's
# `jobs` in machines.toml caps both (e.g. 2 on the 8 GB Windows machine,
# where each MSVC link of a Bevy binary takes ~5 GB).
jobs_env=""
machine_jobs=$(field jobs)
if [ "$branch" != "main" ]; then
  jobs=${OCW_REMOTE_JOBS-${machine_jobs:-4}}
else
  jobs=${OCW_REMOTE_JOBS-$machine_jobs}
fi
[ -n "$jobs" ] && jobs_env="CARGO_BUILD_JOBS=$jobs "

# Compiler cache (spec §11.2): with sccache in ~/.local/bin on the machine
# (`make deps-sccache`; Windows: %USERPROFILE%\.local\bin\sccache.exe,
# `make deps-sccache-windows`), every checkout there shares one cache, so a new
# agent checkout takes Bevy and the other dependencies from it instead of
# compiling them (workspace crates of debug builds are incremental and
# compiled as before). Cache size: the machine's `sccache_size` in
# machines.toml (default 30G). The checkout's own path is a base directory
# (SCCACHE_BASEDIRS, hashed in the client: SCCACHE_CLIENT_SIDE=1), so
# crates whose arguments or environment name it (OUT_DIR of build scripts,
# and everything built on them) hit across checkouts too. OCW_SCCACHE=0
# turns it off.
# Build directory per feature set (spec §11.2), opt-in (OCW_CARGO_SPLIT=1):
# scripts/cargo-shim/cargo first on PATH gives cargo calls with the voice
# feature their own build-dir (the Makefile keeps it first too, via
# OCW_CARGO_SHIM). Measured on marge on 2026-11-13: alternating plain and
# voice clippy runs in one target/ took 0.5-0.7 s each after the first two
# (cargo 1.99 keeps both feature sets' artefacts side by side), while the
# split built the shared dependencies twice (+63 s, +2.3 GB), so it is off.
shim_sh=""
if [ "${OCW_CARGO_SPLIT:-0}" = 1 ]; then
  shim_sh="if [ -x scripts/cargo-shim/cargo ]; then export OCW_CARGO_SHIM=\$PWD/scripts/cargo-shim PATH=\$PWD/scripts/cargo-shim:\$PATH; fi; "
fi
sccache_sh=""
sccache_size=$(field sccache_size)
if [ "${OCW_SCCACHE:-1}" != 0 ]; then
  sccache_sh="if [ -x \$HOME/.local/bin/sccache ]; then export RUSTC_WRAPPER=\$HOME/.local/bin/sccache SCCACHE_CACHE_SIZE=${sccache_size:-30G} SCCACHE_CLIENT_SIDE=1 SCCACHE_BASEDIRS=\$PWD; fi; "
fi

SSH=(ssh -i "$key" -p "${port:-22}" -o IdentitiesOnly=yes -o BatchMode=yes
     -o UserKnownHostsFile="$config_dir/${ssh_dir:-ssh}/known_hosts" -o StrictHostKeyChecking=yes
     -o ConnectTimeout="${timeout_s:-10}")
target="$user@$host"
# TOML basic strings escape backslashes ("B:\\src\\OCW"): unescape for Windows.
if [ "$os" = windows ]; then repo=${repo//\\\\/\\}; fi
# A PowerShell single-quoted string literal.
psq() { local q="'"; printf "'%s'" "${1//$q/$q$q}"; }

# Seeding (spec §11.2): a new checkout (no target/ yet) on Linux or macOS
# starts with a copy-on-write clone of the main checkout's target/debug and
# target/release – `cp --reflink=always` (Btrfs, XFS) or `cp -c` (APFS
# clonefile) – so cargo finds the dependencies built; the clone shares the
# disk blocks until either side rewrites a file, and each checkout keeps its
# own target/ (never shared: cargo locks it for a build, and feature sets
# would overwrite each other). A probe file tells whether the file system
# clones; where it does not (marge's ext4) nothing is copied and sccache
# does the work. Not while a build runs in the main checkout (a half
# written artefact would be cloned). Opt-in (OCW_SEED=1): measured on the
# Mac on 2026-11-13 the clone took 46 s but cargo still rebuilt 358 of 529
# crates (899 s), so it is off until a seed from the same dependency state
# proves faster than sccache alone.
read -r -d '' seed_sh <<'EOF' || true
ocw_seed() { m=$1; r=$2; [ "$m" = "$r" ] && return 0; [ -e "$r/target" ] && return 0; [ -d "$m/target" ] || return 0;
for g in $(find /tmp/ocw-tasks/ -name '*.json' 2>/dev/null); do grep -q '"state":"running"' $g && grep -q "\"checkout\":\"${m##*/}\"" $g && { echo "remote.sh: main checkout busy, no target/ seed" >&2; return 0; }; done;
[ -d $m/target/debug/.fingerprint ] || return 0; probe=$(find $m/target $m/target/debug -maxdepth 1 -type f 2>/dev/null | head -n 1); [ -n "$probe" ] || return 0;
ocw_cpc() { if [ "$(uname -s)" = Darwin ]; then cp -c -Rp "$@"; else cp -a --reflink=always "$@"; fi; };
mkdir -p $r/target.seed; ocw_cpc $probe $r/target.seed/ 2>/dev/null || { rm -rf $r/target.seed; echo "remote.sh: no copy-on-write clones here, no target/ seed (sccache only)" >&2; return 0; };
t0=$(date +%s); for d in debug release build-voice; do [ -d $m/target/$d ] && ocw_cpc $m/target/$d $r/target.seed/ 2>/dev/null; done;
mv $r/target.seed $r/target && echo "remote.sh: target/ seeded from the main checkout (copy-on-write) in $(($(date +%s) - t0)) s" >&2; };
EOF

# Snapshot at invocation (v2.63, spec §11.2): `run` and `sync` copy the
# working tree (tracked and untracked files, git-ignored ones and config/
# excluded) into a temporary directory the moment they are invoked; a run
# that then waits for a build slot syncs that snapshot once it has the slot
# (before, it synced whatever the tree was, and a run invoked during a merge
# in the main checkout could build a half-merged tree – or a later run's
# sync overwrote the remote checkout while an earlier one waited). Files:
# $snap_dir/list (all paths, also deleted ones), present (the existing
# ones), tree/ (their copies). Removed when remote.sh exits.
snapshot_tree() {
  snap_dir=$(mktemp -d "${TMPDIR:-/tmp}/ocw-snap.XXXXXX")
  { git ls-files -co --exclude-standard | grep -v '^config/' || true; } >"$snap_dir/list"
  local f
  while IFS= read -r f; do if [ -e "$f" ]; then printf '%s\n' "$f"; fi; done <"$snap_dir/list" >"$snap_dir/present"
  mkdir "$snap_dir/tree"
  tar -cf - -T "$snap_dir/present" | tar -xf - -C "$snap_dir/tree"
}

# Copying without the source mtimes (v2.63): rsync -a kept them, so a file
# replaced by an older version (a branch switch, a revert, an older
# snapshot) arrived with an mtime older than the build that had already
# compiled the newer one, and cargo kept the stale artefact. Now files are
# compared by content (--checksum, no -t): unchanged files are not touched,
# changed ones get the time of the copy, so cargo rebuilds exactly them.
rsync_sync_opts=(-rlpgoD --checksum)

sync_tree() {
  if [ "$os" = windows ]; then
    sync_tree_windows
    return
  fi
  if [ "${OCW_SEED:-0}" = 1 ]; then
    "${SSH[@]}" "$target" "${seed_sh//$'\n'/ } mkdir -p $repo && ocw_seed $main_repo $repo"
  else
    "${SSH[@]}" "$target" "mkdir -p $repo"
  fi
  if [ "$os" = macos ]; then
    # macOS ships openrsync, which lacks --delete-missing-args: send only the
    # files that exist and delete tracked files removed here with rm.
    local missing=() f
    while IFS= read -r f; do [ -e "$snap_dir/tree/$f" ] || missing+=("'${f//\'/\'\\\'\'}'"); done <"$snap_dir/list"
    if [ ${#missing[@]} -gt 0 ]; then
      "${SSH[@]}" "$target" "cd $repo && rm -f ${missing[*]}"
    fi
    (cd "$snap_dir/tree" && rsync "${rsync_sync_opts[@]}" --files-from="$snap_dir/present" -e "${SSH[*]}" ./ "$target:$repo/")
    return
  fi
  (cd "$snap_dir/tree" && rsync "${rsync_sync_opts[@]}" --delete-missing-args --files-from="$snap_dir/list" \
    -e "${SSH[*]}" ./ "$target:$repo/")
}

# A PowerShell script as an encoded command (no quoting issues).
ps_encoded() { printf 'powershell -NoProfile -NonInteractive -EncodedCommand %s' "$(printf '%s' "$1" | iconv -f utf-8 -t utf-16le | base64 -w0)"; }

# Windows sync state: $repo\.ocw-sync\manifest.tsv holds, per file sent,
# its SHA-1, and the size and mtime (ticks) it got there. This prints
# "sha<TAB>path" for the entries whose file is still unchanged there.
read -r -d '' win_manifest_ps <<'EOF' || true
$r = @REPO@; $m = Join-Path $r '.ocw-sync\manifest.tsv'
if (Test-Path -LiteralPath $m) {
  foreach ($l in [IO.File]::ReadAllLines($m)) {
    $p = $l.Split([char]9, 4); if ($p.Count -lt 4) { continue }
    $fi = New-Object IO.FileInfo (Join-Path $r $p[3])
    if ($fi.Exists -and $fi.Length -eq [int64]$p[1] -and $fi.LastWriteTimeUtc.Ticks -eq [int64]$p[2]) { [Console]::Out.WriteLine($p[0] + [char]9 + $p[3]) }
  }
}
EOF
# After extraction: the new manifest from .ocw-sync\incoming.tsv (sha<TAB>path
# of every file of the snapshot) and the files' size and mtime there now.
read -r -d '' win_manifest_write_ps <<'EOF' || true
$r = @REPO@; $d = Join-Path $r '.ocw-sync'; $in = Join-Path $d 'incoming.tsv'
$sb = New-Object Text.StringBuilder
foreach ($l in [IO.File]::ReadAllLines($in)) {
  $p = $l.Split([char]9, 2); if ($p.Count -lt 2) { continue }
  $fi = New-Object IO.FileInfo (Join-Path $r $p[1])
  if ($fi.Exists) { [void]$sb.Append($p[0] + [char]9 + $fi.Length + [char]9 + $fi.LastWriteTimeUtc.Ticks + [char]9 + $p[1] + [char]10) }
}
[IO.File]::WriteAllText((Join-Path $d 'manifest.tsv'), $sb.ToString()); Remove-Item -LiteralPath $in
EOF

# The files of the snapshot whose content is not (known to be) on the
# machine: `win_changed VALID HASHES` prints the paths of the HASHES lines
# (sha<TAB>path) that are not in VALID (the same format, from the machine).
# (FILENAME, not NR == FNR: with an empty VALID – a new checkout – NR == FNR
# also held for HASHES, which then sent nothing.)
win_changed() { awk -F '\t' 'FILENAME == ARGV[1] { v[$0] = 1; next } !($0 in v) { print $2 }' "$1" "$2"; }

# Windows has no rsync; its built-in tar.exe unpacks a tar stream. Only
# files whose content differs from what the manifest says is there are
# sent, and unpacked with -m (mtime = now) for the same reason as
# rsync_sync_opts. The first sync into a checkout sends everything (it has
# no manifest yet). Files deleted here are removed there with Remove-Item.
sync_tree_windows() {
  "${SSH[@]}" "$target" "New-Item -ItemType Directory -Force -Path $(psq "$repo") | Out-Null"
  local gone=() f list
  while IFS= read -r f; do [ -e "$snap_dir/tree/$f" ] || gone+=("$(psq "$f")"); done <"$snap_dir/list"
  if [ ${#gone[@]} -gt 0 ]; then
    list=$(IFS=,; echo "${gone[*]}")
    "${SSH[@]}" "$target" "Set-Location -LiteralPath $(psq "$repo"); Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $list"
  fi
  "${SSH[@]}" "$target" "$(ps_encoded "${win_manifest_ps//@REPO@/"$(psq "$repo")"}")" | tr -d '\r' >"$snap_dir/valid" || : >"$snap_dir/valid"
  (cd "$snap_dir/tree" && tr '\n' '\0' <"$snap_dir/present" | xargs -0 -r sha1sum) |
    sed 's/^\([0-9a-f]*\) [ *]/\1\t/' >"$snap_dir/hashes"
  mkdir -p "$snap_dir/tree/.ocw-sync"
  cp "$snap_dir/hashes" "$snap_dir/tree/.ocw-sync/incoming.tsv"
  { win_changed "$snap_dir/valid" "$snap_dir/hashes"; echo .ocw-sync/incoming.tsv; } >"$snap_dir/send"
  echo "remote.sh: sending $(($(wc -l <"$snap_dir/send") - 1)) changed of $(wc -l <"$snap_dir/present") files" >&2
  (cd "$snap_dir/tree" && tar -cf - -T "$snap_dir/send") |
    "${SSH[@]}" "$target" "tar -xmf - -C $(psq "$repo"); $(ps_encoded "${win_manifest_write_ps//@REPO@/"$(psq "$repo")"}")"
}

# Dev profile change (spec §11.2, v2.63): when the [profile.dev] /
# [profile.test] sections of Cargo.toml change, cargo builds everything anew
# and keeps the old artefacts next to the new ones; then target/debug is
# removed once, before the build. The hash of those sections is kept in
# .ocw-profile-hash next to target/. Not while another run of the same
# checkout is running; the first run only records the hash. OCW_PROFILE_SWEEP=0
# turns it off. Logged to the build log's janitor.jsonl.
profile_hash() {
  awk '/^\[/ { p = ($0 ~ /^\[profile\.(dev|test)[].]/) } p' "$snap_dir/tree/Cargo.toml" 2>/dev/null | cksum | awk '{ print $1 }'
}
profile_check() { # REMOTE_PID (of the run, its own marker is skipped)
  [ "${OCW_PROFILE_SWEEP:-1}" = 0 ] && return 0
  [ -f "$snap_dir/tree/Cargo.toml" ] || return 0
  local h out kb
  h=$(profile_hash)
  if [ "$os" = windows ]; then
    out=$("${SSH[@]}" "$target" "$(ps_encoded "\$ErrorActionPreference = 'SilentlyContinue'; Set-Location -LiteralPath $(psq "$repo"); \$h = '$h'; \$f = '.ocw-profile-hash'; \$old = if (Test-Path -LiteralPath \$f) { ([IO.File]::ReadAllText(\$f)).Trim() } else { '' }; if (\$old -eq \$h) { exit 0 }; if (\$old -and (Test-Path -LiteralPath 'target\\debug')) { \$busy = @(Get-ChildItem -LiteralPath (Join-Path \$env:TEMP 'ocw-tasks') -Filter '*.json' | Where-Object { \$_.BaseName -notlike '$1-*' -and (Get-Process -Id (\$_.BaseName -split '-')[0]) -and ((Get-Content -Raw -LiteralPath \$_.FullName) -match ('\"checkout\":\"' + $(psq "$checkout_dir") + '\"')) -and ((Get-Content -Raw -LiteralPath \$_.FullName) -match '\"state\":\"running\"') }); if (\$busy.Count) { '@@profile busy'; exit 0 }; \$kb = [int64](((Get-ChildItem -LiteralPath 'target\\debug' -Recurse -File -Force | Measure-Object Length -Sum).Sum) / 1KB); Rename-Item -LiteralPath 'target\\debug' -NewName ('debug.ocw-profile.' + \$PID); Remove-Item -Recurse -Force -LiteralPath ('target\\debug.ocw-profile.' + \$PID); '@@profile removed ' + \$kb }; [IO.File]::WriteAllText(\$f, \$h)")" </dev/null 2>/dev/null | tr -d '\r') || return 0
  else
    out=$("${SSH[@]}" "$target" "cd $repo 2>/dev/null || exit 0; h=$h; old=\$(cat .ocw-profile-hash 2>/dev/null); [ \"\$old\" = \"\$h\" ] && exit 0; if [ -n \"\$old\" ] && [ -d target/debug ]; then for g in /tmp/ocw-tasks/*.json; do [ -e \$g ] || continue; q=\${g##*/}; q=\${q%%-*}; [ \"\$q\" = '$1' ] && continue; grep -q '\"checkout\":\"$checkout_dir\"' \$g && grep -q '\"state\":\"running\"' \$g && ps -p \$q >/dev/null 2>&1 && { echo '@@profile busy'; exit 0; }; done; kb=\$(du -sk target/debug | cut -f1); mv target/debug target/debug.ocw-profile.\$\$ && rm -rf target/debug.ocw-profile.\$\$ && echo \"@@profile removed \$kb\"; fi; echo \$h >.ocw-profile-hash" </dev/null 2>/dev/null) || return 0
  fi
  case "$out" in
    *"@@profile removed"*)
      kb=${out##*@@profile removed }; kb=${kb//[!0-9]/}
      echo "remote.sh: dev profile changed in Cargo.toml: removed target/debug ($((${kb:-0} / 1048576)) GB) before building" >&2
      janitor_log "$checkout_dir/target/debug" "${kb:-0}" "dev profile changed in Cargo.toml" ;;
    *"@@profile busy"*) echo "remote.sh: dev profile changed, but another run of this checkout is building: target/debug kept" >&2 ;;
  esac
}

# One line in the build log's janitor.jsonl (like the dashboard's janitor).
janitor_log() { # path kb reason
  [ "${OCW_BUILD_LOG:-1}" = 0 ] && return 0
  local d t
  d="$log_root/$(date +%Y-%m-%d)"
  mkdir -p "$d" 2>/dev/null || return 0
  t=$(date --iso-8601=seconds 2>/dev/null || date +%Y-%m-%dT%H:%M:%S%z)
  printf '{"action":"delete","bytes":%s,"kind":"profile","machine":%s,"mode":"on","path":%s,"reason":%s,"source":"remote.sh","time":%s}\n' \
    "$(($2 * 1024))" "$(json_str "$machine")" "$(json_str "$1")" "$(json_str "$3")" "$(json_str "$t")" >>"$d/janitor.jsonl"
}

# The sync handshake: `with_sync CMD...` runs CMD (the run's ssh) with its
# stdin from a FIFO. The remote side prints "@@ocw-sync PID" once it holds
# its build slot and waits for a line on stdin; sync_watch (reading CMD's
# stdout) then syncs the snapshot, checks the dev profile and answers "go"
# (or "fail"), and passes everything else through. stdin is closed after
# the answer, so the command itself reads EOF as before.
with_sync() {
  local fifo=$snap_dir/go rc
  rm -f "$fifo"
  mkfifo "$fifo"
  set +e
  "$@" <"$fifo" | sync_watch "$fifo"
  rc=${PIPESTATUS[0]}
  set -e
  rm -f "$fifo"
  return "$rc"
}
sync_watch() {
  local fifo=$1 line ok t0
  exec 3>"$fifo"
  while IFS= read -r line; do
    case "$line" in
      "@@ocw-sync"*)
        t0=$(date +%s.%N)
        ok=go
        sync_tree </dev/null >&2 || ok=fail
        if [ "$ok" = go ]; then profile_check "${line#@@ocw-sync }" </dev/null >&2 || true; fi
        LC_ALL=C awk -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN { printf "%.1f", b - a }' >"$snap_dir/sync_s"
        printf '%s\n' "$ok" >&3
        exec 3>&-
        cat
        return 0
        ;;
      *) printf '%s\n' "$line" ;;
    esac
  done
  if [ -n "$line" ]; then printf '%s' "$line"; fi
  exec 3>&-
}

# Build slots: at most N commands run at once on a machine, because many
# parallel builds of the Bevy app (~4-5 GB each) run it out of memory.
# N = OCW_REMOTE_SLOTS, else the machine's `slots` in build-machines.toml
# (set in the build dashboard), else its `slots` in machines.toml (or
# [defaults] slots), else per OS: linux 2, windows 1, macos 1. 0 = no limit.
# A changed N applies to runs started afterwards: they try slots 1..N, while
# a running one keeps the slot it holds (also one above a lowered N; its
# lock file or directory just stays unused once it ends).
# A run that finds all slots taken marks itself "waiting" and polls every
# second, taking whichever slot frees first – in order of arrival: while an
# older run (by its task marker's start, then PID) is still waiting, a
# waiting run leaves the free slot to it (Linux/macOS; before v2.59 the
# first poll won, and a run could wait 20 min while later ones went
# ahead). Linux: flock on
# /tmp/ocw-build-slot-N.lock; macOS (no flock): atomic mkdir of
# /tmp/ocw-build-slot-N.d holding the owner's PID (taken over when that PID
# is gone); Windows: %TEMP%\ocw-tasks\slot-N.lock opened exclusively.
case "$os" in windows | macos) default_slots=1 ;; *) default_slots=2 ;; esac
uint_or_empty() { case "$1" in '' | *[!0-9]*) ;; *) printf '%s' "$1" ;; esac; }
over_slots=$(uint_or_empty "$(ofield slots)")
conf_slots=$(uint_or_empty "$(field slots)")
slots=$(uint_or_empty "${OCW_REMOTE_SLOTS:-}")
slots=${slots:-${over_slots:-${conf_slots:-$default_slots}}}
slot_list=""
for ((i = 1; i <= slots; i++)); do slot_list+="$i "; done
checkout_dir=${repo##*[\\/]}

# The command for task markers: one line, at most 200 characters.
short_cmd() {
  local c
  c=$(printf '%s' "$*" | tr '\000-\037' ' ')
  [ ${#c} -gt 200 ] && c="${c:0:197}..."
  printf '%s' "$c"
}
# A JSON string literal (input without control characters).
# (Replacements are quoted: bash 5.2's patsub_replacement treats & and \.)
json_str() { local b='\' q='"' s; s=${1//"$b"/"$b$b"}; printf '"%s"' "${s//"$q"/"$b$q"}"; }
# A POSIX shell single-quoted literal.
shq() { local q="'" r="'\\''"; printf "'%s'" "${1//"$q"/"$r"}"; }

# Prelude of a run on Linux/macOS (bash or zsh, joined into one line):
# `ocw_mark STATE [SLOT]` writes the task marker, the EXIT trap removes it
# (and frees a macOS slot dir, $ocw_ld), markers of dead PIDs are swept.
# POSIX sh only: the remote shell is bash on Linux, zsh on macOS.
read -r -d '' marker_sh <<'EOF' || true
ocw_d=/tmp/ocw-tasks; mkdir -p $ocw_d 2>/dev/null; ocw_t0=$(date +%s); ocw_f=$ocw_d/$$-$ocw_t0.json;
for g in $(find $ocw_d/ -name '*.json' 2>/dev/null); do p=${g##*/}; ps -p ${p%%-*} >/dev/null 2>&1 || rm -f $g; done;
ocw_mark() { printf '{"v":1,"pid":%s,"state":"%s","slot":%s,"start":%s,"since":%s,%s}\n' $$ $1 ${2:-null} $ocw_t0 $(date +%s) "$ocw_meta" >$ocw_f.tmp 2>/dev/null && mv -f $ocw_f.tmp $ocw_f; };
ocw_ld=; trap 'rm -f $ocw_f $ocw_f.tmp; [ -n "$ocw_ld" ] && [ "$(cat $ocw_ld/pid 2>/dev/null)" = "$$" ] && rm -rf $ocw_ld; :' EXIT; trap 'exit 129' HUP; trap 'exit 130' INT; trap 'exit 143' TERM;
EOF

# A run on Windows (PowerShell, joined into one line): task marker, build
# slot (lock files opened with FileShare None, polled while all are taken),
# the command (@RUN@), marker removed in finally. Exit code: that of the last
# native program (cargo ...), else 1 if the last PowerShell command failed
# ($LASTEXITCODE alone gave 0 for a failed cmdlet such as Get-Item).
read -r -d '' marker_ps <<'EOF' || true
$ocwD = Join-Path $env:TEMP 'ocw-tasks'; [void](New-Item -ItemType Directory -Force -Path $ocwD);
$ocwT0 = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); $ocwF = Join-Path $ocwD ('' + $PID + '-' + $ocwT0 + '.json');
Get-ChildItem -LiteralPath $ocwD -Filter '*.json' | ForEach-Object { if (-not (Get-Process -Id ($_.BaseName -split '-')[0] -ErrorAction SilentlyContinue)) { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue } };
function ocwMark($st, $sl) { $m = [ordered]@{ v = 1; pid = $PID; state = $st; slot = $sl; start = $ocwT0; since = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); slots = @SLOTS@; checkout = @CHECKOUT@; branch = @BRANCH@; command = @CMD@; os = 'windows' }; [IO.File]::WriteAllText($ocwF + '.tmp', ($m | ConvertTo-Json -Compress)); Move-Item -Force -LiteralPath ($ocwF + '.tmp') -Destination $ocwF };
$ocwRc = 0; $ocwLock = $null; $ocwSlot = $null; $ocwWarn = $false;
try {
if (@SLOTS@ -gt 0) { while (-not $ocwLock) { foreach ($s in 1..@SLOTS@) { try { $ocwLock = [IO.File]::Open((Join-Path $ocwD ('slot-' + $s + '.lock')), 'OpenOrCreate', 'ReadWrite', 'None'); $ocwSlot = $s; break } catch {} }; if (-not $ocwLock) { if (-not $ocwWarn) { ocwMark 'waiting' $null; [Console]::Error.WriteLine('remote.sh: waiting for a free build slot'); $ocwWarn = $true }; Start-Sleep -Milliseconds 1000 } }; if ($ocwWarn) { [Console]::Error.WriteLine('remote.sh: got build slot ' + $ocwSlot + ' after ' + ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() - $ocwT0) + ' s') } };
ocwMark 'running' $ocwSlot;
@SYNC@
if ($ocwGo -ne 'go') { [Console]::Error.WriteLine('remote.sh: tree sync failed'); $ocwRc = 1 } else {
@RUN@; $ocwRc = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } elseif ($ocwOk) { 0 } else { 1 } }
} finally { if ($ocwLock) { $ocwLock.Dispose() }; Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $ocwF, ($ocwF + '.tmp') };
exit $ocwRc
EOF

# Linux: flock(1) on a lock file per slot, held on fd 9 by the run's shell
# (the kernel drops it when the shell dies).
read -r -d '' slot_take_flock <<'EOF' || true
ocw_take() { exec 9>/tmp/ocw-build-slot-$1.lock && flock -n 9; };
ocw_free() { exec 9>&-; };
EOF

# First come, first served: `ocw_older` is true while a live run that
# started earlier (or in the same second with a lower PID) is waiting.
read -r -d '' slot_fifo <<'EOF' || true
ocw_older() { for g in $(find $ocw_d/ -name '*.json' 2>/dev/null); do [ "$g" = "$ocw_f" ] && continue;
grep -q '"state":"waiting"' $g 2>/dev/null || continue;
ocw_gs=$(sed -n 's/.*"start":\([0-9]*\).*/\1/p' $g); ocw_gp=${g##*/}; ocw_gp=${ocw_gp%%-*}; [ -n "$ocw_gs" ] || continue;
if [ $ocw_gs -lt $ocw_t0 ] || { [ $ocw_gs -eq $ocw_t0 ] && [ $ocw_gp -lt $$ ]; }; then ps -p $ocw_gp >/dev/null 2>&1 && return 0; fi;
done; return 1; };
EOF

# macOS has no flock(1): a slot is the directory /tmp/ocw-build-slot-N.d
# (mkdir is atomic) with the owner's PID in it. A slot whose owner is gone
# (killed run; or no PID written after a minute) is taken over under a
# second mkdir lock (N.d.take), re-checking that the PID is still the dead
# one; the EXIT trap removes the directory (also after INT/TERM/HUP).
read -r -d '' slot_take_macos <<'EOF' || true
ocw_take() { ocw_s=/tmp/ocw-build-slot-$1.d;
if mkdir $ocw_s 2>/dev/null; then echo $$ >$ocw_s/pid; ocw_ld=$ocw_s; return 0; fi;
ocw_p=$(cat $ocw_s/pid 2>/dev/null);
if [ -n "$ocw_p" ]; then ps -p $ocw_p >/dev/null 2>&1 && return 1;
else [ -n "$(find $ocw_s -maxdepth 0 -mmin +1 2>/dev/null)" ] || return 1; fi;
[ -n "$(find $ocw_s.take -maxdepth 0 -mmin +1 2>/dev/null)" ] && rmdir $ocw_s.take 2>/dev/null;
mkdir $ocw_s.take 2>/dev/null || return 1;
if [ -d $ocw_s ] && [ "$(cat $ocw_s/pid 2>/dev/null)" = "$ocw_p" ]; then echo $$ >$ocw_s/pid; ocw_ld=$ocw_s; rmdir $ocw_s.take; return 0; fi;
rmdir $ocw_s.take; return 1; };
ocw_free() { :; };
EOF

# Runs a command in PowerShell in the checkout: rustup's cargo first on PATH,
# fewer build jobs for branch checkouts.
run_windows() {
  local jobs_num jobs_ps="" ps run
  jobs_num=$(printf '%s' "$jobs_env" | grep -o '[0-9]*' || true)
  [ -n "$jobs_num" ] && jobs_ps="\$env:CARGO_BUILD_JOBS='$jobs_num'; "
  # sccache from %USERPROFILE%\.local\bin (make deps-sccache-windows).
  local sccache_ps=""
  if [ "${OCW_SCCACHE:-1}" != 0 ]; then
    sccache_ps="\$ocwSc = Join-Path \$env:USERPROFILE '.local\\bin\\sccache.exe'; if (Test-Path -LiteralPath \$ocwSc) { \$env:RUSTC_WRAPPER = \$ocwSc; \$env:SCCACHE_CACHE_SIZE = '${sccache_size:-30G}'; \$env:SCCACHE_CLIENT_SIDE = '1'; \$env:SCCACHE_BASEDIRS = (Get-Location).Path }; "
  fi
  # The command runs as a script block (dot-sourced): the wrapper is one
  # line, which a `#` comment in the command would otherwise cut off. It
  # records $? of its last statement in $ocwOk (for the exit code).
  # rustup's cargo: in CARGO_HOME when it was moved off C:
  # (scripts/windows-move-toolchain.ps1), else %USERPROFILE%\.cargo.
  run="Set-Location -LiteralPath $(psq "$repo"); New-Item -ItemType File -Force -Path '.ocw-last-run' | Out-Null; \$ocwCh = if (\$env:CARGO_HOME) { \$env:CARGO_HOME } else { Join-Path \$env:USERPROFILE '.cargo' }; \$env:Path = (Join-Path \$ocwCh 'bin') + ';' + \$env:Path;\$env:OCW_BUILD_ID='${build_id:-}'; ${jobs_ps}${sccache_ps}. ([scriptblock]::Create($(psq "$*") + [char]10 + '\$ocwOk = \$?'))"
  ps=${marker_ps//$'\n'/ }
  ps=${ps//@CHECKOUT@/"$(psq "$checkout_dir")"}
  ps=${ps//@BRANCH@/"$(psq "$branch")"}
  ps=${ps//@CMD@/"$(psq "$(short_cmd "$@")")"}
  ps=${ps//@SLOTS@/"$slots"}
  ps=${ps//@SYNC@/"${win_sync_ps:-\$ocwGo = 'go';}"}
  ps=${ps//@RUN@/"$run"}
  "${SSH[@]}" "$target" "$ps"
}

# Build log for the art project (art/ART-PROJECT.md §2): every `run` is
# recorded in the main checkout's test-output/build-log/YYYY-MM-DD/ – one
# JSON line per run in runs.jsonl (machine, command, start, seconds, exit
# code; sync_s = the copy of the tree; from the output, by
# scripts/build_profile.py cargo-log: cargo_s = cargo's summed "Finished …
# in" times, cargo_builds, compiled = crates compiled or checked, wait_s =
# the build-slot wait; null where cargo ran with -q or its output was
# filtered) and the run's output in HHMMSS-MACHINE.log. Host, user and key
# paths are replaced by the machine name before the files are kept.
# OCW_BUILD_LOG=0 turns it off.
log_root="$(dirname "$config_dir")/test-output/build-log"
log_start() {
  [ "${OCW_BUILD_LOG:-1}" = 0 ] && return 0
  log_t0=$(date +%s.%N)
  log_iso=$(date --iso-8601=seconds 2>/dev/null || date +%Y-%m-%dT%H:%M:%S%z)
  log_dir="$log_root/$(date +%Y-%m-%d)"
  mkdir -p "$log_dir" 2>/dev/null || return 0
  log_file="$log_dir/$(date +%H%M%S)-$machine-$$.log"
  : >"$log_file"
}
log_end() {
  local rc=$1; shift
  [ "${OCW_BUILD_LOG:-1}" = 0 ] && return 0
  [ -n "${log_file:-}" ] || return 0
  local t1 secs extra=""
  t1=$(date +%s.%N)
  secs=$(awk -v a="$log_t0" -v b="$t1" 'BEGIN { printf "%.1f", b - a }')
  # Mask host, user@host and the key path (they never leave config/).
  if [ -s "$log_file" ]; then
    sed -i -e "s|$user@$host|$machine|g" -e "s|$host|$machine|g" -e "s|$key|<key>|g" "$log_file" 2>/dev/null || true
  fi
  # Cargo's own numbers from the output: {"cargo_s":…,…} without braces.
  extra=$(python3 scripts/build_profile.py cargo-log "$log_file" 2>/dev/null | sed -n 's/^{\(.*\)}$/,\1/p' || true)
  printf '{"start":%s,"seconds":%s,"machine":%s,"os":%s,"checkout":%s,"branch":%s,"command":%s,"exit":%s,"log":%s,"sync_s":%s%s}\n' \
    "$(json_str "$log_iso")" "$secs" "$(json_str "$machine")" "$(json_str "$os")" \
    "$(json_str "$(basename "$PWD")")" "$(json_str "$branch")" "$(json_str "$(short_cmd "$@")")" \
    "$rc" "$(json_str "$(basename "$log_file")")" "${sync_s:-null}" "$extra" >>"$log_dir/runs.jsonl"
}

# Runs "$@" with its stdout and stderr also appended to the run's log file.
logged() {
  if [ -n "${log_file:-}" ] && [ "${OCW_BUILD_LOG:-1}" != 0 ]; then
    "$@" > >(tee -a "$log_file") 2> >(tee -a "$log_file" >&2)
  else
    "$@"
  fi
}

case "$action" in
  sync) refuse_disabled; snapshot_tree; sync_tree ;;
  enabled)
    if [ "$build_enabled" = false ]; then
      echo "$machine is disabled for building${build_reason:+ ($build_reason)}"
      exit 3
    fi
    echo "$machine is enabled for building" ;;
  slots) echo "$slots" ;;
  run)
    refuse_disabled
    log_start
    # The tree as it is now (see snapshot_tree); synced once the run has its
    # build slot (with_sync, sync_watch).
    snapshot_tree
    status=0
    # The remote copy has no .git: pass the commit for the build id
    # (apps/desktop/build.rs, scripts/build-id.sh).
    build_id=$(scripts/build-id.sh)
    if [ "$os" = windows ]; then
      # The slot-time handshake on Windows is opt-in (OCW_WIN_HANDSHAKE=1):
      # in its first real runs (2026-11-13) the remote side stayed in
      # ReadLine after the sync, and Windows runs could not be tested
      # further. By default the snapshot is synced now, before the run waits
      # for its slot (the content/mtime fix applies either way).
      if [ "${OCW_WIN_HANDSHAKE:-0}" = 1 ]; then
        win_sync_ps="[Console]::Out.WriteLine('@@ocw-sync ' + \$PID); [Console]::Out.Flush(); \$ocwGo = [Console]::In.ReadLine();"
        logged with_sync run_windows "$@" || status=$?
        sync_s=$(cat "$snap_dir/sync_s" 2>/dev/null || true)
      else
        win_sync_ps="\$ocwGo = 'go';"
        sync_t0=$(date +%s.%N)
        logged sync_tree </dev/null || status=$?
        profile_check none </dev/null || true
        sync_s=$(LC_ALL=C awk -v a="$sync_t0" -v b="$(date +%s.%N)" 'BEGIN { printf "%.1f", b - a }')
        if [ "$status" = 0 ]; then logged run_windows "$@" </dev/null || status=$?; fi
      fi
      "${SSH[@]}" "$target" "tar -cf - -C $(psq "$repo") Cargo.lock" 2>/dev/null | tar -xf - 2>/dev/null || true
      log_end "$status" "$@"
      exit "$status"
    fi
    slot_guard=""
    if [ "$slots" -gt 0 ]; then
      # Shell functions (joined into one line): `ocw_take N` tries to take
      # slot N; `ocw_slot` tries all slots once a second until one is free.
      if [ "$os" = macos ]; then
        take=${slot_take_macos//$'\n'/ }
      else
        take=${slot_take_flock//$'\n'/ }
      fi
      # A waiting run whose ssh session is gone (remote.sh interrupted; the
      # shell's parent sshd ended) gives up instead of running later. A run
      # that waited prints "remote.sh: got build slot N after S s" (the
      # build log's wait_s).
      slot_guard="$take ${slot_fifo//$'\n'/ } ocw_slot() { ocw_w=; ocw_pp=\$PPID; while :; do if [ -z \"\$ocw_w\" ] || ! ocw_older; then for s in $slot_list; do ocw_take \$s && { ocw_n=\$s; [ -n \"\$ocw_w\" ] && echo \"remote.sh: got build slot \$s after \$((\$(date +%s) - ocw_t0)) s\" >&2; return 0; }; done; fi; ocw_free; if [ -z \"\$ocw_w\" ]; then ocw_w=1; ocw_mark waiting; echo 'remote.sh: waiting for a free build slot' >&2; fi; sleep 1; ps -p \$ocw_pp >/dev/null 2>&1 || exit 129; done; }; ocw_slot; "
    fi
    meta="\"slots\":$slots,\"checkout\":$(json_str "$checkout_dir"),\"branch\":$(json_str "$branch"),\"command\":$(json_str "$(short_cmd "$@")"),\"os\":\"$os\""
    marker="ocw_meta=$(shq "$meta"); ${marker_sh//$'\n'/ } "
    # After the slot: the handshake (@@ocw-sync, see with_sync), then the
    # command; .ocw-last-run tells the janitor when the checkout was used.
    sync_sh="echo \"@@ocw-sync \$\$\"; read -r ocw_go; [ \"\$ocw_go\" = go ] || { echo 'remote.sh: tree sync failed' >&2; exit 1; }; "
    logged with_sync "${SSH[@]}" "$target" "${marker}${slot_guard}ocw_mark running \$ocw_n; ${sync_sh}cd $repo && : >.ocw-last-run && . ~/.cargo/env && ${sccache_sh}${shim_sh}export ${jobs_env:-OCW_REMOTE=1} OCW_BUILD_ID='$build_id' && $*" || status=$?
    sync_s=$(cat "$snap_dir/sync_s" 2>/dev/null || true)
    # Builds only happen remotely: bring back a Cargo.lock that cargo
    # updated there (new dependencies), so it can be committed.
    rsync -a -e "${SSH[*]}" "$target:$repo/Cargo.lock" ./Cargo.lock 2>/dev/null || true
    log_end "$status" "$@"
    exit "$status"
    ;;
  ssh) "${SSH[@]}" "$target" "$@" ;;
  field) field "${1:?usage: remote.sh MACHINE field KEY}" ;;
  put)
    [ $# -ge 2 ] || { echo "usage: remote.sh MACHINE put LOCAL_FILE... REMOTE_DIR" >&2; exit 2; }
    dst=${*: -1}
    set -- "${@:1:$#-1}"
    # One tar stream (Windows has tar.exe but no rsync), files flat in dst.
    tar_args=()
    for f in "$@"; do
      [ -f "$f" ] || { echo "$f: no such file" >&2; exit 1; }
      tar_args+=(-C "$(cd "$(dirname "$f")" && pwd)" "$(basename "$f")")
    done
    if [ "$os" = windows ]; then
      tar -cf - "${tar_args[@]}" | "${SSH[@]}" "$target" \
        "\$d = Join-Path $(psq "$repo") $(psq "$dst"); New-Item -ItemType Directory -Force -Path \$d | Out-Null; tar -xf - -C \$d"
    else
      tar -cf - "${tar_args[@]}" | "${SSH[@]}" "$target" "mkdir -p $repo/$(shq "$dst") && tar -xf - -C $repo/$(shq "$dst")"
    fi
    ;;
  fetch)
    src=${1:?remote path}; dst=${2:?local dir}
    mkdir -p "$dst"
    if [ "$os" = windows ]; then
      "${SSH[@]}" "$target" "tar -cf - -C $(psq "$repo") $(psq "$src")" | tar -xf - -C "$dst"
    else
      rsync -a -e "${SSH[*]}" "$target:$repo/$src" "$dst/"
    fi
    ;;
  *) echo "unknown action '$action'" >&2; exit 2 ;;
esac
