"""The cleanup rules of the build machines' disks (spec 11.2) – the one rule
set used by both `scripts/remote-gc.sh` (remote_gc.py) and the dashboard's
janitor (janitor.py).

A build machine holds the main checkout and, next to it, one checkout per
branch that was ever built there (`<repo>-<branch, "/" as "-">`, made by
`scripts/remote.sh`). Each is sorted into a class by its name; the rule of
the class says whether its `target/` goes, and whether the whole checkout
goes too:

* never touched: the main checkout, `<repo>-HEAD` (the release / deploy
  checkout of detached builds), a directory with a `.git` of its own (a
  clone such as `<repo>-ART`: remote.sh's checkouts are plain copies), a
  checkout a live task marker names (running or waiting), and anything
  synced within the last 24 h (`recent_h`) – except an agent checkout whose
  branch is gone;
* agent (`worktree-agent-*`): branch gone → at once; merged into main with
  its local worktree removed; merged into main with commits of its own (so
  not a fresh agent, whose branch also looks merged) and not synced for
  2 days (`agent_days`), even while the local worktree exists;
* merge (`merge-*`, branch `merge/…`): branch gone or merged into main, or
  not synced for 3 days (`merge_days`);
* release (`release-X.Y.Z`): the newest release (the highest version among
  the tags, Cargo.toml and the checkouts there) stays; older ones go when
  not synced for 7 days (`release_days`);
* other (`fix-*`, `strava-*`, `agent-*`, …): merged into main with commits
  of its own; no branch of that name (gone, or a name given by hand) → when
  not synced for 14 days (`other_days`); an open branch stays.

The whole checkout directory goes when its `target/` may go by a rule
above for a reason that does not end (branch gone or merged, no branch, an
older release), it was not synced for 30 days (`checkout_days`), no local
worktree uses the branch and it holds no file under `test-output/` or
`data/` (output that may never have been fetched: such a checkout stays and
the plan says so).

Besides: `target.ocw-gc.*` / `target.ocw-janitor.*` (and `<checkout>.ocw-gc.*`)
left by a failed deletion, older than 1 h (`stray_h`); the browser profiles
our web tests leave in the temp directory (`ocw-chromium-*`), older than
24 h (`temp_h`) and not named by a running process; release output folders
`~/ocw-release-X.Y.Z` older than the newest release and not changed for 7
days, unless they hold world packages. Never: sources of a kept checkout,
world data, the sccache cache (capped by `sccache_size`), task markers
(`remote.sh` sweeps those of dead processes on every run).

"Last synced" is read on the machine: the newest of the checkout's
`.ocw-sync/` (rewritten by every sync), `.ocw-last-run` (touched by every
run) and cargo's directories in `target/`; without any of them the
directory's own time. Ages are counted with the machine's clock.

Everything here is pure (decisions, snippet texts, parsers) and
unit-tested in test_cleanup_rules.py.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
from pathlib import Path

AGENT_PREFIX = "worktree-agent-"
HEAD_NAME = "HEAD"
STRAY_MARKS = (".ocw-gc.", ".ocw-janitor.")
TEMP_PREFIXES = ("ocw-chromium-",)
RELOUT_RE = re.compile(r"^ocw-release-(\d+)\.(\d+)\.(\d+)$")
RELEASE_RE = re.compile(r"^release-(\d+)\.(\d+)\.(\d+)$")
_SAFE_NAME = re.compile(r"^[A-Za-z0-9._+-]+$")
CLASSES = ("main", "head", "foreign", "agent", "merge", "release", "other")
KINDS = ("target", "checkout", "stray", "temp", "relout")

# The thresholds, in one place: (default, unit in seconds, what it is).
LIMITS = {
    "recent_h": (24.0, 3600, "never touch a checkout synced within this many hours (except agent, branch gone)"),
    "agent_days": (2.0, 86400, "agent checkout merged into main with its own commits: not synced for this many days"),
    "merge_days": (3.0, 86400, "merge checkout: not synced for this many days (whatever its branch)"),
    "release_days": (7.0, 86400, "release checkout / release output folder older than the newest release: idle days"),
    "other_days": (14.0, 86400, "other checkout without a branch of its name: not synced for this many days"),
    "checkout_days": (30.0, 86400, "the whole checkout directory goes after this many days (no test-output/, data/)"),
    "stray_h": (1.0, 3600, "leftover of a failed deletion (target.ocw-gc.*): older than this many hours"),
    "temp_h": (24.0, 3600, "browser profile of a web test in the temp directory: older than this many hours"),
}


def env_name(key: str) -> str:
    return "OCW_GC_" + key.upper()


def limits(env=None) -> dict:
    """The thresholds in seconds: the defaults, each overridden by its
    environment variable (OCW_GC_RECENT_H=48, OCW_GC_MERGE_DAYS=5, …;
    fractions allowed, a negative or unreadable value keeps the default)."""
    env = os.environ if env is None else env
    out = {}
    for key, (default, unit, _what) in LIMITS.items():
        v = default
        try:
            raw = env.get(env_name(key))
            if raw not in (None, ""):
                f = float(raw)
                if f >= 0:
                    v = f
        except (TypeError, ValueError):
            pass
        out[key] = v * unit
    return out


def limits_help(lim: dict | None = None) -> str:
    lim = limits() if lim is None else lim
    rows = []
    for key, (default, unit, what) in LIMITS.items():
        rows.append("  %-22s %-5g (default %g)  %s" % (env_name(key), lim[key] / unit, default, what))
    return "\n".join(rows)


# --------------------------------------------------------------------------
# Names
# --------------------------------------------------------------------------


def dir_suffix(branch: str) -> str:
    """The checkout directory suffix remote.sh gives a branch."""
    return branch.replace("/", "-")


def version_of(text: str):
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", text or "")
    return tuple(int(x) for x in m.groups()) if m else None


def fmt_version(v) -> str:
    return ".".join(str(x) for x in v) if v else "?"


def classify(name: str, base: str):
    """(class, suffix) of a directory next to the main checkout `base`
    ("OCW"); (None, None) for a directory that is no checkout of it."""
    if name == base:
        return "main", ""
    if not name.startswith(base + "-"):
        return None, None
    s = name[len(base) + 1:]
    if s == HEAD_NAME:
        return "head", s
    if s.startswith(AGENT_PREFIX):
        return "agent", s
    if s == "merge" or s.startswith("merge-"):
        return "merge", s
    if RELEASE_RE.match(s):
        return "release", s
    return "other", s


def is_stray(name: str) -> bool:
    return any(m in name for m in STRAY_MARKS)


def fmt_age(s) -> str:
    s = int(max(0, s or 0))
    if s >= 86400:
        return "%dd %dh" % (s // 86400, s % 86400 // 3600)
    if s >= 3600:
        return "%dh %02dm" % (s // 3600, s % 3600 // 60)
    return "%dm" % (s // 60)


def gb(n) -> str:
    return "%.1f GB" % ((n or 0) / 1e9)


# --------------------------------------------------------------------------
# Git facts (on the coordinator)
# --------------------------------------------------------------------------


def branch_state(suffix: str, facts: dict) -> str:
    """"gone" (no branch gives this directory name), "merged-own" (merged
    into main, with commits of its own), "merged-fresh" (its tip is a commit
    of main's own line: a branch just made, or fast-forwarded) or "open"."""
    if suffix not in facts.get("branches", ()):
        return "gone"
    if suffix in facts.get("own", ()):
        return "merged-own"
    if suffix in facts.get("merged", ()):
        return "merged-fresh"
    return "open"


def make_facts(branches, merged=(), own=(), worktrees=(), release=None) -> dict:
    """Facts from branch names; two branches with one directory name
    (a/b and a-b) count as the more careful state of the two."""
    branches, merged, own = set(branches), set(merged), set(own)
    by = {}
    for b in branches:
        by.setdefault(dir_suffix(b), []).append(b)
    return {
        "branches": set(by),
        "merged": {s for s, bs in by.items() if all(b in merged for b in bs)},
        "own": {s for s, bs in by.items() if all(b in own for b in bs)},
        "worktrees": {dir_suffix(b) for b in worktrees},
        "release": release,
    }


def git_facts(repo: Path, main: str = "main") -> dict:
    """The facts of the repository at `repo`: its branches, those merged
    into main, those merged with commits of their own (the tip is not on
    main's first-parent line), the branches of live worktrees and the newest
    release (tags vX.Y.Z and main's Cargo.toml)."""
    def git(*a):
        return subprocess.run(["git", "-C", str(repo)] + list(a), capture_output=True, text=True,
                              timeout=60).stdout
    tips = {}
    for l in git("for-each-ref", "--format=%(objectname) %(refname:short)", "refs/heads").splitlines():
        sha, _, name = l.partition(" ")
        if name:
            tips[name] = sha
    if main not in tips:
        # Without the branch list every checkout would look like "branch gone".
        raise RuntimeError("no branch %r in %s: no git facts, nothing is decided" % (main, repo))
    merged = set(git("for-each-ref", "--merged", main, "--format=%(refname:short)", "refs/heads").split())
    line = set(git("rev-list", "--first-parent", main).split())
    if tips[main] not in line or main not in merged:
        raise RuntimeError("git gave no history of %r in %s: nothing is decided" % (main, repo))
    own = {b for b in merged if tips.get(b) not in line}
    wt = {l[len("branch refs/heads/"):] for l in git("worktree", "list", "--porcelain").splitlines()
          if l.startswith("branch refs/heads/")}
    versions = [v for v in (version_of(t) for t in git("tag", "--list", "v[0-9]*").split()) if v]
    m = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+)"', git("show", main + ":Cargo.toml"), re.M)
    if m:
        versions.append(version_of(m.group(1)))
    return make_facts(tips, merged, own, wt, max(versions) if versions else None)


