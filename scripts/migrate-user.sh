#!/usr/bin/env bash
# Copy the git-ignored state of OCW and of the art project OCW-ART from one user
# on deb to another (docs/MIGRATE-DEB-USER.md). Run as root. The two repositories
# move one by one: first `ocw`, then `art`. A dry run by default; --apply copies.
#
#   sudo scripts/migrate-user.sh ocw                   # move 1, dry run
#   sudo scripts/migrate-user.sh ocw --apply --linger  # copy OCW's state, linger for NEW_USER
#   sudo scripts/migrate-user.sh art                   # move 2, dry run
#   sudo scripts/migrate-user.sh art --apply           # copy the art archive's media
#
# Environment (defaults: nils -> claude):
#   SRC_USER, SRC_REPO   the old user and OCW checkout (nils, /home/nils/data/git/OCW)
#   DST_USER, DST_REPO   the new user and OCW checkout (claude, /home/claude/src/OCW);
#                        DST_REPO must already be a git clone made by DST_USER
#   SRC_ART, DST_ART     the OCW-ART checkouts, next to OCW by default
#                        (/home/nils/data/git/OCW-ART, /home/claude/src/OCW-ART);
#                        DST_ART must already be a git clone of newkit/OCW-ART
#
# What is copied (nothing tracked in git; the clones have that):
#  ocw:
#   config/                machines.toml, build-machines.toml, ssh/ (keys, known_hosts),
#                          dashboard-token  – mode 600/700 kept, never printed
#   test-output/           build log, answers inbox, scenes, web runs, art status
#   target/web/            the browser build and its worlds (optional, can be re-fetched)
#   target/web-keys/root.hex
#   Claude Code: the old project folder (transcripts for the art project's prompt and
#   token history, and its memory) into DST_USER's ~/.claude/projects/ under its own
#   name; its memory/ also into the new checkout's project folder.
#  art (after ocw: the art tool reads ../OCW and writes its status into its test-output/):
#   build-history/         the art archive (media, logs, zips, diary videos; the tracked
#                          text is in the clone) from SRC_ART/build-history/ (or, before
#                          it moved there, SRC_REPO/art/build-history/)
# Re-running is safe (rsync updates only what changed); run each move once more at
# its cutover.
set -euo pipefail

SRC_USER=${SRC_USER:-nils}
SRC_REPO=${SRC_REPO:-/home/nils/data/git/OCW}
DST_USER=${DST_USER:-claude}
DST_REPO=${DST_REPO:-/home/claude/src/OCW}
SRC_ART=${SRC_ART:-$(dirname "$SRC_REPO")/OCW-ART}
DST_ART=${DST_ART:-$(dirname "$DST_REPO")/OCW-ART}

usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d'; }
what="" apply=0 linger=0
for a in "$@"; do
  case $a in
    ocw|art) [ -z "$what" ] || { echo "name one move: ocw or art" >&2; exit 2; }; what=$a ;;
    --apply) apply=1 ;;
    --linger) linger=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done
[ -n "$what" ] || { echo "which move? $0 ocw|art [--apply] [--linger]  (first ocw, then art)" >&2; exit 2; }

[ "$(id -u)" = 0 ] || { echo "run as root (sudo), the files belong to $SRC_USER and go to $DST_USER" >&2; exit 2; }
id "$DST_USER" >/dev/null 2>&1 || { echo "no user $DST_USER" >&2; exit 2; }
[ -d "$DST_REPO/.git" ] || { echo "$DST_REPO is not a git clone yet – $DST_USER clones newkit/OCW first (docs/MIGRATE-DEB-USER.md, move 1)" >&2; exit 2; }
if [ "$what" = ocw ]; then
  [ -d "$SRC_REPO/.git" ] || [ -f "$SRC_REPO/.git" ] || { echo "$SRC_REPO is not a git checkout" >&2; exit 2; }
else
  [ -d "$SRC_ART/.git" ] || { echo "$SRC_ART is not a git checkout" >&2; exit 2; }
  [ -d "$DST_ART/.git" ] || { echo "$DST_ART is not a git clone yet – $DST_USER clones newkit/OCW-ART first (docs/MIGRATE-DEB-USER.md, move 2)" >&2; exit 2; }
  [ -f "$DST_REPO/config/machines.toml" ] || { echo "move OCW first ($0 ocw --apply): the art tool needs $DST_REPO/config/ and test-output/" >&2; exit 2; }
