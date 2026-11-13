"""Answers to the spec's open questions, given in the build dashboard
(spec §11.2, v2.63).

The user answers a question on the dashboard page; the answer is appended
to test-output/answers/answers.jsonl of the main checkout (git-ignored,
append-only: a later line for the same question supersedes the earlier
one) and as one line to test-output/answers/inbox.log, which the
coordinator watches (`tail -f`) to record the decision in SPECIFICATION.md.
The spec stays the source of truth: once it marks the question decided, the
dashboard shows it as decided.

Writing needs the token in config/dashboard-token (git-ignored, created on
first start with mode 0600), sent as the X-OCW-Token header; the token is
never logged or served.
"""

from __future__ import annotations

import hmac
import json
import os
import re
import secrets
import threading
import time
from pathlib import Path

TOKEN_HEADER = "X-OCW-Token"
MAX_ANSWER = 4000
_CTRL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f]")


def load_token(path: Path) -> str:
    """The dashboard's write token; created (random, 0600) if missing."""
    path = Path(path)
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        tok = secrets.token_urlsafe(32)
        try:
            fd = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(tok + "\n")
        except FileExistsError:  # created meanwhile by another start
            pass
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    tok = path.read_text(encoding="utf-8").strip()
    if len(tok) < 16:
        raise ValueError("%s: token too short" % path.name)
    return tok


def token_ok(expected: str, given) -> bool:
    return bool(expected) and isinstance(given, str) and hmac.compare_digest(expected.encode(), given.encode())


def clean_answer(text) -> str:
    """Answer text: control characters (except newline and tab) removed,
    at most MAX_ANSWER characters, trimmed."""
    if not isinstance(text, str):
        raise ValueError("answer must be text")
    s = _CTRL.sub("", text.replace("\r\n", "\n").replace("\r", "\n")).strip()
    if not s:
        raise ValueError("empty answer")
    if len(s) > MAX_ANSWER:
        raise ValueError("answer longer than %d characters" % MAX_ANSWER)
    return s


def iso(t: float) -> str:
    lt = time.localtime(t)
    off = time.strftime("%z", lt)
    return time.strftime("%Y-%m-%dT%H:%M:%S", lt) + (off[:3] + ":" + off[3:] if off else "")


class AnswerStore:
    """answers.jsonl + inbox.log in `root` (test-output/answers)."""

    def __init__(self, root: Path, clock=time.time):
        self.root = Path(root)
        self.clock = clock
        self.lock = threading.Lock()

    @property
    def path(self) -> Path:
        return self.root / "answers.jsonl"

    @property
    def inbox(self) -> Path:
        return self.root / "inbox.log"

    def all(self) -> list:
        try:
            lines = self.path.read_text(encoding="utf-8").splitlines()
        except OSError:
            return []
        out = []
        for l in lines:
            try:
                e = json.loads(l)
            except ValueError:
                continue
            if isinstance(e, dict) and e.get("id"):
                out.append(e)
        return out

    def latest(self) -> dict:
        """question id -> its latest answer (later lines supersede)."""
        out = {}
        for e in self.all():
            out[e["id"]] = e
        return out

    def since(self, ts: float) -> list:
        return [e for e in self.all() if (e.get("epoch") or 0) > ts]

    def add(self, question: dict, answer: str, choice=None, spec_version=None) -> dict:
        """Appends an answer to `question` (a parsed spec question: its id and
        exact text are taken from the spec, not from the client)."""
        ans = clean_answer(answer)
        if choice is not None:
            choice = clean_answer(str(choice))[:300]
        with self.lock:
            now = self.clock()
            prev = self.latest().get(question["id"])
            e = {"time": iso(now), "epoch": round(now, 3), "id": question["id"], "title": question.get("title"),
                 "question": question.get("text"), "answer": ans, "choice": choice,
                 "status_in_spec": question.get("status"), "spec_version": spec_version,
                 "supersedes": prev["time"] if prev else None}
            self.root.mkdir(parents=True, exist_ok=True)
            with open(self.path, "a", encoding="utf-8") as f:
                f.write(json.dumps(e, ensure_ascii=False, sort_keys=True) + "\n")
            line = "%s %s%s: %s%s" % (e["time"], e["id"], (" (" + e["title"] + ")") if e.get("title") else "",
                                      ans.replace("\n", " / "), " [replaces the answer of %s]" % prev["time"] if prev else "")
            with open(self.inbox, "a", encoding="utf-8") as f:
                f.write(line + "\n")
            return e


def annotate(questions: dict, latest: dict) -> dict:
    """Adds "answer" (the latest) and "answer_state" to every question of a
    spec_questions summary: "answered" (open in the spec, answered here,
    waiting for the coordinator), "decided" (the spec says so), or "open"."""
    out = dict(questions)
    groups = []
    answered = 0
    for g in questions.get("groups") or []:
        g = dict(g)
        items = []
        for it in g.get("items") or []:
            it = dict(it)
            a = latest.get(it["id"])
            if a:
                it["answer"] = {k: a.get(k) for k in ("time", "epoch", "answer", "choice")}
            if it["status"] == "decided":
                it["answer_state"] = "decided"
            elif a:
                it["answer_state"] = "answered"
                answered += 1
            else:
                it["answer_state"] = "open"
            items.append(it)
        g["items"] = items
        groups.append(g)
    out["groups"] = groups
    out["answered"] = answered
    return out
