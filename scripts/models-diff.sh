#!/usr/bin/env bash
# The structural diff of the model files changed against main (spec 14
# Phase 15, 15D: `make models-diff`): for every `.glb` / `.gltf` under
# artwork/3dmodels/ that differs between the merge base with BASE (default
# main) and the working tree – committed, staged, unstaged or untracked –
# `ocw-assets model diff` of the old and the new file (nodes, meshes,
# materials, extras, the manifest entry), plus added and removed files and
# manifest entries changed without their file.
#
#   scripts/models-diff.sh [--base REF]            here (builds ocw-assets)
#   scripts/models-diff.sh --base REF --stage DIR  only write the old files
#                                                  and the list into DIR (git)
#   scripts/models-diff.sh --staged DIR|TAR        only diff a staged list
#                                                  (no git: on a build machine)
#   MACHINE=marge make models-diff                 stage here, diff on MACHINE
#
# OCW_ASSETS names a built ocw-assets binary (default: cargo build). The old
# files are LFS objects: `git lfs smudge` turns their pointers into models.
# Exit 0 also when files differ (it is a report); 2 on a usage or git error.
set -euo pipefail
root=${OCW_MODELS_DIFF_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$root"
models=artwork/3dmodels
base_ref=main
stage=""
staged=""
while [ $# -gt 0 ]; do
  case $1 in
    --base) base_ref=${2:?--base REF}; shift 2 ;;
    --stage) stage=${2:?--stage DIR}; shift 2 ;;
    --staged) staged=${2:?--staged DIR}; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "models-diff: unknown argument $1" >&2; exit 2 ;;
  esac
done

# Writes DIR/list ("A|M|D<TAB>path" per model file, "manifest" when the
# manifest changed) and DIR/base/<path> (the old files and manifest).
stage_changes() {
  local dir=$1 base
  if ! base=$(git merge-base "$base_ref" HEAD 2>/dev/null); then
    echo "models-diff: no merge base with $base_ref" >&2; exit 2
  fi
  rm -rf "$dir"; mkdir -p "$dir/base"
  {
    git diff --name-status --no-renames "$base" -- "$models"
    git ls-files -o --exclude-standard -- "$models" | sed 's/^/A\t/'
  } | awk -F'\t' '$2 ~ /\.(glb|gltf)$/ || $2 ~ /\/manifest\.toml$/' | sort -t$'\t' -k2 -u >"$dir/changes"
  : >"$dir/list"
  while IFS=$'\t' read -r st path; do
    if [ "$path" = "$models/manifest.toml" ]; then echo manifest >>"$dir/list"; continue; fi
    printf '%s\t%s\n' "$st" "$path" >>"$dir/list"
    [ "$st" = A ] && continue
    old_blob "$base" "$path" "$dir/base/$path"
  done <"$dir/changes"
  # The old manifest, so the old files' entries resolve too.
  if git cat-file -e "$base:$models/manifest.toml" 2>/dev/null; then
    mkdir -p "$dir/base/$models"
    git show "$base:$models/manifest.toml" >"$dir/base/$models/manifest.toml"
  fi
  git rev-parse --short "$base" >"$dir/base-commit"
  echo "$base_ref" >"$dir/base-ref"
}

old_blob() { # base path out
  mkdir -p "$(dirname "$3")"
  git show "$1:$2" >"$3"
  if head -c 40 "$3" | grep -q '^version https://git-lfs'; then
    git lfs smudge -- "$2" <"$3" >"$3.smudged" && mv "$3.smudged" "$3"
  fi
}

diff_staged() {
  local dir=$1 tool n=0
  if [ -f "$dir" ]; then # a tar from --stage on another machine
    local t=${dir%.tar}; rm -rf "$t"; mkdir -p "$t"; tar -xf "$dir" -C "$t"; dir=$t
  fi
  tool=${OCW_ASSETS:-}
  if [ -z "$tool" ]; then
    ${CARGO:-cargo} build -q -p ocw-assets --bin ocw-assets
    tool=${CARGO_TARGET_DIR:-target}/debug/ocw-assets
  fi
  local files
  files=$(grep -vc '^manifest$' "$dir/list" || true)
  echo "Model files changed against $(cat "$dir/base-ref") (merge base $(cat "$dir/base-commit")): $files"
  while IFS=$'\t' read -r st path; do
    [ "$st" = manifest ] && continue
    n=$((n + 1))
    echo
    case $st in
      A) echo "+ $path (added, $(( $(wc -c <"$path") / 1024 )) KiB)"
         "$tool" info "$path" | grep -E '^(copyright|extras):' | sed 's/^/    /' || true ;;
      D) echo "- $path (removed)" ;;
      *) echo "~ $path"
         "$tool" model diff --models "$models" "$dir/base/$path" "$path" | sed 's/^/    /' || true ;;
    esac
  done <"$dir/list"
  if grep -qx manifest "$dir/list"; then
    echo
    echo "~ $models/manifest.toml"
    if [ -f "$dir/base/$models/manifest.toml" ]; then
      diff -U0 --label "$(cat "$dir/base-commit")" --label "working tree" \
        "$dir/base/$models/manifest.toml" "$models/manifest.toml" | grep -E '^[-+][^-+]' | sed 's/^/    /' || true
    else
      echo "    (new)"
    fi
  fi
  [ "$n" -gt 0 ] || echo "no model file changed"
}

if [ -n "$staged" ]; then
  diff_staged "$staged"
elif [ -n "$stage" ]; then
  stage_changes "$stage"
  echo "staged $(wc -l <"$stage/list") changes into $stage"
else
  dir=$(mktemp -d "${TMPDIR:-/tmp}/models-diff.XXXXXX")
  trap 'rm -rf "$dir"' EXIT
  stage_changes "$dir"
  diff_staged "$dir"
fi
