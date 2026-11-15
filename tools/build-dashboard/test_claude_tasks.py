"""Tests of the Claude tasks section: the agents.jsonl state machine, the
liveness from the task files' mtime and size (fake files in a temp
directory, never opened), worktree porcelain parsing, the branch state,
and the cutover check.

    python3 -m unittest discover tools/build-dashboard
"""

import builtins
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import claude_tasks as ct  # noqa: E402

A = "aa822ba7b91d3a262"
B = "a3b9cf0b40cbf0ccc"
C = "a61e55158bce4740b"


def ev(time_, short, task, branch, event, commits="", note=""):
    return {"time": time_, "agent_id_short": short, "task": task, "branch": branch, "event": event,
            "commits": commits, "note": note}


EVENTS = [
    ev("2026-11-15T00:01:00+01:00", A[:8], "wave 2", "worktree-agent-" + A, "started"),
    ev("2026-11-15T00:39:00+01:00", A[:8], "wave 2", "worktree-agent-" + A, "interrupted", note="00:39 OOM"),
    ev("2026-11-15T08:04:00+01:00", A[:8], "Realistic vegetation and crops", "worktree-agent-" + A, "resumed"),
    ev("2026-11-15T06:15:00+01:00", B[:8], "VR X1", "worktree-agent-" + B, "started"),
    ev("2026-11-15T12:24:00+01:00", B[:8], "VR X1", "worktree-agent-" + B, "reported", commits="866a5a2"),
    ev("2026-11-15T07:55:00+01:00", C[:8], "Sign-in", "worktree-agent-" + C, "resumed"),
    ev("2026-11-15T12:18:00+01:00", C[:8], "Sign-in", "worktree-agent-" + C, "merged", commits="e2b4bda"),
    ev("2026-11-15T09:00:00+01:00", "abcd1234", "Old task", "worktree-agent-abcd1234abcd1234a", "interrupted"),
    # The fields of the coordinator's newest lines: the task holds the agent id.
    ev("2026-11-15T14:11:23+01:00", "dashboar", "a2c89e06798607f0d", "dashboard-deb", "started"),
]


def write_log(root: Path, events, day="2026-11-15"):
    d = root / day
    d.mkdir(parents=True, exist_ok=True)
    with open(d / "agents.jsonl", "w") as f:
        for e in events:
            f.write(json.dumps(e) + "\n")
        f.write("{not json\n")


def epoch(s):
    return ct._epoch(s)


NOW = epoch("2026-11-15T14:30:00+01:00")


class StateMachine(unittest.TestCase):
    def tasks(self):
        return {t["key"]: t for t in ct.task_states([ct.normalize_event(e) for e in EVENTS])}

    def test_last_event_decides(self):
        t = self.tasks()
        self.assertEqual(t[A[:8]]["state"], "running")         # started, interrupted, resumed
        self.assertTrue(t[A[:8]]["running"])
        self.assertEqual(t[B[:8]]["state"], "reported")
        self.assertEqual(t[C[:8]]["state"], "merged")
        self.assertEqual(t["abcd1234"]["state"], "interrupted")
        self.assertFalse(t["abcd1234"]["running"])

    def test_start_time_title_commits(self):
        t = self.tasks()[A[:8]]
        self.assertEqual(t["started"], epoch("2026-11-15T00:01:00+01:00"))  # the start, not the resume
        self.assertEqual(t["task"], "Realistic vegetation and crops")       # the newest title
        self.assertEqual(t["last_event"], "resumed")
        self.assertEqual(t["full_id"], A)
        self.assertEqual(self.tasks()[B[:8]]["commits"], "866a5a2")
        # Only resumed (started before the window): the first event is the start.
        self.assertEqual(self.tasks()[C[:8]]["started"], epoch("2026-11-15T07:55:00+01:00"))

    def test_swapped_fields(self):
        t = self.tasks()["a2c89e06"]
        self.assertEqual(t["full_id"], "a2c89e06798607f0d")
        self.assertEqual(t["id_short"], "a2c89e06")
        self.assertEqual(t["task"], "dashboard-deb")
        self.assertEqual(t["state"], "running")

    def test_bad_lines(self):
        self.assertIsNone(ct.normalize_event({"event": "started"}))  # no time
        self.assertIsNone(ct.normalize_event("x"))

    def test_read_events_from_the_day_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_log(Path(tmp), EVENTS[:3], "2026-11-14")
            write_log(Path(tmp), EVENTS[3:], "2026-11-15")
            evs = ct.read_events(Path(tmp), NOW)
            self.assertEqual(len(evs), len(EVENTS))
            self.assertEqual(evs, sorted(evs, key=lambda e: e["t"]))
            self.assertEqual(ct.read_events(None, NOW), [])


