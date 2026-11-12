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
#
# On a branch other than main the remote checkout is REPO_PATH-BRANCH.
#
# Files are copied with rsync; config/ (machine config, keys) is never sent.
#
# Each `run` leaves a task marker on the machine while it runs (read by
# tools/build-dashboard): a JSON file PID-START.json in /tmp/ocw-tasks/
# (Windows: %TEMP%\ocw-tasks\) with the checkout, branch, command (shortened),
# start time, the remote shell's PID, its state (waiting for a build slot,
# running in slot N) and since when, and the machine's slot count. The run removes it when it ends; markers whose PID is gone
# (killed runs) are ignored by the dashboard and swept by the next run.
set -euo pipefail
cd "$(dirname "$0")/.."

machine=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch ...}; shift
action=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch ...}; shift

# config/ is git-ignored, so another worktree uses the main checkout's copy.
config_dir=config
if [ ! -f "$config_dir/machines.toml" ]; then
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
  [ -n "$common" ] && config_dir="$(dirname "$common")/config"
fi
conf=$config_dir/machines.toml
[ -f "$conf" ] || { echo "$conf missing (copy config/machines.example.toml)" >&2; exit 1; }

# Minimal TOML lookup: key of the [[machine]] block with name = MACHINE,
# falling back to [defaults].
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
    END { if (want in vals) print vals[want]; else print def }' "$conf"
}

host=$(field host); user=$(field user); port=$(field port)
key=$(field identity_file); ssh_dir=$(field ssh_dir); repo=$(field repo_path)
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

SSH=(ssh -i "$key" -p "${port:-22}" -o IdentitiesOnly=yes -o BatchMode=yes
     -o UserKnownHostsFile="$config_dir/${ssh_dir:-ssh}/known_hosts" -o StrictHostKeyChecking=yes
     -o ConnectTimeout="${timeout_s:-10}")
