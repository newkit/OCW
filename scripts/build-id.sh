#!/usr/bin/env bash
# Prints the commit part of the build id: the short commit, with "-dirty" if
# tracked files have uncommitted changes (e.g. 7d538c0-dirty). Empty outside
# a git checkout. scripts/remote.sh passes it to remote builds as
# OCW_BUILD_ID (read by apps/desktop/build.rs), since they get no .git.
cd "$(dirname "$0")/.." || exit 0
sha=$(git rev-parse --short HEAD 2>/dev/null) || exit 0
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
  echo "$sha-dirty"
else
  echo "$sha"
fi