fi

opts=(-a --human-readable --info=stats1 --chown="$DST_USER:$DST_USER")
[ $apply = 1 ] || opts+=(--dry-run)

copy() { # SRC DST [extra rsync options]
  local s=$1 d=$2; shift 2
  [ -e "$s" ] || { echo "  (skip, missing: $s)"; return 0; }
  echo "== $s -> $d"
  if [ $apply = 1 ]; then
    runuser -u "$DST_USER" -- mkdir -p "$(dirname "$d")"   # parents owned by DST_USER
  elif [ ! -d "$(dirname "$d")" ]; then
    echo "  (dry run: would create $(dirname "$d") and copy $(du -sh "$s" | cut -f1))"; return 0
  fi
  rsync "${opts[@]}" "$@" "$s" "$d" | grep -E 'Number of (regular )?files|Total transferred|total size' || true
}

dry=$([ $apply = 1 ] || echo '   [dry run]')
if [ "$what" = ocw ]; then
  src_home=$(getent passwd "$SRC_USER" | cut -d: -f6)
  dst_home=$(getent passwd "$DST_USER" | cut -d: -f6)
  mangle() { printf '%s' "$1" | sed 's#[/.]#-#g'; }   # Claude Code's project folder name
  src_proj="$src_home/.claude/projects/$(mangle "$SRC_REPO")"
  dst_proj_old="$dst_home/.claude/projects/$(mangle "$SRC_REPO")"
  dst_proj_new="$dst_home/.claude/projects/$(mangle "$DST_REPO")"

  echo "Move 1, OCW: $SRC_USER ($SRC_REPO) -> $DST_USER ($DST_REPO)$dry"
  copy "$SRC_REPO/config/"            "$DST_REPO/config/"
  copy "$SRC_REPO/test-output/"       "$DST_REPO/test-output/"
  copy "$SRC_REPO/target/web/"        "$DST_REPO/target/web/"
  copy "$SRC_REPO/target/web-keys/root.hex" "$DST_REPO/target/web-keys/root.hex"
  if [ $apply = 1 ]; then
    runuser -u "$DST_USER" -- mkdir -p "$dst_home/.claude/projects"
    chmod 700 "$dst_home/.claude"
  fi
  copy "$src_proj/" "$dst_proj_old/"
  copy "$src_proj/memory/" "$dst_proj_new/memory/"

  if [ $apply = 1 ]; then
    # Keep the secrets private (rsync -a keeps the modes; make sure anyway).
    chmod 700 "$DST_REPO/config/ssh" 2>/dev/null || true
    find "$DST_REPO/config/ssh" -type f -exec chmod 600 {} + 2>/dev/null || true
    for f in machines.toml build-machines.toml dashboard-token; do
      [ -f "$DST_REPO/config/$f" ] && chmod 600 "$DST_REPO/config/$f"
    done
    echo "copied. Secrets are mode 600; nothing was printed."
  fi
else
  # The art archive: in the OCW-ART checkout since 2026-11-15 (its ART-PROJECT.md
  # §5.2), before that in OCW's art/build-history/. The media's diary/ folder tells
  # where it is: a fresh OCW-ART clone has only the tracked text.
  src_hist="$SRC_ART/build-history"
  if [ ! -d "$src_hist/diary" ] && [ -d "$SRC_REPO/art/build-history/diary" ]; then
    src_hist="$SRC_REPO/art/build-history"
  fi
  echo "Move 2, OCW-ART: $SRC_USER ($SRC_ART) -> $DST_USER ($DST_ART)$dry"
  # .lock: the collector's flock file. No ocw-art.toml: the new clone finds ../OCW.
  copy "$src_hist/" "$DST_ART/build-history/" --exclude=.lock
  if [ $apply = 1 ]; then echo "copied."; fi
fi

if [ $linger = 1 ]; then
  if [ $apply = 1 ]; then loginctl enable-linger "$DST_USER" && echo "linger on for $DST_USER"
  else echo "(dry run) would run: loginctl enable-linger $DST_USER"; fi
fi
