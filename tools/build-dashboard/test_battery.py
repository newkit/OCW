"""Unit tests for the battery probes (laptops, Meta Quest on Link); no network.

    python3 -m unittest discover tools/build-dashboard
"""

import base64
import json
import struct
import sys
import unittest
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import battery as b  # noqa: E402
import dashboard as d  # noqa: E402

# Weela on 2026-11-17, charging at ~10 W on mains: GetSystemPowerStatus and
# SYSTEM_BATTERY_STATE as the probe returns them (charging and discharging
# flags both set, the rate decides).
WEELA_SPS = base64.b64decode("AQguAP//////////")
WEELA_SBS = base64.b64decode("AQEBAQAAAAOxJwEA9ogAAO8mAAD/////tQgAAAYdAAA=")


def sps(ac=1, flag=0, pct=50, life=-1):
    return struct.pack("<BBBBii", ac, flag, pct, 0, life, -1)


def sbs(ac=1, present=1, charging=0, discharging=0, mx=75000, rem=37500, rate=0, est=0xFFFFFFFF):
    return bytes([ac, present, charging, discharging, 0, 0, 0, 0]) + struct.pack("<IIiIII", mx, rem, rate, est, 0, 0)


class WindowsPower(unittest.TestCase):
    def test_weela_charging(self):
        r = b.parse_windows_power(WEELA_SPS, WEELA_SBS)
        self.assertEqual(r["percent"], 46.0)
        self.assertTrue(r["ac"])
        self.assertEqual(r["state"], "charging")
        self.assertAlmostEqual(r["rate_w"], 9.967, places=2)
        # (75697 - 35062) mWh at 9967 mW
        self.assertAlmostEqual(r["time_to_full_s"], int((75697 - 35062) / 9967 * 3600), delta=1)
        self.assertIsNone(r["time_left_s"])

    def test_mains_but_discharging(self):
        r = b.parse_windows_power(sps(ac=1, flag=0, pct=38), sbs(ac=1, discharging=1, rem=28500, rate=-12000))
        self.assertEqual(r["state"], "mains_discharging")
        self.assertEqual(r["rate_w"], -12.0)
        self.assertEqual(r["time_left_s"], int(28500 / 12000 * 3600))
        self.assertEqual(b.level(r["percent"], r["state"]), "warn")
        self.assertIn("power supply", b.warning_text(r))
        self.assertIn("on mains but discharging", b.battery_text(r))

    def test_mains_discharging_from_flags_without_rate(self):
        r = b.parse_windows_power(sps(ac=1, pct=70), sbs(ac=1, discharging=1, rate=-0x80000000))
        self.assertEqual(r["state"], "mains_discharging")
        self.assertIsNone(r["rate_w"])

    def test_on_battery_uses_windows_estimate(self):
        r = b.parse_windows_power(sps(ac=0, pct=15), sbs(ac=0, discharging=1, rate=-20000, est=1800))
        self.assertEqual(r["state"], "battery")
        self.assertEqual(r["time_left_s"], 1800)
        self.assertEqual(b.level(r["percent"], r["state"]), "bad")
        self.assertEqual(b.warning_text(r), "Battery low")

    def test_only_power_status(self):
        r = b.parse_windows_power(sps(ac=0, pct=55, life=5400), None)
        self.assertEqual((r["state"], r["time_left_s"], r["rate_w"]), ("battery", 5400, None))
        r = b.parse_windows_power(sps(ac=1, flag=8, pct=55), None)
        self.assertEqual(r["state"], "charging")

    def test_full_on_mains(self):
        r = b.parse_windows_power(sps(ac=1, pct=100), sbs(rate=0, rem=75000))
        self.assertEqual(r["state"], "full")

    def test_desktop_without_battery(self):
        self.assertIsNone(b.parse_windows_power(sps(ac=1, flag=128, pct=255), None))
        self.assertIsNone(b.parse_windows_power(sps(ac=1, flag=255, pct=255), None))
        self.assertIsNone(b.parse_windows_power(sps(), sbs(present=0)))
        self.assertIsNone(b.parse_windows_power(None, None))


SYSFS_LAPTOP = """\
AC type Mains
AC online 1
BAT0 type Battery
BAT0 status Discharging
BAT0 capacity 35
BAT0 energy_now 20000000
BAT0 energy_full 57000000
BAT0 power_now 8000000
"""