target="$user@$host"
# TOML basic strings escape backslashes ("B:\\src\\OCW"): unescape for Windows.
if [ "$os" = windows ]; then repo=${repo//\\\\/\\}; fi
# A PowerShell single-quoted string literal.
psq() { local q="'"; printf "'%s'" "${1//$q/$q$q}"; }

sync_tree() {
  if [ "$os" = windows ]; then
    sync_tree_windows
    return
  fi
  "${SSH[@]}" "$target" "mkdir -p $repo"
  if [ "$os" = macos ]; then
    # macOS ships openrsync, which lacks --delete-missing-args: send only the
    # files that exist and delete tracked files removed here with rm.
    local files missing=()
    files=$(git ls-files -co --exclude-standard | grep -v '^config/')
    while IFS= read -r f; do [ -e "$f" ] || missing+=("'${f//\'/\'\\\'\'}'"); done <<<"$files"
    if [ ${#missing[@]} -gt 0 ]; then
      "${SSH[@]}" "$target" "cd $repo && rm -f ${missing[*]}"
    fi
    while IFS= read -r f; do if [ -e "$f" ]; then printf "%s\n" "$f"; fi; done <<<"$files" |
      rsync -a --files-from=- -e "${SSH[*]}" ./ "$target:$repo/"
    return
  fi
  git ls-files -co --exclude-standard | grep -v '^config/' |
    rsync -a --delete-missing-args --files-from=- \
      -e "${SSH[*]}" ./ "$target:$repo/"
}

# Windows has no rsync; its built-in tar.exe unpacks a tar stream. Files
# deleted here are removed there with Remove-Item.
sync_tree_windows() {
  "${SSH[@]}" "$target" "New-Item -ItemType Directory -Force -Path $(psq "$repo") | Out-Null"
  local files gone=() f list
  files=$(git ls-files -co --exclude-standard | grep -v '^config/')
  while IFS= read -r f; do [ -e "$f" ] || gone+=("$(psq "$f")"); done <<<"$files"
  if [ ${#gone[@]} -gt 0 ]; then
    list=$(IFS=,; echo "${gone[*]}")
    "${SSH[@]}" "$target" "Set-Location -LiteralPath $(psq "$repo"); Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $list"
  fi
  while IFS= read -r f; do if [ -e "$f" ]; then printf '%s\n' "$f"; fi; done <<<"$files" |
    tar -cf - -T - | "${SSH[@]}" "$target" "tar -xf - -C $(psq "$repo")"
}

# Build slots: at most N commands run at once on a machine, because many
# parallel builds of the Bevy app (~4-5 GB each) run it out of memory.
# N = OCW_REMOTE_SLOTS, else the machine's `slots` in machines.toml (or
# [defaults] slots), else per OS: linux 2, windows 1, macos 1. 0 = no limit.
# A run that finds all slots taken marks itself "waiting" and polls every
# second, taking whichever slot frees first. Linux: flock on
# /tmp/ocw-build-slot-N.lock; macOS (no flock): atomic mkdir of
# /tmp/ocw-build-slot-N.d holding the owner's PID (taken over when that PID
# is gone); Windows: %TEMP%\ocw-tasks\slot-N.lock opened exclusively.
case "$os" in windows | macos) default_slots=1 ;; *) default_slots=2 ;; esac
slots=${OCW_REMOTE_SLOTS:-$(field slots)}
slots=${slots:-$default_slots}
[ "$slots" -ge 0 ] 2>/dev/null || slots=$default_slots
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
if (@SLOTS@ -gt 0) { while (-not $ocwLock) { foreach ($s in 1..@SLOTS@) { try { $ocwLock = [IO.File]::Open((Join-Path $ocwD ('slot-' + $s + '.lock')), 'OpenOrCreate', 'ReadWrite', 'None'); $ocwSlot = $s; break } catch {} }; if (-not $ocwLock) { if (-not $ocwWarn) { ocwMark 'waiting' $null; [Console]::Error.WriteLine('remote.sh: waiting for a free build slot'); $ocwWarn = $true }; Start-Sleep -Milliseconds 1000 } } };
ocwMark 'running' $ocwSlot;
@RUN@; $ocwRc = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } elseif ($ocwOk) { 0 } else { 1 }
} finally { if ($ocwLock) { $ocwLock.Dispose() }; Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $ocwF, ($ocwF + '.tmp') };
exit $ocwRc
EOF

# Linux: flock(1) on a lock file per slot, held on fd 9 by the run's shell
# (the kernel drops it when the shell dies).
read -r -d '' slot_take_flock <<'EOF' || true
ocw_take() { exec 9>/tmp/ocw-build-slot-$1.lock && flock -n 9; };
ocw_free() { exec 9>&-; };
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
  # The command runs as a script block (dot-sourced): the wrapper is one
  # line, which a `#` comment in the command would otherwise cut off. It
  # records $? of its last statement in $ocwOk (for the exit code).
  run="Set-Location -LiteralPath $(psq "$repo"); \$env:Path = \"\$env:USERPROFILE\\.cargo\\bin;\" + \$env:Path; \$env:OCW_BUILD_ID='${build_id:-}'; ${jobs_ps}. ([scriptblock]::Create($(psq "$*") + [char]10 + '\$ocwOk = \$?'))"
  ps=${marker_ps//$'\n'/ }
  ps=${ps//@CHECKOUT@/"$(psq "$checkout_dir")"}
  ps=${ps//@BRANCH@/"$(psq "$branch")"}
  ps=${ps//@CMD@/"$(psq "$(short_cmd "$@")")"}
  ps=${ps//@SLOTS@/"$slots"}
  ps=${ps//@RUN@/"$run"}
  "${SSH[@]}" "$target" "$ps"
}

case "$action" in
  sync) sync_tree ;;
  run)
    sync_tree
    status=0
    # The remote copy has no .git: pass the commit for the build id
    # (apps/desktop/build.rs, scripts/build-id.sh).
    build_id=$(scripts/build-id.sh)
    if [ "$os" = windows ]; then
      run_windows "$@" || status=$?
      "${SSH[@]}" "$target" "tar -cf - -C $(psq "$repo") Cargo.lock" 2>/dev/null | tar -xf - 2>/dev/null || true
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
      # shell's parent sshd ended) gives up instead of running later.
      slot_guard="$take ocw_slot() { ocw_w=; ocw_pp=\$PPID; while :; do for s in $slot_list; do ocw_take \$s && { ocw_n=\$s; return 0; }; done; ocw_free; if [ -z \"\$ocw_w\" ]; then ocw_w=1; ocw_mark waiting; echo 'remote.sh: waiting for a free build slot' >&2; fi; sleep 1; ps -p \$ocw_pp >/dev/null 2>&1 || exit 129; done; }; ocw_slot; "
    fi
    meta="\"slots\":$slots,\"checkout\":$(json_str "$checkout_dir"),\"branch\":$(json_str "$branch"),\"command\":$(json_str "$(short_cmd "$@")"),\"os\":\"$os\""
    marker="ocw_meta=$(shq "$meta"); ${marker_sh//$'\n'/ } "
    "${SSH[@]}" "$target" "${marker}${slot_guard}ocw_mark running \$ocw_n; cd $repo && . ~/.cargo/env && export ${jobs_env:-OCW_REMOTE=1} OCW_BUILD_ID='$build_id' && $*" || status=$?
    # Builds only happen remotely: bring back a Cargo.lock that cargo
    # updated there (new dependencies), so it can be committed.
    rsync -a -e "${SSH[*]}" "$target:$repo/Cargo.lock" ./Cargo.lock 2>/dev/null || true
    exit "$status"
    ;;
  ssh) "${SSH[@]}" "$target" "$@" ;;
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
