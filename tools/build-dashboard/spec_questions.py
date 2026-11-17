"""Open questions for the user, read from SPECIFICATION.md (spec §11.2).

The spec stays the single source of truth: this module only parses it. A
question is a bullet of the form

    - **Q3 – Short title:** the question text … (proposal) …
    - **Q2 – Commercial use: decided (2026-11-13):** the answer …

anywhere in the spec (mostly the "Open questions (for the user)"
subsections of the §14 phases). Its phase comes from the nearest
"### Phase N – …" heading above it, its section from the nearest heading
("#### 4L – Open questions (for the user)" gives "4L"). A question is
decided when its bold label says "decided" or "postponed"; everything else
(also "research") is open.

Milestones that name an open question (e.g. "W6 – Later: … (if Phase 12 Q6
says so)") are "blocked work".

Pure: `parse_questions(text)` and `blocked_work(text, questions)` take the
spec's text; `SpecQuestions` re-reads the file when its mtime changes.
"""

from __future__ import annotations

import os
import re
import threading
from pathlib import Path

_HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
_PHASE = re.compile(r"^Phase\s+(\d+)\s*[–-]\s*(.*)$")
_SECTION = re.compile(r"^(\d+[A-Z]|\d+(?:\.\d+)+|\d+(?=\.\s)|[A-Z]\.\d+)\b")
# "- **Q1 – Title: decided (…):** text" (the label may end with "?" or ":"
# inside the bold part, or the colon may follow it).
# "Q30 – Title:", also "Q30 (open) – Title:" (a status in brackets before the
# dash is moved into the label so _split_label sees it).
_QUESTION = re.compile(r"^\s*[-*]\s+\*\*Q(\d+)\s*(\([^)]*\))?\s*[–-]\s*(.*?)\*\*\s*:?\s*(.*)$")
_DECIDED = re.compile(r"\b(decided|postponed)\b", re.I)
_DATE = re.compile(r"\((?:v[\d.]+|\d{4}-\d{2}-\d{2})[^)]*\)")
_MILESTONE = re.compile(r"^\s*[-*]\s+\*\*([A-Z]\d+)\b[^*]*\*\*\s*(.*)$")
_QREF = re.compile(r"(?:Phase\s+(\d+)\s+)?\bQ(\d+)\b")


def plain(md: str) -> str:
    """Markdown inline formatting removed: bold, italics, code, links."""
    s = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", md)
    s = s.replace("**", "").replace("`", "")
    s = re.sub(r"(?<!\w)\*(\S[^*]*?)\*(?!\w)", r"\1", s)
    s = re.sub(r"(?<![\w*])_(\S[^_]*?)_(?![\w])", r"\1", s)
    return re.sub(r"\s+", " ", s).strip()


def _split_label(label: str):
    """'Title: decided (2026-11-13):' -> ('Title', 'decided (2026-11-13)')."""
    label = label.strip().rstrip(":").strip()
    m = _DECIDED.search(label)
    if m:
        title = label[:m.start()].rstrip(" :.?–-")
        if label[:m.start()].rstrip().endswith("?"):
            title += "?"
        return title, label[m.start():].strip(" :")
    m = re.search(r"\(([^)]*research[^)]*)\)\s*$", label, re.I)
    if m:
        return label[:m.start()].rstrip(" :"), m.group(1)
    return label, ""


def proposal_of(text: str):
    """The proposed or recommended option of a question, if the text names
    one: "Proposal: X", "(proposal: X)", or the option marked "(proposal)" /
    "(recommended)" / ", proposal)" among the "…, or …" alternatives."""
    t = plain(text)
    m = re.search(r"\b[Pp]roposal:\s*([^)]*?)(?:\)|$)", t)
    if m and m.group(1).strip():
        return m.group(1).strip().rstrip(".?")
    if not re.search(r"\b(proposal|proposed|recommended)\b", t):
        return None
    for p in _alternatives(t):
        if re.search(r"\b(proposal|proposed|recommended)\b", p):
            return _clean_option(p)
    return None


