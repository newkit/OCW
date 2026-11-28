"""Tests of cleanup_rules.py: the rule of every checkout class with made-up
listings, the thresholds, the snippets (the POSIX ones run here under
/bin/sh against a scratch directory; the PowerShell ones are checked as
text – `remote-gc.sh --selftest MACHINE` runs them on a Windows machine),
the parsers and the git facts of a scratch repository."""

import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

import cleanup_rules as r

GB = 10 ** 9
NOW = 1_800_000_000
H, D = 3600, 86400
LIM = r.limits({})


def co(suffix, idle=5 * D, gb=10, busy=False, out=0, other=1, target=True, strays=()):
    name = "OCW-" + suffix if suffix else "OCW"
    cls, s = r.classify(name, "OCW")
    return {"name": name, "cls": cls, "suffix": s, "target_bytes": gb * GB if target else None, "out_bytes": out,
            "other_bytes": other * GB, "sync": NOW - idle, "used": 0, "mtime": NOW - idle, "busy": busy,
            "strays": list(strays)}


# Made-up branches: what the coordinator's git knows.
FACTS = r.make_facts(
    branches=["main", "worktree-agent-open", "worktree-agent-new", "worktree-agent-done", "worktree-agent-donelive",
              "merge/hud", "merge-open", "fix-saddle", "fix-open", "fix-ff"],
    merged=["main", "worktree-agent-new", "worktree-agent-done", "worktree-agent-donelive", "merge/hud", "fix-saddle",
            "fix-ff"],
    own=["worktree-agent-done", "worktree-agent-donelive", "merge/hud", "fix-saddle"],
    worktrees=["main", "worktree-agent-open", "worktree-agent-new", "worktree-agent-donelive"],
    release=(0, 2, 7))


def decide(c, lim=LIM, newest=(0, 2, 7), facts=FACTS):
    d = r.decide(c, facts, lim, NOW, newest)
    return d["action"], d["reason"]


class ClassTest(unittest.TestCase):
    def test_classes_by_name(self):
        names = {"OCW": "main", "OCW-HEAD": "head", "OCW-worktree-agent-a1": "agent", "OCW-merge-hud": "merge",
                 "OCW-merge-batch-2026-11-16": "merge", "OCW-merge": "merge", "OCW-release-0.2.1": "release",
                 "OCW-release-0.2.7-webtest": "other", "OCW-fix-thumb-framing": "other", "OCW-agent-vr-x1": "other",
                 "OCW-strava-pending": "other", "OCW-merger": "other"}
        self.assertEqual({n: r.classify(n, "OCW")[0] for n in names}, names)
        self.assertEqual(r.classify("OCW-ART", "OCW"), ("other", "ART"))  # a sibling repo: see test_a_clone_is_foreign
        self.assertEqual(r.classify("other", "OCW"), (None, None))
        self.assertEqual(r.classify("OCWX", "OCW"), (None, None))

    def test_branch_names_with_slashes(self):
        self.assertEqual(r.dir_suffix("merge/hud"), "merge-hud")
        self.assertEqual(r.branch_state("merge-hud", FACTS), "merged-own")
        # a/b and a-b share a directory: the more careful state counts
        f = r.make_facts(["x/y", "x-y"], merged=["x/y"], own=["x/y"])
        self.assertEqual(r.branch_state("x-y", f), "open")

    def test_version_order_is_numeric(self):
        self.assertGreater(r.version_of("release-0.2.10"), r.version_of("release-0.2.9"))
        self.assertEqual(r.version_of("ocw-release-1.10.3"), (1, 10, 3))
        self.assertIsNone(r.version_of("release-next"))


class NeverTest(unittest.TestCase):
    def test_main_and_head_are_never_cleaned(self):
        self.assertEqual(decide(co("", idle=400 * D))[0], "keep")
        a, why = decide(co("HEAD", idle=400 * D))
        self.assertEqual(a, "keep")
        self.assertIn("release / deploy", why)

    def test_busy_checkout_is_kept_whatever_its_branch(self):
        for s in ("worktree-agent-gone", "merge-hud", "release-0.2.1", "fix-saddle"):
            a, why = decide(co(s, idle=90 * D, busy=True))
            self.assertEqual((a, why), ("keep", "in use (running or waiting task)"), s)

    def test_recently_synced_is_kept(self):
        for s in ("worktree-agent-done", "merge-hud", "merge-gone", "fix-saddle"):
            a, why = decide(co(s, idle=23 * H))
            self.assertEqual(a, "keep", s)
            self.assertIn("but synced 23h 00m ago", why)
        self.assertEqual(decide(co("merge-hud", idle=25 * H))[0], "target")

    def test_a_clone_is_foreign(self):
        # A directory with a .git next to the main checkout (a clone of another
        # repository whose name starts alike) is no remote.sh checkout.
        c = dict(co("ART", idle=400 * D), cls="foreign")
        a, why = decide(c)
        self.assertEqual(a, "keep")
        self.assertIn(".git", why)
        inv = r.parse_posix_inventory("@@checkouts\nc 9 5 0 1 1 1 1 OCW-ART\nc 9 5 0 1 1 1 0 OCW-fix-x\n", "OCW")
        self.assertEqual([c["cls"] for c in inv["checkouts"]], ["foreign", "other"])
        self.assertEqual(r.lru_candidates(r.plan({"now": NOW, "checkouts": [c]}, FACTS, LIM), 0), [])

    def test_unknown_last_sync_is_kept(self):
        c = dict(co("merge-hud"), sync=0, used=0, mtime=0)
        self.assertEqual(decide(c), ("keep", "last sync unknown"))

    def test_last_sync_is_the_newest_of_sync_and_use(self):
        c = dict(co("merge-hud", idle=9 * D), used=NOW - 2 * H)
        self.assertEqual(decide(c)[0], "keep")


