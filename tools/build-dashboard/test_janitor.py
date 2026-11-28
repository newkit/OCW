"""Tests of janitor.py: the disk levels and what a pass does on top of the
cleanup rules (test_cleanup_rules.py tests the rules), the sweep of old
build units, the schedule (daily routine pass, disk pressure, kill switch)
and the pass loop with a fake machine (no network)."""

import json
import os
import tempfile
import time
import unittest
from pathlib import Path

import cleanup_rules as rules
import janitor as j

GB = 10 ** 9
NOW = 1_800_000_000.0
H = 3600
LIM = rules.limits({})
FACTS = rules.make_facts(["main", "worktree-agent-open", "worktree-agent-older", "worktree-agent-old",
                          "worktree-agent-young", "worktree-agent-busy", "bisect-1"], merged=["main"],
                         worktrees=["main"])


def co(name, gb=10, idle_h=48, busy=False):
    cls, suffix = rules.classify(name, "OCW")
    return {"name": name, "cls": cls, "suffix": suffix, "target_bytes": gb * GB, "out_bytes": 0, "other_bytes": 0,
            "sync": NOW - idle_h * H, "used": 0, "mtime": NOW - idle_h * H, "busy": busy, "strays": []}


def disk(pct, size_gb=1000):
    used = size_gb * GB * pct / 100
    return {"total": size_gb * GB * 1.05, "used": used, "free": size_gb * GB - used}


class DecisionTest(unittest.TestCase):
    def test_levels(self):
        self.assertEqual([j.level_for(p) for p in (None, 50, 75, 75.1, 85.5, 92.1)], [0, 0, 0, 75, 85, 92])
        # short of free space counts like above 75 %
        self.assertEqual(j.level_for(50, 30 * GB, 40 * GB), 75)
        self.assertEqual(j.level_for(50, 50 * GB, 40 * GB), 0)
        self.assertEqual(j.level_for(90, 30 * GB, 40 * GB), 85)

    def test_pct_like_df(self):
        # used / (used + free): reserved blocks and other APFS volumes do not count
        self.assertEqual(j.disk_pct({"total": 1000, "used": 600, "free": 200}), 75.0)
        self.assertIsNone(j.disk_pct({}))

    def inv(self, pct, busy_main=False):
        return {"now": NOW, "disk": disk(pct), "checkouts": [
            co("OCW", 30, 0.1, busy=busy_main), co("OCW-bisect-1", 3, 500),
            co("OCW-worktree-agent-gone", gb=20, idle_h=3), co("OCW-worktree-agent-old", gb=40, idle_h=72),
            co("OCW-worktree-agent-older", gb=50, idle_h=100), co("OCW-worktree-agent-young", gb=30, idle_h=5),
            co("OCW-worktree-agent-busy", gb=60, idle_h=200, busy=True)]}

    def test_below_75_only_the_rules(self):
        p = j.plan_machine(self.inv(70), FACTS, NOW, LIM, {"bytes": 5 * GB, "units": 12})
        self.assertEqual((p["level"], p["alert"]), (0, False))
        self.assertEqual([(a["kind"], a["path"], a["level"]) for a in p["actions"]],
                         [("target", "OCW-worktree-agent-gone/target", 0)])
        self.assertEqual(p["actions"][0]["reason"], "branch gone")

    def test_above_75_also_the_sweep(self):
        p = j.plan_machine(self.inv(80), FACTS, NOW, LIM, {"bytes": 5 * GB, "units": 12})
        self.assertEqual([(a["kind"], a["path"]) for a in p["actions"]],
                         [("target", "OCW-worktree-agent-gone/target"), ("sweep", "OCW/target")])

    def test_above_85_lru_oldest_first_until_below(self):
        # 88 % of 1000 GB: the gone branch frees 20 GB (86 %), not enough:
        # then the oldest idle > 24 h (the open branch bisect-1, 500 h, 3 GB),
        # then the next (100 h, 50 GB) – and then it is below.
        p = j.plan_machine(self.inv(88), FACTS, NOW, LIM)
        self.assertEqual([a["path"].split("/")[0] for a in p["actions"]],
                         ["OCW-worktree-agent-gone", "OCW-bisect-1", "OCW-worktree-agent-older"])
        self.assertEqual(p["actions"][2]["level"], 85)
        self.assertIn("least recently used, idle for 4d 4h (branch open)", p["actions"][2]["reason"])
        p = j.plan_machine(self.inv(93), FACTS, NOW, LIM)
        self.assertEqual([a["path"].split("/")[0] for a in p["actions"]],
                         ["OCW-worktree-agent-gone", "OCW-bisect-1", "OCW-worktree-agent-older", "OCW-worktree-agent-old"])
        self.assertTrue(p["alert"])
        # never: young (5 h), busy, the main checkout
        names = {a["path"].split("/")[0] for a in p["actions"]}
        self.assertFalse(names & {"OCW-worktree-agent-young", "OCW-worktree-agent-busy", "OCW"})

    def test_busy_main_checkout_is_not_swept(self):
        p = j.plan_machine(self.inv(80, busy_main=True), FACTS, NOW, LIM, {"bytes": 5 * GB, "units": 12})
        self.assertNotIn("sweep", [a["kind"] for a in p["actions"]])

    def test_alerts(self):
        ms = [{"name": "a", "disks": [{"name": "C:", "pct": 93.0, "free": 10 * GB}, {"name": "B:", "pct": 50}]},
              {"name": "b", "disks": [{"name": "home", "pct": 92.0}]}]
        self.assertEqual([(a["machine"], a["disk"]) for a in j.disk_alerts(ms)], [("a", "C:")])

    def test_kill_switch_from_the_environment(self):
        self.assertEqual([j.default_mode({"OCW_JANITOR": v}) for v in ("off", "0", "dry-run", "dry", "on", "", "what")],
                         ["off", "off", "dry-run", "dry-run", "on", "on", "on"])
        self.assertEqual(j.default_mode({}), "on")


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


