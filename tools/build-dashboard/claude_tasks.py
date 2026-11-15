"""The Claude tasks on the build dashboard (spec §11.2.1): which agents of
the coordinator run, how alive they are, their worktrees and branches, and
the cutover check ("can the coordinator move now?").

Sources, all read-only, cheap and throttled:

- test-output/build-log/DAY/agents.jsonl (written, masked, by OCW-ART's
  `diary.py agent-event`): one line per event; the last event of a task
  decides its state (started / resumed = running).
- Liveness: Claude Code's task files /tmp/claude-<uid>/<project>/<session>/
  tasks/<task id>.output. Only their mtime and size are used (os.stat);
  they are full transcripts and are **never opened**. Session directory
  names are never shown.
- Worktrees: `git worktree list --porcelain` of OCW's main checkout and of
  OCW-ART, with each branch's last commit, commits not in main, merged into
  main, and pushed (compared with refs/remotes/origin/<branch>, no fetch).
"""

from __future__ import annotations

import glob
import json
import os
import re
import subprocess
import threading
import time
from pathlib import Path

RUNNING_EVENTS = ("started", "resumed")
DONE_EVENTS = ("reported", "merged", "abandoned")
RECENT_S = 24 * 3600          # finished tasks shown behind the toggle
IDLE_S = 20 * 60              # a running task's file unchanged this long: idle / waiting
DAYS_BACK = 3                 # agents.jsonl files read (today and the days before)
GIT_INTERVAL_S = 120.0        # worktrees and branches
TASK_GLOB = "/tmp/claude-*/*/*/tasks"
_FULL_ID = re.compile(r"^a[0-9a-f]{16}$")
_BRANCH_ID = re.compile(r"(?:^|[-/])agent-(a[0-9a-f]{16})$")


# --------------------------------------------------------------------------
# agents.jsonl: the state machine
# --------------------------------------------------------------------------


def _epoch(ts):
    """ISO time with offset -> Unix time (None if not one)."""
    if not isinstance(ts, str) or not ts:
        return None
    try:
        from datetime import datetime
        return datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def normalize_event(e: dict):
    """One agents.jsonl line -> {"key", "id_short", "full_id", "task",
    "branch", "event", "t", "commits", "note"} (None if it is not one).
    Tolerates lines whose task field holds the agent id (and the short id
    a slice of the name): then the branch names the task."""
    if not isinstance(e, dict):
        return None
    ev, t = str(e.get("event") or ""), _epoch(e.get("time"))
    if not ev or t is None:
        return None
    short, task, branch = str(e.get("agent_id_short") or ""), str(e.get("task") or ""), str(e.get("branch") or "")
    full = None
    m = _BRANCH_ID.search(branch)
    if m:
        full = m.group(1)
    if _FULL_ID.match(task):          # swapped fields: the task is the agent id
        full, task = task, ""
    if full:
        short = full[:8]
    return {"key": short or branch, "id_short": short, "full_id": full, "task": task, "branch": branch,
            "event": ev, "t": t, "time": e.get("time"), "commits": str(e.get("commits") or ""),
            "note": str(e.get("note") or "")}


def read_events(build_log: Path | None, now: float | None = None, days: int = DAYS_BACK) -> list:
    """The events of the last `days` days' agents.jsonl, oldest first."""
    if build_log is None:
        return []
    now = time.time() if now is None else now
    out = []
    for k in range(days - 1, -1, -1):
        f = build_log / time.strftime("%Y-%m-%d", time.localtime(now - k * 86400)) / "agents.jsonl"
        try:
            text = f.read_text(encoding="utf-8")
        except OSError:
            continue
        for line in text.splitlines():
            try:
                e = normalize_event(json.loads(line))
            except ValueError:
                continue
            if e:
                out.append(e)
    out.sort(key=lambda e: e["t"])
    return out


def task_states(events: list) -> list:
    """Per task (newest activity first): its last event decides the state:
    running (started / resumed), interrupted, reported, merged, abandoned."""
    tasks = {}
    for e in events:
        k = e["key"]
        t = tasks.setdefault(k, {"key": k, "id_short": e["id_short"], "full_id": None, "task": "", "branch": "",
                                 "started": None, "events": 0})
        t["events"] += 1
        if e["full_id"]:
            t["full_id"] = e["full_id"]
        if e["task"]:
            t["task"] = e["task"]         # titles get better over time ("wave 2" -> the real one)
        if e["branch"]:
            t["branch"] = e["branch"]
        if e["commits"]:
            t["commits"] = e["commits"]
        if e["event"] == "started" or t["started"] is None:
            t["started"] = e["t"]
        t.update(last_event=e["event"], last_event_t=e["t"], last_event_time=e["time"], note=e["note"])
    out = []
    for t in tasks.values():
        t["state"] = "running" if t["last_event"] in RUNNING_EVENTS else t["last_event"]
        t["running"] = t["state"] == "running"
        if not t["task"]:
            t["task"] = t["branch"] or t["key"]
        out.append(t)
    out.sort(key=lambda t: -t["last_event_t"])
    return out


