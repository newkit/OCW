"""Tests of janitor.py: the decisions, the sweep of old build units, the
remote snippets' parsers and the pass loop with a fake machine (no network)."""

import json
import os
import tempfile
import time
import unittest
from pathlib import Path

import janitor as j

GB = 1 << 30
NOW = 1_800_000_000.0
H = 3600


def co(name, kind="agent", gb=10, idle_h=48, busy=False, branch=None):
    return {"name": name, "kind": kind, "branch": branch or (name.split("-", 1)[1] if "-" in name else "main"),
            "target_bytes": gb * GB, "last_used": NOW - idle_h * H, "busy": busy}


def disk(pct, size_gb=1000):
    used = size_gb * GB * pct / 100
    return {"total": size_gb * GB * 1.05, "used": used, "free": size_gb * GB - used}


class DecisionTest(unittest.TestCase):
    def test_levels(self):
        self.assertEqual([j.level_for(p) for p in (None, 50, 75, 75.1, 85.5, 92.1)], [0, 0, 0, 75, 85, 92])

    def test_pct_like_df(self):
        # used / (used + free): reserved blocks and other APFS volumes do not count
        self.assertEqual(j.disk_pct({"total": 1000, "used": 600, "free": 200}), 75.0)
        self.assertIsNone(j.disk_pct({}))

    def test_checkout_kind(self):
        self.assertEqual(j.checkout_kind("OCW", "OCW"), ("main", "main"))
        self.assertEqual(j.checkout_kind("OCW-worktree-agent-abc", "OCW"), ("agent", "worktree-agent-abc"))
        self.assertEqual(j.checkout_kind("OCW-bisect-123", "OCW"), ("other", "bisect-123"))
        self.assertEqual(j.checkout_kind("other", "OCW"), (None, None))

    def test_branch_verdict_like_remote_gc(self):
        branches = {"main", "worktree-agent-open", "worktree-agent-merged", "worktree-agent-live"}
        wt = {"main", "worktree-agent-live"}
        merged = {"main", "worktree-agent-merged", "worktree-agent-live"}
        v = lambda b: j.branch_verdict(b, branches, wt, merged)  # noqa: E731
        self.assertEqual(v("worktree-agent-gone"), "branch gone")
        self.assertEqual(v("worktree-agent-merged"), "merged into main, worktree removed")
        self.assertIsNone(v("worktree-agent-live"))  # a new agent's branch looks merged
        self.assertIsNone(v("worktree-agent-open"))
        self.assertIsNone(v("bisect-1"))

    def checkouts(self):
        return [co("OCW", "main", 30, 0.1), co("OCW-bisect-1", "other", 3, 500),
                co("OCW-worktree-agent-gone", gb=20, idle_h=3), co("OCW-worktree-agent-fresh", gb=15, idle_h=1),
                co("OCW-worktree-agent-old", gb=40, idle_h=72), co("OCW-worktree-agent-older", gb=50, idle_h=100),
                co("OCW-worktree-agent-young", gb=30, idle_h=5), co("OCW-worktree-agent-busy", gb=60, idle_h=200, busy=True)]

    verdicts = {"worktree-agent-gone": "branch gone", "worktree-agent-fresh": "branch gone"}

    def test_below_75_nothing(self):
        p = j.plan_machine(self.checkouts(), disk(70), NOW, self.verdicts)
        self.assertEqual((p["level"], p["actions"], p["alert"]), (0, [], False))

    def test_above_75_merged_or_gone_idle_2h_and_the_sweep(self):
        p = j.plan_machine(self.checkouts(), disk(80), NOW, self.verdicts, {"bytes": 5 * GB, "units": 12})
        self.assertEqual([(a["kind"], a["path"]) for a in p["actions"]],
                         [("target", "OCW-worktree-agent-gone/target"), ("sweep", "OCW/target")])
        self.assertIn("branch gone, unused for 3h", p["actions"][0]["reason"])
        kept = {c["name"]: c["reason"] for c in p["candidates"]["kept"]}
        self.assertIn("used 1h", kept["OCW-worktree-agent-fresh"])  # gone, but used within 2 h
        self.assertIn("in use", kept["OCW-worktree-agent-busy"])
        self.assertEqual(kept["OCW-bisect-1"], "not an agent checkout")

    def test_above_85_lru_oldest_first_until_below(self):
        # 88 % of 1000 GB: the gone branch frees 20 GB (86 %), not enough:
        # then the oldest idle > 24 h (100 h, 50 GB) – and then it is below.
        p = j.plan_machine(self.checkouts(), disk(88), NOW, self.verdicts)
        self.assertEqual([a["checkout"] for a in p["actions"]],
                         ["OCW-worktree-agent-gone", "OCW-worktree-agent-older"])
        self.assertEqual(p["actions"][1]["level"], 85)
        p = j.plan_machine(self.checkouts(), disk(93), NOW, self.verdicts)
        self.assertEqual([a["checkout"] for a in p["actions"]],
                         ["OCW-worktree-agent-gone", "OCW-worktree-agent-older", "OCW-worktree-agent-old"])
        self.assertTrue(p["alert"])
        # never: young (5 h), busy, the main checkout, other checkouts
        names = {a["checkout"] for a in p["actions"]}
        self.assertFalse(names & {"OCW-worktree-agent-young", "OCW-worktree-agent-busy", "OCW", "OCW-bisect-1"})

    def test_busy_main_checkout_is_not_swept(self):
        cs = self.checkouts()
        cs[0]["busy"] = True
        p = j.plan_machine(cs, disk(80), NOW, self.verdicts, {"bytes": 5 * GB, "units": 12})
        self.assertNotIn("sweep", [a["kind"] for a in p["actions"]])

    def test_safe_paths_only(self):
        self.assertTrue(j.safe_target("OCW-worktree-agent-x/target"))
        for bad in ("OCW/data", "../target", "OCW-x/target/..", "/target", "OCW x/target", "OCW-x/test-output"):
            self.assertFalse(j.safe_target(bad), bad)
        with self.assertRaises(ValueError):
            j.posix_delete("~/src/OCW", ["x; rm -rf ~"])
        with self.assertRaises(ValueError):
            j.windows_delete("B:\\src\\OCW", ["x'; Remove-Item C:\\"])

    def test_alerts(self):
        ms = [{"name": "a", "disks": [{"name": "C:", "pct": 93.0, "free": 10 * GB}, {"name": "B:", "pct": 50}]},
              {"name": "b", "disks": [{"name": "home", "pct": 92.0}]}]
        self.assertEqual([(a["machine"], a["disk"]) for a in j.disk_alerts(ms)], [("a", "C:")])