# --------------------------------------------------------------------------
# The rules
# --------------------------------------------------------------------------


def last_synced(c: dict):
    """Epoch of the checkout's last sync or use; None when unknown."""
    t = max(c.get("sync") or 0, c.get("used") or 0)
    return t or (c.get("mtime") or None)


def decide(c: dict, facts: dict, lim: dict, now: float, newest_release=None) -> dict:
    """The rule for one checkout: {"action": "keep" | "target" | "checkout",
    "reason", "cls", "state", "idle", "note"?}. `c`: {"name", "cls",
    "suffix", "busy", "sync", "used", "mtime", "target_bytes" (None: no
    target/), "out_bytes", "other_bytes"}."""
    cls, suffix = c["cls"], c["suffix"]
    last = last_synced(c)
    idle = None if last is None else max(0.0, now - last)
    state = branch_state(suffix, facts) if cls in ("agent", "merge", "other") else None
    live = suffix in facts.get("worktrees", ())
    out = {"cls": cls, "state": state, "idle": idle}

    def keep(reason):
        return dict(out, action="keep", reason=reason)

    if cls == "main":
        return keep("main checkout")
    if cls == "head":
        return keep("release / deploy checkout (never cleaned)")
    if cls == "foreign":
        return keep("has a .git of its own: not a remote.sh checkout (never cleaned)")
    if c.get("busy"):
        return keep("in use (running or waiting task)")
    if idle is None:
        return keep("last sync unknown")
    ago = fmt_age(idle)
    recent = idle < lim["recent_h"]
    why, final = None, False  # final: the reason cannot end (the whole checkout may go)
    if cls == "agent":
        if state == "gone":
            why, final, recent = "branch gone", True, False
        elif state.startswith("merged") and not live:
            why, final = "merged into main, worktree removed", True
        elif state == "merged-own":
            if idle >= lim["agent_days"]:
                why = "merged into main (with its own commits), not synced for %s" % ago
            else:
                return keep("merged into main, synced %s ago (worktree exists)" % ago)
        elif state == "merged-fresh":
            if idle < lim["other_days"]:
                return keep("new agent branch (no commits of its own yet)")
            why = "agent branch without commits of its own, not synced for %s" % ago
        else:
            return keep("branch open")
    elif cls == "merge":
        if state == "gone":
            why, final = "branch gone", True
        elif state.startswith("merged"):
            why, final = "merged into main", True
        elif idle >= lim["merge_days"]:
            why = "merge checkout not synced for %s" % ago
        else:
            return keep("merge in progress (branch open, synced %s ago)" % ago)
    elif cls == "release":
        v = version_of(suffix)
        if newest_release is None or v >= newest_release:
            return keep("newest release checkout")
        if idle < lim["release_days"]:
            return keep("older release (newest: %s), synced %s ago" % (fmt_version(newest_release), ago))
        why, final = "older release (newest: %s), not synced for %s" % (fmt_version(newest_release), ago), True
    else:
        if state == "merged-own":
            why, final = "merged into main", True
        elif state == "gone":
            if idle < lim["other_days"]:
                return keep("no branch of this name, synced %s ago" % ago)
            why, final = "no branch of this name, not synced for %s" % ago, True
        elif state == "merged-fresh":
            if idle < lim["other_days"]:
                return keep("branch without commits of its own (new, or main fast-forwarded to it), synced %s ago" % ago)
            why = "branch without commits of its own (new, or main fast-forwarded to it), not synced for %s" % ago
        else:
            return keep("branch open")
    if recent:
        return keep("%s, but synced %s ago" % (why, ago))
    has_target = c.get("target_bytes") is not None
    if final and idle >= lim["checkout_days"] and not live:
        if c.get("out_bytes"):
            note = "checkout kept: test-output/ or data/ in it (%s) may never have been fetched" % gb(c["out_bytes"])
            if has_target:
                return dict(out, action="target", reason=why, note=note)
            return keep("%s; %s" % (why, note))
        return dict(out, action="checkout", reason="%s; whole checkout: idle for %s" % (why, ago))
    if not has_target:
        return keep("%s; no target/ (sources stay until idle for %s)" % (why, fmt_age(lim["checkout_days"])))
    return dict(out, action="target", reason=why)