# --------------------------------------------------------------------------
# Liveness from the task files' mtime and size (never their content)
# --------------------------------------------------------------------------


def task_files(pattern: str = TASK_GLOB) -> dict:
    """task id -> path of its .output file, from the directory listings
    only (names; nothing is opened)."""
    out = {}
    for d in glob.glob(pattern):
        try:
            with os.scandir(d) as it:
                for e in it:
                    if e.name.endswith(".output"):
                        out.setdefault(e.name[:-len(".output")], e.path)
        except OSError:
            continue
    return out


def find_task_file(files: dict, full_id, id_short: str):
    if full_id and full_id in files:
        return files[full_id]
    if id_short and len(id_short) >= 6:
        hits = [k for k in files if k.startswith(id_short)]
        if len(hits) == 1:
            return files[hits[0]]
    return None


class Liveness:
    """mtime / size of a task's file: active (growing or not), idle /
    waiting (unchanged for over 20 min), unknown (no file)."""

    def __init__(self):
        self.sizes = {}

    def check(self, path, now: float, stat=os.stat) -> dict:
        if not path:
            return {"activity": "unknown", "last_activity": None, "age_s": None, "growing": None}
        try:
            s = stat(path)  # follows the symlink to the transcript; never opened
        except OSError:
            return {"activity": "unknown", "last_activity": None, "age_s": None, "growing": None}
        prev = self.sizes.get(path)
        self.sizes[path] = s.st_size
        age = max(0.0, now - s.st_mtime)
        growing = prev is not None and s.st_size > prev
        return {"activity": "idle / waiting" if age > IDLE_S else ("growing" if growing else "active"),
                "last_activity": s.st_mtime, "age_s": age, "growing": growing, "size": s.st_size}


# --------------------------------------------------------------------------
# Worktrees and branches
# --------------------------------------------------------------------------


def parse_worktree_porcelain(text: str) -> list:
    """`git worktree list --porcelain` -> [{"path", "head", "branch",
    "detached", "bare", "locked", "prunable"}] (the first is the main one)."""
    out, cur = [], None
    for line in text.splitlines() + [""]:
        if not line.strip():
            if cur:
                out.append(cur)
            cur = None
            continue
        k, _, v = line.partition(" ")
        if k == "worktree":
            cur = {"path": v, "head": None, "branch": None, "detached": False, "bare": False, "locked": False,
                   "prunable": False}
        elif cur is None:
            continue
        elif k == "HEAD":
            cur["head"] = v
        elif k == "branch":
            cur["branch"] = v[len("refs/heads/"):] if v.startswith("refs/heads/") else v
        elif k in ("detached", "bare", "locked", "prunable"):
            cur[k] = True
    return out


def parse_refs(text: str) -> dict:
    """for-each-ref lines "refname NUL sha NUL unix time NUL subject" -> refname -> dict."""
    out = {}
    for line in text.splitlines():
        f = line.split("\x00")
        if len(f) < 4:
            continue
        try:
            ct = int(f[2])
        except ValueError:
            ct = None
        out[f[0]] = {"sha": f[1], "time": ct, "subject": f[3]}
    return out


SUBJECT_MAX = 90
_SESSION_DIR = re.compile(r"/tmp/claude-[^/]+/[^/]+/[^/]+")


def _clip(s, n):
    return s if not isinstance(s, str) or len(s) <= n else s[:n - 1] + "…"


def safe_path(p: str, home) -> str:
    """A worktree path for the page: the home directory as ~, and Claude
    Code's per-session directories (/tmp/claude-UID/PROJECT/SESSION, the
    session id) as /tmp/claude-…"""
    p = _SESSION_DIR.sub("/tmp/claude-…", p)
    h = str(home).rstrip("/")
    return "~" + p[len(h):] if p == h or p.startswith(h + "/") else p


def push_state(sha, remote_sha) -> str:
    if remote_sha is None:
        return "not pushed"
    return "pushed" if remote_sha == sha else "differs from origin"


def _git(repo: Path, *args, timeout: float = 15.0):
    try:
        r = subprocess.run(["git", "-C", str(repo)] + list(args), capture_output=True, text=True, timeout=timeout,
                           errors="replace", stdin=subprocess.DEVNULL)
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.SubprocessError):
        return None