def touch(p: Path, age_s: float, size: int = 10):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(b"x" * size)
    t = time.time() - age_s
    os.utime(p, (t, t))


class SweepTest(unittest.TestCase):
    def unit(self, prof: Path, name: str, h: str, age: float, kind="lib"):
        touch(prof / ".fingerprint" / ("%s-%s" % (name, h)) / ("%s-%s" % (kind, name)), age)
        touch(prof / "deps" / ("lib%s-%s.rlib" % (name.replace("-", "_"), h)), age, 1000)
        touch(prof / "deps" / ("%s-%s.d" % (name.replace("-", "_"), h)), age)

    def test_old_replaced_units_and_old_incremental_go(self):
        day = 86400
        with tempfile.TemporaryDirectory() as d:
            t = Path(d) / "target"
            prof = t / "debug"
            self.unit(prof, "bevy_ecs", "a" * 16, 5 * day)       # old, replaced by the next one
            self.unit(prof, "bevy_ecs", "b" * 16, 0.1 * day)     # current
            self.unit(prof, "serde", "c" * 16, 5 * day)          # old but the only one: kept
            self.unit(prof, "ocw-sim", "d" * 16, 4 * day)        # old, replaced (test kind is another kind)
            self.unit(prof, "ocw-sim", "e" * 16, 1 * day)
            self.unit(prof, "ocw-sim", "f" * 16, 9 * day, kind="test-lib")  # the only test unit: kept
            touch(prof / "build" / ("bevy_ecs-%s" % ("a" * 16)) / "out" / "gen.rs", 5 * day)
            touch(prof / "incremental" / "ocw_sim-1x2y" / "s-old" / "dep-graph.bin", 6 * day)
            touch(prof / "incremental" / "ocw_sim-3z4w" / "s-new" / "dep-graph.bin", 0.5 * day)
            touch(prof / "ocw-desktop", 30 * day)                # uplifted binary: never
            wasm = t / "wasm32-unknown-unknown" / "debug"
            self.unit(wasm, "glam", "1" * 16, 5 * day)
            self.unit(wasm, "glam", "2" * 16, 2 * day)
            dry = j.sweep_units(str(t), time.time(), 3 * day, False)
            self.assertEqual((dry["units"], dry["incremental"], dry["deleted"]), (3, 1, False))
            self.assertTrue((prof / "deps" / ("libbevy_ecs-%s.rlib" % ("a" * 16))).exists(), "dry run deletes nothing")
            done = j.sweep_units(str(t), time.time(), 3 * day, True)
            self.assertEqual((done["units"], done["incremental"]), (3, 1))
            self.assertEqual(done["bytes"], dry["bytes"])
            gone = [prof / ".fingerprint" / ("bevy_ecs-" + "a" * 16), prof / "deps" / ("libbevy_ecs-%s.rlib" % ("a" * 16)),
                    prof / "build" / ("bevy_ecs-" + "a" * 16), prof / "deps" / ("libocw_sim-%s.rlib" % ("d" * 16)),
                    prof / "incremental" / "ocw_sim-1x2y", wasm / "deps" / ("libglam-%s.rlib" % ("1" * 16))]
            for p in gone:
                self.assertFalse(p.exists(), p)
            kept = [prof / "deps" / ("libbevy_ecs-%s.rlib" % ("b" * 16)), prof / "deps" / ("libserde-%s.rlib" % ("c" * 16)),
                    prof / "deps" / ("libocw_sim-%s.rlib" % ("f" * 16)), prof / "incremental" / "ocw_sim-3z4w",
                    prof / "ocw-desktop", wasm / "deps" / ("libglam-%s.rlib" % ("2" * 16))]
            for p in kept:
                self.assertTrue(p.exists(), p)

    def test_script_for_the_machine_is_self_contained(self):
        src = j.sweep_script()
        self.assertIn("def sweep_units", src)
        compile(src, "sweep", "exec")