def plan(inv: dict, facts: dict, lim: dict | None = None, now: float | None = None) -> dict:
    """What the rules do on one machine: {"items": [...], "newest_release"}.
    An item: {"kind" (target | checkout | stray | temp | relout), "cls",
    "name", "op" ("kind|name": what the delete snippet takes), "path" (for
    the log), "bytes" (freed if deleted), "delete", "reason", "note",
    "idle", "busy"}. `inv` is a parsed inventory; ages use its clock."""
    lim = limits() if lim is None else lim
    now = inv.get("now") or now or 0
    versions = [facts.get("release")] + [version_of(c["suffix"]) for c in inv.get("checkouts", []) if c["cls"] == "release"]
    versions += [version_of(r["name"]) for r in inv.get("relout", [])]
    versions = [v for v in versions if v]
    newest = max(versions) if versions else None
    items = []
    for c in inv.get("checkouts", []):
        d = decide(c, facts, lim, now, newest)
        tb = c.get("target_bytes")
        whole = (tb or 0) + (c.get("other_bytes") or 0) + (c.get("out_bytes") or 0)
        kind = "checkout" if d["action"] == "checkout" else "target"
        items.append({
            "kind": kind, "cls": d["cls"], "name": c["name"], "op": "%s|%s" % (kind, c["name"]),
            "path": c["name"] + ("/" if kind == "checkout" else "/target"),
            "bytes": whole if kind == "checkout" else (tb or 0), "delete": d["action"] != "keep",
            "reason": d["reason"], "note": d.get("note"), "idle": d["idle"], "busy": bool(c.get("busy")),
            "has_target": tb is not None, "rest_bytes": (c.get("other_bytes") or 0) + (c.get("out_bytes") or 0),
            "out_bytes": c.get("out_bytes") or 0, "state": d["state"]})
        for s in c.get("strays", []):
            items.append(_aged("stray", c["cls"], c["name"] + "/" + s["name"], c["name"] + "/" + s["name"], s, now,
                               lim["stray_h"], "left by a failed deletion", c.get("busy")))
    for s in inv.get("strays", []):
        items.append(_aged("stray", "other", s["name"], s["name"] + "/", s, now, lim["stray_h"],
                           "checkout left by a failed deletion", False))
    for t in inv.get("temp", []):
        items.append(_aged("temp", "temp", t["name"], "tmp/" + t["name"], t, now, lim["temp_h"],
                           "browser profile of a web test", t.get("in_use"), "a running process uses it"))
    for r in inv.get("relout", []):
        v = version_of(r["name"])
        it = _aged("relout", "release-output", r["name"], "~/" + r["name"], r, now, lim["release_days"],
                   "output of an older release (newest: %s)" % fmt_version(newest), False)
        if v >= newest:
            it.update(delete=False, reason="output of the newest release")
        elif r.get("worlds"):
            it.update(delete=False, reason="holds world packages (never deleted)")
        items.append(it)
    return {"items": items, "newest_release": newest}


def _aged(kind, cls, name, path, e, now, limit, what, busy, busy_text="in use (running or waiting task)") -> dict:
    age = max(0.0, now - (e.get("mtime") or now))
    it = {"kind": kind, "cls": cls, "name": name, "op": "%s|%s" % (kind, name), "path": path,
          "bytes": e.get("bytes") or 0, "idle": age, "busy": bool(busy), "note": None}
    if busy:
        return dict(it, delete=False, reason=busy_text)
    if age < limit:
        return dict(it, delete=False, reason="%s, only %s old" % (what, fmt_age(age)))
    return dict(it, delete=True, reason="%s, %s old" % (what, fmt_age(age)))