class Worktrees:
    """The worktrees of the repositories with their branch state; the git
    calls run at most every GIT_INTERVAL_S, the commit counts are cached per
    (branch sha, main sha)."""

    def __init__(self, repos: list, home=None, git=_git):
        self.repos = [(name, Path(p)) for name, p in repos]
        self.home = str(home or Path.home())
        self.git = git
        self.ahead_cache = {}
        self.at = None
        self.items = []

    def _main_ref(self, refs: dict) -> str | None:
        for b in ("main", "master"):
            if "refs/heads/" + b in refs:
                return b
        return None

    def refresh(self, now: float | None = None) -> list:
        items = []
        for name, repo in self.repos:
            text = self.git(repo, "worktree", "list", "--porcelain")
            if text is None:
                continue
            refs = parse_refs(self.git(repo, "for-each-ref", "--format=%(refname)%00%(objectname)%00"
                                       "%(committerdate:unix)%00%(subject)", "refs/heads", "refs/remotes/origin") or "")
            main = self._main_ref(refs)
            main_sha = (refs.get("refs/heads/" + main) or {}).get("sha") if main else None
            merged = set()
            if main:
                merged = set((self.git(repo, "branch", "--merged", main, "--format=%(refname:short)") or "").split())
            for i, w in enumerate(parse_worktree_porcelain(text)):
                if w["bare"]:
                    continue
                b = w["branch"]
                ref = refs.get("refs/heads/" + b) if b else None
                sha = (ref or {}).get("sha") or w["head"]
                item = {"repo": name, "path": self._home(w["path"]), "main": i == 0, "branch": b,
                        "detached": w["detached"], "prunable": w["prunable"], "sha": (sha or "")[:7],
                        "subject": _clip((ref or {}).get("subject"), SUBJECT_MAX), "commit_time": (ref or {}).get("time")}
                if b and main and b != main:
                    key = (repo, b, sha, main_sha)
                    if key not in self.ahead_cache:
                        n = self.git(repo, "rev-list", "--count", "%s..%s" % (main, b))
                        self.ahead_cache[key] = int(n.strip()) if n and n.strip().isdigit() else None
                    item["ahead"] = self.ahead_cache[key]
                    item["merged"] = b in merged
                    item["new"] = sha != main_sha  # False: the branch is still at main's commit (no commits yet)
                    item["pushed"] = push_state(sha, (refs.get("refs/remotes/origin/" + b) or {}).get("sha"))
                elif b == main:
                    item["pushed"] = push_state(sha, (refs.get("refs/remotes/origin/" + b) or {}).get("sha"))
                items.append(item)
        self.items, self.at = items, time.time() if now is None else now
        return items

    def _home(self, p: str) -> str:
        return safe_path(p, self.home)

    def get(self, now: float | None = None) -> list:
        now = time.time() if now is None else now
        if self.at is None or now - self.at >= GIT_INTERVAL_S:
            self.refresh(now)
        return self.items


# --------------------------------------------------------------------------
# Status and the cutover check
# --------------------------------------------------------------------------


def remote_runs(machines: list) -> list:
    """The build machines' running and queued remote.sh runs (the queue)."""
    out = []
    for m in machines or []:
        if not m.get("online"):
            continue
        for t in m.get("tasks") or []:
            if t.get("state") != "waiting":
                out.append({"machine": m.get("name"), "checkout": t.get("checkout"), "state": "running",
                            "command": t.get("command"), "elapsed": t.get("elapsed")})
        for w in (m.get("queue") or {}).get("queued") or []:
            out.append({"machine": m.get("name"), "checkout": w.get("checkout"), "state": "waiting",
                        "command": w.get("command"), "elapsed": w.get("elapsed")})
    return out


def cutover(tasks: list, runs: list, sessions=None) -> dict:
    """"Cutover OK: no running tasks, no remote runs", or what still runs."""
    running = [t for t in tasks if t.get("running")]
    items = ["task %s (%s) %s" % (t["id_short"] or t["key"], t["task"], t.get("activity") or "running")
             for t in running]
    items += ["remote run on %s: %s%s" % (r["machine"], r["checkout"], " (waiting for a slot)"
                                          if r["state"] == "waiting" else "") for r in runs]
    ok = not items
    return {"ok": ok, "items": items, "running_tasks": len(running), "remote_runs": len(runs),
            "claude_sessions": sessions,
            "text": "Cutover OK: no running tasks, no remote runs" if ok else
            "Not ready for the cutover: %d running task(s), %d remote run(s)" % (len(running), len(runs))}