class LivenessFromMtime(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name) / "claude-1000" / "-proj" / "session-1" / "tasks"
        base.mkdir(parents=True)
        self.glob = str(Path(self.tmp.name) / "claude-*" / "*" / "*" / "tasks")
        self.files = {}
        for tid, age in ((A, 60), (B, 3600)):
            p = base / (tid + ".output")
            p.write_text("TRANSCRIPT-SECRET prompt and tool output\n")
            os.utime(p, (NOW - age, NOW - age))
            self.files[tid] = p
        (base / "notes.txt").write_text("x")

    def tearDown(self):
        self.tmp.cleanup()

    def test_listing_by_name_and_prefix(self):
        files = ct.task_files(self.glob)
        self.assertEqual(sorted(files), sorted([A, B]))
        self.assertEqual(ct.find_task_file(files, A, A[:8]), str(self.files[A]))
        self.assertEqual(ct.find_task_file(files, None, B[:8]), str(self.files[B]))
        self.assertIsNone(ct.find_task_file(files, None, "zz"))
        self.assertIsNone(ct.find_task_file(files, "a0000000000000000", "a0000000"))

    def test_active_growing_idle_unknown(self):
        live = ct.Liveness()
        p = str(self.files[A])
        r = live.check(p, NOW)
        self.assertEqual(r["activity"], "active")
        self.assertAlmostEqual(r["age_s"], 60, delta=1)
        with open(p, "a") as f:   # the test writes; the code only stats
            f.write("more\n")
        os.utime(p, (NOW - 5, NOW - 5))
        self.assertEqual(live.check(p, NOW)["activity"], "growing")
        self.assertEqual(live.check(str(self.files[B]), NOW)["activity"], "idle / waiting")  # 60 min > 20 min
        self.assertEqual(live.check(None, NOW)["activity"], "unknown")
        self.assertEqual(live.check(p + ".gone", NOW)["activity"], "unknown")

    def test_transcripts_are_never_opened(self):
        opened = []
        real = builtins.open

        def spy(path, *a, **k):
            opened.append(str(path))
            return real(path, *a, **k)

        with tempfile.TemporaryDirectory() as logs:
            write_log(Path(logs), EVENTS)
            tasks = ct.ClaudeTasks(Path(logs), [], task_glob=self.glob, home=self.tmp.name)
            for p in self.files.values():
                os.chmod(p, 0)  # unreadable: a read would fail; stat still works
            builtins.open = spy
            try:
                tasks.refresh(NOW)
                st = tasks.status([], 1, [1000], now=NOW)
            finally:
                builtins.open = real
                for p in self.files.values():
                    os.chmod(p, 0o600)
        self.assertFalse([p for p in opened if p.endswith(".output")], opened)
        text = json.dumps(st)
        self.assertNotIn("TRANSCRIPT-SECRET", text)
        self.assertNotIn("session-1", text)   # no session ids
        run = {t["key"]: t for t in st["running"]}
        self.assertEqual(run[A[:8]]["activity"], "active")
        self.assertEqual(run["a2c89e06"]["activity"], "unknown")  # no task file
        self.assertIn("abcd1234", run)          # interrupted ones are listed with the running
        self.assertEqual(sorted(t["key"] for t in st["recent"]), [B[:8], C[:8]])
        self.assertNotIn("full_id", text)


PORCELAIN = """worktree /home/u/data/git/OCW
HEAD 9df5de0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
branch refs/heads/main

worktree /home/u/data/git/OCW/.claude/worktrees/agent-aa822ba7b91d3a262
HEAD 866a5a2bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
branch refs/heads/worktree-agent-aa822ba7b91d3a262
locked

worktree /tmp/claude-1000/-proj/e8b4736e-3828-4ebd/scratchpad/wt
HEAD b57f03cccccccccccccccccccccccccccccccccc
detached

worktree /home/u/data/git/OCW/.claude/worktrees/agent-gone
HEAD 1111111ddddddddddddddddddddddddddddddddd
branch refs/heads/worktree-agent-gone
prunable gitdir file points to non-existent location
"""


