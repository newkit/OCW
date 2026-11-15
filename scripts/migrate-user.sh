#!/usr/bin/env bash
# Copy the OCW coordinator's git-ignored state from one user on deb to another
# (docs/MIGRATE-DEB-USER.md). Run as root. A dry run by default; --apply copies.
#
#   sudo scripts/migrate-user.sh                 # dry run: what would be copied
#   sudo scripts/migrate-user.sh --apply         # copy (rsync, owner = the new user)
#   sudo scripts/migrate-user.sh --apply --linger  # also: loginctl enable-linger NEW_USER
#
# Environment (defaults: nils -> claude):
#   SRC_USER, SRC_REPO   the old user and checkout (nils, /home/nils/data/git/OCW)
#   DST_USER, DST_REPO   the new user and checkout (claude, /home/claude/OCW);
#                        DST_REPO must already be a git clone made by DST_USER
#
# What is copied (nothing tracked in git; the clone has that):
#   config/                machines.toml, build-machines.toml, ssh/ (keys, known_hosts),
#                          dashboard-token  – mode 600/700 kept, never printed
#   test-output/           build log, answers inbox, scenes, web runs
#   art/build-history/     the art archive (media and logs; tracked text is identical)
#   target/web/            the browser build and its worlds (optional, can be re-fetched)
#   target/web-keys/root.hex
#   Claude Code: the old project folder (transcripts for the art project's prompt and
#   token history, and its memory) into DST_USER's ~/.claude/projects/ under its own
#   name; its memory/ also into the new checkout's project folder.
# Re-running is safe (rsync updates only what changed); run it once more at the cutover.
set -euo pipefail

SRC_USER=${SRC_USER:-nils}
SRC_REPO=${SRC_REPO:-/home/nils/data/git/OCW}
DST_USER=${DST_USER:-claude}
DST_REPO=${DST_REPO:-/home/claude/OCW}

apply=0 linger=0
for a in "$@"; do
  case $a in
    --apply) apply=1 ;;
    --linger) linger=1 ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

[ "$(id -u)" = 0 ] || { echo "run as root (sudo), the files belong to $SRC_USER and go to $DST_USER" >&2; exit 2; }
id "$DST_USER" >/dev/null 2>&1 || { echo "no user $DST_USER" >&2; exit 2; }
[ -d "$SRC_REPO/.git" ] || [ -f "$SRC_REPO/.git" ] || { echo "$SRC_REPO is not a git checkout" >&2; exit 2; }
[ -d "$DST_REPO/.git" ] || { echo "$DST_REPO is not a git clone yet – $DST_USER clones it first (docs/MIGRATE-DEB-USER.md, step 3)" >&2; exit 2; }

src_home=$(getent passwd "$SRC_USER" | cut -d: -f6)
dst_home=$(getent passwd "$DST_USER" | cut -d: -f6)
mangle() { printf '%s' "$1" | sed 's#[/.]#-#g'; }   # Claude Code's project folder name
src_proj="$src_home/.claude/projects/$(mangle "$SRC_REPO")"
dst_proj_old="$dst_home/.claude/projects/$(mangle "$SRC_REPO")"
dst_proj_new="$dst_home/.claude/projects/$(mangle "$DST_REPO")"

opts=(-a --human-readable --info=stats1 --chown="$DST_USER:$DST_USER")
[ $apply = 1 ] || opts+=(--dry-run)

copy() { # SRC DST [extra rsync options]
  local s=$1 d=$2; shift 2
  [ -e "$s" ] || { echo "  (skip, missing: $s)"; return 0; }
  echo "== $s -> $d"
  mkdir -p "$(dirname "$d")" 2>/dev/null || true
  rsync "${opts[@]}" "$@" "$s" "$d" | grep -E 'Number of (regular )?files|Total transferred|total size' || true
}

echo "OCW state: $SRC_USER ($SRC_REPO) -> $DST_USER ($DST_REPO)$([ $apply = 1 ] || echo '   [dry run]')"
copy "$SRC_REPO/config/"            "$DST_REPO/config/"
copy "$SRC_REPO/test-output/"       "$DST_REPO/test-output/"
copy "$SRC_REPO/art/build-history/" "$DST_REPO/art/build-history/" --exclude=.lock
copy "$SRC_REPO/target/web/"        "$DST_REPO/target/web/"
copy "$SRC_REPO/target/web-keys/root.hex" "$DST_REPO/target/web-keys/root.hex"
copy "$src_proj/" "$dst_proj_old/"
copy "$src_proj/memory/" "$dst_proj_new/memory/"

if [ $apply = 1 ]; then
  # Keep the secrets private (rsync -a keeps the modes; make sure anyway).
  chmod 700 "$DST_REPO/config/ssh" 2>/dev/null || true
  find "$DST_REPO/config/ssh" -type f -exec chmod 600 {} + 2>/dev/null || true
  for f in machines.toml build-machines.toml dashboard-token; do
    [ -f "$DST_REPO/config/$f" ] && chmod 600 "$DST_REPO/config/$f"
  done
  chmod 700 "$dst_home/.claude" 2>/dev/null || true
  chown "$DST_USER:$DST_USER" "$dst_home/.claude" "$dst_home/.claude/projects" 2>/dev/null || true
  echo "copied. Secrets are mode 600; nothing was printed."
fi

if [ $linger = 1 ]; then
  if [ $apply = 1 ]; then loginctl enable-linger "$DST_USER" && echo "linger on for $DST_USER"
  else echo "(dry run) would run: loginctl enable-linger $DST_USER"; fi
fi