class AgentTest(unittest.TestCase):
    def test_branch_gone_goes_at_once(self):
        self.assertEqual(decide(co("worktree-agent-gone", idle=60)), ("target", "branch gone"))

    def test_merged_and_worktree_removed(self):
        self.assertEqual(decide(co("worktree-agent-done", idle=2 * D)), ("target", "merged into main, worktree removed"))

    def test_merged_with_own_commits_and_a_live_worktree_after_two_days(self):
        a, why = decide(co("worktree-agent-donelive", idle=47 * H))
        self.assertEqual(a, "keep")
        a, why = decide(co("worktree-agent-donelive", idle=49 * H))
        self.assertEqual(a, "target")
        self.assertIn("with its own commits", why)

    def test_fresh_agent_branch_is_kept(self):
        # a new agent's branch has no commits yet: it looks merged into main
        a, why = decide(co("worktree-agent-new", idle=5 * D))
        self.assertEqual((a, why), ("keep", "new agent branch (no commits of its own yet)"))
        # … until nothing was synced for 14 days
        self.assertEqual(decide(co("worktree-agent-new", idle=15 * D))[0], "target")

    def test_open_branch_is_kept(self):
        self.assertEqual(decide(co("worktree-agent-open", idle=200 * D)), ("keep", "branch open"))


class MergeTest(unittest.TestCase):
    def test_gone_or_merged(self):
        self.assertEqual(decide(co("merge-voice", idle=2 * D)), ("target", "branch gone"))
        self.assertEqual(decide(co("merge-hud", idle=2 * D)), ("target", "merged into main"))

    def test_open_goes_after_three_days_without_a_sync(self):
        self.assertEqual(decide(co("merge-open", idle=2 * D))[0], "keep")
        a, why = decide(co("merge-open", idle=4 * D))
        self.assertEqual((a, why), ("target", "merge checkout not synced for 4d 0h"))


class ReleaseTest(unittest.TestCase):
    def test_newest_stays_older_go_after_a_week(self):
        self.assertEqual(decide(co("release-0.2.7", idle=300 * D)), ("keep", "newest release checkout"))
        self.assertEqual(decide(co("release-0.2.6", idle=6 * D))[0], "keep")
        a, why = decide(co("release-0.2.6", idle=8 * D))
        self.assertEqual(a, "target")
        self.assertIn("older release (newest: 0.2.7)", why)

    def test_the_newest_is_the_highest_version_anywhere(self):
        inv = {"now": NOW, "checkouts": [co("release-0.2.9", idle=20 * D), co("release-0.2.10", idle=20 * D)]}
        p = r.plan(inv, dict(FACTS, release=(0, 2, 7)), LIM)
        self.assertEqual(p["newest_release"], (0, 2, 10))
        self.assertEqual({i["name"]: i["delete"] for i in p["items"]},
                         {"OCW-release-0.2.9": True, "OCW-release-0.2.10": False})
        # the tags know a newer release than any checkout there: both are old
        p = r.plan(inv, dict(FACTS, release=(0, 3, 0)), LIM)
        self.assertEqual([i["delete"] for i in p["items"]], [True, True])


class OtherTest(unittest.TestCase):
    def test_merged_branch_goes(self):
        self.assertEqual(decide(co("fix-saddle", idle=2 * D)), ("target", "merged into main"))

    def test_open_branch_stays(self):
        self.assertEqual(decide(co("fix-open", idle=100 * D)), ("keep", "branch open"))

    def test_no_branch_of_that_name_after_14_days(self):
        self.assertEqual(decide(co("strava-pending", idle=13 * D))[0], "keep")
        a, why = decide(co("strava-pending", idle=15 * D))
        self.assertEqual((a, why), ("target", "no branch of this name, not synced for 15d 0h"))

    def test_branch_without_own_commits_after_14_days(self):
        self.assertEqual(decide(co("fix-ff", idle=3 * D))[0], "keep")
        self.assertEqual(decide(co("fix-ff", idle=15 * D))[0], "target")


