#!/usr/bin/env bash
# Frees disk on the build machines (spec §11.2): deletes the target/ of
# remote branch checkouts (`<repo_path>-worktree-agent-…`) whose agent
# branch is gone here, or merged into main with its worktree removed. The
# main checkout, the sccache
# cache and checkouts in use (a running or waiting remote.sh task) are
# kept, as are the checkouts themselves (only target/ goes).
#
#   scripts/remote-gc.sh [MACHINE...]     dry run: what would be freed
#   GC=1 scripts/remote-gc.sh [MACHINE...] delete       (make remote-gc [GC=1])
#   scripts/remote-gc.sh --branch BRANCH [--dry-run] [MACHINE...]
#                                         cleanup on merge: delete the target/
#                                         of BRANCH's checkout on every
#                                         reachable machine (make remote-gc-branch
#                                         BRANCH=…); the coordinator runs it
#                                         after merging or abandoning a branch
#
# Default: every machine of config/machines.toml; unreachable ones are
# skipped. Prints per machine the checkouts, their target/ size and the
# decision, and the space freed (or that would be freed). Only machine names
# are printed, never hosts. A checkout with a live task marker (running or
# waiting) is never touched: the machine checks again right before deleting,
# and the target/ is renamed before it is removed. Every deletion is logged
# to test-output/build-log/DAY/janitor.jsonl of the main checkout (machine,
# path, size, reason), like the dashboard's janitor.
set -euo pipefail
cd "$(dirname "$0")/.."
config_dir=config
if [ ! -f "$config_dir/machines.toml" ]; then
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
  [ -n "$common" ] && config_dir="$(dirname "$common")/config"
fi
conf=$config_dir/machines.toml
[ -f "$conf" ] || { echo "$conf missing" >&2; exit 1; }
only_branch=""
delete=${GC:-0}
machines=()
while [ $# -gt 0 ]; do
  case "$1" in
    --branch) only_branch=${2:?--branch needs a branch name}; shift 2; [ "${GC:-}" = 0 ] || delete=1 ;;
    --branch=*) only_branch=${1#--branch=}; shift; [ "${GC:-}" = 0 ] || delete=1 ;;
    -n | --dry-run) delete=0; dry_flag=1; shift ;;
    -h | --help) sed -n '2,25p' "$0"; exit 0 ;;
    *) machines+=("$1"); shift ;;
  esac
done
# --dry-run wins over --branch's default of deleting.
[ "${dry_flag:-0}" = 1 ] && delete=0
if [ -n "$only_branch" ]; then
  case "$only_branch" in main | "" | *[!A-Za-z0-9._/+-]*) echo "remote-gc: refusing branch '$only_branch'" >&2; exit 2 ;; esac