# marge: a desktop with a Logitech mouse (HID++ battery, scope Device).
SYSFS_DESKTOP_MOUSE = """\
hidpp_battery_21 type Battery
hidpp_battery_21 scope Device
hidpp_battery_21 status Discharging
hidpp_battery_21 online 1
"""


class Sysfs(unittest.TestCase):
    def test_discharging_on_mains(self):
        r = b.parse_sysfs(SYSFS_LAPTOP.splitlines())
        self.assertEqual((r["percent"], r["ac"], r["state"]), (35.0, True, "mains_discharging"))
        self.assertEqual(r["rate_w"], -8.0)
        self.assertEqual(r["time_left_s"], int(20 / 8 * 3600))
        self.assertEqual(b.level(r["percent"], r["state"]), "warn")

    def test_on_battery_and_charging(self):
        r = b.parse_sysfs(SYSFS_LAPTOP.replace("AC online 1", "AC online 0").splitlines())
        self.assertEqual(r["state"], "battery")
        r = b.parse_sysfs(SYSFS_LAPTOP.replace("Discharging", "Charging").splitlines())
        self.assertEqual(r["state"], "charging")
        self.assertEqual(r["time_to_full_s"], int(37 / 8 * 3600))
        self.assertEqual(r["rate_w"], 8.0)

    def test_charge_and_current_instead_of_energy(self):
        lines = ["BAT1 type Battery", "BAT1 status Full", "BAT1 charge_now 4000000", "BAT1 charge_full 4000000"]
        r = b.parse_sysfs(lines)
        self.assertEqual((r["percent"], r["state"]), (100.0, "full"))

    def test_peripheral_battery_is_not_the_machines(self):
        self.assertIsNone(b.parse_sysfs(SYSFS_DESKTOP_MOUSE.splitlines()))
        self.assertIsNone(b.parse_sysfs([]))

    def test_posix_output(self):
        out = "@@now\n1794956000\n@@sysfs\n" + SYSFS_LAPTOP + "@@end\n"
        self.assertEqual(b.parse_output("linux", out)["battery"]["state"], "mains_discharging")
        out = "@@now\n1\n@@sysfs\n" + SYSFS_DESKTOP_MOUSE + "@@end\n"
        self.assertIsNone(b.parse_output("linux", out)["battery"])
        with self.assertRaises(ValueError):
            b.parse_output("linux", "@@now\n1\n@@sysfs\n")  # cut off


class Pmset(unittest.TestCase):
    def test_variants(self):
        r = b.parse_pmset(["Now drawing from 'AC Power'",
                           " -InternalBattery-0 (id=4653155)\t85%; charging; 1:23 remaining present: true"])
        self.assertEqual((r["percent"], r["state"], r["time_to_full_s"]), (85.0, "charging", 83 * 60))
        r = b.parse_pmset(["Now drawing from 'Battery Power'",
                           " -InternalBattery-0 (id=1)\t33%; discharging; 2:05 remaining present: true"])
        self.assertEqual((r["ac"], r["state"], r["time_left_s"]), (False, "battery", 125 * 60))
        self.assertEqual(b.level(r["percent"], r["state"]), "warn")
        r = b.parse_pmset(["Now drawing from 'AC Power'",
                           " -InternalBattery-0 (id=1)\t60%; discharging; (no estimate) present: true"])
        self.assertEqual(r["state"], "mains_discharging")
        r = b.parse_pmset(["Now drawing from 'AC Power'", " -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true"])
        self.assertEqual(r["state"], "full")
        r = b.parse_pmset(["Now drawing from 'AC Power'", " -InternalBattery-0 (id=1)\t80%; AC attached; not charging present: true"])
        self.assertEqual(r["state"], "not_charging")

    def test_mac_mini(self):
        self.assertIsNone(b.parse_pmset(["Now drawing from 'AC Power'"]))
        out = "@@now\n1\n@@pmset\nNow drawing from 'AC Power'\n@@end\n"
        self.assertIsNone(b.parse_output("macos", out)["battery"])