class WholeCheckoutTest(unittest.TestCase):
    def test_idle_30_days_and_no_output(self):
        for s in ("worktree-agent-gone", "merge-hud", "fix-saddle", "strava-pending", "release-0.2.1"):
            self.assertEqual(decide(co(s, idle=29 * D))[0], "target", s)
            a, why = decide(co(s, idle=31 * D))
            self.assertEqual(a, "checkout", s)
            self.assertIn("whole checkout: idle for 31d 0h", why)
        # also when its target/ is gone already
        self.assertEqual(decide(co("merge-hud", idle=31 * D, target=False))[0], "checkout")
        self.assertEqual(decide(co("merge-hud", idle=29 * D, target=False))[0], "keep")

    def test_output_that_may_not_be_fetched_keeps_the_checkout(self):
        d = r.decide(co("merge-hud", idle=40 * D, out=3 * GB), FACTS, LIM, NOW)
        self.assertEqual(d["action"], "target")
        self.assertIn("test-output/ or data/ in it (3.0 GB) may never have been fetched", d["note"])
        a, why = decide(co("merge-hud", idle=40 * D, out=3 * GB, target=False))
        self.assertEqual(a, "keep")
        self.assertIn("may never have been fetched", why)

    def test_never_for_a_reason_that_may_end_or_a_live_worktree(self):
        self.assertEqual(decide(co("merge-open", idle=40 * D))[0], "target")  # an open branch: only target/
        self.assertEqual(decide(co("worktree-agent-donelive", idle=40 * D))[0], "target")
        self.assertEqual(decide(co("worktree-agent-open", idle=400 * D))[0], "keep")

    def test_sizes_in_the_plan(self):
        inv = {"now": NOW, "checkouts": [co("merge-hud", idle=31 * D, gb=10, other=2), co("merge-voice", idle=2 * D)]}
        items = {i["name"]: i for i in r.plan(inv, FACTS, LIM)["items"]}
        self.assertEqual((items["OCW-merge-hud"]["kind"], items["OCW-merge-hud"]["bytes"], items["OCW-merge-hud"]["path"]),
                         ("checkout", 12 * GB, "OCW-merge-hud/"))
        self.assertEqual((items["OCW-merge-voice"]["op"], items["OCW-merge-voice"]["bytes"]),
                         ("target|OCW-merge-voice", 10 * GB))


class ThresholdTest(unittest.TestCase):
    def test_defaults_and_env(self):
        self.assertEqual((LIM["recent_h"], LIM["agent_days"], LIM["merge_days"], LIM["release_days"], LIM["other_days"],
                          LIM["checkout_days"]), (24 * H, 2 * D, 3 * D, 7 * D, 14 * D, 30 * D))
        lim = r.limits({"OCW_GC_RECENT_H": "48", "OCW_GC_MERGE_DAYS": "0.5", "OCW_GC_OTHER_DAYS": "x",
                        "OCW_GC_CHECKOUT_DAYS": "-1"})
        self.assertEqual((lim["recent_h"], lim["merge_days"], lim["other_days"], lim["checkout_days"]),
                         (48 * H, 12 * H, 14 * D, 30 * D))

    def test_thresholds_move_the_decisions(self):
        c = co("merge-hud", idle=30 * H)
        self.assertEqual(decide(c)[0], "target")
        self.assertEqual(decide(c, r.limits({"OCW_GC_RECENT_H": "48"}))[0], "keep")
        c = co("release-0.2.1", idle=3 * D)
        self.assertEqual(decide(c)[0], "keep")
        self.assertEqual(decide(c, r.limits({"OCW_GC_RELEASE_DAYS": "2"}))[0], "target")
        c = co("fix-saddle", idle=10 * D)
        self.assertEqual(decide(c, r.limits({"OCW_GC_CHECKOUT_DAYS": "9"}))[0], "checkout")

    def test_help_lists_every_threshold(self):
        text = r.limits_help()
        for key in r.LIMITS:
            self.assertIn(r.env_name(key), text)


class LeftoverTest(unittest.TestCase):
    def inv(self):
        return {"now": NOW, "checkouts": [
            co("HEAD", strays=[{"name": "target.ocw-gc.16184", "bytes": 7 * GB, "mtime": NOW - 5 * D}]),
            co("worktree-agent-open", strays=[{"name": "target.ocw-janitor.7", "bytes": GB, "mtime": NOW - 600}]),
            co("fix-open", busy=True, strays=[{"name": "target.ocw-gc.9", "bytes": GB, "mtime": NOW - 5 * D}])],
            "strays": [{"name": "OCW-merge-x.ocw-gc.44", "bytes": 2 * GB, "mtime": NOW - 3 * H}],
            "temp": [{"name": "ocw-chromium-old", "bytes": GB, "mtime": NOW - 2 * D, "in_use": False},
                     {"name": "ocw-chromium-new", "bytes": GB, "mtime": NOW - 2 * H, "in_use": False},
                     {"name": "ocw-chromium-used", "bytes": GB, "mtime": NOW - 9 * D, "in_use": True}],
            "relout": [{"name": "ocw-release-0.2.1", "bytes": GB, "mtime": NOW - 20 * D, "worlds": False},
                       {"name": "ocw-release-0.2.3", "bytes": 5 * GB, "mtime": NOW - 20 * D, "worlds": True},
                       {"name": "ocw-release-0.2.6", "bytes": GB, "mtime": NOW - 3 * D, "worlds": False},
                       {"name": "ocw-release-0.2.7", "bytes": GB, "mtime": NOW - 30 * D, "worlds": False}]}

    def test_strays_temp_and_release_output(self):
        items = {i["path"]: i for i in r.plan(self.inv(), FACTS, LIM)["items"] if i["kind"] != "target"}
        got = {p: (i["kind"], i["delete"]) for p, i in items.items()}
        self.assertEqual(got, {
            "OCW-HEAD/target.ocw-gc.16184": ("stray", True),              # also in the checkout that is never cleaned
            "OCW-worktree-agent-open/target.ocw-janitor.7": ("stray", False),  # a deletion may still run
            "OCW-fix-open/target.ocw-gc.9": ("stray", False),             # busy checkout
            "OCW-merge-x.ocw-gc.44/": ("stray", True),
            "tmp/ocw-chromium-old": ("temp", True), "tmp/ocw-chromium-new": ("temp", False),
            "tmp/ocw-chromium-used": ("temp", False),
            "~/ocw-release-0.2.1": ("relout", True), "~/ocw-release-0.2.3": ("relout", False),
            "~/ocw-release-0.2.6": ("relout", False), "~/ocw-release-0.2.7": ("relout", False)})
        self.assertIn("world packages", items["~/ocw-release-0.2.3"]["reason"])
        self.assertIn("newest release", items["~/ocw-release-0.2.7"]["reason"])
        self.assertIn("running process", items["tmp/ocw-chromium-used"]["reason"])

    def test_totals_and_text(self):
        p = r.plan(self.inv(), FACTS, LIM)
        t = r.totals(p["items"])
        self.assertEqual(t["temp"], {"delete": [1, GB], "keep": [2, 2 * GB]})
        text = "\n".join(r.plan_text(p))
        self.assertIn("would delete", text)
        self.assertIn("head: left by a failed deletion, 5d 0h old", text)
        self.assertIn("per class:", text)

    def test_lru_candidates_oldest_first_never_main_head_release_or_busy(self):
        inv = {"now": NOW, "checkouts": [co(""), co("HEAD"), co("release-0.2.7"), co("worktree-agent-open", idle=3 * D),
                                         co("fix-open", idle=9 * D), co("merge-open", idle=2 * D, busy=True),
                                         co("worktree-agent-new", idle=2 * H)]}
        c = r.lru_candidates(r.plan(inv, FACTS, LIM), 24 * H)
        self.assertEqual([i["name"] for i in c], ["OCW-fix-open", "OCW-worktree-agent-open"])