class ParserTest(unittest.TestCase):
    def test_posix_inventory(self):
        text = ("@@disks\n/dev/sda2 1000 600 300 67% /home\n/dev/sda2 1000 600 300 67% /home\n@@checkouts\n"
                "2048 1700000000 OCW\n1024 1700000100 OCW-worktree-agent-x\n5 1 OCW-bisect-1\n9 9 junk-dir\n"
                "@@busy\nOCW-worktree-agent-x\n@@end\n")
        inv = j.parse_posix_inventory(text, "OCW")
        self.assertEqual(len(inv["disks"]), 1)  # same file system once
        self.assertEqual(inv["disk"]["used"], 600 * 1024)
        self.assertEqual([(c["name"], c["kind"], c["busy"]) for c in inv["checkouts"]],
                         [("OCW", "main", False), ("OCW-worktree-agent-x", "agent", True), ("OCW-bisect-1", "other", False)])
        self.assertEqual(inv["checkouts"][1]["target_bytes"], 1024 * 1024)

    def test_windows_inventory(self):
        text = "#< CLIXML\n" + json.dumps({
            "disks": [{"name": "C:", "total": 100, "used": 95, "free": 5}, {"name": "B:", "total": 1000, "used": 500, "free": 500}],
            "checkouts": {"name": "OCW-worktree-agent-y", "bytes": 4096, "last_used": 1700000000},
            "busy": "OCW-worktree-agent-y"})
        inv = j.parse_windows_inventory(text, "OCW")
        self.assertEqual(inv["disk"]["name"], "B:")
        self.assertEqual(inv["checkouts"][0]["busy"], True)

    def test_results_and_sweep(self):
        r = j.parse_results("@@deleted OCW-a 1024\n@@busy OCW-b\nnoise\n@@gone OCW-c\n@@failed OCW-d\n")
        self.assertEqual([(x["result"], x["checkout"], x["bytes"]) for x in r],
                         [("deleted", "OCW-a", 1024 * 1024), ("busy", "OCW-b", None), ("gone", "OCW-c", None),
                          ("failed", "OCW-d", None)])
        self.assertEqual(j.parse_sweep('x\n@@sweep {"bytes": 5, "units": 1}\n')["units"], 1)
        self.assertIsNone(j.parse_sweep("nothing"))

    def test_snippets_quote_paths(self):
        self.assertIn('r="$HOME"\'/src/OCW\'', j.posix_inventory("~/src/OCW"))
        self.assertIn("$r = 'B:\\src\\OCW'", j.windows_inventory("B:\\src\\OCW"))
        self.assertIn("'OCW-worktree-agent-x'", j.posix_delete("~/src/OCW", ["OCW-worktree-agent-x"]))