def branch_item(base: str, branch: str) -> dict:
    """Cleanup on merge: the target/ of that branch's checkout, whatever
    the rules say (the coordinator names the branch it merged or abandoned)."""
    name = base + "-" + dir_suffix(branch)
    cls, _ = classify(name, base)
    if cls in (None, "main", "head") or not _SAFE_NAME.match(name):
        raise ValueError("refusing branch %r" % branch)
    return {"kind": "target", "cls": cls, "name": name, "op": "target|" + name, "path": name + "/target",
            "bytes": 0, "delete": True, "reason": "cleanup on merge (--branch %s)" % branch, "note": None}


def lru_candidates(p: dict, idle_s: float) -> list:
    """Under disk pressure: kept checkouts whose target/ may go anyway –
    agent, merge and other checkouts idle for `idle_s`, oldest first."""
    c = [i for i in p["items"] if i["kind"] == "target" and not i["delete"] and i.get("has_target") and not i["busy"]
         and i["cls"] in ("agent", "merge", "other") and i["idle"] is not None and i["idle"] >= idle_s]
    return sorted(c, key=lambda i: -i["idle"])


def totals(items: list) -> dict:
    """Per class: {"delete": [n, bytes], "keep": [n, bytes]}."""
    out = {}
    for i in items:
        t = out.setdefault(i["cls"], {"delete": [0, 0], "keep": [0, 0]})
        k = t["delete" if i["delete"] else "keep"]
        k[0] += 1
        k[1] += i["bytes"] or 0
    return out


def safe_op(op: str, base: str) -> bool:
    """A delete op the snippets may get: "kind|name" with plain names, a
    checkout of `base` that is neither the main one nor <base>-HEAD (strays
    may sit in any checkout), temp and release output names by pattern."""
    kind, _, name = op.partition("|")
    parts = name.split("/")
    if kind not in KINDS or not all(_SAFE_NAME.match(x) and x not in (".", "..") for x in parts):
        return False
    cls, _ = classify(parts[0], base)
    if kind in ("target", "checkout"):
        return len(parts) == 1 and cls in ("agent", "merge", "release", "other") and not is_stray(name)
    if kind == "stray":
        if len(parts) == 1:
            return cls is not None and cls != "main" and is_stray(name)
        return len(parts) == 2 and cls is not None and parts[1].startswith("target.") and is_stray(parts[1])
    if kind == "temp":
        return len(parts) == 1 and name.startswith(TEMP_PREFIXES)
    return len(parts) == 1 and bool(RELOUT_RE.match(name))


# --------------------------------------------------------------------------
# Remote snippets: inventory and deletion. POSIX ones run under /bin/sh
# (never the login shell: the Mac's is zsh, where an unmatched glob aborts
# the command); the Windows ones are PowerShell.
# --------------------------------------------------------------------------


def _sh_path(p: str) -> str:
    """A POSIX shell word for a path from machines.toml ("~/src/OCW")."""
    if p == "~" or p.startswith("~/"):
        return '"$HOME"' + _shq(p[1:]) if len(p) > 1 else '"$HOME"'
    return _shq(p)


def _shq(s: str) -> str:
    return "'" + s.replace("'", "'\\''") + "'"


def _psq(s: str) -> str:
    return "'" + s.replace("'", "''") + "'"


# (Its variables have names of their own: sh functions share the caller's.
# OCW_TASKS_DIR moves the marker directory in the tests.)
POSIX_BUSY = (r'''busy() { for bm in "${OCW_TASKS_DIR:-/tmp/ocw-tasks}"/*.json; do [ -e "$bm" ] || continue; bq=${bm##*/}; bq=${bq%%-*}; '''
              r'''grep -q "\"checkout\":\"$1\"" "$bm" && ps -p "$bq" >/dev/null 2>&1 && return 0; done; return 1; }''')


def posix_inventory(repo_path: str) -> str:
    """Read-only: the machine's clock, the disks, every checkout (sizes of
    target/, of test-output/ + data/ when they hold files, of the rest; last
    sync and use; leftovers of failed deletions), our web tests' browser
    profiles in the temp directory, release output folders, and the
    checkouts of live task markers."""
    return r"""
export LC_ALL=C
r=@REPO@; p=${r%/*}
mt() { stat -c %Y "$@" 2>/dev/null || stat -f %m "$@" 2>/dev/null; }
newest() { for f in "$@"; do [ -e "$f" ] && mt "$f"; done | sort -n | tail -n 1; }
kb() { du -sk "$1" 2>/dev/null | cut -f1; }
echo "@@now $(date +%s)"
echo @@disks; df -Pk "$HOME" "$p" 2>/dev/null | tail -n +2
echo @@checkouts
for d in "$r" "$r"-*; do
  [ -d "$d" ] || continue
  n=${d##*/}
  tk=-1; [ -d "$d/target" ] && tk=$(kb "$d/target")
  ok=0; all=0
  if [ "$d" != "$r" ]; then
    for s in test-output data; do
      if [ -d "$d/$s" ] && [ -n "$(find "$d/$s" -type f 2>/dev/null | head -n 1)" ]; then k=$(kb "$d/$s"); ok=$((ok + ${k:-0})); fi
    done
    all=$(kb "$d")
  fi
  sy=$(newest "$d/.ocw-sync" "$d/.ocw-sync/synced.txt" "$d/.ocw-last-run")
  us=$(newest "$d/target" "$d"/target/*/deps "$d"/target/*/.fingerprint "$d"/target/*/incremental)
  dm=$(mt "$d"); gi=0; [ -e "$d/.git" ] && gi=1
  echo "c ${all:-0} ${tk:-0} $ok ${sy:-0} ${us:-0} ${dm:-0} $gi $n"
  for g in "$d"/target.ocw-gc.* "$d"/target.ocw-janitor.*; do
    [ -e "$g" ] || continue; k=$(kb "$g"); gm=$(mt "$g"); echo "s ${k:-0} ${gm:-0} $n/${g##*/}"
  done
done
echo @@temp
for t in "${TMPDIR:-/tmp}" /tmp; do
  for g in "$t"/ocw-chromium-*; do
    [ -d "$g" ] || continue
    u=0; pgrep -f "${g##*/}" >/dev/null 2>&1 && u=1
    k=$(kb "$g"); gm=$(newest "$g" "$g"/*); echo "t ${k:-0} ${gm:-0} $u ${g##*/}"
  done
done
echo @@relout
for g in "$HOME"/ocw-release-*; do
  [ -d "$g" ] || continue
  w=0; ls "$g" 2>/dev/null | grep -qi "^world" && w=1
  k=$(kb "$g"); gm=$(newest "$g" "$g"/*); echo "r ${k:-0} ${gm:-0} $w ${g##*/}"
done
echo @@busy
for g in "${OCW_TASKS_DIR:-/tmp/ocw-tasks}"/*.json; do [ -e "$g" ] || continue; q=${g##*/}; q=${q%%-*}; ps -p "$q" >/dev/null 2>&1 && sed -n 's/.*"checkout":"\([^"]*\)".*/\1/p' "$g"; done
echo @@end
""".replace("@REPO@", _sh_path(repo_path))