class SafetyTest(unittest.TestCase):
    def test_ops(self):
        ok = ["target|OCW-merge-hud", "checkout|OCW-worktree-agent-a1", "stray|OCW-HEAD/target.ocw-gc.5",
              "stray|OCW/target.ocw-janitor.5", "stray|OCW-merge-x.ocw-gc.44", "temp|ocw-chromium-ab_1",
              "relout|ocw-release-0.2.1"]
        for op in ok:
            self.assertTrue(r.safe_op(op, "OCW"), op)
        bad = ["target|OCW", "checkout|OCW", "target|OCW-HEAD", "checkout|OCW-HEAD", "target|other", "target|OCW-a/target",
               "checkout|OCW-a/..", "target|../OCW-a", "target|OCW-a b", "target|OCW-a;rm", "stray|OCW-a/target",
               "stray|OCW-a/data.ocw-gc.1", "stray|OCW.ocw-gc.1", "stray|OCW-a", "temp|ocw", "temp|ocw-tasks",
               "temp|x/ocw-chromium-1", "relout|ocw-release-0.2.7-worlds", "relout|ocw-world-release-0.2.1",
               "rm|OCW-a", "target|", "checkout|OCW-a.ocw-gc.1"]
        for op in bad:
            self.assertFalse(r.safe_op(op, "OCW"), op)

    def test_unsafe_ops_never_reach_a_snippet(self):
        with self.assertRaises(ValueError):
            r.ops_stdin(["target|OCW-x; rm -rf ~"], "OCW")
        with self.assertRaises(ValueError):
            r.windows_delete("B:\\src\\OCW", ["target|x'; Remove-Item C:\\"])
        with self.assertRaises(ValueError):
            r.windows_delete("B:\\src\\OCW", ["target|OCW-HEAD"])

    def test_branch_cleanup_refuses_main_and_head(self):
        self.assertEqual(r.branch_item("OCW", "merge/hud")["op"], "target|OCW-merge-hud")
        for b in ("HEAD", "x y", "a;b"):
            with self.assertRaises(ValueError):
                r.branch_item("OCW", b)


