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
# OCW_REMOTE_JOBS overrides (empty = cargo's default, all cores).
jobs_env=""
if [ "$branch" != "main" ]; then
  jobs=${OCW_REMOTE_JOBS-4}
  [ -n "$jobs" ] && jobs_env="CARGO_BUILD_JOBS=$jobs "
fi

SSH=(ssh -i "$key" -p "${port:-22}" -o IdentitiesOnly=yes -o BatchMode=yes
     -o UserKnownHostsFile="$config_dir/${ssh_dir:-ssh}/known_hosts" -o StrictHostKeyChecking=yes
     -o ConnectTimeout="${timeout_s:-10}")
target="$user@$host"

sync_tree() {
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

case "$action" in
  sync) sync_tree ;;
  run)
    sync_tree
    status=0
    # At most OCW_REMOTE_SLOTS commands (default 2) run at once on a machine
    # (flock on lock files there; others wait for a free slot): many
    # parallel builds of the Bevy app run the machine out of memory.
    slots=${OCW_REMOTE_SLOTS:-2}
    slot_guard=""
    if [ "$os" != macos ] && [ "$slots" -gt 0 ] 2>/dev/null; then
      slot_guard="ocw_slot() { for s in \$(seq 1 $slots); do exec 9>/tmp/ocw-build-slot-\$s.lock; flock -n 9 && return; done; echo 'remote.sh: waiting for a free build slot' >&2; exec 9>/tmp/ocw-build-slot-1.lock; flock 9; }; ocw_slot; "
    fi
    "${SSH[@]}" "$target" "${slot_guard}cd $repo && . ~/.cargo/env && export ${jobs_env:-OCW_REMOTE=1} && $*" || status=$?
    # Builds only happen remotely: bring back a Cargo.lock that cargo
    # updated there (new dependencies), so it can be committed.
    rsync -a -e "${SSH[*]}" "$target:$repo/Cargo.lock" ./Cargo.lock 2>/dev/null || true
    exit "$status"
    ;;
  ssh) "${SSH[@]}" "$target" "$@" ;;
  fetch)
    src=${1:?remote path}; dst=${2:?local dir}
    mkdir -p "$dst"
    rsync -a -e "${SSH[*]}" "$target:$repo/$src" "$dst/"
    ;;
  *) echo "unknown action '$action'" >&2; exit 2 ;;
esac