def posix_delete(repo_path: str, dry: bool = False) -> str:
    """Deletes the ops read from stdin (lines "kind name", see ops_stdin):
    never the main checkout or <repo>-HEAD, never a checkout a live task
    marker names (checked right before each deletion), never a checkout with
    files under test-output/ or data/; target/ and whole checkouts are
    renamed first, so a run starting meanwhile sees no half-deleted
    directory. The names come on stdin, not in the command line, so `pgrep`
    for a browser profile's name does not find this shell. Prints
    "@@deleted|would|busy|gone|kept|refused|failed KIND NAME [KB]"."""
    return r"""
export LC_ALL=C
r=@REPO@; p=${r%/*}; b=${r##*/}; dry=@DRY@
@BUSY@
while read -r k n; do
  [ -n "$n" ] || continue
  case "$n" in /*|*..*|*/*/*) echo "@@refused $k $n"; continue ;; esac
  c=; g=; x=
  case "$k" in
    target) c=$n; x="$p/$n/target"; g="$p/$n/target.ocw-gc.$$" ;;
    checkout) c=$n; x="$p/$n"; g="$p/$n.ocw-gc.$$" ;;
    stray) c=${n%%/*}; x="$p/$n"; case "$n" in *.ocw-gc.*|*.ocw-janitor.*) ;; *) echo "@@refused $k $n"; continue ;; esac ;;
    temp) for t in "${TMPDIR:-/tmp}" /tmp; do [ -d "$t/$n" ] && x="$t/$n"; done; case "$n" in ocw-chromium-*) ;; *) echo "@@refused $k $n"; continue ;; esac ;;
    relout) x="$HOME/$n"; case "$n" in ocw-release-[0-9]*) ;; *) echo "@@refused $k $n"; continue ;; esac ;;
    *) echo "@@refused $k $n"; continue ;;
  esac
  case "$k" in target|checkout) case "$n" in "$b"|"$b"-HEAD|*/*) echo "@@refused $k $n"; continue ;; "$b"-*) ;; *) echo "@@refused $k $n"; continue ;; esac ;; esac
  case "$c" in "") ;; "$b"|"$b"-*) ;; *) echo "@@refused $k $n"; continue ;; esac
  if [ -z "$x" ] || [ ! -e "$x" ]; then echo "@@gone $k $n"; continue; fi
  case "$k" in target|checkout) [ -e "$p/$n/.git" ] && { echo "@@refused $k $n"; continue; } ;; esac
  if [ -n "$c" ] && busy "$c"; then echo "@@busy $k $n"; continue; fi
  if [ "$k" = temp ] && pgrep -f "$n" >/dev/null 2>&1; then echo "@@busy $k $n"; continue; fi
  if [ "$k" = checkout ] && [ -n "$(find "$x/test-output" "$x/data" -type f 2>/dev/null | head -n 1)" ]; then echo "@@kept $k $n"; continue; fi
  if [ "$k" = relout ] && ls "$x" 2>/dev/null | grep -qi "^world"; then echo "@@kept $k $n"; continue; fi
  s=$(du -sk "$x" 2>/dev/null | cut -f1)
  if [ "$dry" = 1 ]; then echo "@@would $k $n ${s:-0}"; continue; fi
  if [ -n "$g" ]; then mv "$x" "$g" 2>/dev/null || { echo "@@failed $k $n"; continue; }; x=$g; fi
  rm -rf "$x" 2>/dev/null
  if [ -e "$x" ]; then echo "@@failed $k $n"; else echo "@@deleted $k $n ${s:-0}"; fi
done
echo @@end
""".replace("@REPO@", _sh_path(repo_path)).replace("@DRY@", "1" if dry else "0").replace("@BUSY@", POSIX_BUSY)


def ops_stdin(ops: list, base: str) -> str:
    """The stdin of posix_delete: one "kind name" line per (checked) op."""
    for op in ops:
        if not safe_op(op, base):
            raise ValueError("unsafe cleanup op %r" % op)
    return "".join(op.replace("|", " ", 1) + "\n" for op in ops)


WINDOWS_BUSY = r"""$busy = @(Get-ChildItem -LiteralPath (Join-Path $env:TEMP 'ocw-tasks') -Filter '*.json' | ForEach-Object {
  if (Get-Process -Id ($_.BaseName -split '-')[0]) { try { ([IO.File]::ReadAllText($_.FullName).TrimStart([char]0xFEFF) | ConvertFrom-Json).checkout } catch {} } })"""

WINDOWS_FUNCS = r"""function Sz($p) { [int64]((Get-ChildItem -LiteralPath $p -Recurse -File -Force | Measure-Object Length -Sum).Sum) }
function Mt($p) { $n = 0; foreach ($f in $p) { $i = Get-Item -LiteralPath $f -Force; if ($i) { $u = ([DateTimeOffset]$i.LastWriteTimeUtc).ToUnixTimeSeconds(); if ($u -gt $n) { $n = $u } } }; $n }"""

