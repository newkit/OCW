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
#
# Default: every machine of config/machines.toml. Prints per machine the
# checkouts, their target/ size and the decision, and the space freed (or
# that would be freed). Only machine names are printed, never hosts.
set -euo pipefail
cd "$(dirname "$0")/.."
config_dir=config
if [ ! -f "$config_dir/machines.toml" ]; then
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
  [ -n "$common" ] && config_dir="$(dirname "$common")/config"
fi
conf=$config_dir/machines.toml
[ -f "$conf" ] || { echo "$conf missing" >&2; exit 1; }
machines=("$@")
if [ ${#machines[@]} -eq 0 ]; then
  mapfile -t machines < <(awk '/^\[\[machine\]\]/ { m = 1; next } /^\[/ { m = 0 }
    m && /^[ \t]*name[ \t]*=/ { v = $0; sub(/^[^=]*=[ \t]*"?/, "", v); sub(/"?[ \t]*(#.*)?$/, "", v); print v }' "$conf")
fi
delete=${GC:-0}
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
  if ! listing=$(timeout 600 scripts/remote.sh "$m" ssh "$list_cmd" 2>/dev/null | tr -d '\r'); then
    echo "== $m: unreachable, skipped"
    continue
  fi
  busy=$(timeout 60 scripts/remote.sh "$m" ssh "$([ "$os" = windows ] && echo "Get-Content (Join-Path \$env:TEMP 'ocw-tasks\\*.json') -ErrorAction SilentlyContinue" || echo "cat /tmp/ocw-tasks/*.json 2>/dev/null; true")" 2>/dev/null || true)
  prefix="${repo##*[\\/]}-"
  freed=0
  echo "== $m"
  while read -r kb dir; do
    [ -n "${dir:-}" ] || continue
    branch=${dir#"$prefix"}
    why=$(verdict "$branch")
    if [ -n "$why" ] && grep -qF "\"checkout\":\"$dir\"" <<<"$busy"; then why=""; note="in use"; else note="kept"; fi
    printf '  %8.1f GB  %-50s %s\n' "$(awk -v k="$kb" 'BEGIN { print k / 1048576 }')" "$dir" "${why:-$note}"
    [ -n "$why" ] || continue
    freed=$((freed + kb))
    if [ "$delete" = 1 ]; then
      if [ "$os" = windows ]; then
        scripts/remote.sh "$m" ssh "Remove-Item -Recurse -Force -LiteralPath (Join-Path (Split-Path '$repo') '$dir\\target')" >/dev/null 2>&1 || echo "  (delete failed: $dir)"
      else
        scripts/remote.sh "$m" ssh "rm -rf $repo-${branch}/target" >/dev/null 2>&1 || echo "  (delete failed: $dir)"
      fi
    fi
  done <<<"$listing"
  total_all=$((total_all + freed))
  printf '  %s %.1f GB on %s\n' "$([ "$delete" = 1 ] && echo freed || echo "would free (GC=1)")" \
    "$(awk -v k="$freed" 'BEGIN { print k / 1048576 }')" "$m"
done
printf 'total: %s %.1f GB\n' "$([ "$delete" = 1 ] && echo freed || echo "would free")" "$(awk -v k="$total_all" 'BEGIN { print k / 1048576 }')"