class FakeMachine:
    def __init__(self, name="m1", os_name="linux"):
        self.name, self.os, self.repo_path = name, os_name, "~/src/OCW"


class LoopTest(unittest.TestCase):
    def make(self, mode, pct, checkouts=None):
        self.clock = [NOW]
        self.calls = []
        inv_text = "@@disks\n/dev/x %d %d %d 0%% /\n@@checkouts\n" % (1000 * GB // 1024, pct * 10 * GB // 1024,
                                                                      (100 - pct) * 10 * GB // 1024)
        for c in checkouts or ["OCW-worktree-agent-gone"]:
            inv_text += "%d %d %s\n" % (20 * GB // 1024, NOW - 5 * H, c)
        inv_text += "@@busy\n@@end\n"

        def ssh(m, script, timeout, stdin=None):
            self.calls.append(script)
            if "@@checkouts" in script:
                return inv_text
            if "@@deleted" in script:
                return "@@deleted OCW-worktree-agent-gone %d\n@@end\n" % (20 * GB // 1024)
            return ""
        d = tempfile.mkdtemp()
        jan = j.Janitor([FakeMachine()], Path(d), Path(d) / "log", mode, ssh, clock=lambda: self.clock[0])
        jan.plan = lambda m, inv, now, with_sweep=True: j.plan_machine(
            inv["checkouts"], inv["disk"], now, {"worktree-agent-gone": "branch gone"})
        return jan, Path(d) / "log"

    def state(self, pct):
        return [{"name": "m1", "online": True, "disk_pct": pct}]

    def log_lines(self, root):
        return [json.loads(l) for p in root.glob("*/janitor.jsonl") for l in p.read_text().splitlines()]

    def test_on_deletes_logs_and_rate_limits(self):
        jan, log = self.make("on", 80)
        res = jan.tick(self.state(80))
        self.assertEqual(res["actions"][0]["result"], "deleted")
        self.assertEqual(res["freed"], 20 * GB)
        lines = self.log_lines(log)
        self.assertEqual([(e["action"], e["machine"], e["path"], e["bytes"]) for e in lines],
                         [("delete", "m1", "OCW-worktree-agent-gone/target", 20 * GB)])
        self.assertIn("branch gone", lines[0]["reason"])
        self.assertIsNone(jan.tick(self.state(80)), "at most one pass every 10 min")
        self.clock[0] += j.PASS_INTERVAL_S
        self.assertIsNotNone(jan.tick(self.state(80)))

    def test_dry_run_deletes_nothing_and_logs_a_plan_once(self):
        jan, log = self.make("dry-run", 80)
        res = jan.tick(self.state(80))
        self.assertEqual(res["actions"][0]["result"], "planned")
        self.assertFalse([c for c in self.calls if "@@deleted" in c], "no deletion script was sent")
        self.clock[0] += j.PASS_INTERVAL_S
        jan.tick(self.state(80))
        self.assertEqual([e["action"] for e in self.log_lines(log)], ["would-delete"], "an unchanged plan is logged once")

    def test_off_and_below_threshold_do_nothing(self):
        jan, _ = self.make("off", 95)
        self.assertIsNone(jan.tick(self.state(95)))
        jan, _ = self.make("on", 60)
        self.assertIsNone(jan.tick(self.state(60)))
        self.assertEqual(self.calls, [])

    def test_alert_is_logged(self):
        jan, log = self.make("on", 95)
        jan.tick(self.state(95))
        self.assertIn("alert", [e["action"] for e in self.log_lines(log)])

    def test_pick_fullest_then_least_recently_visited(self):
        jan, _ = self.make("on", 80)
        jan.machines = {n: FakeMachine(n) for n in ("a", "b", "c")}
        st = [{"name": "a", "online": True, "disk_pct": 80}, {"name": "b", "online": True, "disk_pct": 90},
              {"name": "c", "online": False, "disk_pct": 99}]
        self.assertEqual(jan.pick(st), "b")
        st[1]["disk_pct"] = 80
        jan.visited = {"a": NOW, "b": NOW - 100}
        self.assertEqual(jan.pick(st), "b")


if __name__ == "__main__":
    unittest.main()