# Long-path safe removal: Remove-Item fails on paths over 260 characters
# (release targets hold deep bundle paths; on 2026-11-28 two renamed
# target.ocw-gc.PID folders stayed on Win11Builder), so what is left goes
# with cmd's `rd` on the \\?\ form of the path, which has no such limit.
WINDOWS_RM = r"""function OcwRemove($p) { Remove-Item -LiteralPath $p -Recurse -Force
  if (Test-Path -LiteralPath $p) { $q = (Get-Item -LiteralPath $p -Force).FullName; if ($q -and -not $q.StartsWith('\\')) { & cmd.exe /d /c ('rd /s /q "\\?\' + $q + '"') 2>&1 | Out-Null } } }"""


def windows_inventory(repo_path: str) -> str:
    return r"""
$ErrorActionPreference = 'SilentlyContinue'; $ProgressPreference = 'SilentlyContinue'
$r = @REPO@; $parent = Split-Path $r; $leaf = Split-Path -Leaf $r
@FUNCS@
$o = [ordered]@{}
$o.now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$want = @($r.Substring(0, 1).ToUpper(), 'C')
$o.disks = @([System.IO.DriveInfo]::GetDrives() | Where-Object { $_.DriveType -eq 'Fixed' -and $_.IsReady -and $want -contains $_.Name.Substring(0, 1).ToUpper() } |
  ForEach-Object { [ordered]@{ name = $_.Name.Substring(0, 2); total = [int64]$_.TotalSize; used = [int64]($_.TotalSize - $_.TotalFreeSpace); free = [int64]$_.AvailableFreeSpace } })
$o.checkouts = @(Get-ChildItem -LiteralPath $parent -Directory -Force | Where-Object { $_.Name -eq $leaf -or $_.Name -like ($leaf + '-*') } | ForEach-Object {
  $d = $_.FullName; $t = Join-Path $d 'target'; $main = ($_.Name -eq $leaf)
  $tb = [int64]-1; $ob = [int64]0; $rest = [int64]0; $strays = @()
  foreach ($e in Get-ChildItem -LiteralPath $d -Force) {
    $isT = ($e.PSIsContainer -and $e.Name -eq 'target'); $isS = ($e.Name -like 'target.ocw-gc.*' -or $e.Name -like 'target.ocw-janitor.*')
    if ($main -and -not ($isT -or $isS)) { continue }
    if ($e.PSIsContainer) { $s = Sz $e.FullName } else { $s = [int64]$e.Length }
    if ($isT) { $tb = $s }
    elseif ($isS) { $strays += [ordered]@{ name = $e.Name; bytes = $s; mtime = (Mt @($e.FullName)) } }
    elseif ($e.Name -eq 'test-output' -or $e.Name -eq 'data') { $ob += $s }
    else { $rest += $s }
  }
  $us = @($t); if ($tb -ge 0) { $us += @(Get-ChildItem -LiteralPath $t -Directory -Force | ForEach-Object { Join-Path $_.FullName 'deps'; Join-Path $_.FullName '.fingerprint'; Join-Path $_.FullName 'incremental' }) }
  [ordered]@{ name = $_.Name; target = $tb; out = $ob; other = $rest; strays = $strays; mtime = (Mt @($d)); used = (Mt $us); git = [int](Test-Path -LiteralPath (Join-Path $d '.git'))
    sync = (Mt @((Join-Path $d '.ocw-sync'), (Join-Path $d '.ocw-sync\manifest.tsv'), (Join-Path $d '.ocw-last-run'))) } })
$o.temp = @(Get-ChildItem -LiteralPath $env:TEMP -Directory -Force -Filter 'ocw-chromium-*' | ForEach-Object {
  [ordered]@{ name = $_.Name; bytes = (Sz $_.FullName); mtime = (Mt (@($_.FullName) + @(Get-ChildItem -LiteralPath $_.FullName -Force | ForEach-Object { $_.FullName }))); in_use = 0 } })
$o.relout = @(Get-ChildItem -LiteralPath $env:USERPROFILE -Directory -Force -Filter 'ocw-release-*' | ForEach-Object {
  $kids = @(Get-ChildItem -LiteralPath $_.FullName -Force)
  [ordered]@{ name = $_.Name; bytes = (Sz $_.FullName); mtime = (Mt (@($_.FullName) + @($kids | ForEach-Object { $_.FullName }))); worlds = [int][bool](@($kids | Where-Object { $_.Name -like 'world*' }).Count) } })
@BUSY@
$o.busy = $busy
$o | ConvertTo-Json -Depth 6 -Compress
""".replace("@REPO@", _psq(repo_path)).replace("@FUNCS@", WINDOWS_FUNCS).replace("@BUSY@", WINDOWS_BUSY)