class WorktreeParsing(unittest.TestCase):
    def test_porcelain(self):
        w = ct.parse_worktree_porcelain(PORCELAIN)
        self.assertEqual(len(w), 4)
        self.assertEqual(w[0]["branch"], "main")
        self.assertTrue(w[1]["locked"])
        self.assertTrue(w[2]["detached"])
        self.assertIsNone(w[2]["branch"])
        self.assertTrue(w[3]["prunable"])
        self.assertEqual(ct.parse_worktree_porcelain(""), [])

    def test_refs_and_push_state(self):
        refs = ct.parse_refs("refs/heads/main\x00abc\x001794748244\x00Subject with spaces\nbroken line\n")
        self.assertEqual(refs["refs/heads/main"], {"sha": "abc", "time": 1794748244, "subject": "Subject with spaces"})
        self.assertEqual(ct.push_state("abc", "abc"), "pushed")
        self.assertEqual(ct.push_state("abc", None), "not pushed")
        self.assertEqual(ct.push_state("abc", "def"), "differs from origin")

    def test_safe_path_hides_session_dirs(self):
        self.assertEqual(ct.safe_path("/tmp/claude-1000/-proj/e8b4736e-3828/scratchpad/wt", "/home/u"),
                         "/tmp/claude-…/scratchpad/wt")
        self.assertEqual(ct.safe_path("/home/u/data/git/OCW", "/home/u"), "~/data/git/OCW")

    def test_branch_state_with_a_fake_git(self):
        A_BR = "worktree-agent-" + A
        refs = ("refs/heads/main\x00" + "9" * 40 + "\x001794748244\x00Main subject\n"
                "refs/heads/" + A_BR + "\x00" + "8" * 40 + "\x001794700000\x00Vegetation\n"
                "refs/heads/worktree-agent-gone\x00" + "9" * 40 + "\x001794748244\x00Main subject\n"
                "refs/remotes/origin/main\x00" + "9" * 40 + "\x001794748244\x00Main subject\n")
        calls = []

        def git(repo, *args, timeout=15):
            calls.append(args)
            if args[:2] == ("worktree", "list"):
                return PORCELAIN
            if args[0] == "for-each-ref":
                return refs
            if args[:2] == ("branch", "--merged"):
                return "main\nworktree-agent-gone\n"
            if args[0] == "rev-list":
                return "3\n" if args[2].endswith(A_BR) else "0\n"
            return None

        wt = ct.Worktrees([("OCW", "/repo")], home="/home/u", git=git)
        items = {(w["branch"] or "detached"): w for w in wt.refresh(NOW)}
        self.assertEqual(items["main"]["pushed"], "pushed")
        a = items[A_BR]
        self.assertEqual((a["ahead"], a["merged"], a["pushed"], a["sha"]), (3, False, "not pushed", "8888888"))
        self.assertTrue(a["new"])
        self.assertFalse(items["worktree-agent-gone"]["new"])  # still main's commit
        self.assertTrue(items["worktree-agent-gone"]["merged"])
        self.assertEqual(items["detached"]["path"], "/tmp/claude-…/scratchpad/wt")
        n = len(calls)
        wt.get(NOW + 10)                       # throttled: no git
        self.assertEqual(len(calls), n)
        wt.refresh(NOW + 200)                  # the commit counts are cached
        self.assertEqual(sum(1 for c in calls if c[0] == "rev-list"), 2)


class Cutover(unittest.TestCase):
    MACHINES = [
        {"name": "marge", "online": True, "tasks": [{"state": "running", "checkout": "main", "command": "make check"}],
         "queue": {"queued": [{"checkout": "agent a1", "command": "cargo test"}]}},
        {"name": "macos", "online": False, "tasks": [{"state": "running", "checkout": "x"}]},
    ]

    def test_remote_runs_from_the_queue(self):
        runs = ct.remote_runs(self.MACHINES)
        self.assertEqual([(r["machine"], r["state"]) for r in runs], [("marge", "running"), ("marge", "waiting")])

    def test_ok_and_not_ok(self):
        ok = ct.cutover([{"running": False}], [], 1)
        self.assertTrue(ok["ok"])
        self.assertEqual(ok["text"], "Cutover OK: no running tasks, no remote runs")
        no = ct.cutover([{"running": True, "id_short": "aa822ba7", "key": "aa822ba7", "task": "Veg",
                          "activity": "active"}], ct.remote_runs(self.MACHINES), 1)
        self.assertFalse(no["ok"])
        self.assertEqual(len(no["items"]), 3)
        self.assertIn("aa822ba7", no["items"][0])
        self.assertIn("marge", no["items"][1])
        self.assertIn("1 running task(s), 2 remote run(s)", no["text"])

    def test_text(self):
        with tempfile.TemporaryDirectory() as logs:
            write_log(Path(logs), EVENTS)
            tasks = ct.ClaudeTasks(Path(logs), [], task_glob=str(Path(logs) / "none"), home=logs)
            tasks.refresh(NOW)
            lines = ct.text_lines(tasks.status(self.MACHINES, 2, now=NOW), NOW)
        self.assertTrue(lines[0].startswith("Claude tasks: Not ready for the cutover"))
        self.assertTrue(any("still running: remote run on marge" in x for x in lines))


if __name__ == "__main__":
    unittest.main()