def _alternatives(t: str) -> list:
    """The alternatives of a question, split at ", or " outside parentheses."""
    parts, depth, cur, i = [], 0, "", 0
    while i < len(t):
        c = t[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth = max(0, depth - 1)
        if depth == 0 and t.startswith(", or ", i):
            parts.append(cur)
            cur, i = "", i + 5
            continue
        cur += c
        i += 1
    parts.append(cur)
    return parts


def _clean_option(p: str):
    q = re.sub(r"\s*\((?:proposal|proposed|recommended)\)", "", p)
    q = re.sub(r"[;,]\s*(?:proposal|proposed|recommended)(?=\))", "", q)
    q = re.sub(r"\((?:proposal|recommended);\s*", "(", q)
    q = re.sub(r"\s*\((?:proposal|recommended):[^)]*\)", "", q)
    q = re.sub(r"\s+[–-]\s+.*$", "", q)
    if ":" in q:  # "who controls …: host / organiser only"
        q = q.rsplit(":", 1)[1]
    return q.strip().rstrip(".?,").strip() or None


def options_of(text: str) -> list:
    """The named options of a question ("A (proposal), or B?" -> [A, B]),
    for the dashboard's quick answer buttons: only the question's first
    sentence, 2–4 options of at most 120 characters, else []."""
    t = plain(text)
    m = re.search(r"\?(\s|$)", t)
    if not m:
        return []
    alts = _alternatives(t[:m.start() + 1])
    if not 2 <= len(alts) <= 4:
        return []
    opts = [_clean_option(a) for a in alts]
    if any(not o or len(o) > 120 for o in opts):
        return []
    return opts


def parse_questions(text: str) -> list:
    """Every "Q<n> –" bullet of the spec, in document order: dicts with
    phase (int or None), phase_title, section ("4L"), heading, id
    ("Phase 4 Q1"), n, title, text, proposal, status ("open"/"decided"),
    status_note ("decided (2026-11-13)"), line (1-based)."""
    out = []
    phase = phase_title = None
    section = heading = None
    for no, line in enumerate(text.splitlines(), 1):
        h = _HEADING.match(line)
        if h:
            level, title = len(h.group(1)), plain(h.group(2))
            pm = _PHASE.match(title)
            if pm:
                phase, phase_title = int(pm.group(1)), re.sub(r"\s*\(.*\)\s*$", "", pm.group(2)).strip()
            elif level <= 3:
                phase = phase_title = None
            heading = title
            sm = _SECTION.match(title)
            section = sm.group(1) if sm else None
            continue
        q = _QUESTION.match(line)
        if not q:
            continue
        n = int(q.group(1))
        title, note = _split_label(plain(q.group(3).rstrip().rstrip(":")))
        if q.group(2) and not note:  # "Q30 (open) – …": the bracket is the status
            note = plain(q.group(2)).strip("() ")
            if not _DECIDED.search(note):
                note = ""
        body = plain(q.group(4))
        if not title:  # "**Q1 – decided (v2.48):** an Oculus Rift CV1 …"
            title = re.split(r"[:(;,.]", body, maxsplit=1)[0].strip()[:60]
        decided = bool(_DECIDED.search(note))
        where = ("Phase %d" % phase) if phase is not None else (("§" + section) if section else (heading or "Spec"))
        out.append({
            "phase": phase, "phase_title": phase_title, "section": section, "heading": heading,
            "id": "%s Q%d" % (where, n), "n": n, "title": title, "text": body,
            "proposal": None if decided else proposal_of(q.group(4)),
            "options": [] if decided else options_of(q.group(4)),
            "status": "decided" if decided else "open", "status_note": note, "line": no,
        })
    return out


def blocked_work(text: str, questions: list) -> list:
    """Milestones (bullets "**X1 – …**" under a heading with "milestones")
    that reference an open question: "Q4" (of their own phase) or
    "Phase 12 Q6". Dicts: phase, milestone, title, waits_for [ids], line."""
    by_id = {q["id"]: q for q in questions}
    out = []
    phase = None
    in_ms = False
    for no, line in enumerate(text.splitlines(), 1):
        h = _HEADING.match(line)
        if h:
            title = plain(h.group(2))
            pm = _PHASE.match(title)
            if pm:
                phase = int(pm.group(1))
            elif len(h.group(1)) <= 3:
                phase = None
            in_ms = "milestone" in title.lower()
            continue
        if not in_ms and "**Milestones" not in line:
            continue
        if "**Milestones" in line:
            in_ms = True
        m = _MILESTONE.match(line)
        if not m:
            continue
        refs = []
        for rm in _QREF.finditer(line):
            ph = int(rm.group(1)) if rm.group(1) else phase
            if ph is None:
                continue
            qid = "Phase %d Q%s" % (ph, rm.group(2))
            q = by_id.get(qid)
            if q and q["status"] == "open" and qid not in refs:
                refs.append(qid)
        if refs:
            label = plain(line.strip().lstrip("-* "))
            out.append({"phase": phase, "milestone": m.group(1), "title": label[:200],
                        "waits_for": refs, "line": no})
    return out


def summary(questions: list, blocked=()) -> dict:
    """Counts and the questions grouped by phase, for the dashboard."""
    groups: dict = {}
    order = []
    for q in questions:
        key = q["id"].rsplit(" Q", 1)[0]
        if key not in groups:
            groups[key] = {"group": key, "title": q.get("phase_title") or q.get("heading") or "",
                           "section": q.get("section"), "open": 0, "decided": 0, "items": []}
            order.append(key)
        g = groups[key]
        g[q["status"]] += 1
        g["items"].append(q)
    return {"open": sum(1 for q in questions if q["status"] == "open"),
            "decided": sum(1 for q in questions if q["status"] == "decided"),
            "groups": [groups[k] for k in order], "blocked": list(blocked)}


class SpecQuestions:
    """The questions of a spec file, re-parsed when its mtime changes."""

    def __init__(self, path: Path | None):
        self.path = Path(path) if path else None
        self.mtime = None
        self.data = {"open": 0, "decided": 0, "groups": [], "blocked": [], "error": "not loaded"}
        self.lock = threading.Lock()

    def get(self) -> dict:
        with self.lock:
            if self.path is None:
                return dict(self.data, error="no spec file")
            try:
                mt = os.stat(self.path).st_mtime
            except OSError as e:
                return dict(self.data, error="%s: %s" % (self.path.name, e.strerror))
            if mt != self.mtime:
                try:
                    text = self.path.read_text(encoding="utf-8")
                    qs = parse_questions(text)
                    self.data = dict(summary(qs, blocked_work(text, qs)), source=self.path.name, mtime=mt)
                    m = re.search(r"^Version\s+([\d.]+)", text, re.M)
                    self.data["spec_version"] = m.group(1) if m else None
                    self.mtime = mt
                except Exception as e:  # a parse problem must not stop the dashboard
                    self.data = dict(self.data, error="%s: %s" % (type(e).__name__, e))
            return self.data


# --- Keeping the question list current (periodic job, dashboard 1.6.0) -----

# A bullet that starts like a question ("- **Q12" …) but that parse_questions
# does not read, and a question whose decision is written outside its bold
# label (the dashboard only reads "decided" in the label).
_LOOKS_LIKE_Q = re.compile(r"^\s*[-*]\s+\*\*Q\d+")
_DECIDED_IN_BODY = re.compile(r"^\W*(decided|postponed)\b", re.I)


def lint(text: str) -> list:
    """Format problems of the spec's question bullets: dicts with line, kind
    ("unparsed" / "decision outside the label") and the line's start."""
    out = []
    parsed = {q["line"]: q for q in parse_questions(text)}
    for no, line in enumerate(text.splitlines(), 1):
        if _LOOKS_LIKE_Q.match(line) and no not in parsed:
            out.append({"line": no, "kind": "unparsed", "text": line.strip()[:120]})
        q = parsed.get(no)
        if q and q["status"] == "open" and _DECIDED_IN_BODY.search(plain(q["text"]) or ""):
            out.append({"line": no, "kind": "decision outside the label", "text": line.strip()[:120]})
    return out


def _git(repo: Path, *args: str) -> tuple:
    import subprocess
    p = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip()


def refresh(repo: Path, report: Path | None = None) -> dict:
    """Fast-forward the main checkout to origin/main when it is on main and has
    no changes of tracked files (the dashboard re-reads the spec by its mtime),
    then lint the spec's questions; writes the result as JSON to `report`."""
    import json
    import time as _time
    res = {"time": _time.strftime("%Y-%m-%dT%H:%M:%S%z"), "pulled": False, "note": ""}
    rc, branch = _git(repo, "rev-parse", "--abbrev-ref", "HEAD")
    rc2, dirty = _git(repo, "status", "--porcelain", "--untracked-files=no")
    if rc or branch != "main":
        res["note"] = "not on main (%s): not pulled" % branch
    elif rc2 or dirty:
        res["note"] = "tracked changes in the checkout: not pulled"
    else:
        rc, out = _git(repo, "fetch", "-q", "origin", "main")
        if rc == 0:
            rc, out = _git(repo, "merge", "-q", "--ff-only", "origin/main")
            res["pulled"] = rc == 0
            res["note"] = "up to date" if rc == 0 else "fast-forward failed: " + out[:200]
        else:
            res["note"] = "fetch failed: " + out[:200]
    spec = repo / "SPECIFICATION.md"
    text = spec.read_text(encoding="utf-8")
    qs = parse_questions(text)
    res.update(open=sum(q["status"] == "open" for q in qs), decided=sum(q["status"] == "decided" for q in qs),
               problems=lint(text))
    if report:
        report.parent.mkdir(parents=True, exist_ok=True)
        tmp = report.with_suffix(".tmp")
        tmp.write_text(json.dumps(res, indent=1), encoding="utf-8")
        tmp.replace(report)
    return res


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(description="Open questions of SPECIFICATION.md (the build dashboard's question list)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("refresh", help="fast-forward the main checkout and lint the questions (the periodic job)")
    r.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    r.add_argument("--report", type=Path, help="JSON report (default: REPO/test-output/answers/questions-refresh.json)")
    sub.add_parser("lint", help="list question bullets the dashboard cannot read").add_argument("spec", type=Path)
    a = ap.parse_args()
    if a.cmd == "refresh":
        out = refresh(a.repo, a.report or a.repo / "test-output" / "answers" / "questions-refresh.json")
        print("%s; %d open, %d decided, %d format problems" % (out["note"], out["open"], out["decided"], len(out["problems"])))
        for p in out["problems"]:
            print("  line %d: %s: %s" % (p["line"], p["kind"], p["text"]))
    else:
        probs = lint(a.spec.read_text(encoding="utf-8"))
        for p in probs:
            print("line %d: %s: %s" % (p["line"], p["kind"], p["text"]))
        raise SystemExit(1 if probs else 0)