def windows_delete(repo_path: str, ops: list, dry: bool = False) -> str:
    """The PowerShell twin of posix_delete (the ops are part of the script:
    it is sent encoded, so no process list shows them)."""
    base = re.split(r"[\\/]", repo_path.rstrip("\\/"))[-1]
    for op in ops:
        if not safe_op(op, base):
            raise ValueError("unsafe cleanup op %r" % op)
    return r"""
$ErrorActionPreference = 'SilentlyContinue'; $ProgressPreference = 'SilentlyContinue'
$r = @REPO@; $parent = Split-Path $r; $leaf = Split-Path -Leaf $r; $dry = @DRY@
@FUNCS@
@RM@
@BUSY@
foreach ($op in @(@OPS@)) {
  $k, $n = $op -split '\|', 2
  $c = $null; $g = $null; $x = $null; $tag = $k + ' ' + $n
  if ($n -match '^[\\/]|\.\.|/.*/|\\') { '@@refused ' + $tag; continue }
  if ($k -eq 'target') { $c = $n; $x = Join-Path (Join-Path $parent $n) 'target'; $g = 'target.ocw-gc.' + $PID }
  elseif ($k -eq 'checkout') { $c = $n; $x = Join-Path $parent $n; $g = $n + '.ocw-gc.' + $PID }
  elseif ($k -eq 'stray' -and $n -match '\.ocw-(gc|janitor)\.') { $c = ($n -split '/')[0]; $x = Join-Path $parent ($n -replace '/', '\') }
  elseif ($k -eq 'temp' -and $n -like 'ocw-chromium-*') { $x = Join-Path $env:TEMP $n }
  elseif ($k -eq 'relout' -and $n -match '^ocw-release-[0-9]') { $x = Join-Path $env:USERPROFILE $n }
  else { '@@refused ' + $tag; continue }
  if (($k -eq 'target' -or $k -eq 'checkout') -and ($n -eq $leaf -or $n -eq ($leaf + '-HEAD') -or $n -match '/' -or -not $n.StartsWith($leaf + '-'))) { '@@refused ' + $tag; continue }
  if ($c -and $c -ne $leaf -and -not $c.StartsWith($leaf + '-')) { '@@refused ' + $tag; continue }
  if (-not (Test-Path -LiteralPath $x)) { '@@gone ' + $tag; continue }
  if (($k -eq 'target' -or $k -eq 'checkout') -and (Test-Path -LiteralPath (Join-Path (Join-Path $parent $n) '.git'))) { '@@refused ' + $tag; continue }
  if ($c -and ($busy -contains $c)) { '@@busy ' + $tag; continue }
  if ($k -eq 'checkout') {
    $has = $false; foreach ($s in 'test-output', 'data') { if (Get-ChildItem -LiteralPath (Join-Path $x $s) -Recurse -File -Force | Select-Object -First 1) { $has = $true } }
    if ($has) { '@@kept ' + $tag; continue } }
  if ($k -eq 'relout' -and @(Get-ChildItem -LiteralPath $x -Force | Where-Object { $_.Name -like 'world*' }).Count) { '@@kept ' + $tag; continue }
  $kb = [int64]((Sz $x) / 1024)
  if ($dry) { '@@would ' + $tag + ' ' + $kb; continue }
  if ($g) { Rename-Item -LiteralPath $x -NewName $g; $x = Join-Path (Split-Path $x) $g; if (-not (Test-Path -LiteralPath $x)) { '@@failed ' + $tag; continue } }
  OcwRemove $x
  if (Test-Path -LiteralPath $x) { '@@failed ' + $tag } else { '@@deleted ' + $tag + ' ' + $kb }
}
'@@end'
""".replace("@REPO@", _psq(repo_path)).replace("@DRY@", "$true" if dry else "$false") \
        .replace("@FUNCS@", WINDOWS_FUNCS).replace("@RM@", WINDOWS_RM).replace("@BUSY@", WINDOWS_BUSY) \
        .replace("@OPS@", ", ".join(_psq(op) for op in ops))


# --------------------------------------------------------------------------
# Parsers
# --------------------------------------------------------------------------


def _checkout(name, base, target, out, other, sync, used, mtime, busy, strays, git=False):
    cls, suffix = classify(name, base)
    if cls is None or not _SAFE_NAME.match(name):
        return None
    if git and cls not in ("main", "head"):
        cls = "foreign"
    return {"name": name, "cls": cls, "suffix": suffix, "target_bytes": None if target < 0 else target,
            "out_bytes": out, "other_bytes": max(0, other), "sync": sync, "used": used, "mtime": mtime,
            "busy": name in busy, "strays": strays}


def _split_strays(checkouts: list):
    """Renamed whole checkouts (<name>.ocw-gc.PID) are leftovers, not checkouts."""
    keep, strays = [], []
    for c in checkouts:
        if c is None:
            continue
        if is_stray(c["name"]):
            if c["cls"] != "main":
                strays.append({"name": c["name"], "mtime": c["mtime"],
                               "bytes": (c["target_bytes"] or 0) + c["out_bytes"] + c["other_bytes"]})
        else:
            keep.append(c)
    return keep, strays


def parse_posix_inventory(text: str, base: str) -> dict:
    sec, cur = {}, None
    for line in text.splitlines():
        s = line.strip()
        m = re.fullmatch(r"@@(\w+)(?: (\d+))?", s)
        if m:
            cur = m.group(1)
            sec[cur] = [m.group(2)] if m.group(2) else []
        elif cur:
            sec[cur].append(line)
    disks, seen = [], set()
    for i, l in enumerate(sec.get("disks", [])):
        f = l.split()
        if len(f) >= 6 and f[1].isdigit() and f[0] not in seen:
            seen.add(f[0])
            disks.append({"name": "home" if i == 0 else "src", "total": int(f[1]) * 1024,
                          "used": int(f[2]) * 1024, "free": int(f[3]) * 1024})
    busy = {l.strip() for l in sec.get("busy", []) if l.strip()}
    rows, strays = [], {}
    for l in sec.get("checkouts", []):
        f = l.split(None, 8)
        try:
            if f and f[0] == "c" and len(f) == 9:
                rows.append((f[8], [int(x) for x in f[1:8]]))
            elif f and f[0] == "s" and len(f) >= 4:
                f = l.split(None, 3)
                owner, _, sname = f[3].partition("/")
                if _SAFE_NAME.match(sname or "") and is_stray(sname):
                    strays.setdefault(owner, []).append({"name": sname, "bytes": int(f[1]) * 1024, "mtime": int(f[2])})
        except ValueError:
            continue
    checkouts = []
    for name, (all_kb, t_kb, o_kb, sync, used, mtime, git) in rows:
        t = -1 if t_kb < 0 else t_kb * 1024
        sb = sum(s["bytes"] for s in strays.get(name, []))
        other = (all_kb - max(t_kb, 0) - o_kb) * 1024 - sb
        checkouts.append(_checkout(name, base, t, o_kb * 1024, other, sync, used, mtime, busy, strays.get(name, []),
                                   bool(git)))
    checkouts, whole = _split_strays(checkouts)
    temp, relout, names = [], [], set()
    for l in sec.get("temp", []):
        f = l.split(None, 4)
        if len(f) == 5 and f[0] == "t" and f[4] not in names and f[4].startswith(TEMP_PREFIXES) and _SAFE_NAME.match(f[4]):
            names.add(f[4])
            temp.append({"name": f[4], "bytes": int(f[1]) * 1024, "mtime": int(f[2]), "in_use": f[3] == "1"})
    for l in sec.get("relout", []):
        f = l.split(None, 4)
        if len(f) == 5 and f[0] == "r" and RELOUT_RE.match(f[4]):
            relout.append({"name": f[4], "bytes": int(f[1]) * 1024, "mtime": int(f[2]), "worlds": f[3] == "1"})
    now = sec.get("now") or [None]
    # The checkouts live on the disk of the repo's directory (the last df line).
    return {"now": int(now[0]) if now[0] else None, "disks": disks, "disk": disks[-1] if disks else {},
            "checkouts": checkouts, "strays": whole, "temp": temp, "relout": relout}