# Lines of the Meta runtime's Service log (17/11 is day/month, Weela's local time).
QUEST_LINES = [
    "17/11 23:20:43.100 [   45640] {INFO}    [xrstreaming] [V3] [commlib] [winusb detector] device with [vid: 10291, pid: 395, added]",
    "17/11 23:20:50.823 [   29348] {INFO}    [RemoteHeadset] Streaming session got invited: encryption supported in invitation: false",
    "17/11 23:20:51.541 [   29348] {INFO}    [RemoteHeadset] HMD battery level update: from -2 to 88",
    "17/11 23:21:09.169 [   45640] {INFO}    [xrstreaming] <!--OVRServer::Streaming:HMD Startup time first displayed frame is 6.1 seconds",
    "17/11 23:44:31.922 [   29348] {INFO}    [RemoteHeadset] HMD battery level update: from 96 to 97",
    "17/11 23:47:55.139 [   29348] {INFO}    [Input] Battery level for ovrControllerType_LTouch changed to 90",
    "17/11 23:47:55.139 [   29348] {INFO}    [Input] Battery level for ovrControllerType_RTouch changed to 85",
]
LOCAL_NOW = "2026-11-17T23:50:00"
UTC_NOW = 1794955800.0  # = LOCAL_NOW in Weela's zone (UTC+1)


def quest(lines=QUEST_LINES, battery=95, connection="connected", last_seen=1794954000):
    return {"cache_mtime": last_seen, "log_mtime": UTC_NOW,
            "headsets": [{"subtype": "quest", "battery": battery, "connection": connection, "power": "active",
                          "airlink": False, "last_seen": last_seen, "operational": "operable"}],
            "lines": lines}


class Quest(unittest.TestCase):
    def test_log_time(self):
        lnow = datetime(2026, 11, 17, 23, 50, 0)
        self.assertEqual(b.log_time(QUEST_LINES[4], lnow, UTC_NOW), UTC_NOW - (5 * 60 + 29.0))
        self.assertIsNone(b.log_time("no time here", lnow, UTC_NOW))
        # December lines read in January belong to last year
        t = b.log_time("31/12 23:59:00.000 x", datetime(2027, 1, 1, 0, 1, 0), 1000000.0)
        self.assertEqual(t, 1000000.0 - 120)

    def test_streaming_charging(self):
        q = b.parse_quest(quest(), UTC_NOW, LOCAL_NOW)
        self.assertEqual((q["battery"], q["battery_source"], q["trend"]), (97.0, "log", "charging"))
        self.assertEqual(q["battery_at"], UTC_NOW - 329)
        self.assertEqual((q["usb"], q["link"]), (True, "streaming"))
        self.assertEqual(q["controllers"], {"left": 90, "right": 85})
        self.assertIsNone(q["mounted"])  # not in the runtime's files
        self.assertEqual(b.quest_text(q, UTC_NOW), "97 % charging · Link streaming (USB) · Touch L 90 % R 85 %")
        self.assertEqual(b.level(q["battery"]), "ok")

    def test_discharging_and_low(self):
        lines = QUEST_LINES[:4] + ["17/11 23:49:00.000 [ 1] {INFO}    [RemoteHeadset] HMD battery level update: from 19 to 18"]
        q = b.parse_quest(quest(lines), UTC_NOW, LOCAL_NOW)
        self.assertEqual((q["battery"], q["trend"]), (18.0, "discharging"))
        self.assertEqual(b.level(q["battery"]), "bad")
        lines[-1] = lines[-1].replace("from 19 to 18", "from 36 to 35")
        self.assertEqual(b.level(b.parse_quest(quest(lines), UTC_NOW, LOCAL_NOW)["battery"]), "warn")

    def test_link_stopped_and_unplugged(self):
        lines = QUEST_LINES + [
            "17/11 23:48:32.532 [ 1] {INFO}    [RemoteHeadset] Received request to stop Link",
            "17/11 23:48:32.543 [ 1] {INFO}    [RemoteHeadset] RemoteHeadsetPlugin::didDisconnect reset State",
            "17/11 23:48:32.700 [ 1] {INFO}    [xrstreaming] [V3] [commlib] [winusb detector] device with [vid: 10291, pid: 395, removed]",
            "17/11 23:48:59.066 [ 1] {INFO}    [Input] Battery level for ovrControllerType_LTouch changed to -2",
        ]
        q = b.parse_quest(quest(lines, connection="disconnected"), UTC_NOW, LOCAL_NOW)
        self.assertEqual((q["usb"], q["link"]), (False, "stopped"))
        self.assertEqual(q["controllers"], {"right": 85})
        self.assertEqual(b.quest_link_text(q), "not connected")
        # not connected: the level is shown with its age, without a trend
        self.assertEqual(b.quest_text(q, UTC_NOW).split(" · ")[:2], ["97 %", "last report 5 min ago"])

    def test_on_usb_without_link(self):
        q = b.parse_quest(quest(QUEST_LINES[:1], battery="unknown", connection="disconnected"), UTC_NOW, LOCAL_NOW)
        self.assertIsNone(q["battery"])
        self.assertEqual(b.quest_link_text(q), "on USB, Link not running")
        self.assertTrue(b.quest_text(q, UTC_NOW).startswith("battery not reported"))
        self.assertEqual(b.level(q["battery"]), "unknown")

    def test_cache_only_and_newer_cache(self):
        q = b.parse_quest(quest([], battery=97, connection="disconnected", last_seen=UTC_NOW - 60), UTC_NOW, LOCAL_NOW)
        self.assertEqual((q["battery"], q["battery_source"], q["battery_at"]), (97.0, "cache", UTC_NOW - 60))
        # a cache entry written after the last log line wins
        q = b.parse_quest(quest(QUEST_LINES[:3], battery=93, last_seen=UTC_NOW - 60), UTC_NOW, LOCAL_NOW)
        self.assertEqual((q["battery"], q["battery_source"]), (93.0, "cache"))

    def test_air_link_and_cache_connected(self):
        q = b.parse_quest(dict(quest([]), headsets=[{"battery": 50, "connection": "connected", "airlink": True}]),
                          UTC_NOW, LOCAL_NOW)
        self.assertEqual(b.quest_link_text(q), "Link connected (Air Link)")

    def test_bad_input(self):
        self.assertIsNone(b.parse_quest(None, UTC_NOW, LOCAL_NOW))
        q = b.parse_quest({"headsets": "x", "lines": [None, 3]}, UTC_NOW, "garbage")
        self.assertIsNone(q["battery"])

    def test_windows_output(self):
        out = "#< CLIXML\n" + json.dumps({"now": UTC_NOW, "local": LOCAL_NOW,
                                          "sps": base64.b64encode(WEELA_SPS).decode(),
                                          "sbs": base64.b64encode(WEELA_SBS).decode(), "quest": quest()}) + "\n<Objs/>"
        p = b.parse_output("windows", out)
        self.assertEqual(p["battery"]["state"], "charging")
        self.assertEqual(p["quest"]["battery"], 97.0)
        # Win11Builder: a desktop, no Meta runtime
        p = b.parse_output("windows", json.dumps({"now": 1, "sps": base64.b64encode(sps(flag=128, pct=255)).decode()}))
        self.assertEqual((p["battery"], p["quest"]), (None, None))

    def test_scripts_read_only(self):
        for s in (b.WINDOWS_SCRIPT, b.POSIX_SCRIPT):
            for bad in ("Stop-Process", "Start-Process", "schtasks", "Remove-Item", "Set-Content", "Out-File", "kill ",
                        "adb", "OculusDebugTool", "rm ", "Add-Type"):
                self.assertNotIn(bad, s)
        self.assertIn("'Read', 'ReadWrite,Delete'", b.WINDOWS_SCRIPT)  # shared read, the runtime keeps writing
        self.assertIn(b.QUEST_LOG_RX, b.WINDOWS_SCRIPT)