class SnippetTextTest(unittest.TestCase):
    def test_paths_are_quoted(self):
        self.assertIn('r="$HOME"\'/src/OCW\'', r.posix_inventory("~/src/OCW"))
        self.assertIn("r='/data/it'\\''s/OCW'", r.posix_delete("/data/it's/OCW"))
        self.assertIn("$r = 'B:\\src\\OCW'", r.windows_inventory("B:\\src\\OCW"))
        self.assertIn("$r = 'D:\\o''c\\OCW'", r.windows_delete("D:\\o'c\\OCW", ["target|OCW-merge-hud"]))
        self.assertIn("@('target|OCW-merge-hud', 'temp|ocw-chromium-x')",
                      r.windows_delete("B:\\src\\OCW", ["target|OCW-merge-hud", "temp|ocw-chromium-x"]))

    def test_posix_snippets_do_not_depend_on_the_login_shell(self):
        # They are sent as `/bin/sh -c SCRIPT` (dashboard.run_ssh): the Mac's
        # login shell is zsh, where an unmatched glob aborts the command. No
        # bashisms either, and the names to delete come on stdin.
        for s in (r.posix_inventory("~/src/OCW"), r.posix_delete("~/src/OCW")):
            for bad in ("[[", "${!", "<<<", "mapfile", "local ", "function ", "=~"):
                self.assertNotIn(bad, s, bad)
            self.assertNotIn("@", s.replace("@@", "").replace('"$@"', ""))  # every placeholder filled
        self.assertNotIn("OCW-merge-hud", r.posix_delete("~/src/OCW"))
        self.assertEqual(r.ops_stdin(["target|OCW-merge-hud", "stray|OCW-HEAD/target.ocw-gc.1"], "OCW"),
                         "target OCW-merge-hud\nstray OCW-HEAD/target.ocw-gc.1\n")

    def test_windows_deletion_is_long_path_safe(self):
        s = r.windows_delete("B:\\src\\OCW", ["target|OCW-release-0.2.1"])
        # Remove-Item fails beyond 260 characters: what is left goes with rd on the \\?\ path
        self.assertIn("function OcwRemove($p)", s)  # not "Rm": rm is an alias of Remove-Item and wins over a function
        self.assertIn("""'rd /s /q "\\\\?\\' + $q + '"'""", s)
        self.assertIn("\n  OcwRemove $x\n", s)
        self.assertNotIn("Remove-Item -LiteralPath $x", s)
        self.assertIn("'@@failed ' + $tag", s)
        self.assertIn("'target.ocw-gc.' + $PID", s)  # the name the next inventory sweeps

    def test_windows_free_space_from_driveinfo(self):
        s = r.windows_inventory("B:\\src\\OCW")
        self.assertIn("[System.IO.DriveInfo]::GetDrives()", s)  # Win32_LogicalDisk is denied over ssh, Get-PSDrive says 0
        for bad in ("Get-CimInstance", "Get-PSDrive", "Get-WmiObject"):
            self.assertNotIn(bad, s)
        self.assertIn("'target.ocw-gc.*'", s)

    def test_windows_snippets_fill_every_placeholder_and_never_touch_head(self):
        for s in (r.windows_inventory("B:\\src\\OCW"), r.windows_delete("B:\\src\\OCW", ["target|OCW-a"], True)):
            self.assertNotRegex(s, r"@[A-Z]+@")
        self.assertIn("$n -eq ($leaf + '-HEAD')", r.windows_delete("B:\\src\\OCW", []))
        self.assertIn("$dry = $true", r.windows_delete("B:\\src\\OCW", [], True))


def put(p: Path, size=10, age=0.0):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(b"x" * size)
    if age:
        t = time.time() - age
        os.utime(p, (t, t))