def parse_windows_inventory(text: str, base: str) -> dict:
    start = text.find("{")
    data = json.loads(text[start:]) if start >= 0 else {}
    lst = lambda v: v if isinstance(v, list) else ([v] if v else [])  # noqa: E731
    num = lambda v: int(v or 0)  # noqa: E731
    disks = [{"name": d.get("name"), "total": d.get("total"), "used": d.get("used"), "free": d.get("free")}
             for d in lst(data.get("disks"))]
    busy = {str(b) for b in lst(data.get("busy"))}
    checkouts = []
    for c in lst(data.get("checkouts")):
        strays = [{"name": str(s.get("name")), "bytes": num(s.get("bytes")), "mtime": num(s.get("mtime"))}
                  for s in lst(c.get("strays")) if _SAFE_NAME.match(str(s.get("name"))) and is_stray(str(s.get("name")))]
        checkouts.append(_checkout(str(c.get("name")), base, num(c.get("target")) if c.get("target") is not None else -1,
                                   num(c.get("out")), num(c.get("other")), num(c.get("sync")), num(c.get("used")),
                                   num(c.get("mtime")), busy, strays, bool(c.get("git"))))
    checkouts, whole = _split_strays(checkouts)
    temp = [{"name": str(t.get("name")), "bytes": num(t.get("bytes")), "mtime": num(t.get("mtime")),
             "in_use": bool(t.get("in_use"))} for t in lst(data.get("temp"))
            if str(t.get("name")).startswith(TEMP_PREFIXES) and _SAFE_NAME.match(str(t.get("name")))]
    relout = [{"name": str(r.get("name")), "bytes": num(r.get("bytes")), "mtime": num(r.get("mtime")),
               "worlds": bool(r.get("worlds"))} for r in lst(data.get("relout")) if RELOUT_RE.match(str(r.get("name")))]
    return {"now": num(data.get("now")) or None, "disks": disks,
            "disk": next((d for d in disks if d["name"] != "C:"), disks[0] if disks else {}),
            "checkouts": checkouts, "strays": whole, "temp": temp, "relout": relout}


def parse_results(text: str) -> dict:
    """op ("kind|name") -> {"result", "bytes"} from the delete snippets' lines."""
    out = {}
    for l in text.splitlines():
        m = re.match(r"@@(deleted|would|busy|gone|kept|refused|failed)\s+(\w+)\s+(\S+)(?:\s+(\d+))?\s*$", l.strip())
        if m:
            out["%s|%s" % (m.group(2), m.group(3))] = {
                "result": m.group(1), "bytes": int(m.group(4)) * 1024 if m.group(4) else None}
    return out


def base_name(repo_path: str) -> str:
    return re.split(r"[\\/]", repo_path.rstrip("\\/"))[-1]


# --------------------------------------------------------------------------
# Text (the dry-run output of remote-gc.sh and `make janitor-plan`)
# --------------------------------------------------------------------------


def plan_text(p: dict, verbose: bool = False, delete: bool = False) -> list:
    """Lines for one machine's plan: every deletion with its size and rule,
    what is kept and why (kept items without anything to free are counted by
    reason unless `verbose`), the totals per class and the leftovers."""
    verb = "delete" if delete else "would delete"
    out, quiet = [], {}
    for i in sorted(p["items"], key=lambda i: (not i["delete"], i["cls"], i["name"])):
        what = i["path"] + (" (whole checkout)" if i["kind"] == "checkout" else "")
        if i["delete"]:
            out.append("  %-12s %9s  %-52s %s: %s" % (verb, gb(i["bytes"]), what, i["cls"], i["reason"]))
            if i.get("note"):
                out.append("  %-12s %9s  %-52s %s" % ("", "", "", i["note"]))
        elif verbose or i["bytes"] >= 50e6 or i["kind"] != "target":
            out.append("  %-12s %9s  %-52s %s: %s" % ("keep", gb(i["bytes"]), what, i["cls"], i["reason"]))
        else:
            k = (i["cls"], re.sub(r"\d+d \d+h|\d+h \d+m|\b\d+m\b", "…", i["reason"]))
            q = quiet.setdefault(k, [0, 0])
            q[0] += 1
            q[1] += i.get("rest_bytes") or 0
    for (cls, reason), (n, rest) in sorted(quiet.items()):
        out.append("  %-12s %9s  %-52s %s: %s" % ("keep", gb(rest), "%d checkout(s) without target/" % n, cls, reason))
    out.append("  per class:")
    for cls, t in sorted(totals(p["items"]).items()):
        out.append("    %-15s %s %3d (%s), keep %3d (%s)" % (cls, verb, t["delete"][0], gb(t["delete"][1]),
                                                             t["keep"][0], gb(t["keep"][1])))
    left = [i for i in p["items"] if i["kind"] in ("target", "checkout") and i["cls"] not in ("main", "head", "foreign")]
    rest = sum(i.get("rest_bytes") or 0 for i in left)
    outb = sum(i.get("out_bytes") or 0 for i in left)
    out.append("  not target/: %s in %d checkouts (sources, synced files), of it test-output/ + data/: %s in %d" % (
        gb(rest), len(left), gb(outb), sum(1 for i in left if i.get("out_bytes"))))
    return out