class ClaudeTasks:
    """Collects the tasks in the dashboard's thread (`refresh`); `status()`
    is cheap."""

    def __init__(self, build_log: Path | None, repos: list, task_glob: str = TASK_GLOB, home=None):
        self.build_log = build_log
        self.task_glob = task_glob
        self.live = Liveness()
        self.worktrees = Worktrees(repos, home)
        self.lock = threading.Lock()
        self.tasks: list = []
        self.wt: list = []
        self.at = None

    def refresh(self, now: float | None = None):
        now = time.time() if now is None else now
        tasks = task_states(read_events(self.build_log, now))
        files = task_files(self.task_glob)
        wts = [dict(w, task=None) for w in self.worktrees.get(now)]
        by_branch = {w["branch"]: w for w in wts if w.get("branch")}
        for t in tasks:
            if t["running"] or now - t["last_event_t"] < RECENT_S:
                t.update(self.live.check(find_task_file(files, t["full_id"], t["id_short"]), now))
            t["running_s"] = (now - t["started"]) if t["running"] and t.get("started") else None
            w = by_branch.get(t["branch"]) or (t["full_id"] and by_branch.get("worktree-agent-" + t["full_id"]))
            if w:
                w["task"] = t["id_short"] or t["key"]
            if w:
                t["worktree"] = {k: w.get(k) for k in ("repo", "path", "sha", "subject", "commit_time", "ahead",
                                                       "merged", "new", "pushed")}
            t.pop("full_id", None)  # the short id is what agents.jsonl shows
        with self.lock:
            self.tasks, self.wt, self.at = tasks, wts, now

    def status(self, machines: list, sessions=None, session_rss=None, now: float | None = None) -> dict:
        now = time.time() if now is None else now
        with self.lock:
            tasks, wts, at = list(self.tasks), list(self.wt), self.at
        shown = [t for t in tasks if t["running"] or t["state"] == "interrupted"]
        recent = [t for t in tasks if t["state"] in DONE_EVENTS and now - t["last_event_t"] < RECENT_S]
        runs = remote_runs(machines)
        # The page lists the worktrees that matter: main, unmerged branches
        # with commits, and those of a task; the rest only as counts.
        listed = [w for w in wts if w["main"] or w.get("task") or (w.get("branch") and not w.get("merged")
                                                                  and w.get("new", True))]
        counts = {"total": len(wts), "merged": sum(1 for w in wts if w.get("merged")),
                  "unmerged": sum(1 for w in wts if w.get("branch") and not w["main"] and not w.get("merged")),
                  "detached": sum(1 for w in wts if w.get("detached"))}
        return {"at": at, "running": shown, "recent": recent, "worktrees": listed, "worktree_counts": counts,
                "claude_sessions": sessions, "claude_session_rss": session_rss or [],
                "remote_runs": runs, "cutover": cutover(tasks, runs, sessions)}


def text_lines(st: dict, now: float | None = None) -> list:
    now = time.time() if now is None else now
    if not st:
        return []
    c = st["cutover"]
    out = ["Claude tasks: %s" % c["text"]]
    for i in c["items"]:
        out.append("  still running: %s" % i)
    if st.get("claude_sessions") is not None:
        out.append("  Claude Code sessions on the coordinator: %s" % st["claude_sessions"])
    for t in st.get("running") or []:
        w = t.get("worktree") or {}
        act = t.get("activity") or "?"
        if t.get("age_s") is not None:
            act += " (last activity %dm ago)" % (t["age_s"] // 60)
        out.append("  [%s] %s – %s, %s, %s; branch %s%s%s" % (
            t["id_short"] or t["key"], t["task"][:60], t["state"],
            ("running %dh %02dm" % (t["running_s"] // 3600, t["running_s"] % 3600 // 60)) if t.get("running_s") else
            "last event %s" % t["last_event"], act, t["branch"],
            (", %s commit(s) not in main" % w["ahead"]) if w.get("ahead") is not None else "",
            (", %s, %s" % ("merged" if w.get("merged") else "not merged", w.get("pushed"))) if w else ""))
    if st.get("recent"):
        out.append("  finished in the last 24 h: " + ", ".join(
            "%s %s" % (t["id_short"] or t["key"], t["state"]) for t in st["recent"]))
    wc = st.get("worktree_counts") or {}
    if wc:
        out.append("  worktrees: %d (%d branches merged into main, %d not merged, %d detached)" % (
            wc["total"], wc["merged"], wc["unmerged"], wc["detached"]))
    return out + [""]
