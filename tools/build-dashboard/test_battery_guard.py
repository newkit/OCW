"""Tests of the battery guard (battery_guard.py): hysteresis, the two-reading
confirmation, never re-enabling a machine disabled for another reason, no
change on unreachable / stale probes, the config keys and the CLI option,
the log lines and the card text (no network, no machines)."""

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import battery
import battery_guard as bg
import build_machines as bm
import dashboard as d
import test_build_machines as tbm
from test_build_machines import MACHINES, machines

HERE = Path(__file__).resolve().parent


def reading(pct):
    return {"percent": pct, "ac": False, "state": "battery", "rate_w": None, "time_left_s": None,
            "time_to_full_s": None, "source": "sysfs"}


class GuardTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.t = [1_800_000_000.0]
        self.bm = bm.BuildMachines(self.dir / "config" / bm.FILE_NAME, self.dir / "build-log", clock=lambda: self.t[0])
        self.ms = {m.name: m for m in machines()}
        self.leela = self.ms["leela"]
        self.bm.update(self.leela, battery_guard="20:80", by="tester")
        self.logged = []
        self.guard = bg.BatteryGuard(self.bm, list(self.ms.values()), log=self.logged.append)

    def tearDown(self):
        shutil.rmtree(self.dir)

    def feed(self, *pcts, ok=True):
        out = []
        for p in pcts:
            self.t[0] += 120
            out.append(self.guard.observe(self.leela, None if p is None else reading(p), ok, self.t[0]))
        return out

    def eff(self):
        return self.bm.effective(self.leela)

    def log_lines(self):
        return [json.loads(x) for f in sorted((self.dir / "build-log").glob("*/" + bm.LOG_NAME))
                for x in f.read_text().splitlines()]

    def test_below_stop_disables_with_reason(self):
        self.assertEqual(self.feed(19, 18), [None, "disabled"])
        e = self.eff()
        self.assertFalse(e["enabled"])
        self.assertEqual(e["reason"], "battery guard: 18 % (below 20 %) – building again above 80 %")
        self.assertEqual(e["disabled_by"], bm.GUARD)
        self.assertEqual((e["guard_last"]["action"], e["guard_last"]["percent"]), ("disabled", 18))
        text = self.bm.path.read_text()
        self.assertIn('disabled_by = "battery-guard"', text)
        self.assertNotIn("#", text.split("[[machine]]", 1)[1])
        last = self.log_lines()[-1]
        self.assertEqual((last["source"], last["kind"], last["guard"]["action"]), ("battery-guard", "battery-guard", "disabled"))
        self.assertEqual(last["changes"]["enabled"], [True, False])
        self.assertIn("leela disabled for building at 18 %", self.logged[-1])
        self.assertEqual(self.feed(10, 9), [None, None], "already disabled: nothing more")

    def test_above_start_reenables(self):
        self.feed(15, 15)
        self.assertEqual(self.feed(81, 85), [None, "enabled"])
        e = self.eff()
        self.assertTrue(e["enabled"])
        self.assertEqual((e["reason"], e["disabled_by"]), ("", ""))
        self.assertEqual((e["guard_last"]["action"], e["guard_last"]["percent"]), ("enabled", 85))
        self.assertEqual(self.log_lines()[-1]["guard"], {"action": "enabled", "percent": 85})
        self.assertIn("enabled for building again at 85 %", self.logged[-1])

    def test_between_unchanged(self):
        self.assertEqual(self.feed(50, 21, 79, 80, 20), [None] * 5)
        self.assertTrue(self.eff()["enabled"])
        self.feed(10, 10)
        self.assertEqual(self.feed(50, 79, 80, 60), [None] * 4)
        self.assertFalse(self.eff()["enabled"], "between the thresholds a guard-disabled machine stays off")

    def test_never_reenables_a_machine_disabled_for_another_reason(self):
        self.bm.update(self.leela, enabled=False, reason="no new jobs (user)", by="dashboard")
        self.assertEqual(self.feed(90, 95, 99), [None, None, None])
        self.assertEqual((self.eff()["enabled"], self.eff()["reason"]), (False, "no new jobs (user)"))
        # Low battery on an already disabled machine keeps the user's reason too.
        self.feed(5, 5)
        self.assertEqual(self.eff()["reason"], "no new jobs (user)")
        self.assertEqual(self.feed(90, 95), [None, None])

    def test_taken_over_by_someone_else(self):
        self.feed(10, 10)
        self.bm.update(self.leela, reason="coordinator: keep it off for the kiosk test", by="claude", source="cli")
        self.assertEqual(self.eff()["disabled_by"], "claude")
        self.assertEqual(self.feed(90, 90), [None, None])
        self.assertFalse(self.eff()["enabled"])

    def test_user_enables_then_guard_acts_again(self):
        self.feed(10, 10)
        self.bm.update(self.leela, enabled=True, by="dashboard")
        self.assertEqual(self.eff()["disabled_by"], "")
        self.assertEqual(self.feed(12, 12), [None, "disabled"])

    def test_unreachable_and_stale_change_nothing(self):
        self.assertEqual(self.feed(10, None, 10), [None, None, None], "a probe without a value breaks the streak")
        self.t[0] += 120
        self.assertIsNone(self.guard.observe(self.leela, None, False, self.t[0]), "unreachable breaks it too")
        self.assertEqual(self.feed(10), [None])
        # The same reading seen again (no new probe) is not a second reading.
        self.assertIsNone(self.guard.observe(self.leela, reading(10), True, self.t[0]))
        self.assertTrue(self.eff()["enabled"])
        self.assertEqual(self.feed(10), ["disabled"])
        self.assertIsNone(self.guard.observe(self.leela, None, False, self.t[0] + 1))
        self.assertIsNone(self.guard.observe(self.leela, {"percent": None}, True, self.t[0] + 2))
        self.assertFalse(self.eff()["enabled"], "unreachable never enables")

    def test_two_reading_confirmation(self):
        self.assertEqual(self.feed(19, 50, 19, 81, 19), [None] * 5, "never two in a row")
        self.assertTrue(self.eff()["enabled"])
        self.assertEqual(self.guard.pending("leela")["n"], 1)
        self.assertEqual(self.feed(19), ["disabled"])

    def test_no_guard_no_action(self):
        self.bm.update(self.leela, battery_guard="off")
        self.assertIsNone(self.eff()["battery_guard"])
        self.assertEqual(self.feed(1, 1, 1), [None] * 3)
        self.assertTrue(self.eff()["enabled"])
        self.assertEqual(self.guard.observe(self.ms["marge"], reading(1), True, 1), None)

    def test_read_only_dashboard_never_writes(self):
        self.bm.read_only = True
        self.assertEqual(self.feed(1, 1), [None, None])
        self.assertTrue(self.eff()["enabled"])

    def test_monitor_hook(self):
        """BatteryMonitor.poll hands each probe to the guard, failures as ok=False."""
        outs = []

        def run(m, script, timeout):
            v = outs.pop(0)
            if v is None:
                raise OSError("ssh: timeout")
            return "@@sysfs\nBAT0 type Battery\nBAT0 status Discharging\nBAT0 capacity %d\n@@end\n" % v

        mon = battery.BatteryMonitor([self.leela], run, clock=lambda: self.t[0])
        mon.on_poll = self.guard.safe_observe
        res = []
        for v in (10, None, 10, 10):
            outs.append(v)
            self.t[0] += 120
            mon.poll(self.leela)
            res.append(self.eff()["enabled"])
        self.assertEqual(res, [True, True, True, False])


class ConfigTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_parse_guard(self):
        self.assertEqual(bm.parse_guard("20:80"), {"stop": 20, "start": 80})
        self.assertEqual(bm.parse_guard(" 20 / 80 "), {"stop": 20, "start": 80})
        self.assertEqual(bm.parse_guard({"stop": 30, "start": 70}), {"stop": 30, "start": 70})
        for off in ("off", "", None, "OFF"):
            self.assertIsNone(bm.parse_guard(off))
        for bad in ("80:20", "20:20", "0:50", "20:100", "x", "20", 20, {"stop": "20", "start": 80}, "20:80; rm"):
            with self.assertRaises(ValueError, msg=bad):
                bm.parse_guard(bad)

    def test_file_round_trip_and_remote_sh_keys(self):
        text = ('[[machine]]\nname = "leela"\nenabled = false\nbattery_stop = 20\nbattery_start = 80\n'
                'disabled_by = "battery-guard"\nguard_action = "disabled"\nguard_at = "2026-11-18T21:00:00+01:00"\n'
                'guard_percent = 18\n[[machine]]\nname = "marge"\nbattery_stop = 90\nbattery_start = 10\n'
                'guard_action = "exploded"\n')
        e = bm.parse(text)
        self.assertEqual(e["leela"]["battery_guard"], {"stop": 20, "start": 80})
        self.assertEqual((e["leela"]["disabled_by"], e["leela"]["guard_percent"]), ("battery-guard", 18))
        self.assertIsNone(e["marge"]["battery_guard"], "a broken guard is dropped")
        self.assertEqual(e["marge"]["guard_action"], "")
        self.assertEqual(bm.parse(bm.render(e)), e)

    def test_cli_option(self):
        cfg = self.dir / "config"
        cfg.mkdir()
        (cfg / "machines.toml").write_text(MACHINES)
        run = lambda *a: subprocess.run(["python3", str(HERE / "build_machines.py"), "--config", str(cfg), "--log",  # noqa: E731
                                         str(self.dir / "log")] + list(a), capture_output=True, text=True, timeout=60)
        r = run("set", "leela", "--battery-guard", "20:80")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("battery guard 20 % / 80 %", next(l for l in r.stdout.splitlines() if l.startswith("leela")))
        text = (cfg / bm.FILE_NAME).read_text()
        self.assertIn("battery_stop = 20", text)
        self.assertIn("battery_start = 80", text)
        self.assertEqual(run("set", "leela", "--battery-guard", "80:20").returncode, 2)
        r = run("set", "leela", "--battery-guard", "off")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("battery_stop", (cfg / bm.FILE_NAME).read_text())
        self.assertNotIn("battery guard", r.stdout)
        # remote.sh's lookup still reads `enabled` from the file.
        run("set", "leela", "--battery-guard", "20:80", "--disable", "--reason", "test")
        out = subprocess.run(["bash", "-c", 'machine=leela; source <(sed -n "/^field() {/,/^}/p" "$1"); field enabled "$2"',
                              "_", str(HERE.parent.parent / "scripts" / "remote.sh"), str(cfg / bm.FILE_NAME)],
                             capture_output=True, text=True, timeout=30)
        self.assertEqual(out.stdout.strip(), "false", out.stderr)


class TextTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.t = [1_800_000_000.0]
        self.bm = bm.BuildMachines(self.dir / bm.FILE_NAME, None, clock=lambda: self.t[0])
        self.ms = {m.name: m for m in machines()}

    def tearDown(self):
        shutil.rmtree(self.dir)

    def test_status_and_text(self):
        leela = self.ms["leela"]
        self.bm.update(leela, battery_guard="20:80")
        guard = bg.BatteryGuard(self.bm, list(self.ms.values()), log=lambda s: None)
        guard.observe(leela, reading(15), True, 1)
        states = [dict(name=n, os=m.os, online=False, error="timeout", slots=m.slots) for n, m in self.ms.items()]
        d.apply_settings(states, list(self.ms.values()), self.bm, guard)
        st = {s["name"]: s for s in states}
        g = st["leela"]["battery_guard"]
        self.assertEqual((g["text"], g["active"], g["pending_text"]), ("battery guard 20 % / 80 %", False,
                                                                       "1 of 2 readings below 20 %"))
        self.assertNotIn("battery_guard", st["marge"])
        guard.observe(leela, reading(14), True, 2)
        d.apply_settings(states, list(self.ms.values()), self.bm, guard)
        g = states[[s["name"] for s in states].index("leela")]["battery_guard"]
        self.assertTrue(g["active"])
        self.assertRegex(g["last_text"], r"^disabled building at 14 %, \d{4}-\d\d-\d\d \d\d:\d\d$")
        text = d.format_text(d.summarize(states))
        self.assertIn("battery guard 20 % / 80 % (last: disabled building at 14 %", text)
        self.assertIn("build: DISABLED (battery guard: 14 % (below 20 %) – building again above 80 %)", text)


class GuardCardTest(tbm.CardTest):
    """The card shows the guard ("battery guard 20 % / 80 %", when it acted)."""

    def test_disabled_card_escaped_with_controls(self):  # covered in test_build_machines
        pass

    def test_without_settings_no_buttons(self):
        pass

    def test_guard_on_card(self):
        g = {"stop": 20, "start": 80, "text": "battery guard 20 % / 80 %", "active": True,
             "last": {"action": "disabled", "at": "2026-11-18T21:04:00+01:00", "percent": 18},
             "last_text": "disabled building at 18 %, 2026-11-18 21:04", "pending_text": ""}
        html = self.render([self.machine(battery_guard=g, disabled_reason=bg.reason_text(18, 20, 80),
                                         disabled_by="battery-guard", note="")])
        self.assertIn("battery guard 20 % / 80 %", html)
        self.assertIn("last: disabled building at 18 %, 2026-11-18 21:04", html)
        self.assertIn('class="mnote guard on"', html)
        self.assertIn("battery guard: 18 % (below 20 %) – building again above 80 %", html)
        self.assertIn('data-act="guard"', html)
        html = self.render([self.machine(name="marge", enabled=True, note="")])
        self.assertNotIn("mnote guard", html)
        self.assertNotIn('data-act="guard"', html, "no battery, no guard: no button")


if __name__ == "__main__":
    unittest.main()