@unittest.skipUnless(os.path.exists("/bin/sh") and shutil.which("du"), "needs a POSIX shell")
class PosixRunTest(unittest.TestCase):
    """The POSIX snippets against a scratch "machine" (HOME and TMPDIR moved)."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="ocw-gc-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        src = self.tmp / "src"
        self.repo = src / "OCW"
        put(self.repo / "target" / "debug" / "deps" / "a.rlib", 5000)
        put(self.repo / "target.ocw-gc.77" / "x", 3000)
        put(self.repo / "data" / "pilot" / "world.pkg", 100)
        put(src / "OCW-HEAD" / "target" / "x", 100)
        put(src / "OCW-merge-hud" / "target" / "debug" / "deps" / "b.rlib", 9000)
        put(src / "OCW-merge-hud" / ".ocw-sync" / "synced.txt", 10)
        put(src / "OCW-merge-hud" / "crates" / "lib.rs", 2000)
        put(src / "OCW-fix-saddle" / "test-output" / "shot.jpg", 4000)
        put(src / "OCW-fix-saddle" / "crates" / "lib.rs", 100)
        put(src / "OCW-worktree-agent-a1" / "crates" / "lib.rs", 100)
        (src / "OCW-worktree-agent-a1" / "test-output").mkdir()  # empty: no output
        put(src / "OCW-merge-x.ocw-gc.5" / "crates" / "lib.rs", 100)
        put(src / "not-ours" / "target" / "x", 100)
        put(src / "OCW-ART" / "target" / "x", 100)
        put(src / "OCW-ART" / ".git" / "HEAD", 10)
        put(self.tmp / "tmp" / "ocw-chromium-abc" / "Default" / "Cache", 6000)
        put(self.tmp / "tmp" / "ocw-tasks-other" / "x", 10)
        put(self.tmp / "home" / "ocw-release-0.2.1" / "OpenCyclingWorld-0.2.1-web.zip", 100)
        put(self.tmp / "home" / "ocw-release-0.2.3" / "worlds" / "w.pkg", 100)
        put(self.tmp / "home" / "ocw-release-0.2.7-worlds" / "w.pkg", 100)
        self.env = dict(os.environ, HOME=str(self.tmp / "home"), TMPDIR=str(self.tmp / "tmp"),
                        OCW_TASKS_DIR=str(self.tmp / "tasks"), OCW_GC_TMP=str(self.tmp / "tmp"))

    def marker(self, pid, checkout):
        put(self.tmp / "tasks" / ("%d-1.json" % pid), 0)
        (self.tmp / "tasks" / ("%d-1.json" % pid)).write_text(
            '{"v":1,"pid":%d,"state":"running","slot":1,"checkout":"%s","branch":"x"}\n' % (pid, checkout))

    def test_a_live_task_marker_protects_its_checkout(self):
        self.marker(os.getpid(), "OCW-merge-hud")          # a running task
        self.marker(2 ** 22 + 12345, "OCW-worktree-agent-a1")  # the marker of a dead process
        inv = self.inventory()
        self.assertEqual({c["name"] for c in inv["checkouts"] if c["busy"]}, {"OCW-merge-hud"})
        put(self.repo.parent / "OCW-merge-hud" / "target.ocw-gc.3" / "x", 10)
        res = self.delete(["target|OCW-merge-hud", "checkout|OCW-merge-hud", "stray|OCW-merge-hud/target.ocw-gc.3",
                           "checkout|OCW-worktree-agent-a1"])
        self.assertEqual({k: v["result"] for k, v in res.items()}, {
            "target|OCW-merge-hud": "busy", "checkout|OCW-merge-hud": "busy",
            "stray|OCW-merge-hud/target.ocw-gc.3": "busy", "checkout|OCW-worktree-agent-a1": "deleted"})
        self.assertTrue((self.repo.parent / "OCW-merge-hud" / "target" / "debug" / "deps" / "b.rlib").is_file())

    def sh(self, script, stdin=""):
        res = subprocess.run(["/bin/sh", "-c", script], input=stdin, capture_output=True, text=True, env=self.env,
                             timeout=120)
        self.assertEqual(res.stderr, "")
        return res.stdout

    def inventory(self):
        return r.parse_posix_inventory(self.sh(r.posix_inventory(str(self.repo))), "OCW")

    def test_inventory(self):
        inv = self.inventory()
        self.assertAlmostEqual(inv["now"], time.time(), delta=60)
        self.assertTrue(inv["disk"]["used"])
        cs = {c["name"]: c for c in inv["checkouts"]}
        self.assertEqual(sorted(cs), ["OCW", "OCW-ART", "OCW-HEAD", "OCW-fix-saddle", "OCW-merge-hud",
                                      "OCW-worktree-agent-a1"])
        self.assertEqual(cs["OCW-ART"]["cls"], "foreign")
        self.assertEqual([s["name"] for s in cs["OCW"]["strays"]], ["target.ocw-gc.77"])
        self.assertEqual(cs["OCW"]["out_bytes"], 0)  # the main checkout's data/ is not even measured
        self.assertGreater(cs["OCW-merge-hud"]["target_bytes"], 0)
        self.assertGreater(cs["OCW-merge-hud"]["other_bytes"], 0)
        self.assertAlmostEqual(cs["OCW-merge-hud"]["sync"], time.time(), delta=60)
        self.assertIsNone(cs["OCW-fix-saddle"]["target_bytes"])
        self.assertGreater(cs["OCW-fix-saddle"]["out_bytes"], 0)
        self.assertEqual(cs["OCW-worktree-agent-a1"]["out_bytes"], 0)  # an empty test-output/ is no output
        self.assertEqual([s["name"] for s in inv["strays"]], ["OCW-merge-x.ocw-gc.5"])
        self.assertEqual([(t["name"], t["in_use"]) for t in inv["temp"]], [("ocw-chromium-abc", False)])
        self.assertEqual({x["name"]: x["worlds"] for x in inv["relout"]},
                         {"ocw-release-0.2.1": False, "ocw-release-0.2.3": True})

    def delete(self, ops, dry=False):
        return r.parse_results(self.sh(r.posix_delete(str(self.repo), dry), r.ops_stdin(ops, "OCW")))

    def test_dry_run_deletes_nothing(self):
        res = self.delete(["target|OCW-merge-hud", "temp|ocw-chromium-abc"], dry=True)
        self.assertEqual({k: v["result"] for k, v in res.items()},
                         {"target|OCW-merge-hud": "would", "temp|ocw-chromium-abc": "would"})
        self.assertGreater(res["target|OCW-merge-hud"]["bytes"], 0)
        self.assertTrue((self.repo.parent / "OCW-merge-hud" / "target").is_dir())

    def test_deletes_what_it_is_told_and_nothing_else(self):
        src = self.repo.parent
        res = self.delete(["target|OCW-merge-hud", "stray|OCW/target.ocw-gc.77", "stray|OCW-merge-x.ocw-gc.5",
                           "checkout|OCW-worktree-agent-a1", "checkout|OCW-fix-saddle", "temp|ocw-chromium-abc",
                           "relout|ocw-release-0.2.1", "relout|ocw-release-0.2.3", "target|OCW-merge-none"])
        self.assertEqual({k: v["result"] for k, v in res.items()}, {
            "target|OCW-merge-hud": "deleted", "stray|OCW/target.ocw-gc.77": "deleted",
            "stray|OCW-merge-x.ocw-gc.5": "deleted", "checkout|OCW-worktree-agent-a1": "deleted",
            "checkout|OCW-fix-saddle": "kept",       # files under test-output/: checked on the machine again
            "temp|ocw-chromium-abc": "deleted", "relout|ocw-release-0.2.1": "deleted",
            "relout|ocw-release-0.2.3": "kept",      # world packages
            "target|OCW-merge-none": "gone"})
        left = sorted(p.name for p in src.iterdir())
        self.assertEqual(left, ["OCW", "OCW-ART", "OCW-HEAD", "OCW-fix-saddle", "OCW-merge-hud", "not-ours"])
        self.assertEqual(sorted(p.name for p in (src / "OCW-merge-hud").iterdir()), [".ocw-sync", "crates"])
        self.assertEqual(sorted(p.name for p in self.repo.iterdir()), ["data", "target"])
        self.assertTrue((src / "OCW-fix-saddle" / "test-output" / "shot.jpg").is_file())
        self.assertEqual(sorted(p.name for p in (self.tmp / "tmp").iterdir()), ["ocw-tasks-other"])
        self.assertEqual(sorted(p.name for p in (self.tmp / "home").iterdir()),
                         ["ocw-release-0.2.3", "ocw-release-0.2.7-worlds"])

    def test_the_machine_refuses_what_must_never_go(self):
        # Lines no caller sends (ops_stdin checks them): the snippet refuses them by itself.
        out = self.sh(r.posix_delete(str(self.repo)),
                      "target OCW\ncheckout OCW\ntarget OCW-HEAD\ncheckout OCW-HEAD\ntarget not-ours\n"
                      "checkout ../src/OCW-merge-hud\nstray OCW-merge-hud/crates\ntemp ocw-tasks-other\n"
                      "relout ocw-release-0.2.7-worlds/..\nrm OCW-merge-hud\ntarget OCW-merge-hud/../OCW\n"
                      "target OCW-ART\ncheckout OCW-ART\n")
        res = [l.split()[0] for l in out.splitlines() if l.startswith("@@") and l != "@@end"]
        self.assertEqual(res, ["@@refused"] * 13)
        self.assertTrue((self.repo.parent / "OCW-ART" / "target" / "x").is_file())
        self.assertTrue((self.repo / "target" / "debug" / "deps" / "a.rlib").is_file())
        self.assertTrue((self.repo.parent / "OCW-HEAD" / "target" / "x").is_file())
        self.assertTrue((self.repo.parent / "not-ours" / "target" / "x").is_file())

    def test_plan_from_a_real_inventory(self):
        facts = r.make_facts(["main", "merge/hud"], merged=["main", "merge/hud"], own=["merge/hud"])
        inv = self.inventory()
        inv["now"] += 2 * D  # two days later
        todo = sorted(i["op"] for i in r.plan(inv, facts, LIM)["items"] if i["delete"])
        self.assertEqual(todo, ["stray|OCW-merge-x.ocw-gc.5", "stray|OCW/target.ocw-gc.77", "target|OCW-merge-hud",
                                "temp|ocw-chromium-abc"])


class ParserTest(unittest.TestCase):
    def test_posix_lines(self):
        text = ("@@now 1800000000\n@@disks\n/dev/sda2 1000 600 300 67% /home\n/dev/sda2 1000 600 300 67% /home\n"
                "@@checkouts\nc 0 2048 0 0 1700000000 1700000001 1 OCW\n"
                "c 5000 1024 100 1700000100 1700000050 1700000000 0 OCW-worktree-agent-x\n"
                "s 500 1700000000 OCW-worktree-agent-x/target.ocw-gc.4\n"
                "c 9 -1 0 0 0 1700000000 0 OCW-bisect-1\nc 9 9 9 9 9 9 0 junk-dir\nc 9 9 9 9 9 9 0 OCW-bad name\nnoise\n"
                "@@temp\nt 10 5 0 ocw-chromium-a\nt 10 5 0 ocw-chromium-a\nt 1 1 1 ocw-other\n"
                "@@relout\nr 7 6 1 ocw-release-0.2.3\nr 7 6 0 ocw-release-0.2.7-stage\n"
                "@@busy\nOCW-worktree-agent-x\n@@end\n")
        inv = r.parse_posix_inventory(text, "OCW")
        self.assertEqual(inv["now"], 1800000000)
        self.assertEqual(len(inv["disks"]), 1)  # same file system once
        self.assertEqual(inv["disk"]["used"], 600 * 1024)
        self.assertEqual([(c["name"], c["cls"], c["busy"]) for c in inv["checkouts"]],
                         [("OCW", "main", False), ("OCW-worktree-agent-x", "agent", True), ("OCW-bisect-1", "other", False)])
        x = inv["checkouts"][1]
        self.assertEqual((x["target_bytes"], x["out_bytes"], x["other_bytes"], x["sync"], x["used"]),
                         (1024 * 1024, 100 * 1024, (5000 - 1024 - 100 - 500) * 1024, 1700000100, 1700000050))
        self.assertEqual(x["strays"], [{"name": "target.ocw-gc.4", "bytes": 500 * 1024, "mtime": 1700000000}])
        self.assertIsNone(inv["checkouts"][2]["target_bytes"])
        self.assertEqual([t["name"] for t in inv["temp"]], ["ocw-chromium-a"])
        self.assertEqual([(x["name"], x["worlds"]) for x in inv["relout"]], [("ocw-release-0.2.3", True)])

    def test_windows_json(self):
        text = "#< CLIXML\n" + json.dumps({
            "now": 1800000000,
            "disks": [{"name": "C:", "total": 100, "used": 95, "free": 5}, {"name": "B:", "total": 1000, "used": 500, "free": 500}],
            "checkouts": [
                {"name": "OCW-worktree-agent-y", "target": 4096, "out": 0, "other": 10, "sync": 1700000000, "used": 5,
                 "mtime": 3, "strays": {"name": "target.ocw-gc.25528", "bytes": 7, "mtime": 9}},
                {"name": "OCW-release-0.2.1", "target": -1, "out": 0, "other": 10, "sync": 0, "used": 0, "mtime": 3, "strays": []},
                {"name": "OCW-merge-z.ocw-gc.9", "target": -1, "out": 0, "other": 10, "sync": 0, "used": 0, "mtime": 3}],
            "temp": {"name": "ocw-chromium-q", "bytes": 5, "mtime": 4, "in_use": 0},
            "relout": [], "busy": "OCW-worktree-agent-y"})
        inv = r.parse_windows_inventory(text, "OCW")
        self.assertEqual(inv["disk"]["name"], "B:")
        self.assertEqual([(c["name"], c["cls"], c["busy"], c["target_bytes"]) for c in inv["checkouts"]],
                         [("OCW-worktree-agent-y", "agent", True, 4096), ("OCW-release-0.2.1", "release", False, None)])
        self.assertEqual(inv["checkouts"][0]["strays"][0]["name"], "target.ocw-gc.25528")
        self.assertEqual([s["name"] for s in inv["strays"]], ["OCW-merge-z.ocw-gc.9"])
        self.assertEqual(inv["temp"][0]["name"], "ocw-chromium-q")

    def test_results(self):
        res = r.parse_results("@@deleted target OCW-a 1024\r\n@@busy checkout OCW-b\nnoise\n@@gone target OCW-c\n"
                              "@@failed stray OCW-d/target.ocw-gc.1\n@@would temp ocw-chromium-x 2\n@@kept relout ocw-release-0.2.3\n@@end\n")
        self.assertEqual(res, {
            "target|OCW-a": {"result": "deleted", "bytes": 1024 * 1024}, "checkout|OCW-b": {"result": "busy", "bytes": None},
            "target|OCW-c": {"result": "gone", "bytes": None},
            "stray|OCW-d/target.ocw-gc.1": {"result": "failed", "bytes": None},
            "temp|ocw-chromium-x": {"result": "would", "bytes": 2048},
            "relout|ocw-release-0.2.3": {"result": "kept", "bytes": None}})


@unittest.skipUnless(shutil.which("git"), "needs git")
class GitFactsTest(unittest.TestCase):
    def test_scratch_repository(self):
        tmp = Path(tempfile.mkdtemp(prefix="ocw-gc-git-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        repo = tmp / "repo"

        def git(*a, cwd=repo):
            subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.org", "-c", "commit.gpgsign=false",
                            "-c", "tag.gpgsign=false"] + list(a), cwd=cwd, check=True, capture_output=True)
        repo.mkdir()
        git("init", "-q", "-b", "main")
        (repo / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.10"\n')
        git("add", "Cargo.toml")
        git("commit", "-q", "-m", "one")
        git("tag", "v0.2.9")
        git("branch", "worktree-agent-done")
        git("branch", "merge/hud")
        git("checkout", "-q", "worktree-agent-done")
        git("commit", "-q", "--allow-empty", "-m", "work")
        git("checkout", "-q", "merge/hud")
        git("merge", "-q", "--no-ff", "-m", "merge the agent", "worktree-agent-done")
        git("checkout", "-q", "main")
        git("merge", "-q", "--ff-only", "merge/hud")
        git("branch", "worktree-agent-new")
        git("worktree", "add", "-q", str(tmp / "live"), "worktree-agent-new")
        git("checkout", "-q", "-b", "worktree-agent-open")
        git("commit", "-q", "--allow-empty", "-m", "two")
        git("checkout", "-q", "main")
        f = r.git_facts(repo)
        state = lambda s: r.branch_state(s, f)  # noqa: E731
        self.assertEqual(state("worktree-agent-done"), "merged-own")   # merged with a merge commit
        self.assertEqual(state("worktree-agent-new"), "merged-fresh")  # a new agent's branch looks merged
        self.assertEqual(state("merge-hud"), "merged-fresh")           # main was fast-forwarded to it
        self.assertEqual(state("worktree-agent-open"), "open")
        self.assertEqual(state("worktree-agent-gone"), "gone")
        self.assertEqual(f["worktrees"], {"main", "worktree-agent-new"})
        self.assertEqual(f["release"], (0, 2, 10))  # Cargo.toml's 0.2.10 is newer than the tag v0.2.9
        # the rules on these facts (like the old remote-gc.sh verdict, and more)
        lim = r.limits({})
        d = lambda s, idle: r.decide(co(s, idle=idle), f, lim, NOW)["action"]  # noqa: E731
        self.assertEqual(d("worktree-agent-done", 2 * D), "target")
        self.assertEqual(d("worktree-agent-new", 2 * D), "keep")
        self.assertEqual(d("worktree-agent-open", 2 * D), "keep")
        self.assertEqual(d("worktree-agent-gone", 60), "target")
        self.assertEqual(d("merge-hud", 2 * D), "target")
        self.assertEqual(d("bisect-123", 2 * D), "keep")

    def test_no_facts_no_decisions(self):
        # A failing git (no repository there) must not make every branch look gone.
        tmp = Path(tempfile.mkdtemp(prefix="ocw-gc-git-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        with self.assertRaises(RuntimeError):
            r.git_facts(tmp / "nowhere")


if __name__ == "__main__":
    unittest.main()