class FakeMachine:
    def __init__(self, name="m1", os_name="linux"):
        self.name, self.os, self.repo_path = name, os_name, "~/src/OCW"


class LoopTest(unittest.TestCase):
    def make(self, mode, pct, checkouts=None, machines=None, **kw):
        self.clock = [NOW]
        self.calls = []
        self.stdin = []
        inv_text = "@@now %d\n@@disks\n/dev/x %d %d %d 0%% /\n@@checkouts\n" % (
            NOW, 1000 * GB // 1024, pct * 10 * GB // 1024, (100 - pct) * 10 * GB // 1024)
        for c in checkouts or ["OCW-worktree-agent-gone"]:
            inv_text += "c %d %d 0 %d 0 0 0 %s\n" % (21 * GB // 1024, 20 * GB // 1024, NOW - 5 * H, c)
        inv_text += "@@busy\n@@end\n"

        def ssh(m, script, timeout, stdin=None):
            self.calls.append(script)
            if "@@checkouts" in script:
                return inv_text
            if "@@deleted" in script:
                self.stdin.append(stdin)
                return "@@deleted target OCW-worktree-agent-gone %d\n@@end\n" % (20 * GB // 1024)
            return ""
        d = tempfile.mkdtemp()
        self.dir = Path(d)
        jan = j.Janitor(machines or [FakeMachine()], Path(d), Path(d) / "log", mode, ssh, clock=lambda: self.clock[0],
                        limits=LIM, **kw)
        jan.facts = lambda: rules.make_facts(["main"], merged=["main"])
        self.ssh = ssh
        return jan, Path(d) / "log"

    def state(self, pct, **kw):
        return [dict({"name": "m1", "online": True, "disk_pct": pct}, **kw)]

    def log_lines(self, root):
        return [json.loads(l) for p in root.glob("*/janitor.jsonl") for l in p.read_text().splitlines()]

    def test_on_deletes_logs_and_rate_limits(self):
        jan, log = self.make("on", 80)
        res = jan.tick(self.state(80))
        self.assertEqual((res["actions"][0]["result"], res["why"], res["count"]), ("deleted", "disk", 1))
        self.assertEqual(res["freed"], 20 * GB // 1024 * 1024)
        self.assertEqual(self.stdin, ["target OCW-worktree-agent-gone\n"])  # the names go by stdin
        lines = self.log_lines(log)
        self.assertEqual([(e["action"], e["machine"], e["path"], e["kind"], e["class"]) for e in lines],
                         [("delete", "m1", "OCW-worktree-agent-gone/target", "target", "agent")])
        self.assertIn("branch gone", lines[0]["reason"])
        self.assertIsNone(jan.tick(self.state(80)), "at most one pass every 10 min")
        self.clock[0] += j.PASS_INTERVAL_S
        self.assertIsNone(jan.tick(self.state(80)), "a full disk: one pass an hour per machine")
        self.clock[0] += j.PRESSURE_INTERVAL_S
        self.assertIsNotNone(jan.tick(self.state(80)))

    def test_dry_run_deletes_nothing_and_logs_a_plan_once(self):
        jan, log = self.make("dry-run", 80)
        res = jan.tick(self.state(80))
        self.assertEqual((res["actions"][0]["result"], res["count"], res["bytes"], res["freed"]),
                         ("planned", 1, 20 * GB // 1024 * 1024, 0))
        self.assertFalse([c for c in self.calls if "@@deleted" in c], "no deletion script was sent")
        self.clock[0] += j.PRESSURE_INTERVAL_S
        jan.tick(self.state(80))
        self.assertEqual([e["action"] for e in self.log_lines(log)], ["would-delete"], "an unchanged plan is logged once")

    def test_off_does_nothing(self):
        jan, _ = self.make("off", 95)
        self.assertIsNone(jan.tick(self.state(95)))
        self.assertEqual(self.calls, [])

    def test_routine_pass_once_a_day_below_the_thresholds(self):
        jan, log = self.make("on", 60)
        res = jan.tick(self.state(60))
        self.assertEqual((res["why"], res["count"], res["level"]), ("routine", 1, 0))
        self.assertEqual(len(self.log_lines(log)), 1)
        self.clock[0] += 23 * H
        self.assertIsNone(jan.tick(self.state(60)), "not again within a day")
        self.clock[0] += 2 * H
        self.assertEqual(jan.tick(self.state(60))["why"], "routine")

    def test_routine_pass_only_on_enabled_reachable_machines(self):
        jan, _ = self.make("on", 60)
        self.assertIsNone(jan.tick(self.state(60, enabled=False)))
        self.assertIsNone(jan.tick([{"name": "m1", "online": False, "disk_pct": 60}]))
        self.assertEqual(self.calls, [])
        # … but a full disk is cleaned on a disabled machine too
        self.assertEqual(jan.tick(self.state(80, enabled=False))["why"], "disk")

    def test_routine_can_be_switched_off(self):
        jan, _ = self.make("on", 60, routine_s=0)
        self.assertIsNone(jan.tick(self.state(60)))
        self.assertEqual(jan.tick(self.state(80))["why"], "disk")

    def test_low_free_space_counts_as_pressure(self):
        jan, _ = self.make("on", 60, routine_s=0, free_min=40 * GB)
        st = self.state(60, disks=[{"name": "home", "role": "home, checkouts", "free": 30 * GB}])
        self.assertEqual(jan.tick(st)["why"], "disk")

    def test_the_last_pass_survives_a_restart(self):
        jan, log = self.make("on", 60)
        jan.tick(self.state(60))
        state = json.loads((log / j.STATE_FILE).read_text())
        self.assertEqual(state["machines"]["m1"]["count"], 1)
        again = j.Janitor([FakeMachine()], self.dir, log, "on", self.ssh, clock=lambda: self.clock[0], limits=LIM)
        self.assertEqual(again.results["m1"]["freed"], 20 * GB // 1024 * 1024)
        self.assertEqual(again.pick(self.state(60)), (None, None), "no second routine pass after a restart")
        st = again.status([])
        self.assertEqual(st["machines"]["m1"]["why"], "routine")
        self.assertEqual(st["next_routine"]["m1"], NOW + 24 * H)
        self.assertEqual(st["routine_h"], 24.0)

    def test_alert_is_logged(self):
        jan, log = self.make("on", 95)
        jan.tick(self.state(95))
        self.assertIn("alert", [e["action"] for e in self.log_lines(log)])

    def test_unreachable_machine_is_an_error_not_a_crash(self):
        jan, _ = self.make("on", 60)

        def down(m, script, timeout, stdin=None):
            raise RuntimeError("timeout after 5 s")
        jan.ssh = down
        res = jan.tick(self.state(60))
        self.assertIn("timeout", res["error"])

    def test_pick_fullest_first_then_the_longest_waiting_routine(self):
        ms = [FakeMachine(n) for n in ("a", "b", "c", "d")]
        jan, _ = self.make("on", 80, machines=ms)
        st = [{"name": "a", "online": True, "disk_pct": 80}, {"name": "b", "online": True, "disk_pct": 90},
              {"name": "c", "online": False, "disk_pct": 99}, {"name": "d", "online": True, "disk_pct": 10}]
        self.assertEqual(jan.pick(st), ("b", "disk"))
        jan.visited = {"b": NOW}                       # b just had its pass: the next full one
        self.assertEqual(jan.pick(st), ("a", "disk"))
        jan.visited = {"a": NOW, "b": NOW}             # then the routine ones
        self.assertEqual(jan.pick(st), ("d", "routine"))
        jan.visited = {"a": NOW, "b": NOW, "d": NOW}
        self.assertEqual(jan.pick(st), (None, None))

    def test_windows_ops_in_chunks(self):
        m = FakeMachine("w", "windows")
        m.repo_path = "B:\\src\\OCW"
        sent = []

        def ssh(mm, script, timeout, stdin=None):
            sent.append(script)
            return ""
        ops = ["target|OCW-worktree-agent-%d" % i for i in range(90)]
        res = j.run_ops(ssh, m, ops + ["target|OCW-HEAD", "target|OCW"])
        self.assertEqual(len(sent), 3)
        self.assertEqual(len(res), 90)                 # the unsafe ops never left
        self.assertEqual(res[ops[0]]["result"], "failed")  # no answer = failed


if __name__ == "__main__":
    unittest.main()
