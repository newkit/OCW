"""The Build Diary's features log (art/ART-PROJECT.md §2.7): pure functions,
no git, gh or files here (art/diary.py feeds them).

- spec rows added on a day (from `git log -p` of SPECIFICATION.md), their
  titles, and the commit (merge) that landed each one;
- the check run of a merge in the build log, with its test results;
- the agent jobs log (agents.jsonl events → one row per job);
- the closing "Today's features" roll of the day video (pages of lines).
"""

from __future__ import annotations

import datetime as dt
import re

ROW_RE = re.compile(r"^\| (\d+\.\d+) \| (.*?) \|\s*$")
ADDED_ROW_RE = re.compile(r"^\+\| (\d+\.\d+) \| (.*?) \|\s*$")
COMMIT_MARK = "@@COMMIT"
TEST_RESULT_RE = re.compile(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed")
CHECK_CMD_RE = re.compile(r"cargo test|make (check|test)\b")
EVENTS = ("started", "reported", "interrupted", "resumed", "merged", "abandoned")


def when(iso: str) -> dt.datetime | None:
    try:
        t = dt.datetime.fromisoformat(str(iso).replace("Z", "+00:00"))
    except ValueError:
        return None
    return t if t.tzinfo else t.astimezone()


# ------------------------------------------------------------------ spec rows

def spec_rows(text: str) -> dict:
    """{version: text} of the version table of a SPECIFICATION.md."""
    out = {}
    for line in text.splitlines():
        m = ROW_RE.match(line)
        if m and m.group(1) not in out:
            out[m.group(1)] = m.group(2)
    return out


def spec_rows_added(diff: str) -> list[tuple[str, str]]:
    """(version, text) of every version row a diff adds."""
    return [(m.group(1), m.group(2)) for line in diff.splitlines() for m in [ADDED_ROW_RE.match(line)] if m]


def row_title(text: str, n: int = 90) -> str:
    """A row's title: its first clause, without parentheses and bold marks
    ("Phase 13 **G5's browser part built** (§14 …): …" → "Phase 13 G5's
    browser part built")."""
    t = text
    for _ in range(3):  # nested parentheses
        t = re.sub(r"\([^()]*\)", "", t)
    t = t.replace("**", "").replace("`", "")
    t = re.split(r":\s| – | — |;\s|\.\s", t, maxsplit=1)[0]
    t = " ".join(t.split()).strip(" ,.")
    return t if len(t) <= n else t[: n - 1].rstrip() + "…"


def parse_spec_log(log: str) -> list[dict]:
    """Commits from `git log --first-parent --diff-merges=first-parent -p
    --format=@@COMMIT%x1f%H%x1f%h%x1f%aI%x1f%P%x1f%s -- SPECIFICATION.md`."""
    out, cur, body = [], None, []
    for line in log.splitlines():
        if line.startswith(COMMIT_MARK):
            if cur:
                cur["rows"] = spec_rows_added("\n".join(body))
                out.append(cur)
            parts = line[len(COMMIT_MARK):].lstrip("\x1f").split("\x1f")
            full, short, iso, parents, subject = (parts + [""] * 5)[:5]
            cur, body = {"sha": full, "short": short, "time": iso, "parents": parents.split(),
                         "subject": subject}, []
        elif cur is not None:
            body.append(line)
    if cur:
        cur["rows"] = spec_rows_added("\n".join(body))
        out.append(cur)
    return out


def features_of(commits: list[dict], start_rows: dict, end_rows: dict) -> list[dict]:
    """The day's features: rows in the end-of-day spec that were not there at
    its start, each with the commit that landed it (a merge whose subject
    names "spec X.Y" first, else the first commit adding that row)."""
    out = []
    new = [v for v in end_rows if v not in start_rows]
    for v in sorted(new, key=lambda s: tuple(int(x) for x in s.split("."))):
        text = end_rows[v]
        named = [c for c in commits if re.search(rf"\bspec {re.escape(v)}\b", c["subject"], re.I)
                 and len(c["parents"]) > 1]
        same = [c for c in commits if any(rv == v and rt[:50] == text[:50] for rv, rt in c["rows"])]
        anyv = [c for c in commits if any(rv == v for rv, _ in c["rows"])]
        # The commit that added the final row; a merge naming the version in
        # its subject wins among those, and only names a lander by itself
        # when no diff shows the row (e.g. a renumbered row).
        c = ([x for x in same if x in named] or same or [x for x in anyv if x in named] or anyv or named
             or [None])[0]
        out.append({"version": v, "title": row_title(text), "commit": c["short"] if c else None,
                    "sha": c["sha"] if c else None, "time": c["time"] if c else None,
                    "merge": bool(c and len(c["parents"]) > 1), "subject": c["subject"] if c else "",
                    "parents": c["parents"] if c else []})
    return out


# ------------------------------------------------------------------ checks

def parse_test_results(text: str) -> dict:
    """Test suites in a cargo test output ("test result: ok. N passed; M failed")."""
    suites = passed = failed = 0
    bad = 0
    for m in TEST_RESULT_RE.finditer(text or ""):
        suites += 1
        passed += int(m.group(2))
        failed += int(m.group(3))
        bad += m.group(1) == "FAILED"
    return {"suites": suites, "passed": passed, "failed": failed, "failed_suites": bad}


def find_check(runs: list[dict], merge_time: str, branch: str | None, commits: list[str],
               until: str | None = None, machine: str = "marge") -> dict | None:
    """The check run of a merge: the latest test run on `machine` of one of
    `commits` (the merge or its branch tip; runs.jsonl's `commit` since
    2026-11-15), else of the agent `branch` before the merge, else of main
    between the merge and `until` (the next merge)."""
    mt, ut = when(merge_time), when(until) if until else None
    best = None
    for r in runs:
        if r.get("machine") != machine or not CHECK_CMD_RE.search(r.get("command", "")):
            continue
        rt = when(r.get("start", ""))
        if rt is None or mt is None:
            continue
        rc = str(r.get("commit") or "").replace("-dirty", "")
        score = 0
        if rc and any(c and (c.startswith(rc) or rc.startswith(c[:7])) for c in commits):
            score = 3
        elif branch and r.get("branch") == branch and rt <= mt + dt.timedelta(minutes=5):
            score = 2
        elif r.get("branch") == "main" and mt <= rt and (ut is None or rt < ut):
            score = 1
        if score and (best is None or (score, rt) > (best[0], best[1])):
            best = (score, rt, r)
    if not best:
        return None
    r = best[2]
    return {"machine": r.get("machine"), "time": r.get("start"), "exit": r.get("exit"),
            "branch": r.get("branch"), "command": r.get("command", "")[:120], "log": r.get("log"),
            "how": {3: "commit", 2: "branch", 1: "main after merge"}[best[0]]}


# ------------------------------------------------------------------ agent jobs

def agent_id(branch: str, given: str | None = None) -> str:
    if given:
        return given[:8]
    m = re.search(r"agent-([0-9a-f]{6,})", branch or "")
    return m.group(1)[:8] if m else (branch or "?")[:8]


def agent_jobs(events: list[dict]) -> list[dict]:
    """One row per job (agent id, else branch) from agents.jsonl events:
    task, branch, start, end, status (the last event), interruptions,
    merged commit."""
    jobs = {}
    for e in sorted(events, key=lambda e: str(e.get("time", ""))):
        key = e.get("agent_id_short") or e.get("branch") or e.get("task")
        j = jobs.setdefault(key, {"agent": key, "task": e.get("task", ""), "branch": e.get("branch", ""),
                                  "start": None, "end": None, "status": None, "interruptions": 0,
                                  "merged": None, "events": 0})
        j["events"] += 1
        j["task"] = j["task"] or e.get("task", "")
        ev = e.get("event")
        if ev == "started" and not j["start"]:
            j["start"] = e.get("time")
        if ev in ("reported", "merged", "abandoned"):
            j["end"] = e.get("time")
        if ev == "interrupted":
            j["interruptions"] += 1
        if ev == "merged":
            j["merged"] = e.get("commits") or j["merged"]
        if not j["start"]:
            j["start"] = e.get("time")
        j["status"] = ev
    return sorted(jobs.values(), key=lambda j: str(j["start"]))


# ------------------------------------------------------------------ the roll

def roll_pages(features: list[dict], per_page: int = 6, width: int = 62) -> list[list[str]]:
    """The "Today's features" roll: pages of `per_page` lines, each
    "3.39  Title" cut to `width` characters; merged rows only, in order."""
    lines = []
    for f in features:
        if not f.get("commit"):
            continue
        t = f"{f['version']}  {f['title']}"
        lines.append(t if len(t) <= width else t[: width - 1].rstrip() + "…")
    return [lines[i:i + per_page] for i in range(0, len(lines), per_page)]