fi
if [ ${#machines[@]} -eq 0 ]; then
  mapfile -t machines < <(awk '/^\[\[machine\]\]/ { m = 1; next } /^\[/ { m = 0 }
    m && /^[ \t]*name[ \t]*=/ { v = $0; sub(/^[^=]*=[ \t]*"?/, "", v); sub(/"?[ \t]*(#.*)?$/, "", v); print v }' "$conf")
fi
worktrees=$(git worktree list --porcelain)

# Should the target/ of checkout directory NAME (without the repo prefix)
# go? Prints the reason, or nothing to keep it.
verdict() {
  local branch=$1
  case "$branch" in worktree-agent-*) ;; *) return 0 ;; esac
  if ! git show-ref --verify --quiet "refs/heads/$branch"; then
    echo "branch gone"
  elif grep -qxF "branch refs/heads/$branch" <<<"$worktrees"; then
    # A live worktree (a new agent's branch is "merged": it has no commits yet).
    return 0
  elif git merge-base --is-ancestor "refs/heads/$branch" main 2>/dev/null; then
    echo "merged into main, worktree removed"
  fi
}

# Janitor log for the art project (spec §11.2, art/ART-PROJECT.md §2).
log_root="$(dirname "$config_dir")/test-output/build-log"
json_str() { local b='\' q='"' s; s=${1//"$b"/"$b$b"}; printf '"%s"' "${s//"$q"/"$b$q"}"; }
jlog() { # machine path kb reason action
  [ "${OCW_BUILD_LOG:-1}" = 0 ] && return 0
  local d t
  d="$log_root/$(date +%Y-%m-%d)"
  mkdir -p "$d" 2>/dev/null || return 0
  t=$(date --iso-8601=seconds 2>/dev/null || date +%Y-%m-%dT%H:%M:%S%z)
  printf '{"action":%s,"bytes":%s,"kind":"target","machine":%s,"mode":"on","path":%s,"reason":%s,"source":"remote-gc.sh","time":%s}\n' \
    "$(json_str "$5")" "$(($3 * 1024))" "$(json_str "$1")" "$(json_str "$2")" "$(json_str "$4")" "$(json_str "$t")" \
    >>"$d/janitor.jsonl"
}

# The remote deletion of DIR/target (DIR = checkout directory name): not
# while a live task marker names DIR; renamed first, then removed. Prints
# "deleted KB", "busy", "gone" or "failed".
delete_cmd() { # os repo dir
  if [ "$1" = windows ]; then
    printf '%s' "\$p = Join-Path (Split-Path '$2') '$3'; \$t = Join-Path \$p 'target'; \$busy = @(Get-ChildItem -LiteralPath (Join-Path \$env:TEMP 'ocw-tasks') -Filter '*.json' -ErrorAction SilentlyContinue | Where-Object { Get-Process -Id (\$_.BaseName -split '-')[0] -ErrorAction SilentlyContinue } | Where-Object { (Get-Content -Raw -LiteralPath \$_.FullName) -match '\"checkout\":\"$3\"' }); if (-not (Test-Path -LiteralPath \$t)) { 'gone' } elseif (\$busy.Count) { 'busy' } else { \$kb = [int64](((Get-ChildItem -LiteralPath \$t -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum) / 1KB); \$g = 'target.ocw-gc.' + \$PID; Rename-Item -LiteralPath \$t -NewName \$g; Remove-Item -Recurse -Force -LiteralPath (Join-Path \$p \$g) -ErrorAction SilentlyContinue; if (Test-Path -LiteralPath (Join-Path \$p \$g)) { 'failed' } else { 'deleted ' + \$kb } }"
  else
    printf '%s' "p=\$(dirname $2)/$3; t=\$p/target; [ -d \$t ] || { echo gone; exit 0; }; for g in /tmp/ocw-tasks/*.json; do [ -e \$g ] || continue; q=\${g##*/}; q=\${q%%-*}; grep -q '\"checkout\":\"$3\"' \$g && ps -p \$q >/dev/null 2>&1 && { echo busy; exit 0; }; done; kb=\$(du -sk \$t | cut -f1); mv \$t \$p/target.ocw-gc.\$\$ && rm -rf \$p/target.ocw-gc.\$\$ && echo \"deleted \$kb\" || echo failed"
  fi
}

total_all=0
for m in "${machines[@]}"; do
  os=$(scripts/remote.sh "$m" field os)
  repo=$(scripts/remote.sh "$m" field repo_path)
  if [ "$os" = windows ]; then
    repo=${repo//\\\\/\\}
    list_cmd="\$p = '$repo'; Get-ChildItem -Directory -Path (Split-Path \$p) -Filter ((Split-Path -Leaf \$p) + '-*') | ForEach-Object { \$t = Join-Path \$_.FullName 'target'; if (Test-Path \$t) { \$k = [math]::Round(((Get-ChildItem -LiteralPath \$t -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum) / 1KB); Write-Output (\"\$k \" + \$_.Name) } }"
  else
    list_cmd="for d in $repo-*/; do d=\${d%/}; [ -d \$d/target ] || continue; echo \"\$(du -sk \$d/target 2>/dev/null | cut -f1) \${d##*/}\"; done; true"
  fi
  prefix="${repo##*[\\/]}-"
  if [ -n "$only_branch" ]; then
    # Only that branch's checkout (no listing of the others).
    dir="$prefix${only_branch//\//-}"
    if [ "$os" = windows ]; then
      list_cmd="\$t = Join-Path (Join-Path (Split-Path '$repo') '$dir') 'target'; if (Test-Path -LiteralPath \$t) { Write-Output (\"\" + [math]::Round(((Get-ChildItem -LiteralPath \$t -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum) / 1KB) + ' $dir') }"
    else
      list_cmd="d=\$(dirname $repo)/$dir; if [ -d \$d/target ]; then echo \"\$(du -sk \$d/target 2>/dev/null | cut -f1) $dir\"; fi; true"
    fi
  fi
  if ! listing=$(timeout 600 scripts/remote.sh "$m" ssh "$list_cmd" 2>/dev/null | tr -d '\r'); then
    echo "== $m: unreachable, skipped"
    continue
  fi
  busy=$(timeout 60 scripts/remote.sh "$m" ssh "$([ "$os" = windows ] && echo "Get-Content (Join-Path \$env:TEMP 'ocw-tasks\\*.json') -ErrorAction SilentlyContinue" || echo "cat /tmp/ocw-tasks/*.json 2>/dev/null; true")" 2>/dev/null || true)
  freed=0
  echo "== $m"
  [ -n "$listing" ] || echo "  (no target/ to delete)"
  while read -r kb dir; do
    [ -n "${dir:-}" ] || continue
    branch=${dir#"$prefix"}
    if [ -n "$only_branch" ]; then
      why="cleanup on merge (--branch $only_branch)"
    else
      why=$(verdict "$branch")
    fi
    if [ -n "$why" ] && grep -qF "\"checkout\":\"$dir\"" <<<"$busy"; then why=""; note="in use"; else note="kept"; fi
    printf '  %8.1f GB  %-50s %s\n' "$(awk -v k="$kb" 'BEGIN { print k / 1048576 }')" "$dir" "${why:-$note}"
    [ -n "$why" ] || continue
    if [ "$delete" = 1 ]; then
      res=$(scripts/remote.sh "$m" ssh "$(delete_cmd "$os" "$repo" "$dir")" 2>/dev/null | tr -d '\r' | tail -n 1 || true)
      case "$res" in
        deleted*) kb=${res#deleted }; kb=${kb//[!0-9]/}; kb=${kb:-0}; freed=$((freed + kb)); jlog "$m" "$dir/target" "$kb" "$why" delete ;;
        busy) echo "  (in use now, kept: $dir)" ;;
        gone) ;;
        *) echo "  (delete failed: $dir)"; jlog "$m" "$dir/target" 0 "$why" failed ;;
      esac
    else
      freed=$((freed + kb))
    fi
  done <<<"$listing"
  total_all=$((total_all + freed))
  printf '  %s %.1f GB on %s\n' "$([ "$delete" = 1 ] && echo freed || echo "would free$([ -z "$only_branch" ] && echo " (GC=1)")")" \
    "$(awk -v k="$freed" 'BEGIN { print k / 1048576 }')" "$m"
done
printf 'total: %s %.1f GB\n' "$([ "$delete" = 1 ] && echo freed || echo "would free")" "$(awk -v k="$total_all" 'BEGIN { print k / 1048576 }')"