class Levels(unittest.TestCase):
    def test_thresholds(self):
        self.assertEqual(b.level(19.9), "bad")
        self.assertEqual(b.level(20), "warn")
        self.assertEqual(b.level(39.9), "warn")
        self.assertEqual(b.level(40), "ok")
        self.assertEqual(b.level(90, "mains_discharging"), "warn")
        self.assertEqual(b.level(10, "mains_discharging"), "bad")
        self.assertEqual(b.level(None), "unknown")

    def test_texts(self):
        r = b.parse_windows_power(WEELA_SPS, WEELA_SBS)
        self.assertEqual(b.battery_text(r), "46 % · charging · +10.0 W · full in 4 h 04 min")
        self.assertIsNone(b.warning_text(r))
        self.assertIsNone(b.warning_text(dict(r, percent=10)))  # low but charging


class Machine:
    def __init__(self, name, os_name="windows"):
        self.name, self.os = name, os_name


class Monitor(unittest.TestCase):
    def setUp(self):
        self.now = 1794950000.0
        self.out = {}
        self.calls = []

        def run(m, script, timeout):
            self.calls.append((m.name, timeout))
            o = self.out[m.name]
            if isinstance(o, Exception):
                raise o
            return o
        self.mon = b.BatteryMonitor([Machine("Weela"), Machine("Win11Builder"), Machine("marge", "linux")], run,
                                    clock=lambda: self.now)

    def weela(self, pct, rate):
        return json.dumps({"now": self.now, "local": "2026-11-17T23:50:00",
                           "sps": base64.b64encode(sps(ac=1, pct=pct)).decode(),
                           "sbs": base64.b64encode(sbs(discharging=1, rem=pct * 750, rate=rate)).decode()})

    def test_poll_view_history(self):
        self.out["Weela"] = self.weela(60, -9000)
        for i in range(10):
            self.mon.poll(Machine("Weela"))
            self.now += b.POLL_S
        self.now -= b.POLL_S
        v = self.mon.view("Weela")
        self.assertEqual(v["status"], "ok")
        self.assertEqual(v["battery"]["level"], "warn")  # draining on mains
        self.assertIn("On mains but discharging (-9.0 W)", v["battery"]["warning"])
        self.assertEqual(len(v["history"]), b.PAGE_POINTS)
        self.assertEqual([x for x in v["history"] if x is not None][-1], 60.0)
        self.assertNotIn("quest", v)
        self.assertEqual(self.calls[0], ("Weela", b.TIMEOUT_S))

    def test_unreachable_keeps_last_value_with_age(self):
        self.out["Weela"] = self.weela(30, -9000)
        self.mon.poll(Machine("Weela"))
        self.now += 600
        self.out["Weela"] = RuntimeError("timeout after 25 s")
        self.mon.poll(Machine("Weela"))
        v = self.mon.view("Weela")
        self.assertEqual((v["status"], v["error"]), ("unreachable", "timeout after 25 s"))
        self.assertEqual(v["age"], 600)
        self.assertEqual(v["battery"]["percent"], 30.0)
        line = b.text_line(v, self.now)
        self.assertIn("UNREACHABLE, last value 10 min ago", line)
        # reachable again
        self.out["Weela"] = self.weela(31, 5000)
        self.mon.poll(Machine("Weela"))
        self.assertEqual(self.mon.view("Weela")["status"], "ok")

    def test_stale_without_fresh_probe(self):
        self.out["Weela"] = self.weela(50, -1000)
        self.mon.poll(Machine("Weela"))
        self.now += b.STALE_S + 1
        self.assertEqual(self.mon.view("Weela")["status"], "stale")

    def test_no_battery_shows_nothing_and_polls_rarely(self):
        self.out["Win11Builder"] = json.dumps({"now": 1, "sps": base64.b64encode(sps(flag=128, pct=255)).decode()})
        self.mon.poll(Machine("Win11Builder"))
        self.assertIsNone(self.mon.view("Win11Builder"))
        self.assertEqual(self.mon.wait_s("Win11Builder"), b.NO_BATTERY_POLL_S)
        self.out["marge"] = RuntimeError("no route to host")
        self.mon.poll(Machine("marge", "linux"))
        self.assertIsNone(self.mon.view("marge"))  # never seen a battery: nothing
        self.assertEqual(self.mon.wait_s("marge"), b.POLL_S)
        self.assertIsNone(self.mon.view("unknown machine"))

    def test_parser_error_is_a_failed_probe(self):
        self.out["Weela"] = "not json at all {"
        self.mon.poll(Machine("Weela"))  # must not raise
        self.assertIsNotNone(self.mon.st["Weela"].error)

    def test_quest_history_and_view(self):
        data = json.loads(self.weela(70, 20000))
        data.update(now=UTC_NOW, quest=quest())
        self.now = UTC_NOW
        self.out["Weela"] = json.dumps(data)
        self.mon.poll(Machine("Weela"))
        v = self.mon.view("Weela")
        self.assertEqual(v["quest"]["text"], "97 % charging · Link streaming (USB) · Touch L 90 % R 85 %")
        self.assertEqual(v["quest"]["level"], "ok")
        self.assertEqual([x for x in v["quest"]["history"] if x is not None], [97.0])
        self.mon.poll(Machine("Weela"))  # the same report again: no second sample
        self.assertEqual(len(self.mon.st["Weela"].qhist), 1)

    def test_add_battery_to_states(self):
        self.out["Weela"] = self.weela(45, 3000)
        self.mon.poll(Machine("Weela"))
        states = [{"name": "Weela", "os": "windows", "online": False}, {"name": "marge", "os": "linux", "online": True}]
        d.add_battery(states, self.mon, self.now)
        self.assertEqual(states[0]["battery"]["battery"]["state"], "charging")
        self.assertNotIn("battery", states[1])
        self.assertIs(d.add_battery(states, None), states)
        txt = d.format_text(dict(d._summarize(states), alerts=[]))
        self.assertIn("battery 45 % · charging", txt)


if __name__ == "__main__":
    unittest.main()
