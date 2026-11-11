#!/usr/bin/env bash
# Build and test on a remote machine from config/machines.toml (see config/README.md).
#
#   scripts/remote.sh MACHINE sync           copy the working tree (tracked and
#                                            untracked, git-ignored files excluded)
#   scripts/remote.sh MACHINE run CMD...     sync, then run CMD in the checkout
#                                            (with ~/.cargo/env sourced)
#   scripts/remote.sh MACHINE ssh [CMD...]   plain ssh (no sync)
#   scripts/remote.sh MACHINE fetch REMOTE_PATH LOCAL_DIR
#                                            copy files back (e.g. test-output/)
#
# Files are copied with rsync; config/ (machine config, keys) is never sent.
set -euo pipefail
cd "$(dirname "$0")/.."

machine=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch ...}; shift
action=${1:?usage: remote.sh MACHINE sync|run|ssh|fetch ...}; shift

conf=config/machines.toml
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
timeout_s=$(field connect_timeout_s)
[ -n "$host" ] || { echo "machine '$machine' not found in $conf" >&2; exit 1; }
case "$key" in /*) ;; *) key="config/${ssh_dir:-ssh}/$key" ;; esac

SSH=(ssh -i "$key" -p "${port:-22}" -o IdentitiesOnly=yes -o BatchMode=yes
     -o UserKnownHostsFile="config/${ssh_dir:-ssh}/known_hosts" -o StrictHostKeyChecking=yes
     -o ConnectTimeout="${timeout_s:-10}")
target="$user@$host"

sync_tree() {
  "${SSH[@]}" "$target" "mkdir -p $repo"
  git ls-files -co --exclude-standard | grep -v '^config/' |
    rsync -a --delete-missing-args --files-from=- \
      -e "${SSH[*]}" ./ "$target:$repo/"
}

case "$action" in
  sync) sync_tree ;;
  run)
    sync_tree
    "${SSH[@]}" "$target" "cd $repo && . ~/.cargo/env && $*"
    ;;
  ssh) "${SSH[@]}" "$target" "$@" ;;
  fetch)
    src=${1:?remote path}; dst=${2:?local dir}
    mkdir -p "$dst"
    rsync -a -e "${SSH[*]}" "$target:$repo/$src" "$dst/"
    ;;
  *) echo "unknown action '$action'" >&2; exit 2 ;;
esac
