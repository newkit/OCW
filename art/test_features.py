"""Unit tests of the features log (art/features.py, ART-PROJECT §2.7) and its
diary.py glue: spec-row parsing from fixture diffs, merge matching, the
check-run lookup on a fixture build log, agent-event appending and masking,
the roll layout. No live git history, gh or journal.

    /usr/bin/python3 -m unittest discover -s art -p 'test_*.py'   (make test-art)
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import features as F  # noqa: E402
from test_diary import Fixture  # noqa: E402

import diary  # noqa: E402

SPEC_LOG = """@@COMMIT\x1faaaa1111\x1faaaa111\x1f2026-11-15T00:00:10+01:00\x1fp0\x1fSpec 3.36: public preview
diff --git a/SPECIFICATION.md b/SPECIFICATION.md
@@ -7,3 +7,4 @@
-Version 3.35 – November 2026
+Version 3.36 – November 2026
 | Version | Change |
 |---|---|
+| 3.36 | Phase 10 **public preview** of the browser app on GitHub Pages (B8a, stage 1): the page … |
@@COMMIT\x1fbbbb2222\x1fbbbb222\x1f2026-11-15T06:15:00+01:00\x1faaaa1111 cccc3333\x1fMerge main (spec 3.36); unicorn becomes spec 3.37
+| 3.37 | Phase 13 **G5's browser part built** (§14 Phase 13): god mode … |
@@COMMIT\x1fdddd4444\x1fdddd444\x1f2026-11-15T06:44:00+01:00\x1fbbbb2222 eeee5555\x1fMerge art project: music, incidents (spec 3.37, 3.38)
+| 3.38 | Art project incident log (decided by the user 2026-11-15; `art/ART-PROJECT.md` v1.7 §2.6): the … |
+| 3.37 | Art project music (decided by the user): each day … |
@@COMMIT\x1fffff6666\x1fffff666\x1f2026-11-15T07:03:00+01:00\x1fdddd4444 gggg7777\x1fMerge Unicorn mode in the browser (spec 3.39)
-| 3.37 | Phase 13 **G5's browser part built** (§14 Phase 13): god mode … |
+| 3.39 | Phase 13 **G5's browser part built** (§14 Phase 13): god mode … |
"""

START = {"3.35": "old row"}
END = {"3.39": "Phase 13 **G5's browser part built** (§14 Phase 13): god mode …",
       "3.38": "Art project incident log (decided by the user 2026-11-15; `art/ART-PROJECT.md` v1.7 §2.6): the …",
       "3.37": "Art project music (decided by the user): each day …",
       "3.36": "Phase 10 **public preview** of the browser app on GitHub Pages (B8a, stage 1): the page …",
       "3.35": "old row"}


class SpecRowTest(unittest.TestCase):
    def test_rows_added_by_a_diff(self):
        commits = F.parse_spec_log(SPEC_LOG)
        self.assertEqual([c["short"] for c in commits], ["aaaa111", "bbbb222", "dddd444", "ffff666"])
        self.assertEqual([v for v, _ in commits[0]["rows"]], ["3.36"])  # not the version header
        self.assertEqual([v for v, _ in commits[2]["rows"]], ["3.38", "3.37"])
        self.assertEqual(commits[1]["parents"], ["aaaa1111", "cccc3333"])

    def test_titles(self):
        self.assertEqual(F.row_title(END["3.39"]), "Phase 13 G5's browser part built")
        self.assertEqual(F.row_title(END["3.38"]), "Art project incident log")
        self.assertEqual(F.row_title(END["3.36"]), "Phase 10 public preview of the browser app on GitHub Pages")
        self.assertTrue(F.row_title("x" * 200).endswith("…"))

    def test_spec_rows_of_a_file(self):
        text = "Version 3.39\n\n| Version | Change |\n|---|---|\n| 3.39 | new |\n| 3.38 | older |\n"
        self.assertEqual(F.spec_rows(text), {"3.39": "new", "3.38": "older"})


class MergeMatchTest(unittest.TestCase):
    def test_rows_matched_to_their_merges(self):
        feats = {f["version"]: f for f in F.features_of(F.parse_spec_log(SPEC_LOG), START, END)}
        self.assertEqual(list(feats), ["3.36", "3.37", "3.38", "3.39"])
        self.assertEqual((feats["3.36"]["commit"], feats["3.36"]["merge"]), ("aaaa111", False))  # direct
        # 3.37: named by the merge subject (the renumbered unicorn row was 3.37 earlier)
        self.assertEqual(feats["3.37"]["commit"], "dddd444")
        self.assertEqual(feats["3.38"]["commit"], "dddd444")
        self.assertEqual(feats["3.39"]["commit"], "ffff666")
        self.assertTrue(feats["3.39"]["merge"])
        self.assertEqual(feats["3.39"]["time"], "2026-11-15T07:03:00+01:00")
        self.assertNotIn("3.35", feats)


RUNS = [
    {"start": "2026-11-15T05:00:00+01:00", "machine": "marge", "branch": "worktree-agent-ab69e4b089e56849c",
     "command": "cargo test --workspace 2>&1 | tail", "exit": 1, "log": "050000-marge-1.log"},
    {"start": "2026-11-15T06:50:00+01:00", "machine": "marge", "branch": "worktree-agent-ab69e4b089e56849c",
     "command": "cargo test --workspace", "exit": 0, "log": "065000-marge-2.log"},
    {"start": "2026-11-15T06:55:00+01:00", "machine": "macos", "branch": "worktree-agent-ab69e4b089e56849c",
     "command": "cargo test -p ocw-ui", "exit": 0, "log": "065500-macos-3.log"},
    {"start": "2026-11-15T06:58:00+01:00", "machine": "marge", "branch": "worktree-agent-ab69e4b089e56849c",
     "command": "cargo clippy --workspace", "exit": 0, "log": "065800-marge-4.log"},
    {"start": "2026-11-15T07:10:00+01:00", "machine": "marge", "branch": "main",
     "command": "make check", "exit": 0, "log": "071000-marge-5.log"},
    {"start": "2026-11-15T07:20:00+01:00", "machine": "marge", "branch": "main", "commit": "ffff666",
     "command": "cargo test --workspace", "exit": 0, "log": "072000-marge-6.log"},
]


class CheckLookupTest(unittest.TestCase):
    def test_by_commit_first(self):
        c = F.find_check(RUNS, "2026-11-15T07:03:00+01:00", "worktree-agent-ab69e4b089e56849c",
                         ["ffff6666", "gggg7777"])
        self.assertEqual((c["how"], c["log"], c["exit"]), ("commit", "072000-marge-6.log", 0))

    def test_by_branch_before_the_merge(self):
        runs = [r for r in RUNS if not r.get("commit")]
        c = F.find_check(runs, "2026-11-15T07:03:00+01:00", "worktree-agent-ab69e4b089e56849c", ["zzz"])
        self.assertEqual((c["how"], c["log"]), ("branch", "065000-marge-2.log"))  # latest marge test, not clippy

    def test_main_after_the_merge(self):
        runs = [r for r in RUNS if r["branch"] == "main" and not r.get("commit")]
        c = F.find_check(runs, "2026-11-15T07:03:00+01:00", None, [], until="2026-11-15T08:00:00+01:00")
        self.assertEqual(c["how"], "main after merge")
        self.assertIsNone(F.find_check(runs, "2026-11-15T07:03:00+01:00", None, [],
                                       until="2026-11-15T07:05:00+01:00"))

    def test_test_results(self):
        text = ("test result: ok. 12 passed; 0 failed; 0 ignored\nnoise\n"
                "test result: FAILED. 3 passed; 1 failed; 0 ignored\ntest result: ok. 0 passed; 0 failed\n")
        self.assertEqual(F.parse_test_results(text),
                         {"suites": 3, "passed": 15, "failed": 1, "failed_suites": 1})
        self.assertEqual(F.parse_test_results("")["suites"], 0)


class FeaturesForTest(Fixture):
    """diary.features_for on the fixture: the check result read from the
    build log, deploys, no gh."""

    def test_feature_with_check_and_deploys(self):
        day = "2026-11-15"
        bl = self.out / "build-log" / day
        bl.mkdir(parents=True)
        (bl / "runs.jsonl").write_text("".join(json.dumps(r) + "\n" for r in RUNS))
        (bl / "072000-marge-6.log").write_text("test result: ok. 40 passed; 0 failed\n" * 3)
        (bl / "deploys.jsonl").write_text(json.dumps({"start": f"{day}T07:30:00+01:00", "machine": "marge",
                                                      "commit": "ffff666", "exit": 0}) + "\n")
        outs = {("rev-list", "-1", "--first-parent", f"--before={day} 00:00:00", "HEAD"): "c0\n",
                ("rev-list", "-1", "--first-parent", "--before=2026-11-16 00:00:00", "HEAD"): "c1\n",
                ("show", "c0:SPECIFICATION.md"): "| 3.35 | old row |\n",
                ("show", "c1:SPECIFICATION.md"): "".join(f"| {v} | {t} |\n" for v, t in END.items())}

        def fake_git(*args):
            if args[0] == "log" and "-p" in args:
                return SPEC_LOG
            if args[0] == "log":
                return "2026-11-15T06:15:00+01:00\n2026-11-15T06:44:00+01:00\n2026-11-15T07:03:00+01:00\n"
            return outs.get(args, "")
        with mock.patch.object(diary, "git_out", side_effect=fake_git), \
                mock.patch.object(diary, "branch_of", return_value="worktree-agent-ab69e4b089e56849c"):
            got = diary.features_for(day)
        f = {x["version"]: x for x in got["features"]}["3.39"]
        self.assertEqual(f["check"]["how"], "commit")
        self.assertEqual((f["check"]["suites"], f["check"]["passed"]), (3, 120))
        self.assertEqual(got["deploys"][0]["machine"], "marge")
        self.assertEqual(got["releases"], [])  # OCW_ART_GH=0 in the fixture


class AgentEventTest(Fixture):
    def test_append_and_mask(self):
        day = "2026-11-15"
        diary.agent_event(day, "Fix /home/builder/src on buildhost.example.lan", "worktree-agent-ab69e4b089e56849c",
                          "started", time_=f"{day}T01:00:00+01:00")
        diary.agent_event(day, "Fix", "worktree-agent-ab69e4b089e56849c", "interrupted",
                          time_=f"{day}T00:39:47+01:00", note="OOM")
        diary.agent_event(day, "Fix", "worktree-agent-ab69e4b089e56849c", "merged", commits="d9a9b05",
                          time_=f"{day}T07:03:00+01:00")
        text = diary.agents_path(day).read_text()
        self.assertNotIn("buildhost.example.lan", text)
        self.assertNotIn("/home/builder", text)
        lines = [json.loads(x) for x in text.splitlines()]
        self.assertEqual(len(lines), 3)
        self.assertEqual(lines[0]["agent_id_short"], "ab69e4b0")
        with self.assertRaises(SystemExit):
            diary.agent_event(day, "x", "b", "exploded")
        jobs = F.agent_jobs(lines)
        self.assertEqual(len(jobs), 1)
        j = jobs[0]
        self.assertEqual((j["status"], j["interruptions"], j["merged"]), ("merged", 1, "d9a9b05"))
        self.assertEqual(j["start"], f"{day}T00:39:47+01:00")  # the first event
        self.assertEqual(j["end"], f"{day}T07:03:00+01:00")

    def test_cli(self):
        env = {**os.environ, "OCW_TEST_OUTPUT": str(self.out), "OCW_ART_HISTORY": str(self.hist)}
        r = subprocess.run([sys.executable, str(HERE / "diary.py"), "agent-event", "--task", "Roll", "--branch",
                            "worktree-agent-a43d6ec0bd80728f8", "--event", "reported", "--commits", "abc1234",
                            "--time", "2026-11-15T09:00:00+01:00"], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        e = json.loads((self.out / "build-log" / "2026-11-15" / "agents.jsonl").read_text())
        self.assertEqual((e["event"], e["agent_id_short"], e["commits"]), ("reported", "a43d6ec0", "abc1234"))


class RollTest(unittest.TestCase):
    def test_pages(self):
        feats = [{"version": f"3.{i:02d}", "title": f"Feature number {i}", "commit": "abc"} for i in range(14)]
        feats.append({"version": "3.99", "title": "not merged", "commit": None})
        feats.append({"version": "4.00", "title": "a very long title " * 6, "commit": "def"})
        pages = F.roll_pages(feats, per_page=6, width=40)
        self.assertEqual([len(p) for p in pages], [6, 6, 3])
        self.assertEqual(pages[0][0], "3.00  Feature number 0")
        self.assertTrue(all(len(x) <= 40 for p in pages for x in p))
        self.assertTrue(pages[-1][-1].startswith("4.00  a very long") and pages[-1][-1].endswith("…"))
        self.assertNotIn("3.99", " ".join(x for p in pages for x in p))
        self.assertEqual(F.roll_pages([]), [])


if __name__ == "__main__":
    unittest.main()
